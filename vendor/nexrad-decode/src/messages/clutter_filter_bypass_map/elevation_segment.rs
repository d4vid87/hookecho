use std::borrow::Cow;

/// The number of azimuth radials per elevation segment.
pub const RADIALS_PER_SEGMENT: usize = 360;

/// The number of Code2 halfwords per radial (512 range bins / 16 bits per halfword = 32).
pub const HALFWORDS_PER_RADIAL: usize = 32;

/// The total number of range bin bytes per elevation segment (360 radials * 32 halfwords * 2 bytes).
pub const RANGE_BIN_BYTES_PER_SEGMENT: usize = RADIALS_PER_SEGMENT * HALFWORDS_PER_RADIAL * 2;

/// An elevation segment of the clutter filter bypass map containing range bin filter flags for 360
/// azimuth radials.
///
/// Each elevation segment consists of a segment number followed by 360 radials of range bin data.
/// Each radial has 32 halfwords (512 range bins), where each bit represents a single range bin:
/// 0 = perform clutter filtering, 1 = bypass clutter filtering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ElevationSegment<'a> {
    /// The segment number for this elevation segment.
    segment_number: u16,

    /// Raw range bin data: 360 radials * 32 halfwords * 2 bytes = 23040 bytes.
    /// Each halfword contains 16 range bin flags as individual bits.
    range_bins: Cow<'a, [u8]>,
}

impl<'a> ElevationSegment<'a> {
    /// Creates a new elevation segment with owned range bin data.
    pub(crate) fn from_owned(segment_number: u16, range_bins: Vec<u8>) -> Self {
        ElevationSegment {
            segment_number,
            range_bins: Cow::Owned(range_bins),
        }
    }

    /// The segment number for this elevation segment.
    pub fn segment_number(&self) -> u16 {
        self.segment_number
    }

    /// The raw range bin data as a byte slice. This contains 360 radials * 32 halfwords * 2 bytes
    /// = 23040 bytes. Each halfword contains 16 range bin flags as individual bits where
    /// 0 = perform clutter filtering and 1 = bypass clutter filtering.
    pub fn range_bins(&self) -> &[u8] {
        &self.range_bins
    }

    /// Returns the bypass flag for a specific radial and range bin.
    ///
    /// `radial` is the azimuth radial index (0-359).
    /// `range_bin` is the range bin index (0-511).
    ///
    /// Returns `true` if the clutter filter should be bypassed for this range bin,
    /// `false` if clutter filtering should be performed.
    ///
    /// Returns `None` if the radial or range_bin index is out of range.
    pub fn bypass_flag(&self, radial: usize, range_bin: usize) -> Option<bool> {
        if radial >= RADIALS_PER_SEGMENT || range_bin >= HALFWORDS_PER_RADIAL * 16 {
            return None;
        }

        let halfword_index = range_bin / 16;
        let bit_index = range_bin % 16;

        // Each radial has 32 halfwords (64 bytes)
        let byte_offset = (radial * HALFWORDS_PER_RADIAL * 2) + (halfword_index * 2);

        // Read the big-endian halfword
        let hi = self.range_bins[byte_offset] as u16;
        let lo = self.range_bins[byte_offset + 1] as u16;
        let halfword = (hi << 8) | lo;

        Some((halfword >> bit_index) & 1 == 1)
    }

    /// Convert this segment to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> ElevationSegment<'static> {
        ElevationSegment {
            segment_number: self.segment_number,
            range_bins: Cow::Owned(self.range_bins.into_owned()),
        }
    }
}
