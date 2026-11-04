//!
//! Message type 31 "Digital Radar Data" consists of base data information such as reflectivity,
//! mean radial velocity, spectrum width, differential reflectivity, differential phase, correlation
//! coefficient, azimuth angle, elevation angle, cut type, scanning strategy, and calibration
//! parameters. The frequency and volume of the message is dependent on the scanning strategy and
//! the type of data associated with that strategy.
//!

mod azimuth_resolution_spacing;
pub use azimuth_resolution_spacing::AzimuthResolutionSpacing;

mod compression_indicator;
pub use compression_indicator::CompressionIndicator;

mod control_flags;
pub use control_flags::ControlFlags;

mod data_block_name;
pub use data_block_name::DataBlockName;

mod data_block_type;
pub use data_block_type::DataBlockType;

mod data_word_size;
pub use data_word_size::DataWordSize;

mod processing_status;
pub use processing_status::ProcessingStatus;

mod radial_status;
pub use radial_status::RadialStatus;

mod volume_coverage_pattern;
pub use volume_coverage_pattern::VolumeCoveragePattern;

mod spot_blanking_status;
pub use spot_blanking_status::SpotBlankingStatus;

mod pointers;
pub use pointers::*;

mod data_block_id;
pub use data_block_id::DataBlockId;

mod header;
pub use header::Header;

mod elevation_data_block;
pub use elevation_data_block::ElevationDataBlock;

mod radial_data_block;
pub use radial_data_block::RadialDataBlock;

mod volume_data_block;
pub use volume_data_block::VolumeDataBlock;

mod generic_data_block_header;
pub use generic_data_block_header::GenericDataBlockHeader;

mod data_block;
pub use data_block::DataBlock;

mod generic_data_block;
pub use generic_data_block::GenericDataBlock;

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
