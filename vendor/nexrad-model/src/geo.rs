//! Geographic coordinate types and radar coordinate system.
//!
//! This module provides types for working with geographic coordinates and converting
//! between radar-relative polar coordinates and geographic (WGS-84) coordinates.

use crate::meta::Site;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A geographic point in WGS-84 coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GeoPoint {
    /// Latitude in degrees (-90 to 90).
    pub latitude: f64,
    /// Longitude in degrees (-180 to 180).
    pub longitude: f64,
}

/// A geographic point with altitude.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GeoPoint3D {
    /// Latitude in degrees (-90 to 90).
    pub latitude: f64,
    /// Longitude in degrees (-180 to 180).
    pub longitude: f64,
    /// Altitude above mean sea level in meters.
    pub altitude_meters: f64,
}

/// A rectangular geographic extent (bounding box).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GeoExtent {
    /// Southwest corner (minimum latitude and longitude).
    pub min: GeoPoint,
    /// Northeast corner (maximum latitude and longitude).
    pub max: GeoPoint,
}

/// A point in radar-relative polar coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PolarPoint {
    /// Azimuth angle in degrees clockwise from north (0-360).
    pub azimuth_degrees: f32,
    /// Slant range from the radar in kilometers.
    pub range_km: f64,
    /// Elevation angle in degrees above the horizon.
    pub elevation_degrees: f32,
}

/// Standard 4/3 effective earth radius in meters.
///
/// The standard atmosphere assumption uses 4/3 of the true earth radius to account
/// for electromagnetic beam refraction. This is the standard meteorological convention
/// used by the NWS and in radar meteorology textbooks.
const EFFECTIVE_EARTH_RADIUS_M: f64 = 6_371_000.0 * 4.0 / 3.0;

/// Radar coordinate system for converting between polar and geographic coordinates.
///
/// Uses the standard 4/3 earth radius beam propagation model, which is the established
/// meteorological convention for radar beam height and range calculations.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RadarCoordinateSystem {
    /// Radar site latitude in degrees.
    latitude: f64,
    /// Radar site longitude in degrees.
    longitude: f64,
    /// Radar antenna height above mean sea level in meters.
    antenna_height_meters: f64,
}

impl RadarCoordinateSystem {
    /// Create a new radar coordinate system from a site's metadata.
    ///
    /// The antenna height is computed as the site's ground elevation plus its tower height.
    pub fn new(site: &Site) -> Self {
        Self {
            latitude: site.latitude() as f64,
            longitude: site.longitude() as f64,
            antenna_height_meters: site.height_meters() as f64 + site.tower_height_meters() as f64,
        }
    }

    /// The radar site latitude in degrees.
    pub fn latitude(&self) -> f64 {
        self.latitude
    }

    /// The radar site longitude in degrees.
    pub fn longitude(&self) -> f64 {
        self.longitude
    }

    /// The radar antenna height above mean sea level in meters.
    pub fn antenna_height_meters(&self) -> f64 {
        self.antenna_height_meters
    }

    /// Convert radar-relative polar coordinates to geographic coordinates.
    ///
    /// Uses the standard 4/3 earth radius beam propagation model to compute the
    /// geographic location and altitude of a point at the given azimuth, range,
    /// and elevation from the radar.
    pub fn polar_to_geo(&self, polar: PolarPoint) -> GeoPoint3D {
        let range_m = polar.range_km * 1000.0;
        let elev_rad = (polar.elevation_degrees as f64).to_radians();
        let az_rad = (polar.azimuth_degrees as f64).to_radians();

        // Beam height using 4/3 earth radius model
        // h = sqrt(r^2 + R_e^2 + 2*r*R_e*sin(el)) - R_e + h_antenna
        let height_m = ((range_m * range_m)
            + (EFFECTIVE_EARTH_RADIUS_M * EFFECTIVE_EARTH_RADIUS_M)
            + (2.0 * range_m * EFFECTIVE_EARTH_RADIUS_M * elev_rad.sin()))
        .sqrt()
            - EFFECTIVE_EARTH_RADIUS_M
            + self.antenna_height_meters;

        // Ground range (arc distance along earth surface)
        // s = R_e * arcsin(r * cos(el) / (R_e + h))
        let ground_range_m = EFFECTIVE_EARTH_RADIUS_M
            * ((range_m * elev_rad.cos()) / (EFFECTIVE_EARTH_RADIUS_M + height_m)).asin();

        // Convert ground range + azimuth to lat/lon offset
        let lat_rad = self.latitude.to_radians();
        let lon_rad = self.longitude.to_radians();

        let angular_distance = ground_range_m / 6_371_000.0; // Use true earth radius for lat/lon

        let new_lat = (lat_rad.sin() * angular_distance.cos()
            + lat_rad.cos() * angular_distance.sin() * az_rad.cos())
        .asin();

        let new_lon = lon_rad
            + (az_rad.sin() * angular_distance.sin() * lat_rad.cos())
                .atan2(angular_distance.cos() - lat_rad.sin() * new_lat.sin());

        GeoPoint3D {
            latitude: new_lat.to_degrees(),
            longitude: new_lon.to_degrees(),
            altitude_meters: height_m,
        }
    }

    /// Convert geographic coordinates to radar-relative polar coordinates.
    ///
    /// Computes the azimuth and range from the radar to the given geographic point
    /// at the specified elevation angle.
    pub fn geo_to_polar(&self, point: GeoPoint, elevation_degrees: f32) -> PolarPoint {
        let lat1 = self.latitude.to_radians();
        let lon1 = self.longitude.to_radians();
        let lat2 = point.latitude.to_radians();
        let lon2 = point.longitude.to_radians();

        let d_lon = lon2 - lon1;

        // Azimuth (forward bearing)
        let y = d_lon.sin() * lat2.cos();
        let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * d_lon.cos();
        let azimuth_rad = y.atan2(x);
        let azimuth_degrees = (azimuth_rad.to_degrees() + 360.0) % 360.0;

        // Great-circle distance
        let a = ((lat2 - lat1) / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
        let angular_distance = 2.0 * a.sqrt().asin();
        let ground_range_m = angular_distance * 6_371_000.0;

        // Invert the beam height equation to get slant range
        let elev_rad = (elevation_degrees as f64).to_radians();
        let range_m = ground_range_m / elev_rad.cos(); // Approximation for small elevations

        PolarPoint {
            azimuth_degrees: azimuth_degrees as f32,
            range_km: range_m / 1000.0,
            elevation_degrees,
        }
    }

    /// Compute the geographic bounding box of a radar sweep at the given maximum range.
    ///
    /// Returns the extent that encompasses the full 360-degree coverage circle at
    /// the specified range in kilometers.
    pub fn sweep_extent(&self, max_range_km: f64) -> GeoExtent {
        // Compute the geographic offset for the max range in each cardinal direction
        let north = self.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: max_range_km,
            elevation_degrees: 0.0,
        });
        let south = self.polar_to_geo(PolarPoint {
            azimuth_degrees: 180.0,
            range_km: max_range_km,
            elevation_degrees: 0.0,
        });
        let east = self.polar_to_geo(PolarPoint {
            azimuth_degrees: 90.0,
            range_km: max_range_km,
            elevation_degrees: 0.0,
        });
        let west = self.polar_to_geo(PolarPoint {
            azimuth_degrees: 270.0,
            range_km: max_range_km,
            elevation_degrees: 0.0,
        });

        GeoExtent {
            min: GeoPoint {
                latitude: south.latitude,
                longitude: west.longitude,
            },
            max: GeoPoint {
                latitude: north.latitude,
                longitude: east.longitude,
            },
        }
    }

    /// Compute the geographic location of a specific gate in a radial.
    pub fn gate_location(
        &self,
        azimuth_degrees: f32,
        elevation_degrees: f32,
        gate_index: usize,
        first_gate_km: f64,
        gate_interval_km: f64,
    ) -> GeoPoint3D {
        let range_km = first_gate_km + (gate_index as f64) * gate_interval_km;
        self.polar_to_geo(PolarPoint {
            azimuth_degrees,
            range_km,
            elevation_degrees,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Site;

    fn test_site() -> Site {
        // KTLX (Oklahoma City) approximate coordinates
        Site::new(*b"KTLX", 35.3331, -97.2778, 370, 10)
    }

    #[test]
    fn test_polar_to_geo_at_origin() {
        let cs = RadarCoordinateSystem::new(&test_site());
        let result = cs.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: 0.0,
            elevation_degrees: 0.5,
        });

        // At zero range, the point should be at the radar location
        assert!((result.latitude - 35.3331).abs() < 0.001);
        assert!((result.longitude - (-97.2778)).abs() < 0.001);
    }

    #[test]
    fn test_polar_to_geo_northward() {
        let cs = RadarCoordinateSystem::new(&test_site());
        let result = cs.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: 100.0,
            elevation_degrees: 0.5,
        });

        // 100km north should increase latitude by roughly 0.9 degrees
        assert!(result.latitude > 35.3331);
        assert!((result.latitude - 35.3331 - 0.9).abs() < 0.1);
        // Longitude should barely change for due-north azimuth
        assert!((result.longitude - (-97.2778)).abs() < 0.01);
    }

    #[test]
    fn test_sweep_extent_symmetric() {
        let cs = RadarCoordinateSystem::new(&test_site());
        let extent = cs.sweep_extent(230.0);

        // Extent should be roughly symmetric around the radar
        let lat_center = (extent.min.latitude + extent.max.latitude) / 2.0;
        let lon_center = (extent.min.longitude + extent.max.longitude) / 2.0;

        assert!((lat_center - 35.3331).abs() < 0.05);
        assert!((lon_center - (-97.2778)).abs() < 0.05);
    }

    #[test]
    fn test_beam_height_increases_with_range() {
        let cs = RadarCoordinateSystem::new(&test_site());
        let near = cs.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: 50.0,
            elevation_degrees: 0.5,
        });
        let far = cs.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: 200.0,
            elevation_degrees: 0.5,
        });

        assert!(far.altitude_meters > near.altitude_meters);
    }

    #[test]
    fn test_beam_height_increases_with_elevation() {
        let cs = RadarCoordinateSystem::new(&test_site());
        let low = cs.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: 100.0,
            elevation_degrees: 0.5,
        });
        let high = cs.polar_to_geo(PolarPoint {
            azimuth_degrees: 0.0,
            range_km: 100.0,
            elevation_degrees: 5.0,
        });

        assert!(high.altitude_meters > low.altitude_meters);
    }
}
