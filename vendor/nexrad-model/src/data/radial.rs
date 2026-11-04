use crate::data::{CFPMomentData, MomentData};

#[cfg(feature = "chrono")]
use chrono::{DateTime, Utc};

#[cfg(feature = "uom")]
use uom::si::{angle::degree, f32::Angle};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A single radar ray composed of a series of gates. This represents a single azimuth angle and
/// elevation angle pair at a point in time and contains the Level II data (reflectivity, velocity,
/// and spectrum width) for each range gate in that ray. The range of the radar and gate interval
/// distance determines the resolution of the ray and the number of gates in the ray.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Radial {
    collection_timestamp: i64,

    azimuth_number: u16,
    azimuth_angle_degrees: f32,
    azimuth_spacing_degrees: f32,

    radial_status: RadialStatus,

    elevation_number: u8,
    elevation_angle_degrees: f32,

    reflectivity: Option<MomentData>,
    velocity: Option<MomentData>,
    spectrum_width: Option<MomentData>,
    differential_reflectivity: Option<MomentData>,
    differential_phase: Option<MomentData>,
    correlation_coefficient: Option<MomentData>,
    clutter_filter_power: Option<CFPMomentData>,
    /// Instrument Nyquist velocity in m/s, retained by HookEcho's decoder patch.
    #[cfg_attr(feature = "serde", serde(default))]
    nyquist_velocity_mps: Option<f32>,
}

impl Radial {
    /// Create a new radial with the given properties.
    pub fn new(
        collection_timestamp: i64,
        azimuth_number: u16,
        azimuth_angle_degrees: f32,
        azimuth_spacing_degrees: f32,
        radial_status: RadialStatus,
        elevation_number: u8,
        elevation_angle_degrees: f32,
        reflectivity: Option<MomentData>,
        velocity: Option<MomentData>,
        spectrum_width: Option<MomentData>,
        differential_reflectivity: Option<MomentData>,
        differential_phase: Option<MomentData>,
        correlation_coefficient: Option<MomentData>,
        clutter_filter_power: Option<CFPMomentData>,
    ) -> Self {
        Self {
            collection_timestamp,
            azimuth_number,
            azimuth_angle_degrees,
            azimuth_spacing_degrees,
            radial_status,
            elevation_number,
            elevation_angle_degrees,
            reflectivity,
            velocity,
            spectrum_width,
            differential_reflectivity,
            differential_phase,
            correlation_coefficient,
            clutter_filter_power,
            nyquist_velocity_mps: None,
        }
    }

    /// Instrument Nyquist velocity in m/s; absent metadata must not be inferred from observed gates.
    pub fn nyquist_velocity_mps(&self) -> Option<f32> { self.nyquist_velocity_mps }

    /// Attach measured instrument metadata, rejecting absent or invalid limits.
    pub fn with_nyquist_velocity_mps(mut self, value: Option<f32>) -> Self {
        self.nyquist_velocity_mps = value.filter(|v| v.is_finite() && *v > 0.0);
        self
    }

    /// The collection timestamp in milliseconds since midnight Jan 1, 1970 (epoch/UNIX timestamp).
    pub fn collection_timestamp(&self) -> i64 {
        self.collection_timestamp
    }

    /// The collection time for this radial and its data.
    #[cfg(feature = "chrono")]
    pub fn collection_time(&self) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp_millis(self.collection_timestamp)
    }

    /// The index number for this radial's azimuth in the elevation sweep, ranging up to 720
    /// depending on the azimuthal resolution.
    pub fn azimuth_number(&self) -> u16 {
        self.azimuth_number
    }

    /// Azimuth angle this radial's data was collected at in degrees.
    pub fn azimuth_angle_degrees(&self) -> f32 {
        self.azimuth_angle_degrees
    }

    /// Azimuth angle this radial's data was collected at.
    #[cfg(feature = "uom")]
    pub fn azimuth(&self) -> Angle {
        Angle::new::<degree>(self.azimuth_angle_degrees)
    }

    /// Azimuthal distance between radials in the sweep in degrees.
    pub fn azimuth_spacing_degrees(&self) -> f32 {
        self.azimuth_spacing_degrees
    }

    /// Azimuthal distance between radials in the sweep.
    #[cfg(feature = "uom")]
    pub fn azimuth_spacing(&self) -> Angle {
        Angle::new::<degree>(self.azimuth_spacing_degrees)
    }

    /// The radial's position in the sequence of radials making up a scan.
    pub fn radial_status(&self) -> RadialStatus {
        self.radial_status
    }

    /// The elevation number for this radial in the scan.
    pub fn elevation_number(&self) -> u8 {
        self.elevation_number
    }

    /// Elevation angle this radial's data was collected at in degrees.
    pub fn elevation_angle_degrees(&self) -> f32 {
        self.elevation_angle_degrees
    }

    /// Elevation angle this radial's data was collected at.
    #[cfg(feature = "uom")]
    pub fn elevation_angle(&self) -> Angle {
        Angle::new::<degree>(self.elevation_angle_degrees)
    }

    /// Reflectivity data for this radial if available.
    pub fn reflectivity(&self) -> Option<&MomentData> {
        self.reflectivity.as_ref()
    }

    /// Velocity data for this radial if available.
    pub fn velocity(&self) -> Option<&MomentData> {
        self.velocity.as_ref()
    }

    /// Spectrum width data for this radial if available.
    pub fn spectrum_width(&self) -> Option<&MomentData> {
        self.spectrum_width.as_ref()
    }

    /// Differential reflectivity data for this radial if available.
    pub fn differential_reflectivity(&self) -> Option<&MomentData> {
        self.differential_reflectivity.as_ref()
    }

    /// Differential phase data for this radial if available.
    pub fn differential_phase(&self) -> Option<&MomentData> {
        self.differential_phase.as_ref()
    }

    /// Correlation coefficient data for this radial if available.
    pub fn correlation_coefficient(&self) -> Option<&MomentData> {
        self.correlation_coefficient.as_ref()
    }

    /// Clutter filter power (CFP) data for this radial if available.
    /// CFP represents the difference between clutter-filtered and unfiltered reflectivity.
    pub fn clutter_filter_power(&self) -> Option<&CFPMomentData> {
        self.clutter_filter_power.as_ref()
    }
}

/// Describe a radial's position within the sequence of radials comprising a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RadialStatus {
    /// First radial of an elevation sweep (not the first sweep in the scan).
    ElevationStart,
    /// A radial within an elevation sweep (not the first or last radial).
    IntermediateRadialData,
    /// Last radial of an elevation sweep.
    ElevationEnd,
    /// First radial of the first elevation sweep in a scan.
    ScanStart,
    /// Last radial of the last elevation sweep in a scan.
    ScanEnd,
    /// Start of new elevation which is the last in the VCP.
    ElevationStartVCPFinal,
    /// An unrecognized radial status value.
    Unknown(u8),
}
