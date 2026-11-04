//!
//! Message type 32 "RDA PRF Data" contains pulse repetition frequency (PRF) data for each
//! waveform type used by the radar. Each waveform section includes a waveform type identifier
//! and the set of PRF values used for that waveform.
//!

mod waveform_prf_data;
pub use waveform_prf_data::WaveformPrfData;

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
