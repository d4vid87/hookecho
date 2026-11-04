//!
//! Message type 33 "RDA Log Data" contains log file data from the RDA. Each message carries a log
//! file identified by name (e.g. "AzServoLog") along with version and compression metadata. The
//! log data payload may be uncompressed or compressed using GZIP, BZIP2, or ZIP.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
