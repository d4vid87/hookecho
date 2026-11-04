//!
//! Message type 13 "Clutter Filter Bypass Map" contains information about which range bins should
//! bypass clutter filtering. The bypass map is a 3D structure of elevation segments, azimuth
//! radials, and range bins where each bit indicates whether the clutter filter should be bypassed
//! for that bin.
//!

mod elevation_segment;
pub use elevation_segment::ElevationSegment;

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
