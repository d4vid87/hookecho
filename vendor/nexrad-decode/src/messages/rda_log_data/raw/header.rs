use crate::messages::primitive_aliases::Integer4;
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Header for an RDA log data message to be read directly from the Archive II file.
///
/// This corresponds to ICD Table XIVV, halfwords 0-33. The total header size is 68 bytes.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// Version number of this log message format (1-10000).
    pub version: Integer4,

    /// The log file name/identifier as a fixed-length string (e.g. "AzServoLog").
    /// 26 bytes (13 halfwords).
    pub identifier: [u8; 26],

    /// Version number of the data payload (1-10000).
    pub data_version: Integer4,

    /// The compression type used for the log data: 0 = Uncompressed, 1 = GZIP,
    /// 2 = BZIP2, 3 = ZIP.
    pub compression_type: Integer4,

    /// The size of the compressed log data in bytes.
    pub compressed_size: Integer4,

    /// The size of the log data when decompressed in bytes.
    pub decompressed_size: Integer4,

    /// Spare bytes reserved for future use (22 bytes / 11 halfwords).
    pub spare: [u8; 22],
}
