use crate::messages::clutter_filter_map::AzimuthSegment;
use crate::messages::primitive_aliases::Integer1;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;

/// A segment of the clutter filter map for a specific elevation containing azimuth segments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ElevationSegment<'a> {
    /// This elevation segment's number from 1 to 5 (oftentimes there are only 2) in increasing
    /// elevation from the ground.
    elevation_segment_number: Integer1,

    /// The azimuth segments defined in this elevation segment.
    azimuth_segments: Vec<AzimuthSegment<'a>>,
}

impl<'a> ElevationSegment<'a> {
    /// Parse an elevation segment (expected to be the specified number) from the reader.
    pub(crate) fn parse(
        reader: &mut SegmentedSliceReader<'a, '_>,
        segment_number: u8,
    ) -> Result<Self> {
        let mut elevation_segment = ElevationSegment {
            elevation_segment_number: segment_number,
            azimuth_segments: Vec::with_capacity(360),
        };

        for azimuth_number in 0..360 {
            // Check if we have enough data for at least the header (2 bytes)
            // This allows graceful handling of truncated CFM data
            if reader.remaining_total() < 2 {
                break;
            }

            let azimuth_segment = AzimuthSegment::parse(reader, azimuth_number)?;
            elevation_segment.azimuth_segments.push(azimuth_segment);
        }

        Ok(elevation_segment)
    }

    /// This elevation segment's number from 1 to 5 (oftentimes there are only 2) in increasing
    /// elevation from the ground.
    pub fn elevation_segment_number(&self) -> u8 {
        self.elevation_segment_number
    }

    /// The azimuth segments defined in this elevation segment.
    pub fn azimuth_segments(&self) -> &[AzimuthSegment<'a>] {
        &self.azimuth_segments
    }

    /// Convert this segment to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> ElevationSegment<'static> {
        ElevationSegment {
            elevation_segment_number: self.elevation_segment_number,
            azimuth_segments: self
                .azimuth_segments
                .into_iter()
                .map(|s| s.into_owned())
                .collect(),
        }
    }
}
