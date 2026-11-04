//! Radar site metadata.
//!
//! This module contains models containing metadata about the radar data collected by the NEXRAD
//! weather network. This data may not change between radials, sweeps, or even scans, and thus it
//! is represented separately to avoid duplication in storage.
//!
//! # Static Site Registry
//!
//! The [`registry`] module provides a compile-time database of all NEXRAD radar sites.
//! Use it to look up sites by identifier or find the nearest site to a location:
//!
//! ```
//! use nexrad_model::meta::registry;
//!
//! // Look up a site by ICAO identifier
//! let site = registry::site_by_id("KTLX").unwrap();
//! println!("{}: {}, {}", site.id, site.city, site.state);
//!
//! // Find the nearest radar to a location
//! let nearest = registry::nearest_site(35.0, -97.0).unwrap();
//! println!("Nearest radar: {}", nearest.id);
//! ```

mod site;
pub use site::*;

pub mod registry;
