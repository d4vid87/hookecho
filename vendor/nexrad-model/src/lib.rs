//! Core data model for NEXRAD weather radar data.
//!
//! This crate provides an ergonomic API for working with NEXRAD radar data, documented
//! for users who may not be familiar with the NOAA Archive II format.
//!
//! # Overview
//!
//! The data model consists of:
//!
//! - [`data::Scan`] - A complete radar scan containing multiple sweeps
//! - [`data::Sweep`] - A single rotation at one elevation angle
//! - [`data::Radial`] - A single beam direction with moment data
//! - [`data::MomentData`] - Gate-by-gate measurements for a product
//! - [`meta::Site`] - Radar site metadata (location, identifier)
//!
//! # Example
//!
//! ```ignore
//! use nexrad_model::data::{Scan, Sweep, MomentValue};
//!
//! fn process_scan(scan: &Scan) {
//!     println!("{}", scan.coverage_pattern_number());
//!
//!     for sweep in scan.sweeps() {
//!         for radial in sweep.radials() {
//!             if let Some(reflectivity) = radial.reflectivity() {
//!                 for value in reflectivity.values() {
//!                     match value {
//!                         MomentValue::Value(dbz) => println!("dBZ: {}", dbz),
//!                         MomentValue::BelowThreshold => {},
//!                         MomentValue::RangeFolded => {},
//!                         _ => {},
//!                     }
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # Features
//!
//! Optional features provide additional functionality:
//!
//! - `uom` - Type-safe units of measure via the [`uom`](https://docs.rs/uom) crate.
//!   Enables methods like `first_gate_range()` returning `Length` instead of raw km values.
//!
//! - `serde` - Serialization/deserialization support via [`serde`](https://docs.rs/serde).
//!   All model types implement `Serialize` and `Deserialize`.
//!
//! - `chrono` - Date/time support via [`chrono`](https://docs.rs/chrono).
//!   Enables `collection_time()` returning `DateTime<Utc>` instead of raw timestamps.
//!
//! # Crate Boundaries
//!
//! This crate is a **pure data model** with the following responsibilities and constraints:
//!
//! ## Responsibilities
//!
//! - ✓ Define domain types (Scan, Sweep, Radial, Site, MomentData)
//! - ✓ Provide data transformations and validations
//! - ✓ Support optional features (serde, chrono, uom)
//!
//! ## Constraints
//!
//! - ✗ **No I/O operations** (file, network, stdio)
//! - ✗ **No binary parsing or encoding**
//! - ✗ **No rendering or visualization**
//! - ✗ **No CLI or user interaction**
//!
//! This crate focuses solely on providing ergonomic data structures for working with
//! NEXRAD radar data. All I/O, parsing, and rendering concerns are handled by separate
//! crates in the NEXRAD library suite.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![warn(clippy::correctness)]
#![allow(clippy::too_many_arguments)]
#![deny(missing_docs)]

mod binary_data;
pub use binary_data::BinaryData;

pub mod data;
pub mod geo;
pub mod meta;
pub mod result;
