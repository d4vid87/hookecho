use crate::messages::primitive_aliases::Integer2;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header for a loopback test message to be read directly from the Archive II file.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// The size of the loopback message in halfwords (range 2-1200), not including the message
    /// header. Halfword 1 is the size itself, and the remaining halfwords contain the bit pattern.
    pub message_size: Integer2,
}
