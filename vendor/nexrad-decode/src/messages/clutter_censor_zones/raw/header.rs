use crate::messages::primitive_aliases::Integer2;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header for a Clutter Censor Zones message to be read directly from the Archive II file.
///
/// Contains the count of override regions that follow this header.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// Number of clutter map override regions (0 to 25).
    pub override_region_count: Integer2,
}
