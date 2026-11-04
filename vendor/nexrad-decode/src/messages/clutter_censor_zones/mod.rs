//!
//! Message type 8 "Clutter Censor Zones" contains override regions that control clutter filtering
//! behavior. Each region defines a range, azimuth, and elevation zone with an operator select code
//! specifying whether the bypass filter is forced, the bypass map is in control, or clutter
//! filtering is forced.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
