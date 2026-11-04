//!
//! Message type 3 "Performance/Maintenance Data" contains detailed performance and maintenance
//! information about the RDA system, including communications status, AME parameters, power supply
//! readings, transmitter status, tower/utilities conditions, antenna/pedestal status, RF
//! generator/receiver measurements, calibration data, file status, RSP/CPU status, and device
//! communication status. This message is 480 halfwords (960 bytes).
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
