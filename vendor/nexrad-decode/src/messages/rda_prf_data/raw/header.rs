use crate::messages::primitive_aliases::Integer2;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header for an RDA PRF data message to be read directly from the Archive II file.
///
/// This corresponds to ICD Table XVIII, halfwords 1-2.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// The number of waveform types included in this message (1-5).
    pub number_of_waveforms: Integer2,

    /// Spare halfword, reserved for future use.
    pub spare: Integer2,
}
