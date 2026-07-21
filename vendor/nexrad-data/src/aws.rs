//! AWS data access for NEXRAD weather radar files.
//!
//! This module provides functions to download NEXRAD data from AWS Open Data buckets
//! populated by NOAA. Both historical archive data and real-time streaming data are available.
//!
//! # Data Sources
//!
//! - `archive` - Historical volume files from the NEXRAD Level II archive
//! - `realtime` - Live radar data with chunk-based streaming
//!
//! # AWS Buckets
//!
//! | Bucket | ARN | Description |
//! |--------|-----|-------------|
//! | Archive | `arn:aws:s3:::unidata-nexrad-level2` | Historical volumes |
//! | Real-time | `arn:aws:s3:::unidata-nexrad-level2-chunks` | Live data chunks |
//!
//! # Resources
//!
//! - [AWS Open Data NOAA NEXRAD](https://registry.opendata.aws/noaa-nexrad/)
//! - [AWS Labs Documentation](https://github.com/awslabs/open-data-docs/tree/main/docs/noaa/noaa-nexrad)

pub mod archive;
pub mod realtime;

mod client;
mod s3;
