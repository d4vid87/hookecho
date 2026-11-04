//!
//! This module contains models representing digital radar data collected by the NEXRAD weather
//! radar network. These models and their APIs are intended to be ergonomic, understandable, and
//! performant. They do not exactly match the encoded structure from common archival formats.
//!
//! Optionally, the `uom` feature provides APIs that use the `uom` crate for type-safe units of
//! measure.
//!

mod sweep;
pub use sweep::*;

mod scan;
pub use scan::*;

mod radial;
pub use radial::*;

mod moment;
pub use moment::*;

mod pulse_width;
pub use pulse_width::*;

mod waveform_type;
pub use waveform_type::*;

mod channel_configuration;
pub use channel_configuration::*;

mod elevation_cut;
pub use elevation_cut::*;

mod vcp_number;
pub use vcp_number::*;

mod volume_coverage_pattern;
pub use volume_coverage_pattern::*;

mod product;
pub use product::*;

mod field;
pub use field::*;
