//!
//! Message type 6 "RDA Control Commands" contains commands sent from the RPG to control the RDA
//! system's state, scanning strategy, and various operational parameters such as power generator
//! control, super resolution, clutter mitigation, and spot blanking.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
