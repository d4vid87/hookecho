//!
//! # Archive NEXRAD Data
//! Archived NEXRAD radar data is stored in an AWS S3 bucket by NOAA. The S3 bucket's directories
//! are organized by year, month, day, and then site. For a given date and site, each object is a
//! "volume" file which contains radar data from a full scan. The volume file starts with an
//! Archive II header which is followed by some number of compressed LDM records. These records in
//! turn contain messages which represent individual radials with radar data.
//!
//! The [crate::aws::realtime] AWS bucket provides LDM records as "chunks". Those are uploaded in
//! real-time and once a full scan has been uploaded to a volume directory, those chunks are
//! combined to create a full Archive II volume file which is uploaded to this archive bucket.
//!

mod identifier;
pub use identifier::Identifier;

mod download_file;
pub use download_file::download_file;

mod list_files;
pub use list_files::list_files;

const ARCHIVE_BUCKET: &str = "unidata-nexrad-level2";
