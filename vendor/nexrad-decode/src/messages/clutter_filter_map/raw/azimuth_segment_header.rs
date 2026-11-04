use crate::messages::primitive_aliases::Integer2;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header information for an azimuth segment to be read directly from the Archive II file.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, FromBytes, Immutable, KnownLayout)]
pub struct AzimuthSegmentHeader {
    /// The number of range zones defined in this azimuth segment, from 1 to 20.
    pub range_zone_count: Integer2,
}
