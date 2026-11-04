use super::raw;
use super::{AzimuthResolutionSpacing, CompressionIndicator, RadialStatus, SpotBlankingStatus};
use crate::util::get_datetime;
use chrono::{DateTime, Duration, Utc};
use std::borrow::Cow;

#[cfg(feature = "uom")]
use uom::si::angle::degree;
#[cfg(feature = "uom")]
use uom::si::f64::{Angle, Information};
#[cfg(feature = "uom")]
use uom::si::information::byte;

/// The digital radar data message header block precedes base data information for a particular
/// radial and includes parameters for that radial and information about the following data blocks.
#[derive(Clone, PartialEq, Debug)]
pub struct Header<'a> {
    inner: Cow<'a, raw::Header>,
}

impl<'a> Header<'a> {
    /// Create a new Header wrapper from a raw Header reference.
    pub(crate) fn new(inner: &'a raw::Header) -> Self {
        Self {
            inner: Cow::Borrowed(inner),
        }
    }

    /// Convert this header to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Header<'static> {
        Header {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }

    /// ICAO radar identifier (raw bytes).
    pub fn radar_identifier_raw(&self) -> &[u8; 4] {
        &self.inner.radar_identifier
    }

    /// Collection time in milliseconds past midnight, GMT.
    pub fn time(&self) -> u32 {
        self.inner.time.get()
    }

    /// This message's date represented as a count of days since 1 January 1970 00:00 GMT.
    pub fn date(&self) -> u16 {
        self.inner.date.get()
    }

    /// Radial number within the elevation scan. These range up to 720, in 0.5 degree increments.
    pub fn azimuth_number(&self) -> u16 {
        self.inner.azimuth_number.get()
    }

    /// Azimuth angle at which the radial was collected in degrees (raw value).
    pub fn azimuth_angle_raw(&self) -> f32 {
        self.inner.azimuth_angle.get()
    }

    /// Indicates if the message is compressed and what type of compression was used (raw value).
    pub fn compression_indicator_raw(&self) -> u8 {
        self.inner.compression_indicator
    }

    /// Uncompressed length of the radial in bytes (raw value).
    pub fn radial_length_raw(&self) -> u16 {
        self.inner.radial_length.get()
    }

    /// Azimuthal spacing between adjacent radials (raw code).
    /// Values: 1 = 0.5 degrees, 2 = 1.0 degrees
    pub fn azimuth_resolution_spacing_raw(&self) -> u8 {
        self.inner.azimuth_resolution_spacing
    }

    /// The radial's status within the larger scan (raw value).
    pub fn radial_status_raw(&self) -> u8 {
        self.inner.radial_status
    }

    /// The radial's elevation number within the volume scan.
    pub fn elevation_number(&self) -> u8 {
        self.inner.elevation_number
    }

    /// The sector number within cut. A value of 0 is only valid for continuous surveillance cuts.
    pub fn cut_sector_number(&self) -> u8 {
        self.inner.cut_sector_number
    }

    /// The radial's collection elevation angle in degrees (raw value).
    pub fn elevation_angle_raw(&self) -> f32 {
        self.inner.elevation_angle.get()
    }

    /// The spot blanking status for the current radial, elevation, and volume scan (raw value).
    pub fn radial_spot_blanking_status_raw(&self) -> u8 {
        self.inner.radial_spot_blanking_status
    }

    /// The azimuth indexing value (raw scaled value).
    /// Values: 0 = No indexing, 1-100 = Indexing angle of 0.01 to 1.00 degrees
    pub fn azimuth_indexing_mode_raw(&self) -> u8 {
        self.inner.azimuth_indexing_mode
    }

    /// The number of "data moment" blocks following this header block.
    pub fn data_block_count(&self) -> u16 {
        self.inner.data_block_count.get()
    }

    /// ICAO radar identifier.
    pub fn radar_identifier(&self) -> String {
        String::from_utf8_lossy(&self.inner.radar_identifier).to_string()
    }

    /// The collection date and time for this data.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.inner.date.get(),
            Duration::milliseconds(self.inner.time.get() as i64),
        )
    }

    /// Azimuth angle at which the radial was collected.
    #[cfg(feature = "uom")]
    pub fn azimuth_angle(&self) -> Angle {
        Angle::new::<degree>(self.inner.azimuth_angle.get() as f64)
    }

    /// Whether the message is compressed and what type of compression was used.
    pub fn compression_indicator(&self) -> CompressionIndicator {
        match self.inner.compression_indicator {
            0 => CompressionIndicator::Uncompressed,
            1 => CompressionIndicator::CompressedBZIP2,
            2 => CompressionIndicator::CompressedZLIB,
            _ => CompressionIndicator::FutureUse,
        }
    }

    /// Uncompressed length of the radial (including the data header block).
    #[cfg(feature = "uom")]
    pub fn radial_length(&self) -> Information {
        Information::new::<byte>(self.inner.radial_length.get() as f64)
    }

    /// Azimuthal spacing between adjacent radials.
    #[cfg(feature = "uom")]
    pub fn azimuth_resolution_spacing(&self) -> Angle {
        Angle::new::<degree>(self.inner.azimuth_resolution_spacing as f64 * 0.5)
    }

    /// The radial's status within the larger scan.
    pub fn radial_status(&self) -> RadialStatus {
        match self.inner.radial_status {
            0 => RadialStatus::ElevationStart,
            1 => RadialStatus::IntermediateRadialData,
            2 => RadialStatus::ElevationEnd,
            3 => RadialStatus::VolumeScanStart,
            4 => RadialStatus::VolumeScanEnd,
            5 => RadialStatus::ElevationStartVCPFinal,
            other => RadialStatus::Unknown(other),
        }
    }

    /// The radial's collection elevation angle.
    #[cfg(feature = "uom")]
    pub fn elevation_angle(&self) -> Angle {
        Angle::new::<degree>(self.inner.elevation_angle.get() as f64)
    }

    /// The spot blanking status for the current radial, elevation, and volume scan.
    pub fn radial_spot_blanking_status(&self) -> SpotBlankingStatus {
        SpotBlankingStatus::new(self.inner.radial_spot_blanking_status)
    }

    /// The azimuth indexing value (if keyed to constant angles).
    #[cfg(feature = "uom")]
    pub fn azimuth_indexing_mode(&self) -> Option<Angle> {
        if self.inner.azimuth_indexing_mode == 0 {
            None
        } else {
            Some(Angle::new::<degree>(
                self.inner.azimuth_indexing_mode as f64 * 0.01,
            ))
        }
    }

    /// Azimuthal spacing between adjacent radials as a typed enum.
    pub fn azimuth_resolution_spacing_type(&self) -> AzimuthResolutionSpacing {
        match self.inner.azimuth_resolution_spacing {
            1 => AzimuthResolutionSpacing::HalfDegree,
            2 => AzimuthResolutionSpacing::OneDegree,
            other => AzimuthResolutionSpacing::Unknown(other),
        }
    }

    /// The azimuth indexing value in degrees (if keyed to constant angles).
    /// Returns `None` if no indexing (value is 0), otherwise the indexing angle in degrees.
    pub fn azimuth_indexing_mode_degrees(&self) -> Option<f64> {
        if self.inner.azimuth_indexing_mode == 0 {
            None
        } else {
            Some(self.inner.azimuth_indexing_mode as f64 * 0.01)
        }
    }
}
