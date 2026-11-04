//!
//! Message type 5 "Volume Coverage Pattern" provides details about the volume
//! coverage pattern being used. The RDA sends the Volume Coverage Pattern message
//! upon wideband connection and at the beginning of each volume scan. The volume
//! coverage pattern message includes a header which describes how the volume is being
//! collected as well as a block for each elevation cut detailing the radar settings
//! being used for that cut.
//!

mod channel_configuration;
pub use channel_configuration::ChannelConfiguration;

mod pattern_type;
pub use pattern_type::PatternType;

mod pulse_width;
pub use pulse_width::PulseWidth;

mod waveform_type;
pub use waveform_type::WaveformType;

mod header;
pub use header::Header;

mod elevation_data_block;
pub use elevation_data_block::ElevationDataBlock;

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
