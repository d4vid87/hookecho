//! Running one rule against an archive day, so the user can see whether it would have fired.
//!
//! A rule is a guess until something tests it. "Rotation over 45 knots near home" sounds right
//! and fires nine times a season or never, and the only honest way to find out which is to run it
//! over a day that already happened.
//!
//! Sequential and capped: this downloads and decodes real volumes, and twenty-four of them is
//! already a couple of hundred megabytes and a minute of CPU. Parallelising it would finish
//! sooner and make the progress readout a lie about what is downloading.
//!
//! Runs in the browser too — it is plain async, and the archive list and download both have wasm
//! paths. The only difference there is the cap (see [`MAX_VOLUMES`]) and that decoding happens on
//! the main thread, so a run is a series of brief hitches rather than a background hum.
//!
//! ponytail: one rule at a time, and decoding on the main thread on the web. Both are one loop
//! away if anyone actually wants a whole outbreak day scored at once.

use crate::rules::Detection;
use crate::settings::{AlertRule, RuleTrigger as T, Settings};
use std::sync::{Arc, Mutex};
use wxdata::level2::{self, Moment};

/// Most volumes one backtest will pull. About two hours at a severe-weather VCP.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_VOLUMES: usize = 24;

/// Half that in the browser. Each volume is tens of megabytes and every one of them crosses a
/// shared proxy on its way in — and decoding happens on the main thread there, so a long run is
/// also a long series of visible hitches. An hour of weather is enough to answer "would this rule
/// have fired", which is the question a backtest is for.
#[cfg(target_arch = "wasm32")]
pub const MAX_VOLUMES: usize = 12;

/// Shared with the UI thread: what the run is doing and what it found.
#[derive(Default)]
pub struct Progress {
    /// Volumes examined so far, and how many there are.
    pub done: usize,
    pub total: usize,
    /// Volume times at which the rule would have fired.
    pub fired: Vec<chrono::DateTime<chrono::Utc>>,
    /// Set when the run ends, successfully or not.
    pub finished: Option<String>,
}

/// A run in flight.
pub type Shared = Arc<Mutex<Progress>>;

/// Score `rule` against up to [`MAX_VOLUMES`] volumes from `site` on `day`.
///
/// Only scan triggers can be replayed: ProbSevere, lightning density and warnings are feeds, not
/// something a decoded volume carries, and pretending otherwise would produce a confident empty
/// result. Compound conditions are not evaluated either — they ask "was there also, recently",
/// which needs the running history the live path keeps.
pub async fn run(
    site: String,
    day: chrono::NaiveDate,
    rule: AlertRule,
    settings: Settings,
    out: Shared,
) {
    let finish = |out: &Shared, msg: &str| {
        if let Ok(mut p) = out.lock() {
            p.finished = Some(msg.to_string());
        }
    };
    if !rule.trigger.is_scan() {
        finish(&out, "only scan triggers can be replayed from the archive");
        return;
    }
    let ids = match level2::list_volumes(&site, day).await {
        Ok(v) => v,
        Err(e) => return finish(&out, &format!("listing {site} for {day} failed: {e}")),
    };
    // The end of the day is where the weather usually is, and the cap has to fall somewhere.
    let ids: Vec<_> = ids.into_iter().rev().take(MAX_VOLUMES).rev().collect();
    if let Ok(mut p) = out.lock() {
        p.total = ids.len();
    }
    if ids.is_empty() {
        return finish(&out, "no volumes archived for that site and day");
    }
    let cache = crate::paths::cache_dir();
    for id in ids {
        let at = id.date_time();
        match level2::download_scan(id, cache.clone()).await {
            Ok(scan) => {
                if let Some(hit) = first_hit(&scan, &rule, &settings) {
                    let _ = hit;
                    if let (Ok(mut p), Some(at)) = (out.lock(), at) {
                        p.fired.push(at);
                    }
                }
            }
            // One bad volume is not a failed backtest; the archive has truncated objects in it.
            Err(e) => log::warn!("backtest: skipping a volume: {e}"),
        }
        if let Ok(mut p) = out.lock() {
            p.done += 1;
        }
    }
    let n = out.lock().map(|p| p.fired.len()).unwrap_or(0);
    finish(&out, &format!("done — would have fired {n} times"));
}

/// The first detection in this volume that the rule accepts, if any. Same detectors and the same
/// thresholds the live path uses, so a backtest verdict means what the app would have done.
fn first_hit(scan: &level2::Scan, rule: &AlertRule, settings: &Settings) -> Option<Detection> {
    let d = &settings.detectors;
    let hits: Vec<Detection> = match rule.trigger {
        T::Tds => {
            let z = level2::bin_scan(scan, Moment::Reflectivity, 0).ok()?;
            let cc = level2::bin_scan(scan, Moment::CorrelationCoefficient, 0).ok()?;
            wxdata::tds::detect(&z, &cc, 0.80, 40.0, 150.0, 4)
                .iter()
                .map(|h| Detection::at(h.lon, h.lat))
                .collect()
        }
        T::Tbss => {
            let z = level2::bin_scan(scan, Moment::Reflectivity, 0).ok()?;
            let cc = level2::bin_scan(scan, Moment::CorrelationCoefficient, 0).ok()?;
            wxdata::dualpol::tbss(&z, &cc, d.tbss_core_dbz, 20.0, 0.8, 4.0, 150.0)
                .iter()
                .map(|h| Detection::at(h.lon, h.lat))
                .collect()
        }
        T::ZdrColumn => {
            let n = level2::elevation_angles(scan).len();
            let take = |m: Moment| -> Vec<level2::BinnedSweep> {
                (0..n)
                    .filter_map(|t| level2::bin_scan(scan, m, t).ok())
                    .collect()
            };
            // No model freezing level in a backtest; 4 km ARL is the warm-season figure the
            // headless dual-pol pass uses too.
            wxdata::dualpol::zdr_columns(
                &take(Moment::DifferentialReflectivity),
                &take(Moment::Reflectivity),
                4.0,
                d.zdr_min_db,
                d.zdr_min_depth_km,
                40.0,
                100.0,
            )
            .iter()
            .map(|h| Detection::at(h.lon, h.lat))
            .collect()
        }
        _ => {
            let vel = level2::bin_scan_opts(scan, Moment::Velocity, 0, true).ok()?;
            wxdata::rotation::detect(&vel, 25.0, 15.0, 150.0, 3)
                .iter()
                .map(|h| Detection::with_strength(h.lon, h.lat, h.vrot_ms as f64 * 1.943_844))
                .collect()
        }
    };
    hits.into_iter()
        .find(|h| crate::rules::matches(rule, h, settings))
}
