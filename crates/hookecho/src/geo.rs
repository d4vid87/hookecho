//! Small geodesy helpers for map annotations (the measure tool).

/// Great-circle distance (km) and initial bearing (° clockwise from north) between two
/// `[lon, lat]` points (degrees), via the haversine formula on a spherical earth.
pub fn great_circle(a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let (lon1, lat1) = (a[0].to_radians(), a[1].to_radians());
    let (lon2, lat2) = (b[0].to_radians(), b[1].to_radians());
    let (dlat, dlon) = (lat2 - lat1, lon2 - lon1);
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    let km = 2.0 * 6371.0088 * h.sqrt().asin();
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    let bearing = (y.atan2(x).to_degrees() + 360.0) % 360.0;
    (km, bearing)
}

/// Kilometers to nautical miles.
pub fn km_to_nmi(km: f64) -> f64 {
    km / 1.852
}

/// Destination `[lon, lat]` reached from `start` heading `bearing_deg` (° from north) for
/// `km` (spherical earth). Used to draw the arrival-cone edges out to a lead distance.
pub fn destination_point(start: [f64; 2], bearing_deg: f64, km: f64) -> [f64; 2] {
    let r = 6371.0088;
    let d = km / r;
    let brg = bearing_deg.to_radians();
    let (lon1, lat1) = (start[0].to_radians(), start[1].to_radians());
    let lat2 = (lat1.sin() * d.cos() + lat1.cos() * d.sin() * brg.cos()).asin();
    let lon2 = lon1
        + (brg.sin() * d.sin() * lat1.cos()).atan2(d.cos() - lat1.sin() * lat2.sin());
    [lon2.to_degrees(), lat2.to_degrees()]
}

/// Estimated minutes for a storm at `cell` [lon,lat] moving toward `mvt_deg` (° from north) at
/// `mvt_kt` knots to reach `target` [lon,lat], if the target lies within `half_angle_deg` of the
/// motion vector and no more than `max_min` ahead. `None` = not on the path (behind, off-angle,
/// stationary, or too far). This is the arrival-cone hit test.
pub fn arrival_eta_min(
    cell: [f64; 2],
    mvt_deg: f32,
    mvt_kt: f32,
    target: [f64; 2],
    half_angle_deg: f64,
    max_min: f64,
) -> Option<f64> {
    if mvt_kt <= 1.0 {
        return None; // effectively stationary — no meaningful ETA
    }
    let (km, bearing) = great_circle(cell, target);
    // Angular offset between the motion heading and the direction to the target.
    let mut off = (bearing - mvt_deg as f64).abs() % 360.0;
    if off > 180.0 {
        off = 360.0 - off;
    }
    if off > half_angle_deg {
        return None; // target is outside the forward cone (incl. behind the storm)
    }
    let speed_kmh = mvt_kt as f64 * 1.852;
    // Along-track distance (project onto the motion vector) / speed.
    let along_km = km * off.to_radians().cos();
    let minutes = along_km / speed_kmh * 60.0;
    (minutes >= 0.0 && minutes <= max_min).then_some(minutes)
}

/// How close a storm at `cell` moving toward `dir_deg` at `kt` knots will come to `target`, and
/// when: returns `(km, minutes)` for the minimum distance over the next `max_min` minutes.
///
/// Sampled once a minute along the projected track rather than solved analytically — the great
/// circle makes the closed form fussy, and a chaser cares about the minute, not the second. A
/// receding storm returns `t = 0` (now is as close as it gets).
pub fn closest_approach(
    cell: [f64; 2],
    dir_deg: f64,
    kt: f64,
    target: [f64; 2],
    max_min: f64,
) -> (f64, f64) {
    let mut best = (great_circle(cell, target).0, 0.0);
    if kt <= 1.0 {
        return best; // effectively stationary: it never gets closer than it is now
    }
    let km_per_min = kt * 1.852 / 60.0;
    let mut t = 1.0;
    while t <= max_min {
        let at = destination_point(cell, dir_deg, km_per_min * t);
        let km = great_circle(at, target).0;
        if km < best.0 {
            best = (km, t);
        }
        t += 1.0;
    }
    best
}

/// The bearing to drive to get off a storm's path fastest: perpendicular to the storm motion, on
/// whichever side `target` already sits (crossing in front of the storm is how chasers die).
pub fn escape_bearing(cell: [f64; 2], dir_deg: f64, target: [f64; 2]) -> f64 {
    let (_, to_target) = great_circle(cell, target);
    // Signed angle from the motion vector to the target direction, in (-180, 180].
    let mut off = (to_target - dir_deg) % 360.0;
    if off > 180.0 {
        off -= 360.0;
    }
    if off < -180.0 {
        off += 360.0;
    }
    // Target on the right of the track → flee right (+90°), else left.
    let turn = if off >= 0.0 { 90.0 } else { -90.0 };
    (dir_deg + turn + 360.0) % 360.0
}

/// The NEXRAD site id nearest to `[lon, lat]` (chase-mode auto-handoff), or `None` if the
/// registry is somehow empty. Delegates to the registry's own nearest-site lookup.
pub fn nearest_site_id(lon: f64, lat: f64) -> Option<String> {
    wxdata::sites::nearest_site(lat as f32, lon as f32).map(|s| s.id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_site_to_okc_is_ktlx() {
        // Oklahoma City sits right by the KTLX (Twin Lakes) radar.
        assert_eq!(nearest_site_id(-97.5, 35.47).as_deref(), Some("KTLX"));
    }

    #[test]
    fn one_degree_north_is_60nmi() {
        let (km, brg) = great_circle([-97.0, 35.0], [-97.0, 36.0]);
        assert!((km_to_nmi(km) - 60.0).abs() < 0.5, "≈60 nmi/deg lat, got {}", km_to_nmi(km));
        assert!(brg.abs() < 0.5 || (brg - 360.0).abs() < 0.5, "due north, got {brg}");
    }

    #[test]
    fn due_east_bearing_90() {
        let (_, brg) = great_circle([-97.0, 35.0], [-96.0, 35.0]);
        assert!((brg - 90.0).abs() < 1.0, "≈east, got {brg}");
    }

    #[test]
    fn arrival_eta_on_path() {
        // Storm at (-97,35) moving due east (90°) at 30 kt; target 1° east (~48 nmi).
        let eta = arrival_eta_min([-97.0, 35.0], 90.0, 30.0, [-96.0, 35.0], 20.0, 240.0)
            .expect("target is on the path");
        // ~48 nmi / 30 kt = ~1.6 h ≈ 96 min.
        assert!((eta - 96.0).abs() < 8.0, "ETA ~96 min, got {eta}");
    }

    #[test]
    fn closest_approach_runs_the_storm_over_you() {
        // Storm 1° west of the chaser moving due east at 30 kt: it passes right over them.
        let (km, min) = closest_approach([-98.0, 35.0], 90.0, 30.0, [-97.0, 35.0], 240.0);
        assert!(km < 3.0, "should nearly hit, got {km} km");
        // ~91 km at 55.6 km/h ≈ 98 min.
        assert!((min - 98.0).abs() < 10.0, "ETA ~98 min, got {min}");
    }

    #[test]
    fn closest_approach_receding_storm_is_closest_now() {
        // Storm already east of the chaser, still moving east — it only gets farther.
        let (km, min) = closest_approach([-96.0, 35.0], 90.0, 40.0, [-97.0, 35.0], 240.0);
        assert_eq!(min, 0.0, "closest approach is now");
        assert!(km > 80.0, "still ~91 km away, got {km}");
    }

    #[test]
    fn escape_bearing_points_away_from_the_track() {
        // Eastbound storm, chaser to the north → flee north.
        let b = escape_bearing([-97.0, 35.0], 90.0, [-97.0, 36.0]);
        assert!(b.min(360.0 - b) < 1.0, "expected ~0° (north), got {b}");
        // Same storm, chaser to the south → flee south.
        let b = escape_bearing([-97.0, 35.0], 90.0, [-97.0, 34.0]);
        assert!((b - 180.0).abs() < 1.0, "expected ~180° (south), got {b}");
    }

    #[test]
    fn arrival_eta_rejects_off_path_and_behind() {
        // Target to the north (off the eastward cone).
        assert!(arrival_eta_min([-97.0, 35.0], 90.0, 30.0, [-97.0, 36.0], 20.0, 240.0).is_none());
        // Target behind (to the west) of an eastbound storm.
        assert!(arrival_eta_min([-97.0, 35.0], 90.0, 30.0, [-98.0, 35.0], 20.0, 240.0).is_none());
        // Stationary storm.
        assert!(arrival_eta_min([-97.0, 35.0], 90.0, 0.5, [-96.0, 35.0], 20.0, 240.0).is_none());
    }
}
