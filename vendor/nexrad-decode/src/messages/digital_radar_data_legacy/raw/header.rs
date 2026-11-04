use crate::binary_data::BinaryData;
use crate::messages::primitive_aliases::{Integer2, Integer4, Real4, SInteger2};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Raw header for Message Type 1 "Digital Radar Data" (pre-Build 10.0 format).
///
/// This is the fixed 100-byte header portion of the 2416-byte message body.
/// It describes a single radial of base data (reflectivity, velocity, spectrum
/// width) using fixed gate arrays rather than the variable-length data block
/// pointers used by Message Type 31.
///
/// Gate data follows immediately after this header at byte offset 100 of the
/// message body, laid out as contiguous 1-byte-per-gate arrays for each
/// available moment (reflectivity, then velocity, then spectrum width).
///
/// # ICD Reference
/// RDA/RPG ICD Table III-A "Digital Radar Data (Message Type 1)".
#[repr(C)]
#[derive(Clone, PartialEq, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// Collection time in milliseconds past midnight GMT.
    pub collection_time: Integer4,

    /// Modified Julian date (days since 1 January 1970).
    pub modified_julian_date: Integer2,

    /// Unambiguous range in units of 100 meters (divide by 10 for km).
    pub unambiguous_range: Integer2,

    /// Azimuth angle as a coded value. Degrees = value * 180 / 32768.
    pub azimuth_angle: Integer2,

    /// Azimuth number within the current elevation (1-indexed).
    pub azimuth_number: Integer2,

    /// Radial status indicator.
    ///
    /// - 0 = Start of new elevation
    /// - 1 = Intermediate radial
    /// - 2 = End of elevation
    /// - 3 = Beginning of volume scan
    /// - 4 = End of volume scan
    pub radial_status: Integer2,

    /// Elevation angle as a coded value. Degrees = value * 180 / 32768.
    pub elevation_angle: Integer2,

    /// Elevation number within the volume scan (1-indexed).
    pub elevation_number: Integer2,

    /// Range to first surveillance (reflectivity) gate in meters.
    pub surveillance_first_gate_range: SInteger2,

    /// Range to first Doppler (velocity/spectrum width) gate in meters.
    pub doppler_first_gate_range: SInteger2,

    /// Surveillance gate spacing in meters.
    pub surveillance_gate_interval: Integer2,

    /// Doppler gate spacing in meters.
    pub doppler_gate_interval: Integer2,

    /// Number of surveillance (reflectivity) gates (0-460).
    pub num_surveillance_gates: Integer2,

    /// Number of Doppler (velocity/spectrum width) gates (0-920).
    pub num_doppler_gates: Integer2,

    /// Sector number (1-3).
    pub sector_number: Integer2,

    /// System gain calibration constant in dB.
    pub calibration_constant: Real4,

    /// Byte offset from start of message body to reflectivity gate data.
    /// Zero if reflectivity data is not present.
    pub reflectivity_pointer: Integer2,

    /// Byte offset from start of message body to velocity gate data.
    /// Zero if velocity data is not present.
    pub velocity_pointer: Integer2,

    /// Byte offset from start of message body to spectrum width gate data.
    /// Zero if spectrum width data is not present.
    pub spectrum_width_pointer: Integer2,

    /// Doppler velocity resolution.
    /// - 2 = 0.5 m/s
    /// - 4 = 1.0 m/s
    pub doppler_velocity_resolution: Integer2,

    /// Volume Coverage Pattern number.
    pub vcp_number: Integer2,

    /// Spare bytes (ICD halfwords 24-50, 27 halfwords = 54 bytes).
    pub spare: BinaryData<[u8; 54]>,
}
