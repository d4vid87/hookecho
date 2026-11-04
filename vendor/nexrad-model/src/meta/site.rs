use std::fmt::Display;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "uom")]
use uom::si::{f32::Length, length::meter};

/// A radar site's metadata including a variety of infrequently-changing properties.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Site {
    identifier: [u8; 4],
    latitude: f32,
    longitude: f32,
    height_meters: i16,
    tower_height_meters: u16,
}

impl Site {
    /// Create new radar site metadata with the given properties.
    pub fn new(
        identifier: [u8; 4],
        latitude: f32,
        longitude: f32,
        height_meters: i16,
        tower_height_meters: u16,
    ) -> Self {
        Self {
            identifier,
            latitude,
            longitude,
            height_meters,
            tower_height_meters,
        }
    }

    /// The four-letter ICAO identifier for the radar site.
    pub fn identifier(&self) -> &[u8; 4] {
        &self.identifier
    }

    /// The four-letter ICAO identifier for the radar site as a string.
    pub fn identifier_string(&self) -> String {
        String::from_utf8_lossy(&self.identifier).to_string()
    }

    /// The latitude of the radar site in degrees.
    pub fn latitude(&self) -> f32 {
        self.latitude
    }

    /// The longitude of the radar site in degrees.
    pub fn longitude(&self) -> f32 {
        self.longitude
    }

    /// The height of the radar site above sea level in meters.
    pub fn height_meters(&self) -> i16 {
        self.height_meters
    }

    /// The height of the radar site above sea level.
    #[cfg(feature = "uom")]
    pub fn height(&self) -> Length {
        Length::new::<meter>(self.height_meters as f32)
    }

    /// The height of the radar tower above ground in meters.
    pub fn tower_height_meters(&self) -> u16 {
        self.tower_height_meters
    }

    /// The height of the radar tower above ground.
    #[cfg(feature = "uom")]
    pub fn tower_height(&self) -> Length {
        Length::new::<meter>(self.tower_height_meters as f32)
    }
}

impl Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({:.4}°, {:.4}°, {}m)",
            self.identifier_string(),
            self.latitude,
            self.longitude,
            self.height_meters
        )
    }
}
