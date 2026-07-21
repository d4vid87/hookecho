//! Data access for NEXRAD weather radar files.
//!
//! This crate provides tools for working with NEXRAD Archive II volume files and
//! accessing data from cloud providers like AWS.
//!
//! # Overview
//!
//! - [`volume`] - Parse and decode local Archive II volume files
//! - `aws` - Download data from AWS (archive and real-time, requires `aws` feature)
//!
//! # Working with Local Files
//!
//! ```ignore
//! use nexrad_data::volume::File;
//!
//! let data = std::fs::read("KTLX20130520_201643_V06.ar2v")?;
//! let volume = File::new(data);
//!
//! // Access the volume header
//! if let Some(header) = volume.header() {
//!     println!("Volume date: {:?}", header.date());
//! }
//!
//! // Process LDM records
//! for record in volume.records()? {
//!     let decompressed = record.decompress()?;
//!     let messages = decompressed.messages()?;
//!     // Process radar messages...
//! }
//! ```
//!
//! # Downloading from AWS
//!
//! The `aws` feature (enabled by default) provides access to NOAA's NEXRAD data on AWS:
//!
//! - **Archive data**: Historical volumes from the NEXRAD Level II archive
//! - **Real-time data**: Live radar data with minimal latency
//!
//! ```ignore
//! use nexrad_data::aws::archive;
//! use chrono::NaiveDate;
//!
//! // List available files for a date and site
//! let date = NaiveDate::from_ymd_opt(2023, 5, 20).unwrap();
//! let files = archive::list_files("KTLX", date).await?;
//!
//! // Download a specific file
//! let data = archive::download_file(&files[0]).await?;
//! ```
//!
//! # Features
//!
//! - `aws` - Enable AWS S3 data access (requires async runtime)
//! - `nexrad-model` - Enable conversion to high-level `Scan` model
//! - `parallel` - Parallelize LDM record decompression and decoding using Rayon
//!
//! # Crate Boundaries
//!
//! This crate provides **data access and I/O operations** with the following
//! responsibilities and constraints:
//!
//! ## Responsibilities
//!
//! - ✓ Read and parse Archive II volume files
//! - ✓ Handle file I/O operations
//! - ✓ Decompress bzip2-compressed LDM records
//! - ✓ AWS S3 integration for archive and real-time data (when `aws` feature enabled)
//! - ✓ Use `nexrad-decode` for message parsing
//! - ✓ Use `nexrad-model` for high-level type conversion (when feature enabled)
//! - ✓ Orchestrate the data pipeline from raw bytes to structured models
//!
//! ## Constraints
//!
//! - ✗ **No rendering or visualization**
//! - ✗ **No CLI or user interaction**
//!
//! This crate serves as the **I/O boundary layer** for the NEXRAD library suite. It
//! handles all file and network operations, decompression, and coordinates between
//! the low-level binary parsing (`nexrad-decode`) and high-level data models
//! (`nexrad-model`).

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![warn(clippy::correctness)]
#![deny(missing_docs)]

#[cfg(feature = "aws")]
/// AWS S3 integration for downloading NEXRAD data from the cloud.
pub mod aws;

/// Local Archive II volume file handling.
pub mod volume;

/// Result and error types for data operations.
pub mod result;
