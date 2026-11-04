use crate::messages::primitive_aliases::{Integer2, Real4, ScaledInteger2};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Legacy radial data moment block (Build 10.0–11.x, 16 bytes).
///
/// This is the original RAD block format used from when Message Type 31 was
/// introduced (Build 10.0, 2008) through Build 11.x (2010). It does not include
/// the channel calibration constant fields added at Build 12.0 for dual polarization.
///
/// The `lrtup` field in this format is 20 (16 struct bytes + 4-byte DataBlockId).
#[repr(C)]
#[derive(Clone, PartialEq, Debug, FromBytes, Immutable, KnownLayout)]
pub struct RadialDataBlockLegacy {
    /// Size of data block in bytes.
    pub lrtup: Integer2,

    /// Unambiguous range, interval size, in km.
    pub unambiguous_range: ScaledInteger2,

    /// Noise level for the horizontal channel in dBm.
    pub horizontal_channel_noise_level: Real4,

    /// Noise level for the vertical channel in dBm.
    pub vertical_channel_noise_level: Real4,

    /// Nyquist velocity in m/s.
    pub nyquist_velocity: ScaledInteger2,

    /// Radial flags to support RPG processing.
    pub radial_flags: Integer2,
}

/// Modern radial data moment block (Build 12.0+, 24 bytes).
///
/// This expanded format was introduced at Build 12.0 (ICD 2620002K, July 2011) for
/// dual polarization. It adds channel calibration constants for horizontal and vertical
/// channels compared to the legacy format.
///
/// The `lrtup` field in this format is 28 (24 struct bytes + 4-byte DataBlockId).
#[repr(C)]
#[derive(Clone, PartialEq, Debug, FromBytes, Immutable, KnownLayout)]
pub struct RadialDataBlock {
    /// Size of data block in bytes.
    pub lrtup: Integer2,

    /// Unambiguous range, interval size, in km.
    pub unambiguous_range: ScaledInteger2,

    /// Noise level for the horizontal channel in dBm.
    pub horizontal_channel_noise_level: Real4,

    /// Noise level for the vertical channel in dBm.
    pub vertical_channel_noise_level: Real4,

    /// Nyquist velocity in m/s.
    pub nyquist_velocity: ScaledInteger2,

    /// Radial flags to support RPG processing.
    pub radial_flags: Integer2,

    /// Calibration constant for the horizontal channel in dBZ.
    pub horizontal_channel_calibration_constant: Real4,

    /// Calibration constant for the vertical channel in dBZ.
    pub vertical_channel_calibration_constant: Real4,
}
