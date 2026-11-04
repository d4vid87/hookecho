use crate::messages::primitive_aliases::{Code2, Integer2};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Defines a range segment of a particular elevation and azimuth with an operation type describing
/// the clutter filter map behavior for the segment.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct RangeZone {
    /// Operation code for the range zone.
    pub op_code: Code2,

    /// Stop range per zone in km. There are 20 possible zones and not all need to be defined. The
    /// last zone must have an end range of 511km.
    pub end_range: Integer2,
}
