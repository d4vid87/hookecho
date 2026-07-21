use crate::volume::util::get_datetime;
use chrono::{DateTime, Duration, Utc};
use std::fmt::Debug;
use zerocopy::{big_endian, FromBytes, Immutable, KnownLayout};

/// Header for an Archive II volume file containing metadata about the radar data. This header is
/// located at the beginning of the file.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Header {
    /// The tape's filename which indicates the version of the data. Name is in the format
    /// `AR2V0 0xx.` where `xx` indicates the version of the data.
    ///
    /// Versions:
    ///   02 = Super Resolution disabled at the RDA (pre RDA Build 12.0)
    ///   03 = Super Resolution (pre RDA Build 12.0)
    ///   04 = Recombined Super Resolution
    ///   05 = Super Resolution disabled at the RDA (RDA Build 12.0 and later)
    ///   06 = Super Resolution (RDA Build 12.0 and later)
    ///   07 = Recombined Super Resolution (RDA Build 12.0 and later)
    /// NOTE: Dual-pol data introduced in RDA Build 12.0
    tape_filename: [u8; 9],

    /// Sequential number assigned to each volume of radar data in the queue, rolling over to 001
    /// after 999.
    extension_number: [u8; 3],

    /// This volume's date represented as a count of days since 1 January 1970 00:00 GMT. It is
    /// also referred-to as a "modified Julian date" where it is the Julian date - 2440586.5.
    date: big_endian::U32,

    /// Milliseconds past midnight, GMT.
    time: big_endian::U32,

    /// The ICAO identifier of the radar site.
    icao_of_radar: [u8; 4],
}

impl Header {
    /// The tape's filename which indicates the version of the data. Name is in the format
    /// `AR2V0 0xx.` where `xx` indicates the version of the data.
    ///
    /// Versions:
    ///   02 = Super Resolution disabled at the RDA (pre RDA Build 12.0)
    ///   03 = Super Resolution (pre RDA Build 12.0)
    ///   04 = Recombined Super Resolution
    ///   05 = Super Resolution disabled at the RDA (RDA Build 12.0 and later)
    ///   06 = Super Resolution (RDA Build 12.0 and later)
    ///   07 = Recombined Super Resolution (RDA Build 12.0 and later)
    /// NOTE: Dual-pol data introduced in RDA Build 12.0
    pub fn tape_filename(&self) -> Option<String> {
        String::from_utf8(self.tape_filename.to_vec()).ok()
    }

    /// Sequential number assigned to each volume of radar data in the queue, rolling over to 001
    /// after 999.
    pub fn extension_number(&self) -> Option<String> {
        String::from_utf8(self.extension_number.to_vec()).ok()
    }

    /// Returns the date and time of the volume.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.date.get() as u16,
            Duration::milliseconds(self.time.get() as i64),
        )
    }

    /// The ICAO identifier of the radar site.
    pub fn icao_of_radar(&self) -> Option<String> {
        String::from_utf8(self.icao_of_radar.to_vec()).ok()
    }
}
