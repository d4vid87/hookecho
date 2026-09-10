//! Scientifically meaningful radar-beam geometry.
//!
//! Distances are slant range from the antenna.  The standard four-thirds effective-Earth
//! approximation bends the beam relative to the physical surface; the result is a ground arc
//! and a height above the antenna which do not depend on any renderer or camera.

/// Mean Earth radius used by the rest of HookEcho's radar analysis (metres).
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Standard-atmosphere effective Earth radius used for weather-radar propagation.
pub const EFFECTIVE_EARTH_RADIUS_M: f64 = EARTH_RADIUS_M * 4.0 / 3.0;

/// Beam centre at one slant range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamPoint {
    /// Height above the radar antenna, metres.
    pub height_above_radar_m: f64,
    /// Arc distance over the effective-Earth surface, metres.
    pub ground_range_m: f64,
    /// Beam-centre altitude above mean sea level, metres.
    pub altitude_msl_m: f64,
    /// Central angle subtended by `ground_range_m`, radians.
    pub central_angle_rad: f64,
}

/// Locate the centre of a radar beam with the four-thirds-Earth model.
pub fn beam_point(slant_range_m: f64, elevation_deg: f64, antenna_altitude_m: f64) -> BeamPoint {
    let r = slant_range_m.max(0.0);
    let e = elevation_deg.to_radians();
    let ae = EFFECTIVE_EARTH_RADIUS_M;
    let height_above_radar_m =
        (r * r + ae * ae + 2.0 * r * ae * e.sin()).sqrt() - ae;
    let central_angle_rad = (r * e.cos()).atan2(ae + r * e.sin());
    let ground_range_m = ae * central_angle_rad;
    BeamPoint {
        height_above_radar_m,
        ground_range_m,
        altitude_msl_m: antenna_altitude_m + height_above_radar_m,
        central_angle_rad,
    }
}

/// Destination reached from a radar site by following a great-circle bearing and distance.
/// Longitude is normalized to `[-180, 180)` so callers remain stable at the date line.
pub fn destination_lonlat(
    radar_lon_deg: f64,
    radar_lat_deg: f64,
    azimuth_deg: f64,
    ground_range_m: f64,
) -> (f64, f64) {
    let phi1 = radar_lat_deg.to_radians();
    let lambda1 = radar_lon_deg.to_radians();
    let bearing = azimuth_deg.to_radians();
    let delta = ground_range_m / EARTH_RADIUS_M;
    let sin_phi2 = phi1.sin() * delta.cos() + phi1.cos() * delta.sin() * bearing.cos();
    let phi2 = sin_phi2.clamp(-1.0, 1.0).asin();
    let lambda2 = lambda1
        + (bearing.sin() * delta.sin() * phi1.cos())
            .atan2(delta.cos() - phi1.sin() * phi2.sin());
    let lon = (lambda2.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    (lon, phi2.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_range_is_the_antenna() {
        let p = beam_point(0.0, 0.5, 372.0);
        assert_eq!(p.height_above_radar_m, 0.0);
        assert_eq!(p.ground_range_m, 0.0);
        assert_eq!(p.altitude_msl_m, 372.0);
    }

    #[test]
    fn standard_nexrad_ranges_are_finite_and_monotonic() {
        let mut last_h = -1.0;
        let mut last_g = -1.0;
        for range_km in [1.0, 25.0, 50.0, 100.0, 150.0, 230.0, 300.0] {
            let p = beam_point(range_km * 1_000.0, 0.5, 0.0);
            assert!(p.height_above_radar_m.is_finite());
            assert!(p.ground_range_m.is_finite());
            assert!(p.height_above_radar_m > last_h, "height at {range_km} km");
            assert!(p.ground_range_m > last_g, "ground range at {range_km} km");
            last_h = p.height_above_radar_m;
            last_g = p.ground_range_m;
        }
    }

    #[test]
    fn higher_elevation_is_higher_at_the_same_slant_range() {
        let low = beam_point(100_000.0, 0.5, 0.0);
        let high = beam_point(100_000.0, 4.0, 0.0);
        assert!(high.altitude_msl_m > low.altitude_msl_m);
    }

    #[test]
    fn antenna_altitude_is_an_msl_offset_only() {
        let sea = beam_point(150_000.0, 0.5, 0.0);
        let hill = beam_point(150_000.0, 0.5, 512.0);
        assert!((hill.altitude_msl_m - sea.altitude_msl_m - 512.0).abs() < 1e-9);
        assert_eq!(hill.ground_range_m, sea.ground_range_m);
    }

    #[test]
    fn destination_handles_cardinal_bearings_and_date_line() {
        let (east_lon, east_lat) = destination_lonlat(-97.0, 35.0, 90.0, 100_000.0);
        assert!(east_lon > -97.0);
        assert!((east_lat - 35.0).abs() < 0.01);
        let (wrapped, _) = destination_lonlat(179.8, 0.0, 90.0, 100_000.0);
        assert!((-180.0..180.0).contains(&wrapped));
        assert!(wrapped < 0.0);
    }
}
