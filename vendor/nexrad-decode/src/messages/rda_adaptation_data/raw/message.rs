use zerocopy::{FromBytes, Immutable, KnownLayout};

/// The identity header of the RDA Adaptation Data message (bytes 0-43).
///
/// This captures the file identity fields at the start of the adaptation data.
/// The remaining ~9424 bytes of adaptation data are read separately as raw bytes
/// since the message spans multiple segments.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// Name of the adaptation data file (bytes 0-11). Null-terminated string.
    pub adap_file_name: [u8; 12],

    /// Format of the adaptation data file (bytes 12-15). Null-terminated string, e.g. "14".
    pub adap_format: [u8; 4],

    /// Revision number of the adaptation data file (bytes 16-19). Null-terminated string, e.g. "20".
    pub adap_revision: [u8; 4],

    /// Last modified date of the adaptation data file (bytes 20-31). Format: "mm/dd/yy".
    pub adap_date: [u8; 12],

    /// Last modified time of the adaptation data file (bytes 32-43). Format: "hh:mm:ss".
    pub adap_time: [u8; 12],
}
