//!
//! Message type 18 "RDA Adaptation Data" contains site-specific configuration parameters for the
//! radar system. This includes antenna parameters, site geographic location, RF path losses,
//! calibration values, temperature alarm thresholds, transmitter characteristics, and many other
//! operational parameters. The full message spans approximately 9468 bytes across multiple
//! fixed-length segments per ICD Table XV.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
