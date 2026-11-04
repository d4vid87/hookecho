use crate::messages::primitive_aliases::{Code2, Integer2};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// A single clutter censor zone override region to be read directly from the Archive II file.
///
/// Each region defines a range, azimuth, and elevation zone along with an operator select code
/// that controls how clutter filtering is applied within the zone.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Region {
    /// Start range in km (0-511).
    pub start_range: Integer2,

    /// Stop range in km (0-511).
    pub stop_range: Integer2,

    /// Start azimuth in degrees (0-360).
    pub start_azimuth: Integer2,

    /// Stop azimuth in degrees (0-360).
    pub stop_azimuth: Integer2,

    /// Elevation segment number (1-5).
    pub elevation_segment_number: Integer2,

    /// Operator select code for clutter filtering behavior.
    ///
    /// Values:
    ///   0 = Bypass filter forced
    ///   1 = Bypass map in control
    ///   2 = Clutter filtering forced
    pub operator_select_code: Code2,
}
