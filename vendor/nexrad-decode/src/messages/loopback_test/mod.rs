//!
//! Message types 11 and 12 "Loopback Test" are used to test the RDA/RPG wideband communication
//! interface. The message contains a size field followed by a variable-length bit pattern. Type 11
//! originates from the RDA and type 12 originates from the RPG.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
