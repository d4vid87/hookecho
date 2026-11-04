use crate::messages::clutter_filter_bypass_map::elevation_segment::{
    ElevationSegment, RANGE_BIN_BYTES_PER_SEGMENT,
};
use crate::messages::clutter_filter_bypass_map::raw::Header;
use crate::messages::primitive_aliases::Integer2;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use crate::util::get_datetime;
use chrono::{DateTime, Duration, Utc};
use std::borrow::Cow;
use std::fmt::Debug;

/// A clutter filter bypass map describing which range bins should bypass clutter filtering for
/// each elevation, azimuth, and range bin. The RDA transmits this any time the map changes.
///
/// This message's contents correspond to ICD 2620002AA section 3.2.4.13 Table IX.
/// The message starts with a brief header followed by elevation segments, each containing
/// 360 radials of 512 range bin flags.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Message<'a> {
    /// Decoded header information for this clutter filter bypass map.
    header: Cow<'a, Header>,

    /// The elevation segments defined in this clutter filter bypass map.
    elevation_segments: Vec<ElevationSegment<'a>>,
}

impl<'a> Message<'a> {
    /// Parse a clutter filter bypass map message from segmented input.
    ///
    /// Clutter filter bypass maps span multiple fixed-length segments. The data is read
    /// across all segment payloads using the SegmentedSliceReader.
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<Header>()?;

        let segment_count = header.number_of_elevation_segments.get() as usize;
        let mut elevation_segments = Vec::with_capacity(segment_count);

        for _ in 0..segment_count {
            let segment_number_ref = reader.take_ref::<Integer2>()?;
            let segment_number = segment_number_ref.get();

            // Range bin data (23040 bytes per elevation) spans multiple segments,
            // so we must read across boundaries into an owned buffer.
            let range_bin_data = reader.read_bytes_owned(RANGE_BIN_BYTES_PER_SEGMENT)?;
            elevation_segments.push(ElevationSegment::from_owned(segment_number, range_bin_data));
        }

        Ok(Message {
            header: Cow::Borrowed(header),
            elevation_segments,
        })
    }

    /// The date the bypass map was generated represented as a count of days since 1 January 1970
    /// 00:00 GMT. It is also referred-to as a "modified Julian date" where it is the Julian
    /// date - 2440586.5.
    pub fn bypass_map_generation_date(&self) -> u16 {
        self.header.bypass_map_generation_date.get()
    }

    /// The time the bypass map was generated in minutes past midnight, GMT.
    pub fn bypass_map_generation_time(&self) -> u16 {
        self.header.bypass_map_generation_time.get()
    }

    /// The number of elevation segments defined in this clutter filter bypass map. There may be
    /// 1 to 5 segments.
    pub fn number_of_elevation_segments(&self) -> u16 {
        self.header.number_of_elevation_segments.get()
    }

    /// The date and time the clutter filter bypass map was generated.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.header.bypass_map_generation_date.get(),
            Duration::minutes(self.header.bypass_map_generation_time.get() as i64),
        )
    }

    /// The elevation segments defined in this clutter filter bypass map.
    pub fn elevation_segments(&self) -> &[ElevationSegment<'a>] {
        &self.elevation_segments
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: Cow::Owned(self.header.into_owned()),
            elevation_segments: self
                .elevation_segments
                .into_iter()
                .map(|s| s.into_owned())
                .collect(),
        }
    }
}
