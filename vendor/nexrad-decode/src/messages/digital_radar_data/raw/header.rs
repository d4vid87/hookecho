use crate::messages::primitive_aliases::{
    Code1, Integer1, Integer2, Integer4, Real4, ScaledInteger1,
};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// The digital radar data message header block precedes base data information for a particular
/// radial and includes parameters for that radial and information about the following data blocks.
#[repr(C)]
#[derive(Clone, PartialEq, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// ICAO radar identifier.
    pub radar_identifier: [u8; 4],

    /// Collection time in milliseconds past midnight, GMT.
    pub time: Integer4,

    /// This message's date represented as a count of days since 1 January 1970 00:00 GMT. It is
    /// also referred-to as a "modified Julian date" where it is the Julian date - 2440586.5.
    pub date: Integer2,

    /// Radial number within the elevation scan. These range up to 720, in 0.5 degree increments.
    pub azimuth_number: Integer2,

    /// Azimuth angle at which the radial was collected in degrees.
    pub azimuth_angle: Real4,

    /// Indicates if the message is compressed and what type of compression was used. This header is
    /// not compressed.
    ///
    /// Values:
    ///   0 = Uncompressed
    ///   1 = Compressed using BZIP2
    ///   2 = Compressed using ZLIB
    ///   3 = Future use
    pub compression_indicator: Code1,

    /// Spare to force halfword alignment.
    pub spare: u8,

    /// Uncompressed length of the radial in bytes (including the data header block).
    pub radial_length: Integer2,

    /// Azimuthal spacing between adjacent radials. Note this is the commanded value, not
    /// necessarily the actual spacing.
    ///
    /// Values:
    ///   1 = 0.5 degrees
    ///   2 = 1.0 degrees
    pub azimuth_resolution_spacing: Code1,

    /// The radial's status within the larger scan (e.g. first, last).
    ///
    /// Statuses:
    ///   0 = Start of elevation
    ///   1 = Intermediate radial data
    ///   2 = End of elevation
    ///   3 = Start of volume scan
    ///   4 = End of volume scan
    ///   5 = Start of new elevation which is the last in the VCP
    pub radial_status: Code1,

    /// The radial's elevation number within the volume scan.
    pub elevation_number: Integer1,

    /// The sector number within cut. A value of 0 is only valid for continuous surveillance cuts.
    pub cut_sector_number: Integer1,

    /// The radial's collection elevation angle.
    pub elevation_angle: Real4,

    /// The spot blanking status for the current radial, elevation, and volume scan.
    ///
    /// Statuses:
    ///   0 = None
    ///   1 = Radial
    ///   2 = Elevation
    ///   4 = Volume
    pub radial_spot_blanking_status: Code1,

    /// The azimuth indexing value (if keyed to constant angles).
    ///
    /// Values:
    ///   0     = No indexing
    ///   1-100 = Indexing angle of 0.01 to 1.00 degrees
    pub azimuth_indexing_mode: ScaledInteger1,

    /// The number of "data moment" blocks following this header block, from 4 to 10. There are
    /// always volume, elevation, and radial information blocks and a reflectivity data moment
    /// block. The following 6 data moment blocks are optional, depending on scanning mode. The next
    /// 10 fields on this header contain pointers to each block, if available in the message.
    pub data_block_count: Integer2,
}
