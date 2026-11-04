//!
//! Message Type 1 "Digital Radar Data" is the legacy base data format used from the original
//! WSR-88D deployment (1991) through the transition to Message Type 31 at Build 10.0 (March
//! 2008). Each message carries a single radial of reflectivity, velocity, and/or spectrum width
//! data in fixed 2432-byte frames.
//!
//! This format predates dual-polarization and uses 1-byte-per-gate encoding with fixed gate
//! spacings (1 km for reflectivity, 250 m for Doppler). It was fully replaced by the
//! variable-length Message Type 31 "Generic Format" in all RDA builds since 2008.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
