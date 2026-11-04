//!
//! Message type 9 "Request for Data" is sent by the RPG to request specific types of data from the
//! RDA. The message contains a single bitfield halfword where each bit combination identifies a
//! specific data request such as RDA status, performance data, or clutter filter maps.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
