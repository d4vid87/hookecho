//! MeteoAlarm — the European severe-weather warnings EUMETNET's members publish in common.
//!
//! Every national met service in Europe issues its own warnings in its own words; MeteoAlarm is
//! the one place they arrive in a single shape (CAP 1.2, wrapped in JSON) with a single severity
//! scale. That scale is the "awareness level": 1 green (no warning in force), 2 yellow, 3 orange,
//! 4 red. This module turns the levels that matter into the same [`GeoFeature`] the NWS alerts
//! produce, so the panel, banner, rules, watch radius, rollup and webhooks downstream need no
//! idea a warning came from Europe.
//!
//! # What this covers, and what it does not
//!
//! A CAP area carries geometry either as an inline `polygon` or as a `geocode` naming a region in
//! some registry. A census of all 31 country feeds on 2026-08-29 found 481 areas with a polygon
//! and 20545 with a geocode only — and the geocodes are in six different registries (EMMA_ID,
//! NUTS2, NUTS3, WARNCELLID, CISORP, FIPS). Resolving them needs a boundary table per registry.
//! MeteoAlarm publishes one, at `api.meteoalarm.org/metadata/v1/regions`, and it answers 401: the
//! metadata API is key-gated, and no key ships in this app. Eurostat's GISCO covers the NUTS
//! codes, but not EMMA_ID, which is most of them — and France's NUTS3 codes match neither the
//! 2024 nor the 2016 vintage, so even that half needs vintage archaeology.
//!
//! ponytail: so this ships the warnings that carry their own geometry, and skips the rest rather
//! than drawing them in the wrong place or guessing a shape. [`COUNTRIES`] is therefore the list
//! of feeds observed to publish polygons, not the list of countries MeteoAlarm serves — Germany,
//! Poland, Austria, Spain and Italy are all geocode-only today and are deliberately not fetched,
//! because their feeds are megabytes that would yield nothing. The upgrade path is a boundary
//! table keyed by EMMA_ID; the day one is publicly available, this list becomes every country.

use crate::overlay::{AlertInfo, FeatureKind, GeoFeature};

const FEED: &str = "https://feeds.meteoalarm.org/api/v1/warnings/feeds-";

/// The feeds that publish inline polygons, with the bounding box used to decide whether the
/// current view needs one. Fetching is bbox-gated because these payloads are not small — the
/// Swiss feed alone is about 12 MB, since a warning there carries a boundary at full survey
/// resolution — and a pan across Europe must not pull all of them.
///
/// Boxes are deliberately generous (whole-country, coasts included); a false positive costs one
/// fetch that parses to nothing, a false negative silently hides real warnings.
const COUNTRIES: &[(&str, f64, f64, f64, f64)] = &[
    ("estonia", 21.5, 57.5, 28.3, 59.8),
    ("france", -5.3, 41.2, 9.7, 51.2),
    ("latvia", 20.9, 55.6, 28.3, 58.1),
    ("lithuania", 20.9, 53.8, 26.9, 56.5),
    ("luxembourg", 5.7, 49.4, 6.6, 50.2),
    ("netherlands", 3.3, 50.7, 7.3, 53.6),
    ("norway", 4.0, 57.9, 31.2, 71.2),
    ("slovenia", 13.3, 45.4, 16.7, 46.9),
    ("sweden", 10.9, 55.3, 24.2, 69.1),
    ("switzerland", 5.9, 45.8, 10.5, 47.9),
    ("united-kingdom", -8.7, 49.8, 1.8, 61.0),
];

/// A hostile or broken feed must not be able to spend the whole frame budget. Both caps are far
/// above anything observed (the busiest feed carried 589 warnings, the largest polygon ~1200
/// vertices) and exist only to bound the work.
const MAX_WARNINGS: usize = 2000;
const MAX_VERTICES: usize = 20_000;

/// Which country feeds a view needs. Public so the caller can skip the whole overlay when the
/// view is not in Europe at all.
pub fn countries_in_view(bounds: (f64, f64, f64, f64)) -> Vec<&'static str> {
    let (x0, y0, x1, y1) = bounds;
    COUNTRIES
        .iter()
        .filter(|(_, a0, b0, a1, b1)| *a0 <= x1 && *a1 >= x0 && *b0 <= y1 && *b1 >= y0)
        .map(|(slug, ..)| *slug)
        .collect()
}

/// Every MeteoAlarm warning with a polygon, for the countries the view touches.
///
/// One failing country does not fail the overlay: a met service can take its feed down without
/// taking the rest of Europe's warnings off the map.
pub async fn fetch_in_view(
    client: &reqwest::Client,
    bounds: (f64, f64, f64, f64),
) -> anyhow::Result<Vec<GeoFeature>> {
    // With a key, every European country is reachable and carries real geometry; without one,
    // only the eleven feeds that publish inline polygons are. See the EDR section at the bottom
    // of this file, and the module header for why the keyless path is as narrow as it is.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(key) = api_key().filter(|_| edr_ready()) {
        match fetch_edr(client, bounds, &key).await {
            Ok(f) => return Ok(f),
            // A key that has expired or a service having a bad day must not take European
            // warnings off the map entirely when the open feeds would still have answered.
            Err(e) => log::warn!("meteoalarm EDR failed, falling back to the open feeds: {e}"),
        }
    }
    // Up to eleven countries can be in view, and they were fetched one after another — eleven
    // round trips deep instead of one wide, on a 120 s refresh.
    let bodies =
        futures_util::future::join_all(countries_in_view(bounds).into_iter().map(|slug| {
            let url = crate::net::fetch_url(&format!("{FEED}{slug}"));
            async move {
                match client.get(&url).send().await {
                    Ok(resp) => match resp.text().await {
                        Ok(body) => Some(body),
                        Err(e) => {
                            log::warn!("meteoalarm {slug}: body read failed ({e})");
                            None
                        }
                    },
                    Err(e) => {
                        log::warn!("meteoalarm {slug}: fetch failed ({e})");
                        None
                    }
                }
            }
        }))
        .await;
    let mut out = Vec::new();
    for body in bodies.into_iter().flatten() {
        out.extend(parse(&body));
    }
    Ok(out)
}

/// The awareness level (1..=4) of an info block, from its `awareness_level` parameter.
///
/// The value reads `"3; orange; Severe"`; only the leading digit is relied on, because the colour
/// word is localised in some feeds and the CAP `severity` field does not always agree with it.
fn awareness_level(info: &serde_json::Value) -> Option<u8> {
    let params = info.get("parameter")?.as_array()?;
    let raw = params.iter().find_map(|p| {
        (p.get("valueName")?.as_str()? == "awareness_level")
            .then(|| p.get("value")?.as_str())
            .flatten()
    })?;
    raw.split(';').next()?.trim().parse().ok()
}

/// The hazard from the `awareness_type` parameter (`"13; rain-flood"` -> `"rain-flood"`), title
/// cased for display. `None` when the feed omits it.
fn awareness_type(info: &serde_json::Value) -> Option<String> {
    let params = info.get("parameter")?.as_array()?;
    let raw = params.iter().find_map(|p| {
        (p.get("valueName")?.as_str()? == "awareness_type")
            .then(|| p.get("value")?.as_str())
            .flatten()
    })?;
    let name = raw.split(';').nth(1)?.trim().replace('-', " ");
    if name.is_empty() {
        return None;
    }
    let mut c = name.chars();
    Some(c.next()?.to_uppercase().collect::<String>() + c.as_str())
}

/// Colour, feature kind and escalation tag for an awareness level.
///
/// The colours are MeteoAlarm's own, so a European warning reads on the map the way it reads on
/// the issuing service's site rather than in NWS red. Orange and red map onto the escalation
/// tiers the rest of the app already sorts and sounds on: tier 3 (tornado emergency / PDS) stays
/// US-only, because Europe has no equivalent product to map onto it.
fn level_style(level: u8) -> ([u8; 3], FeatureKind, Option<&'static str>) {
    match level {
        4 => ([220, 0, 0], FeatureKind::Warning, Some("DESTRUCTIVE")),
        3 => ([255, 136, 0], FeatureKind::Warning, Some("CONSIDERABLE")),
        _ => ([245, 205, 0], FeatureKind::Advisory, None),
    }
}

/// The English info block of an alert, falling back to the first one.
///
/// Every alert repeats itself once per language. English is not guaranteed to be present — a few
/// services publish only their own — so the fallback keeps the warning on the map with a headline
/// the reader may not be able to read, which beats no warning at all.
fn english(infos: &[serde_json::Value]) -> Option<&serde_json::Value> {
    infos
        .iter()
        .find(|i| {
            i.get("language")
                .and_then(|l| l.as_str())
                .is_some_and(|l| l.starts_with("en"))
        })
        .or_else(|| infos.first())
}

/// One CAP `polygon` string — `"lat,lon lat,lon ..."` — as a ring of `[lon, lat]`.
///
/// CAP writes coordinates latitude first; every ring in this app is `[lon, lat]`, and getting the
/// order wrong puts Norway in the Indian Ocean rather than producing an error.
fn ring(poly: &str) -> Vec<[f64; 2]> {
    poly.split_whitespace()
        .take(MAX_VERTICES)
        .filter_map(|pair| {
            let (lat, lon) = pair.split_once(',')?;
            Some([lon.trim().parse().ok()?, lat.trim().parse().ok()?])
        })
        .collect()
}

/// Parse one country feed into features. Anything malformed is skipped, never panicked on: this
/// is a document from eleven different national services and it is not this app's job to be
/// right about all of them.
pub fn parse(body: &str) -> Vec<GeoFeature> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(warnings) = doc.get("warnings").and_then(|w| w.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for w in warnings.iter().take(MAX_WARNINGS) {
        let Some(alert) = w.get("alert") else {
            continue;
        };
        let Some(infos) = alert.get("info").and_then(|i| i.as_array()) else {
            continue;
        };
        let Some(info) = english(infos) else { continue };
        // Level 1 is "green": the all-clear a service publishes to say nothing is in force. It is
        // the bulk of most feeds and there is nothing to draw.
        let level = awareness_level(info).unwrap_or(1);
        if level < 2 {
            continue;
        }
        let Some(areas) = info.get("area").and_then(|a| a.as_array()) else {
            continue;
        };
        let (rgb, kind, threat) = level_style(level);
        let colour = match level {
            4 => "Red",
            3 => "Orange",
            _ => "Yellow",
        };
        let str_of = |k: &str| {
            info.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        // The synthesized name has to contain "warning" whatever the level, because that word is
        // what `severity_rank` and the user's `RuleTrigger::Warning { event_contains }` match on.
        let hazard = awareness_type(info).unwrap_or_else(|| "Weather".into());
        let event = format!("{colour} {hazard} Warning");
        let expires = info
            .get("expires")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        let id = alert
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let sender = str_of("senderName");
        for area in areas {
            let Some(polys) = area.get("polygon").and_then(|p| p.as_array()) else {
                continue;
            };
            let desc = area
                .get("areaDesc")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            for poly in polys {
                let Some(rings) = poly.as_str().map(ring) else {
                    continue;
                };
                // A ring needs three distinct corners to enclose anything; anything less is a
                // feed artefact that would render as an invisible sliver.
                if rings.len() < 3 {
                    continue;
                }
                let detail = format!(
                    "{}\n\n{event}\n\nIssued by: {sender}\nExpires: {}\nArea: {desc}\n\n{}\n\n{}",
                    str_of("headline"),
                    str_of("expires"),
                    str_of("description"),
                    str_of("instruction"),
                );
                out.push(GeoFeature {
                    rings: vec![rings],
                    fill: [rgb[0], rgb[1], rgb[2], 60],
                    stroke: [rgb[0], rgb[1], rgb[2], 255],
                    kind,
                    title: event.clone(),
                    detail,
                    alert: Some(AlertInfo {
                        id: id.clone(),
                        event: event.clone(),
                        headline: str_of("headline"),
                        area: desc.to_string(),
                        description: str_of("description"),
                        instruction: str_of("instruction"),
                        expires,
                        max_hail_in: None,
                        max_wind: None,
                        tornado_detection: None,
                        damage_threat: threat.map(str::to_string),
                        source: Some(sender.clone()),
                        motion: None,
                        // Europe issues no VTEC, so `dedupe_key` falls back to the CAP identifier.
                        // That is a message id, and MeteoAlarm mints a fresh one per update, so an
                        // extended warning announces again — the same behaviour every non-VTEC US
                        // product already has.
                        vtec: None,
                    }),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// EDR: every European warning, not just the ones that carry their own polygon.
// ---------------------------------------------------------------------------------------------

/// MeteoAlarm's OGC EDR API, which answers the question the module header above says is
/// unanswerable — and it is answerable now because MeteoAlarm resolves the geocodes itself.
///
/// The shape is a two-step. `locations/{country}` returns one GeoJSON feature per warning *area*,
/// carrying only that area's bounding box inline plus the `alertId` it belongs to; the alert's
/// full CAP document, polygons and all, is a separate link. So a view is served by listing the
/// countries it touches, throwing away every area whose bbox misses the screen, and then fetching
/// the handful of CAP documents that are left. Those CAP links are pre-signed and public — the
/// key is needed for the index, not for the payload.
///
/// This is gated on a key being present, and the key comes from the environment. Without one the
/// [`FEED`] path above still runs, so the app is unchanged for everyone who has not got one.
#[cfg(not(target_arch = "wasm32"))]
const EDR: &str = "https://api.meteoalarm.org/edr/v1/collections/warnings/locations";

/// The country bounding boxes, which the metadata API serves so this crate does not have to
/// hand-maintain 33 of them.
#[cfg(not(target_arch = "wasm32"))]
const REGIONS: &str = "https://api.meteoalarm.org/metadata/v1/regions";

/// One busy country can publish thousands of warnings in a day; Germany warns per WARNCELLID and
/// runs to sixty-three pages. The "active right now" window is the reason this is tractable at
/// all — it cuts Germany to about 136 — and these caps are the backstop for the day it is not.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PAGES: usize = 4;
#[cfg(not(target_arch = "wasm32"))]
const MAX_ALERTS: usize = 60;

/// Countries queried per refresh. A view zoomed out to the whole continent touches every one of
/// them, and firing thirty-odd requests at once earns a 429 — which is not a hypothetical: the
/// first version of this did exactly that and the API locked the key out for over ten minutes.
#[cfg(not(target_arch = "wasm32"))]
const MAX_COUNTRIES: usize = 8;

/// Shortest gap between two EDR fetches, whatever the overlay's own cadence is.
///
/// The overlays refresh every 120 s. That is 720 refreshes a day and, at up to [`MAX_COUNTRIES`]
/// requests each, comfortably past a daily quota before lunch — the limit here is per day, not
/// per minute, so the usual "it is only a few requests" reasoning does not apply. European
/// warnings are issued hours ahead and updated in tens of minutes, so five is well inside the
/// resolution anyone can use.
#[cfg(not(target_arch = "wasm32"))]
const EDR_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The last EDR answer, reused until [`EDR_MIN_INTERVAL`] is up.
///
/// Keyed by the bounds it was fetched for: panning inside them is served from here, and panning
/// outside is a real question that has to be asked again.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::type_complexity)]
static LAST_EDR: std::sync::Mutex<
    Option<((f64, f64, f64, f64), std::time::Instant, Vec<GeoFeature>)>,
> = std::sync::Mutex::new(None);

/// Whether `bounds` is inside what the cached answer covers.
#[cfg(not(target_arch = "wasm32"))]
fn covers(cached: (f64, f64, f64, f64), bounds: (f64, f64, f64, f64)) -> bool {
    cached.0 <= bounds.0 && cached.1 <= bounds.1 && cached.2 >= bounds.2 && cached.3 >= bounds.3
}

/// How long to leave the API alone after a 429 that does not say when to come back.
///
/// Only a fallback. The server does say, and [`reset_after`] reads it — this is for the day it
/// stops sending the header.
#[cfg(not(target_arch = "wasm32"))]
const RATE_LIMIT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Longest cooldown honoured, so a nonsense `X-RateLimit-Reset` cannot turn the layer off for a
/// week. The quota is daily, so a day and a bit is the most that can be legitimate.
#[cfg(not(target_arch = "wasm32"))]
const MAX_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(25 * 60 * 60);

/// When the EDR path may be used again. Set on a 429; until then `fetch_in_view` falls back to
/// the open feeds, so Europe keeps whatever warnings it can still draw.
#[cfg(not(target_arch = "wasm32"))]
static COOLDOWN_UNTIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Whether the EDR path is allowed right now.
#[cfg(not(target_arch = "wasm32"))]
fn edr_ready() -> bool {
    match COOLDOWN_UNTIL.lock() {
        Ok(g) => g.is_none_or(|t| std::time::Instant::now() >= t),
        // A poisoned lock is not a reason to stop drawing warnings.
        Err(_) => true,
    }
}

/// How long a 429 response asks us to wait, from its `X-RateLimit-Reset` (epoch seconds).
///
/// Split out from [`back_off`] so the arithmetic can be tested without a live 429 — which is not
/// a hypothetical convenience, since earning one costs a day.
#[cfg(not(target_arch = "wasm32"))]
fn reset_after(header: Option<&str>, now_epoch: i64) -> Option<std::time::Duration> {
    let reset: i64 = header?.trim().parse().ok()?;
    let secs = reset.checked_sub(now_epoch)?;
    // A reset in the past is a clock disagreement, not an instruction to hammer the API.
    (secs > 0).then(|| std::time::Duration::from_secs(secs as u64).min(MAX_COOLDOWN))
}

/// Stand down until the server says the quota is back.
///
/// The limit is **daily** — the body reads "Daily rate limit exceeded. Please try again
/// tomorrow." — and the first version of this waited a flat fifteen minutes, which against a
/// daily quota means retrying about ninety-six times a day, every one of them spending a request
/// that is already gone and failing. `X-RateLimit-Reset` is sent for exactly this.
#[cfg(not(target_arch = "wasm32"))]
fn back_off(reset_header: Option<&str>) {
    let wait = reset_after(reset_header, chrono::Utc::now().timestamp())
        .unwrap_or(RATE_LIMIT_COOLDOWN);
    if let Ok(mut g) = COOLDOWN_UNTIL.lock() {
        // Never shorten a cooldown already in force: several requests fly at once, and the last
        // one to land must not talk the others back into trying.
        let until = std::time::Instant::now() + wait;
        if g.is_none_or(|t| until > t) {
            *g = Some(until);
        }
    }
    log::warn!(
        "meteoalarm: rate limited, using the open feeds for {} minutes",
        wait.as_secs() / 60
    );
}

/// One response header as a string, when it is present and is text.
#[cfg(not(target_arch = "wasm32"))]
fn header_str<'a>(resp: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    resp.headers().get(name)?.to_str().ok()
}

/// The API key, from the environment. Never from a file in this repo and never compiled in: it is
/// a personal credential, and a build that carries one publishes it to everyone who downloads it.
#[cfg(not(target_arch = "wasm32"))]
pub fn api_key() -> Option<String> {
    std::env::var("HOOKECHO_METEOALARM_KEY")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Country codes whose bounding box overlaps `bounds`, from the metadata API.
///
/// `ALL` is in that list as a box around the whole continent and is deliberately skipped: it is
/// the aggregate feed, and querying it as well as its members would fetch everything twice.
#[cfg(not(target_arch = "wasm32"))]
async fn countries_from_metadata(
    client: &reqwest::Client,
    key: &str,
    bounds: (f64, f64, f64, f64),
) -> anyhow::Result<Vec<String>> {
    // Country outlines do not move, and the overlay refreshes every 120 s. Fetched once per run
    // and kept: without this it was a request per refresh on a rate-limited API, spent to learn
    // where Denmark is.
    static REGIONS_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
        std::sync::OnceLock::new();
    let cache = REGIONS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Some(body) = cache.lock().ok().and_then(|c| c.clone()) {
        return Ok(regions_in_view(&body, bounds));
    }
    let resp = client
        .get(REGIONS)
        .timeout(crate::net::FEED_TIMEOUT)
        .header("Authorization", format!("Bearer {key}"))
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        back_off(header_str(&resp, "x-ratelimit-reset"));
    }
    let body = resp.error_for_status()?.text().await?;
    if let Ok(mut c) = cache.lock() {
        *c = Some(body.clone());
    }
    Ok(regions_in_view(&body, bounds))
}

/// The overlap test, split out from the fetch so it can be tested against a recorded response.
#[cfg(not(target_arch = "wasm32"))]
pub fn regions_in_view(body: &str, bounds: (f64, f64, f64, f64)) -> Vec<String> {
    let (x0, y0, x1, y1) = bounds;
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(regions) = doc.get("regions").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for r in regions {
        let Some(code) = r.get("code").and_then(|c| c.as_str()) else {
            continue;
        };
        if code == "ALL" || r.get("active").and_then(|a| a.as_bool()) == Some(false) {
            continue;
        }
        // `bb` is a closed ring of [lon, lat] corners, not a [minx, miny, maxx, maxy] tuple.
        let Some(ring) = r.get("bb").and_then(|b| b.as_array()) else {
            continue;
        };
        let pts: Vec<(f64, f64)> = ring
            .iter()
            .filter_map(|p| {
                let a = p.as_array()?;
                Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
            })
            .collect();
        if pts.is_empty() {
            continue;
        }
        let (mut a0, mut b0, mut a1, mut b1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in pts {
            a0 = a0.min(x);
            a1 = a1.max(x);
            b0 = b0.min(y);
            b1 = b1.max(y);
        }
        if a0 <= x1 && a1 >= x0 && b0 <= y1 && b1 >= y0 {
            out.push(code.to_string());
        }
    }
    out
}

/// The alert ids whose area bounding box is on screen, and the CAP link for each.
///
/// Deduplicated by alert: one warning explodes into an area per region it covers — a hundred
/// features for four alerts is an ordinary ratio — and the CAP document behind all of them is the
/// same file.
pub fn alerts_in_view(body: &str, bounds: (f64, f64, f64, f64)) -> Vec<(String, String)> {
    let (x0, y0, x1, y1) = bounds;
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(features) = doc.get("features").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for f in features.iter().take(MAX_WARNINGS) {
        let Some(id) = f
            .get("properties")
            .and_then(|p| p.get("alertId"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        if seen.contains(id) {
            continue;
        }
        let Some(coords) = f
            .get("geometry")
            .and_then(|g| g.get("coordinates"))
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|r| r.as_array())
        else {
            continue;
        };
        let (mut a0, mut b0, mut a1, mut b1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in coords {
            let Some(pt) = p.as_array() else { continue };
            let (Some(x), Some(y)) = (
                pt.first().and_then(|v| v.as_f64()),
                pt.get(1).and_then(|v| v.as_f64()),
            ) else {
                continue;
            };
            a0 = a0.min(x);
            a1 = a1.max(x);
            b0 = b0.min(y);
            b1 = b1.max(y);
        }
        if !(a0 <= x1 && a1 >= x0 && b0 <= y1 && b1 >= y0) {
            continue;
        }
        let Some(link) = f
            .get("links")
            .and_then(|l| l.as_array())
            .and_then(|ls| ls.iter().find(|l| l.get("rel").and_then(|r| r.as_str()) == Some("json")))
            .and_then(|l| l.get("href"))
            .and_then(|h| h.as_str())
        else {
            continue;
        };
        seen.insert(id.to_string());
        out.push((id.to_string(), link.to_string()));
    }
    out
}

/// How many pages a locations response says it has. One when it does not say.
fn total_pages(body: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|d| d.get("metadata")?.get("total_pages")?.as_u64())
        .unwrap_or(1) as usize
}

/// Wrap a bare CAP alert document in the envelope [`parse`] reads.
///
/// The Atom feeds hand out `{"warnings": [{"alert": …}]}`; the EDR API hands out the alert on its
/// own. One line here means the whole level, colour, language and polygon reading below is shared
/// by both paths instead of written twice.
pub fn cap_envelope(cap: &str) -> String {
    format!("{{\"warnings\":[{{\"alert\":{cap}}}]}}")
}

/// Every warning in view, through the EDR API.
///
/// One country failing does not fail the overlay, same as the feed path: a met service can have a
/// bad afternoon without taking the rest of Europe off the map.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch_edr(
    client: &reqwest::Client,
    bounds: (f64, f64, f64, f64),
    key: &str,
) -> anyhow::Result<Vec<GeoFeature>> {
    // Served from the last answer while it is fresh and still covers the screen. See
    // `EDR_MIN_INTERVAL`: the quota is daily, and the overlay cadence alone would exhaust it.
    if let Some(hit) = LAST_EDR.lock().ok().and_then(|c| {
        c.as_ref()
            .filter(|(b, at, _)| at.elapsed() < EDR_MIN_INTERVAL && covers(*b, bounds))
            .map(|(_, _, f)| f.clone())
    }) {
        return Ok(hit);
    }
    let mut countries = countries_from_metadata(client, key, bounds).await?;
    if countries.is_empty() {
        return Ok(Vec::new());
    }
    countries.truncate(MAX_COUNTRIES);
    // "What is in force right now", not "what was issued today". A warning in force started
    // before now, so the window has to reach backwards; a day-wide window instead returns
    // thousands of expired ones — 6272 for Germany alone against 136 for this.
    let now = chrono::Utc::now();
    let window = format!(
        "{}/{}",
        (now - chrono::Duration::hours(1)).format("%Y-%m-%dT%H:%M:%SZ"),
        (now + chrono::Duration::hours(1)).format("%Y-%m-%dT%H:%M:%SZ"),
    );

    // Page 1 of each country first, then only the extra pages the response says exist.
    //
    // Asking for `MAX_PAGES` of every country up front is what a `flat_map` wants to write, and
    // it earns an immediate 429: most countries have one page or none, so it is dozens of
    // requests to learn nothing. The first version did exactly that and the whole overlay came
    // back empty, because the rate-limit response was being swallowed by an `.ok()?`.
    let get = |url: String| {
        let key = key.to_string();
        async move {
            let resp = match client
                .get(&url)
                .timeout(crate::net::FEED_TIMEOUT)
                .header("Authorization", format!("Bearer {key}"))
                .header("User-Agent", crate::alerts::USER_AGENT)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("meteoalarm EDR {url}: {e}");
                    return None;
                }
            };
            // 204 is "nothing in force here", which is the common answer and not a failure.
            if resp.status() == reqwest::StatusCode::NO_CONTENT {
                return None;
            }
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                back_off(header_str(&resp, "x-ratelimit-reset"));
                return None;
            }
            if !resp.status().is_success() {
                // Said out loud on purpose: a 401 is an expired key and a 429 is asking for too
                // much, and both used to look exactly like a quiet continent.
                log::warn!("meteoalarm EDR {}: HTTP {}", url, resp.status());
                return None;
            }
            resp.text().await.ok()
        }
    };

    let firsts = futures_util::future::join_all(
        countries
            .iter()
            .map(|code| get(format!("{EDR}/{code}?datetime={window}&page=1"))),
    )
    .await;

    let mut bodies: Vec<String> = Vec::new();
    let mut more: Vec<String> = Vec::new();
    for (code, body) in countries.iter().zip(firsts) {
        let Some(body) = body else { continue };
        let total = total_pages(&body).min(MAX_PAGES);
        for p in 2..=total {
            more.push(format!("{EDR}/{code}?datetime={window}&page={p}"));
        }
        bodies.push(body);
    }
    bodies.extend(
        futures_util::future::join_all(more.into_iter().map(get))
            .await
            .into_iter()
            .flatten(),
    );

    let mut links: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for body in &bodies {
        for (id, link) in alerts_in_view(body, bounds) {
            if seen.insert(id.clone()) {
                links.push((id, link));
            }
        }
    }
    links.truncate(MAX_ALERTS);

    // The CAP links are pre-signed and public, so these carry no key — and must not, since they
    // point at object storage rather than at the API.
    let caps = futures_util::future::join_all(links.into_iter().map(|(id, url)| async move {
        match client.get(&url).timeout(crate::net::FEED_TIMEOUT).send().await {
            Ok(r) => r.error_for_status().ok()?.text().await.ok(),
            Err(e) => {
                log::warn!("meteoalarm alert {id}: fetch failed ({e})");
                None
            }
        }
    }))
    .await;

    let mut out = Vec::new();
    for cap in caps.into_iter().flatten() {
        out.extend(parse(&cap_envelope(&cap)));
    }
    if let Ok(mut c) = LAST_EDR.lock() {
        *c = Some((bounds, std::time::Instant::now(), out.clone()));
    }
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A red thunderstorm warning in Switzerland, cut down from a live feed: two languages, a
    /// green all-clear alongside it, and one area that carries a geocode instead of a polygon.
    const FEED_JSON: &str = r#"{"warnings":[
      {"alert":{"identifier":"2.49.0.0.756.0.CH.1","sent":"2026-08-29T12:00:00+00:00","info":[
        {"language":"de-CH","event":"Gewitter","area":[]},
        {"language":"en-GB","event":"Severe thunderstorm warning","senderName":"MeteoSwiss",
         "headline":"Severe thunderstorms expected","description":"Large hail.",
         "instruction":"Stay indoors.","expires":"2026-08-29T18:00:00+00:00",
         "parameter":[{"valueName":"awareness_level","value":"4; red; Extreme"},
                      {"valueName":"awareness_type","value":"11; thunderstorm"}],
         "area":[{"areaDesc":"Verzasca","polygon":["46.5,8.8 46.5,8.9 46.6,8.9 46.5,8.8"]},
                 {"areaDesc":"Coded only","geocode":[{"valueName":"EMMA_ID","value":"CH001"}]}]}]}},
      {"alert":{"identifier":"2.49.0.0.756.0.CH.2","info":[
        {"language":"en-GB","event":"No warning","parameter":[{"valueName":"awareness_level","value":"1; green; Minor"}],
         "area":[{"areaDesc":"Nowhere","polygon":["46.0,8.0 46.0,8.1 46.1,8.1 46.0,8.0"]}]}]}}
    ]}"#;

    #[test]
    fn parses_a_red_warning_and_drops_the_green_one() {
        let f = parse(FEED_JSON);
        // One feature: the green all-clear and the geocode-only area both produce nothing.
        assert_eq!(
            f.len(),
            1,
            "{:?}",
            f.iter().map(|x| &x.title).collect::<Vec<_>>()
        );
        let a = f[0].alert.as_ref().expect("a warning carries AlertInfo");
        assert_eq!(a.event, "Red Thunderstorm Warning");
        // The word the rest of the app matches on has to survive the synthesis.
        assert!(a.event.to_ascii_lowercase().contains("warning"));
        assert_eq!(a.damage_threat.as_deref(), Some("DESTRUCTIVE"));
        assert_eq!(
            crate::alerts::escalation(a),
            2,
            "red is the second escalation tier"
        );
        assert_eq!(a.area, "Verzasca");
        assert_eq!(a.id, "2.49.0.0.756.0.CH.1");
        // No VTEC in Europe, so the dedupe key falls back to the CAP identifier.
        assert!(a.vtec.is_none());
        assert_eq!(a.dedupe_key(), a.id);
        assert_eq!(
            a.expires.map(|t| t.to_rfc3339()),
            Some("2026-08-29T18:00:00+00:00".into())
        );
        // The English block is preferred over the German one that comes first.
        assert_eq!(a.headline, "Severe thunderstorms expected");
    }

    /// CAP writes `lat,lon`; every ring in this app is `[lon, lat]`. Swapping them is silent —
    /// the polygon renders, just in the wrong hemisphere.
    #[test]
    fn cap_coordinates_are_latitude_first() {
        let r = ring("46.5,8.8 46.4,8.9");
        assert_eq!(r, vec![[8.8, 46.5], [8.9, 46.4]]);
        let f = parse(FEED_JSON);
        let (x0, y0, ..) = f[0].bbox().expect("a parsed ring has a bbox");
        assert!(
            (5.0..11.0).contains(&x0) && (45.0..48.0).contains(&y0),
            "{x0},{y0}"
        );
    }

    #[test]
    fn awareness_levels_map_onto_the_escalation_tiers() {
        assert_eq!(level_style(4).2, Some("DESTRUCTIVE"));
        assert_eq!(level_style(3).2, Some("CONSIDERABLE"));
        assert_eq!(level_style(2).2, None);
        // Yellow is an advisory; orange and red draw as warnings.
        assert_eq!(level_style(2).1, FeatureKind::Advisory);
        assert_eq!(level_style(3).1, FeatureKind::Warning);
    }

    /// The bbox gate is what keeps a pan across the Atlantic from pulling tens of megabytes of
    /// European CAP, and what keeps a view over Kansas from fetching anything at all.
    #[test]
    fn only_the_countries_under_the_view_are_fetched() {
        // Generous boxes overlap at a border, and that is the safe direction: a view near Geneva
        // asks France as well as Switzerland rather than missing a warning on the other bank.
        assert_eq!(
            countries_in_view((7.0, 46.0, 8.0, 47.0)),
            vec!["france", "switzerland"]
        );
        assert_eq!(countries_in_view((8.0, 60.0, 9.0, 61.0)), vec!["norway"]);
        assert!(
            countries_in_view((-98.0, 35.0, -97.0, 36.0)).is_empty(),
            "Oklahoma is not Europe"
        );
        // A view spanning the Channel needs both sides of it.
        let c = countries_in_view((-2.0, 49.0, 3.0, 52.0));
        assert!(
            c.contains(&"france") && c.contains(&"united-kingdom"),
            "{c:?}"
        );
    }

    /// Garbage in must return nothing, not panic: this parses documents from eleven national
    /// services with no schema enforcement between them and the map.
    #[test]
    fn malformed_feeds_are_skipped_not_fatal() {
        for bad in [
            "",
            "not json",
            "{}",
            r#"{"warnings":"nope"}"#,
            r#"{"warnings":[{"alert":{}}]}"#,
            r#"{"warnings":[{"alert":{"info":[{"language":"en","parameter":[{"valueName":"awareness_level","value":"4"}],"area":[{"polygon":["oops"]}]}]}}]}"#,
        ] {
            assert!(parse(bad).is_empty(), "{bad}");
        }
    }
}

#[cfg(test)]
mod edr_tests {
    use super::*;

    #[test]
    fn a_bare_cap_alert_reads_the_same_as_a_feed_one() {
        // The EDR API hands out the alert on its own; the Atom feeds wrap it. One envelope, and
        // every level, colour, language and polygon rule below is shared rather than duplicated.
        let cap = r#"{"identifier":"x-1","info":[{"language":"en-GB","senderName":"Met Test",
            "headline":"Strong wind","description":"d","instruction":"i",
            "expires":"2026-09-04T00:00:00Z",
            "parameter":[{"valueName":"awareness_level","value":"3; orange; Severe"},
                         {"valueName":"awareness_type","value":"1; Wind"}],
            "area":[{"areaDesc":"Somewhere","polygon":["55.0,10.0 55.0,11.0 56.0,11.0 55.0,10.0"]}]}]}"#;
        let f = parse(&cap_envelope(cap));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].title, "Orange Wind Warning");
        // CAP writes latitude first and every ring here is [lon, lat]; getting that backwards puts
        // Denmark in Somalia rather than failing.
        assert_eq!(f[0].rings[0][0], [10.0, 55.0]);
    }

    #[test]
    fn only_the_alerts_on_screen_are_fetched_and_each_only_once() {
        // Two areas of one alert plus one area of another, all with a bbox geometry — the shape
        // the locations query really returns. The off-screen one must not cost a CAP fetch, and
        // the repeated alert must not cost two.
        let body = r#"{"type":"FeatureCollection","features":[
          {"properties":{"alertId":"a"},"geometry":{"type":"Polygon","coordinates":[[[10.0,55.0],[11.0,55.0],[11.0,56.0],[10.0,55.0]]]},
           "links":[{"rel":"json","href":"https://example.invalid/a.json"}]},
          {"properties":{"alertId":"a"},"geometry":{"type":"Polygon","coordinates":[[[10.5,55.5],[11.5,55.5],[11.5,56.5],[10.5,55.5]]]},
           "links":[{"rel":"json","href":"https://example.invalid/a.json"}]},
          {"properties":{"alertId":"b"},"geometry":{"type":"Polygon","coordinates":[[[-8.0,40.0],[-7.0,40.0],[-7.0,41.0],[-8.0,40.0]]]},
           "links":[{"rel":"json","href":"https://example.invalid/b.json"}]}]}"#;
        let got = alerts_in_view(body, (9.0, 54.0, 12.0, 57.0));
        assert_eq!(got.len(), 1, "one alert, not two areas and not Portugal");
        assert_eq!(got[0].0, "a");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_cached_answer_is_reused_only_where_it_actually_answers() {
        // Panning inside the fetched box is served from the last answer; panning outside it is a
        // question that was never asked, and must not be answered from a cache that has no
        // warnings for the new ground.
        let cached = (0.0, 45.0, 20.0, 55.0);
        assert!(covers(cached, (5.0, 47.0, 15.0, 53.0)), "well inside");
        assert!(covers(cached, cached), "the same view");
        assert!(!covers(cached, (-5.0, 47.0, 15.0, 53.0)), "panned west");
        assert!(!covers(cached, (5.0, 47.0, 15.0, 60.0)), "zoomed out north");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_rate_limit_is_honoured_for_as_long_as_the_server_says() {
        // The real 429, headers and body, recorded 2026-09-03:
        //   X-RateLimit-Reset: 1788563205
        //   {"error":"Daily rate limit exceeded. Please try again tomorrow."}
        // 1788476805 is that response's own `date` header as epoch, so this is the exact 24-hour
        // wait the server asked for. A flat fifteen-minute cooldown against a *daily* quota means
        // retrying ninety-six times on a budget that is already spent.
        let wait = reset_after(Some("1788563205"), 1_788_476_805).unwrap();
        assert_eq!(wait.as_secs(), 86_400);

        // No header: the fallback, not "retry immediately".
        assert_eq!(reset_after(None, 1_788_476_805), None);
        // Garbage, and a reset already in the past (a clock disagreement), are both "no answer"
        // rather than an instruction to hammer the API.
        assert_eq!(reset_after(Some("tomorrow"), 1_788_476_805), None);
        assert_eq!(reset_after(Some("1788476800"), 1_788_476_805), None);
        // A nonsense far-future value cannot switch the layer off for a week.
        assert_eq!(
            reset_after(Some("9999999999"), 1_788_476_805).unwrap(),
            MAX_COOLDOWN
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_aggregate_region_is_skipped_but_its_members_are_not() {
        // `ALL` is a box around the whole continent. Querying it as well as its members would
        // fetch every warning in Europe twice.
        let body = r#"{"regions":[
          {"active":true,"code":"ALL","name":"All of Europe","bb":[[-25.0,35.0],[45.0,35.0],[45.0,75.0],[-25.0,75.0],[-25.0,35.0]]},
          {"active":true,"code":"DK","name":"Denmark","bb":[[8.0,54.5],[8.0,57.8],[15.2,57.8],[15.2,54.5],[8.0,54.5]]},
          {"active":true,"code":"MT","name":"Malta","bb":[[14.1,35.8],[14.1,36.1],[14.6,36.1],[14.6,35.8],[14.1,35.8]]}]}"#;
        assert_eq!(regions_in_view(body, (9.0, 54.0, 12.0, 57.0)), ["DK"]);
        // And a view over Malta gets Malta, so the box test is not simply always false.
        assert_eq!(regions_in_view(body, (14.2, 35.9, 14.4, 36.0)), ["MT"]);
    }
    /// Live check against MeteoAlarm's EDR API. Needs `HOOKECHO_METEOALARM_KEY`:
    /// `HOOKECHO_METEOALARM_KEY=… cargo test -p wxdata live_edr -- --ignored --nocapture`
    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    #[ignore]
    async fn live_edr() {
        let Some(key) = api_key() else {
            eprintln!("no HOOKECHO_METEOALARM_KEY set");
            return;
        };
        let client = reqwest::Client::new();
        // Central Europe, which the open feeds cannot draw at all: Germany, Austria, Poland and
        // the Czech Republic are geocode-only and are not in `COUNTRIES`.
        let bounds = (6.0, 45.0, 20.0, 55.0);
        // Not `unwrap`: the quota is daily, so the ordinary way for this to fail is "you already
        // ran it today", and a backtrace is the wrong way to say that. The 429 body reads
        // "Daily rate limit exceeded. Please try again tomorrow." and `X-RateLimit-Reset` names
        // the hour.
        let f = match fetch_edr(&client, bounds, &key).await {
            Ok(f) => f,
            Err(e) => {
                eprintln!("EDR unavailable: {e}");
                eprintln!("if this is a 429, the quota is daily — check X-RateLimit-Reset:");
                eprintln!(
                    "  curl -sI -H \"Authorization: Bearer $HOOKECHO_METEOALARM_KEY\" \\\n    https://api.meteoalarm.org/metadata/v1/regions"
                );
                panic!("EDR request failed: {e}");
            }
        };
        eprintln!("EDR returned {} features", f.len());
        assert!(
            !f.is_empty(),
            "central Europe with nothing in force at all is possible but unusual — check the \
             warning before believing it"
        );
        for x in f.iter().take(5) {
            eprintln!("  {} — {} rings", x.title, x.rings.len());
        }
    }
}
