//! "Rain reaching Home in about 12 minutes."
//!
//! The nowcast layer advects every echo forward and paints the result; this asks the narrower
//! question for a handful of points you care about, which is the one worth interrupting you for.
//!
//! The method is deliberately simple: walk *upstream* from the point along the reversed storm
//! motion and report where the first meaningful echo is. That is honest for the ordinary case of
//! a line or cluster translating across the map, and wrong for backbuilding or discretely
//! propagating storms — hence the "~", the 45-minute ceiling, and the two-scan persistence gate
//! before anything fires.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Echo at or above this counts as "rain arriving" — below it is drizzle and clutter.
pub const THRESH_DBZ: f32 = 25.0;
/// Don't promise anything further out than this; motion extrapolation degrades fast.
pub const MAX_MIN: f32 = 45.0;
/// Consecutive scans that must agree before firing, and again before re-arming.
const PERSIST: u8 = 2;
const COOLDOWN: Duration = Duration::from_secs(1800);

/// Minutes until echo reaches `point`, marching upstream along the reversed storm motion.
///
/// `sample` returns the reflectivity at a `(lon, lat)`, or `None` outside coverage. `mvt_deg` is
/// the direction storms are moving *toward*; `mvt_kt` their speed.
pub fn upstream_eta(
    sample: impl Fn(f64, f64) -> Option<f32>,
    point: [f64; 2],
    mvt_deg: f64,
    mvt_kt: f64,
    max_min: f32,
) -> Option<f32> {
    if mvt_kt <= 1.0 {
        return None;
    }
    let kmh = mvt_kt * 1.852;
    let reach_km = kmh * (max_min as f64 / 60.0);
    // Look back along the direction the weather came from.
    let upwind = (mvt_deg + 180.0).rem_euclid(360.0);
    let step_km = 1.0;
    let mut d = 0.0;
    while d <= reach_km {
        let at = crate::geo::destination_point(point, upwind, d);
        if sample(at[0], at[1]).is_some_and(|v| v >= THRESH_DBZ) {
            return Some((d / kmh * 60.0) as f32);
        }
        d += step_km;
    }
    None
}

/// Per-point state: how many consecutive scans have agreed, and when we last spoke up.
#[derive(Default)]
pub struct Detector {
    hits: HashMap<String, u8>,
    misses: HashMap<String, u8>,
    fired: HashMap<String, Instant>,
}

/// What [`Detector::update`] decided for one point this scan.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Nothing to say.
    Quiet,
    /// Rain is approaching and this is the first confirmed scan — alert now.
    Fire(f32),
}

impl Detector {
    /// Feed one scan's result for `name`. `eta` is `None` when no echo is upstream.
    pub fn update(&mut self, name: &str, eta: Option<f32>) -> Verdict {
        match eta {
            Some(min) => {
                self.misses.remove(name);
                let n = self.hits.entry(name.to_string()).or_insert(0);
                *n = n.saturating_add(1);
                // One scan is noise — a speck of clutter upstream shouldn't ring the alarm.
                if *n < PERSIST {
                    return Verdict::Quiet;
                }
                let cooling = self.fired.get(name).is_some_and(|t| t.elapsed() < COOLDOWN);
                if cooling {
                    return Verdict::Quiet;
                }
                self.fired.insert(name.to_string(), Instant::now());
                Verdict::Fire(min)
            }
            None => {
                self.hits.remove(name);
                // Re-arm only after the echo has been gone a couple of scans, so a gap in a
                // ragged line doesn't produce a second alert for the same rain.
                let n = self.misses.entry(name.to_string()).or_insert(0);
                *n = n.saturating_add(1);
                if *n >= PERSIST {
                    self.fired.remove(name);
                }
                Verdict::Quiet
            }
        }
    }

    /// Drop state for points that no longer exist (markers renamed or deleted).
    pub fn retain(&mut self, keep: &[String]) {
        self.hits.retain(|k, _| keep.contains(k));
        self.misses.retain(|k, _| keep.contains(k));
        self.fired.retain(|k, _| keep.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A band of echo `band_km` upstream of the origin, along the reversed motion.
    fn band(mvt_deg: f64, band_km: f64) -> impl Fn(f64, f64) -> Option<f32> {
        let upwind = (mvt_deg + 180.0).rem_euclid(360.0);
        let center = crate::geo::destination_point([0.0, 40.0], upwind, band_km);
        move |lon, lat| {
            let (d, _) = crate::geo::great_circle([lon, lat], center);
            Some(if d < 2.0 { 40.0 } else { 5.0 })
        }
    }

    #[test]
    fn eta_matches_distance_over_speed() {
        // 30 kt = 55.6 km/h; echo 27.8 km upstream is half an hour out.
        let eta = upstream_eta(band(90.0, 27.8), [0.0, 40.0], 90.0, 30.0, MAX_MIN).unwrap();
        assert!((eta - 30.0).abs() < 2.0, "expected ~30 min, got {eta}");
    }

    #[test]
    fn no_echo_upstream_is_none() {
        let eta = upstream_eta(|_, _| Some(5.0), [0.0, 40.0], 90.0, 30.0, MAX_MIN);
        assert_eq!(eta, None);
    }

    #[test]
    fn echo_beyond_the_horizon_is_none() {
        // 300 km upstream at 30 kt is ~5 hours — well past the cap.
        let eta = upstream_eta(band(90.0, 300.0), [0.0, 40.0], 90.0, 30.0, MAX_MIN);
        assert_eq!(eta, None);
    }

    #[test]
    fn stationary_storms_produce_no_eta() {
        let eta = upstream_eta(band(90.0, 10.0), [0.0, 40.0], 90.0, 0.5, MAX_MIN);
        assert_eq!(eta, None, "no motion means no arrival time");
    }

    #[test]
    fn one_scan_is_silent_two_scans_fire() {
        let mut d = Detector::default();
        assert_eq!(d.update("Home", Some(12.0)), Verdict::Quiet);
        assert_eq!(d.update("Home", Some(11.0)), Verdict::Fire(11.0));
    }

    #[test]
    fn cooldown_suppresses_a_repeat() {
        let mut d = Detector::default();
        d.update("Home", Some(12.0));
        assert!(matches!(d.update("Home", Some(11.0)), Verdict::Fire(_)));
        assert_eq!(d.update("Home", Some(10.0)), Verdict::Quiet);
        assert_eq!(d.update("Home", Some(9.0)), Verdict::Quiet);
    }

    #[test]
    fn a_gap_in_the_line_does_not_re_alert() {
        let mut d = Detector::default();
        d.update("Home", Some(12.0));
        assert!(matches!(d.update("Home", Some(11.0)), Verdict::Fire(_)));
        // One missing scan (a hole in the echo) then rain again — still the same rain.
        d.update("Home", None);
        assert_eq!(d.update("Home", Some(8.0)), Verdict::Quiet);
    }

    #[test]
    fn clears_state_for_deleted_points() {
        let mut d = Detector::default();
        d.update("Home", Some(12.0));
        d.retain(&["Work".to_string()]);
        // "Home" is forgotten, so its next hit starts the count over.
        assert_eq!(d.update("Home", Some(12.0)), Verdict::Quiet);
    }
}
