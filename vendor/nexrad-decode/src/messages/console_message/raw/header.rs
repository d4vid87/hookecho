use crate::messages::primitive_aliases::Integer2;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header for a console message to be read directly from the Archive II file.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// The size of the console message in bytes/characters (range 2-404).
    pub message_size: Integer2,
}
