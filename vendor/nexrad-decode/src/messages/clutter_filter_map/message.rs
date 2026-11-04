use crate::messages::clutter_filter_map::elevation_segment::ElevationSegment;
use crate::messages::clutter_filter_map::raw::Header;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use crate::util::get_datetime;
use chrono::{DateTime, Duration, Utc};
use std::borrow::Cow;
use std::fmt::Debug;

/// A clutter filter map describing elevations, azimuths, and ranges containing clutter to
/// filtered from radar products. The RDA transmits this any time the map changes.
///
/// This message's contents correspond to ICD 2620002AA section 3.2.4.15 Table XIV.
/// The message starts with a brief header followed by a loop of elevation, azimuth,
/// and finally range/gate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Message<'a> {
    /// Decoded header information for this clutter filter map.
    header: Cow<'a, Header>,

    /// The elevation segments defined in this clutter filter map.
    elevation_segments: Vec<ElevationSegment<'a>>,
}

impl<'a> Message<'a> {
    /// Parse a clutter filter map message from segmented input.
    ///
    /// Clutter filter maps span multiple fixed-length segments. The data is read
    /// across all segment payloads using the SegmentedSliceReader.
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<Header>()?;

        let segment_count = header.elevation_segment_count.get() as u8;
        let mut message = Message {
            header: Cow::Borrowed(header),
            elevation_segments: Vec::with_capacity(segment_count as usize),
        };

        for segment_number in 0..segment_count {
            let segment = ElevationSegment::parse(reader, segment_number)?;
            message.elevation_segments.push(segment);
        }

        Ok(message)
    }

    /// The date the clutter filter map was generated represented as a count of days since 1 January
    /// 1970 00:00 GMT. It is also referred-to as a "modified Julian date" where it is the Julian
    /// date - 2440586.5.
    pub fn map_generation_date(&self) -> u16 {
        self.header.map_generation_date.get()
    }

    /// The time the clutter filter map was generated in minutes past midnight, GMT.
    pub fn map_generation_time(&self) -> u16 {
        self.header.map_generation_time.get()
    }

    /// The number of elevation segments defined in this clutter filter map. There may be 1 to 5,
    /// though there are typically 2. They will follow this header in order of increasing elevation.
    pub fn elevation_segment_count(&self) -> u16 {
        self.header.elevation_segment_count.get()
    }

    /// The date and time the clutter filter map was generated.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.header.map_generation_date.get(),
            Duration::minutes(self.header.map_generation_time.get() as i64),
        )
    }

    /// The elevation segments defined in this clutter filter map.
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
