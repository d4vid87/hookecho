use crate::messages::clutter_filter_map::raw::{self, AzimuthSegmentHeader};
use crate::messages::clutter_filter_map::RangeZone;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;

/// A segment of the clutter filter map for a specific elevation and azimuth containing range zones.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AzimuthSegment<'a> {
    /// Header information for this azimuth segment. This is the portion of an azimuth segment that
    /// is read directly from the Archive II file.
    header: Cow<'a, AzimuthSegmentHeader>,

    /// This azimuth segment's number from 0 to 359. Each azimuth segment subtends a range of 1
    /// degree, e.g.: 0 degrees <= azimuth segment 0 < 1 degree.
    azimuth_segment: u16,

    /// The range zones defined in this azimuth segment.
    range_zones: Vec<RangeZone<'a>>,
}

impl<'a> AzimuthSegment<'a> {
    /// Parse an azimuth segment (expected to be the specified number) from the reader.
    pub(crate) fn parse(
        reader: &mut SegmentedSliceReader<'a, '_>,
        segment_number: u16,
    ) -> Result<Self> {
        let header = reader.take_ref::<AzimuthSegmentHeader>()?;
        let range_zone_count = header.range_zone_count.get() as usize;

        let raw_range_zones = reader.take_slice::<raw::RangeZone>(range_zone_count)?;

        let range_zones = raw_range_zones.iter().map(RangeZone::new).collect();

        Ok(AzimuthSegment {
            header: Cow::Borrowed(header),
            azimuth_segment: segment_number,
            range_zones,
        })
    }

    /// The number of range zones defined in this azimuth segment, from 1 to 20.
    pub fn range_zone_count(&self) -> u16 {
        self.header.range_zone_count.get()
    }

    /// This azimuth segment's number from 0 to 359. Each azimuth segment subtends a range of 1
    /// degree, e.g.: 0 degrees <= azimuth segment 0 < 1 degree.
    pub fn azimuth_segment(&self) -> u16 {
        self.azimuth_segment
    }

    /// The range zones defined in this azimuth segment.
    pub fn range_zones(&self) -> &[RangeZone<'a>] {
        &self.range_zones
    }

    /// Convert this segment to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> AzimuthSegment<'static> {
        AzimuthSegment {
            header: Cow::Owned(self.header.into_owned()),
            azimuth_segment: self.azimuth_segment,
            range_zones: self
                .range_zones
                .into_iter()
                .map(|rz| rz.into_owned())
                .collect(),
        }
    }
}
