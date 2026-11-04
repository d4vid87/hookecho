use crate::messages::primitive_aliases::Integer2;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header for a clutter filter bypass map to be read directly from the Archive II file.
///
/// This corresponds to ICD Table IX, halfwords 1-3.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// The date the bypass map was generated represented as a count of days since 1 January 1970
    /// 00:00 GMT. It is also referred-to as a "modified Julian date" where it is the Julian
    /// date - 2440586.5.
    pub bypass_map_generation_date: Integer2,

    /// The time the bypass map was generated in minutes past midnight, GMT.
    pub bypass_map_generation_time: Integer2,

    /// The number of elevation segments defined in this bypass map. There may be 1 to 5.
    pub number_of_elevation_segments: Integer2,
}
