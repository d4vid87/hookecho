use crate::messages::primitive_aliases::Code2;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// A request for data message to be read directly from the Archive II file.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Message {
    /// The data request type bitfield.
    ///
    /// Values:
    ///   129 (bits 0&7) = Request Summary RDA Status
    ///   130 (bits 1&7) = Request RDA Performance/Maintenance Data
    ///   132 (bits 2&7) = Request Clutter Filter Bypass Map
    ///   136 (bits 3&7) = Request Clutter Filter Map
    ///   144 (bits 4&7) = Request RDA Adaptation Data
    ///   160 (bits 5&7) = Request Volume Coverage Pattern Data
    pub data_request_type: Code2,
}
