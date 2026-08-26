//! Looking ahead of the windscreen: where the car will be in a few minutes, and which radar
//! covers it.
//!
//! A chase crosses radar coverage boundaries at 70 mph, and the handoff to the next site is
//! currently a cold start — the app notices the new nearest site the moment it is already the
//! nearest one, and then spends a minute listing and downloading a 6 MB volume while the storm is
//! in front of you. Predicting the handoff a few minutes early turns that into a cache hit.
//!
//! Everything here is a heading extrapolated in a straight line. That is wrong on a curve and
//! wrong at a stop sign, and it does not matter: the cost of a wrong guess is one volume
//! downloaded and never shown.
//
// ponytail: dead-reckoning from the last two fixes, one site ahead, newest volume only. The
// ceiling is "the next site is warm when you get there". A real version would follow the road
// network and warm a whole loop; both need data this app does not carry.

use crate::chaselog::Track;

/// How far ahead to look. Long enough to cover a listing plus a download on cellular, short
/// enough that a straight-line guess is still roughly true.
pub const LOOKAHEAD_MIN: f64 = 6.0;

/// Below this there is no heading worth extrapolating — a parked car's two fixes differ by GPS
/// noise, and the "direction" they imply is random.
const MIN_SPEED_KT: f64 = 15.0;

/// Where the track will be `minutes` from its last fix, dead-reckoned from the last two.
///
/// `None` when there is nothing to reckon from: fewer than two points, no time between them, or
/// a speed low enough that the heading is noise rather than travel.
pub fn predict(track: &Track, minutes: f64) -> Option<[f64; 2]> {
    let n = track.points.len();
    let (a, b) = (track.points.get(n.checked_sub(2)?)?, track.points.last()?);
    let dt_h = (b.ts - a.ts) as f64 / 3600.0;
    if dt_h <= 0.0 {
        return None;
    }
    let (km, bearing_to_a) = crate::geo::great_circle([b.lon, b.lat], [a.lon, a.lat]);
    let kt = crate::geo::km_to_nmi(km) / dt_h;
    if kt < MIN_SPEED_KT {
        return None;
    }
    // `great_circle` gives the bearing from b back to a; the car is going the other way.
    let heading = (bearing_to_a + 180.0) % 360.0;
    Some(crate::geo::destination_point(
        [b.lon, b.lat],
        heading,
        km / dt_h * (minutes / 60.0),
    ))
}

/// The site to warm up: the nearest one to where the track is heading, when that is not the site
/// already on screen. `None` means "no handoff coming", which is the usual answer.
pub fn next_site(track: &Track, current: Option<&str>, minutes: f64) -> Option<String> {
    let [lon, lat] = predict(track, minutes)?;
    let site = crate::geo::nearest_site_id(lon, lat)?;
    match current {
        Some(c) if c.eq_ignore_ascii_case(&site) => None,
        _ => Some(site),
    }
}

/// Pull the newest volume for `site` into the on-disk cache, so the handoff finds it there.
///
/// Nothing is shown and nothing is reported: a failed warm-up is a handoff that behaves the way
/// it did before this module existed.
pub async fn warm(site: String) {
    let date = chrono::Utc::now().date_naive();
    let Ok(ids) = wxdata::level2::list_volumes(&site, date).await else {
        return;
    };
    let Some(newest) = ids.into_iter().next_back() else {
        return;
    };
    match wxdata::level2::download_scan(newest, crate::paths::cache_dir()).await {
        Ok(_) => log::debug!("chase: warmed {site}"),
        Err(e) => log::debug!("chase: warming {site} failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(points: &[(f64, f64, i64)]) -> Track {
        let mut t = Track::default();
        for (lon, lat, ts) in points {
            t.points.push(crate::chaselog::Fix {
                lon: *lon,
                lat: *lat,
                ts: *ts,
            });
        }
        t
    }

    #[test]
    fn a_car_going_east_is_predicted_east_of_itself() {
        // ~0.1° of longitude in 60 s at 35°N is ~9 km — about 300 kt, well clear of the floor.
        let t = track(&[(-97.6, 35.3, 0), (-97.5, 35.3, 60)]);
        let [lon, lat] = predict(&t, 6.0).unwrap();
        assert!(lon > -97.5, "predicted west of the last fix: {lon}");
        assert!((lat - 35.3).abs() < 0.05, "wandered off the heading: {lat}");
        // Six minutes at that speed is a long way: tens of kilometres, not metres.
        let (km, _) = crate::geo::great_circle([-97.5, 35.3], [lon, lat]);
        assert!(km > 40.0 && km < 70.0, "{km} km ahead");
    }

    #[test]
    fn a_parked_car_has_no_heading() {
        // Two fixes a minute apart, a few metres of noise between them.
        assert!(predict(&track(&[(-97.5, 35.3, 0), (-97.5001, 35.3, 60)]), 6.0).is_none());
        // And one fix is not a direction.
        assert!(predict(&track(&[(-97.5, 35.3, 0)]), 6.0).is_none());
        assert!(predict(&Track::default(), 6.0).is_none());
    }

    /// Live: warms a real site off the public NEXRAD bucket and checks a volume landed in the
    /// cache. Ignored by default — it downloads several MB and needs the network.
    #[test]
    #[ignore = "network"]
    fn warming_a_site_leaves_a_volume_in_the_cache() {
        let dir = std::env::temp_dir().join("hookecho-chase-warm-test");
        let _ = std::fs::remove_dir_all(&dir);
        crate::paths::set_base(dir.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(warm("KTLX".to_string()));
        let volumes = crate::paths::cache_dir().unwrap().join("volumes");
        let n = std::fs::read_dir(&volumes).map(|d| d.count()).unwrap_or(0);
        assert!(n > 0, "nothing cached in {}", volumes.display());
    }

    #[test]
    fn the_handoff_only_fires_when_the_site_actually_changes() {
        // Driving hard west out of Oklahoma City towards the Texas panhandle.
        let t = track(&[(-98.6, 35.3, 0), (-99.0, 35.3, 120)]);
        let ahead = next_site(&t, Some("KTLX"), 30.0).expect("a different site is ahead");
        assert_ne!(ahead, "KTLX");
        // Whatever that site is, being on it already means there is nothing to warm.
        assert!(next_site(&t, Some(&ahead), 30.0).is_none());
        assert!(next_site(&t, Some(&ahead.to_lowercase()), 30.0).is_none());
    }
}
