use crate::result::{Error, Result};
use zerocopy::FromBytes;

/// A reader that tracks position across multiple non-contiguous byte slices.
///
/// This is used for decoding segmented messages where the payload spans multiple
/// fixed-length segments. Each segment may have padding at the end that is not
/// part of the logical message data.
///
/// The NEXRAD protocol is designed such that data structures do not span segment
/// boundaries, so `take_ref` and `take_slice` will error if the requested data
/// would cross a boundary.
#[derive(Debug, Clone)]
pub struct SegmentedSliceReader<'a, 'seg> {
    /// The segment payloads (body data, excluding headers).
    segments: &'seg [&'a [u8]],
    /// Current segment index.
    current_segment: usize,
    /// Position within current segment.
    position_in_segment: usize,
}

impl<'a, 'seg> SegmentedSliceReader<'a, 'seg> {
    /// Creates a new SegmentedSliceReader from a slice of segment payloads.
    pub fn new(segments: &'seg [&'a [u8]]) -> Self {
        Self {
            segments,
            current_segment: 0,
            position_in_segment: 0,
        }
    }

    /// Returns the remaining bytes in the current segment.
    pub fn remaining_in_current_segment(&self) -> usize {
        if self.current_segment >= self.segments.len() {
            return 0;
        }
        self.segments[self.current_segment].len() - self.position_in_segment
    }

    /// Returns the total remaining bytes across all segments.
    pub fn remaining_total(&self) -> usize {
        if self.current_segment >= self.segments.len() {
            return 0;
        }
        let mut remaining = self.remaining_in_current_segment();
        for i in (self.current_segment + 1)..self.segments.len() {
            remaining += self.segments[i].len();
        }
        remaining
    }

    /// Returns the remaining unread bytes in the current segment.
    fn current_remaining(&self) -> &'a [u8] {
        if self.current_segment >= self.segments.len() {
            return &[];
        }
        &self.segments[self.current_segment][self.position_in_segment..]
    }

    /// Advances the position by `n` bytes, crossing segment boundaries as needed.
    pub fn advance(&mut self, mut n: usize) {
        while n > 0 && self.current_segment < self.segments.len() {
            let remaining = self.remaining_in_current_segment();
            if n >= remaining {
                n -= remaining;
                self.current_segment += 1;
                self.position_in_segment = 0;
            } else {
                self.position_in_segment += n;
                n = 0;
            }
        }
    }

    /// Reads `count` bytes across segment boundaries into an owned `Vec<u8>`.
    ///
    /// Unlike [`take_slice`](Self::take_slice), this method handles data that spans
    /// multiple segments by copying bytes into a contiguous buffer. Use this for
    /// large data fields (e.g. Clutter Filter Bypass Map range bins) that may not
    /// fit in a single segment.
    pub fn read_bytes_owned(&mut self, count: usize) -> Result<Vec<u8>> {
        if self.remaining_total() < count {
            return Err(Error::UnexpectedEof);
        }

        let mut data = Vec::with_capacity(count);
        let mut remaining_to_read = count;

        while remaining_to_read > 0 {
            let available = self.remaining_in_current_segment();
            if available == 0 {
                self.advance_to_next_segment();
                continue;
            }

            let to_read = remaining_to_read.min(available);
            let chunk = self.take_slice::<u8>(to_read)?;
            data.extend_from_slice(chunk);
            remaining_to_read -= to_read;
        }

        Ok(data)
    }

    /// Advances to the start of the next segment, skipping any remaining bytes
    /// in the current segment (padding).
    pub fn advance_to_next_segment(&mut self) {
        if self.current_segment < self.segments.len() {
            self.current_segment += 1;
            self.position_in_segment = 0;
        }
    }

    /// Returns a typed reference to the next `T` and advances the reader past it.
    ///
    /// If there's not enough room in the current segment for `T`, but there is
    /// enough data in the next segment, this will skip any remaining bytes in
    /// the current segment (padding) and read from the next segment.
    pub fn take_ref<T>(&mut self) -> Result<&'a T>
    where
        T: FromBytes + zerocopy::KnownLayout + zerocopy::Immutable,
    {
        let size = size_of::<T>();

        // If not enough room in current segment, try to advance to next segment
        if self.remaining_in_current_segment() < size {
            if self.remaining_total() >= size {
                // Skip padding in current segment and move to next
                self.advance_to_next_segment();
            } else {
                return Err(Error::UnexpectedEof);
            }
        }

        let remaining = self.current_remaining();
        let (v, rest) = T::ref_from_prefix(remaining).map_err(|_e| Error::UnexpectedEof)?;
        self.advance(remaining.len() - rest.len());
        Ok(v)
    }

    /// Returns a typed slice of `count` elements of `T` and advances the reader past them.
    ///
    /// If there's not enough room in the current segment for the slice, but there is
    /// enough data in the next segment, this will skip any remaining bytes in
    /// the current segment (padding) and read from the next segment.
    pub fn take_slice<T>(&mut self, count: usize) -> Result<&'a [T]>
    where
        T: FromBytes + zerocopy::KnownLayout + zerocopy::Immutable,
    {
        let needed = count * size_of::<T>();

        // If not enough room in current segment, try to advance to next segment
        if self.remaining_in_current_segment() < needed {
            if self.remaining_total() >= needed {
                // Skip padding in current segment and move to next
                self.advance_to_next_segment();
            } else {
                return Err(Error::UnexpectedEof);
            }
        }

        let remaining = self.current_remaining();
        let (slice, rest) = <[T]>::ref_from_prefix_with_elems(remaining, count)
            .map_err(|_e| Error::UnexpectedEof)?;
        self.advance(remaining.len() - rest.len());
        Ok(slice)
    }
}

#[cfg(test)]
impl<'a, 'seg> SegmentedSliceReader<'a, 'seg> {
    /// Returns the total position across all segments (bytes read so far).
    pub fn position(&self) -> usize {
        let mut pos = 0;
        for i in 0..self.current_segment {
            pos += self.segments[i].len();
        }
        pos + self.position_in_segment
    }

    /// Returns a byte slice of `count` bytes and advances the reader past them.
    ///
    /// Returns an error if the bytes would span a segment boundary.
    pub fn take_bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        let remaining = self.current_remaining();
        if remaining.len() < count {
            // Check if this is a boundary issue or just EOF
            if self.remaining_total() >= count {
                return Err(Error::DataSpansSegmentBoundary {
                    position: self.position(),
                });
            } else {
                return Err(Error::UnexpectedEof);
            }
        }
        let bytes = &remaining[..count];
        self.advance(count);
        Ok(bytes)
    }

    /// Returns the current segment index (0-based).
    pub fn current_segment_index(&self) -> usize {
        self.current_segment
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_segment() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let segments = [data.as_slice()];
        let mut reader = SegmentedSliceReader::new(&segments);

        assert_eq!(reader.position(), 0);
        assert_eq!(reader.remaining_total(), 8);

        let bytes = reader.take_bytes(4).unwrap();
        assert_eq!(bytes, &[1, 2, 3, 4]);
        assert_eq!(reader.position(), 4);

        let bytes = reader.take_bytes(4).unwrap();
        assert_eq!(bytes, &[5, 6, 7, 8]);
        assert_eq!(reader.position(), 8);
        assert_eq!(reader.remaining_total(), 0);
    }

    #[test]
    fn test_multiple_segments_advance() {
        let seg1 = [1u8, 2, 3, 4];
        let seg2 = [5u8, 6, 7, 8];
        let segments = [seg1.as_slice(), seg2.as_slice()];
        let mut reader = SegmentedSliceReader::new(&segments);

        assert_eq!(reader.remaining_total(), 8);

        // Read from first segment
        let bytes = reader.take_bytes(2).unwrap();
        assert_eq!(bytes, &[1, 2]);

        // Advance past rest of first segment into second
        reader.advance(3); // 2 remaining in seg1 + 1 into seg2
        assert_eq!(reader.position(), 5);
        assert_eq!(reader.current_segment_index(), 1);

        let bytes = reader.take_bytes(3).unwrap();
        assert_eq!(bytes, &[6, 7, 8]);
    }

    #[test]
    fn test_advance_to_next_segment() {
        let seg1 = [1u8, 2, 3, 4];
        let seg2 = [5u8, 6, 7, 8];
        let segments = [seg1.as_slice(), seg2.as_slice()];
        let mut reader = SegmentedSliceReader::new(&segments);

        reader.take_bytes(2).unwrap();
        reader.advance_to_next_segment();

        assert_eq!(reader.current_segment_index(), 1);
        assert_eq!(reader.position(), 4); // Full first segment

        let bytes = reader.take_bytes(4).unwrap();
        assert_eq!(bytes, &[5, 6, 7, 8]);
    }

    #[test]
    fn test_spans_boundary_error() {
        let seg1 = [1u8, 2];
        let seg2 = [3u8, 4];
        let segments = [seg1.as_slice(), seg2.as_slice()];
        let mut reader = SegmentedSliceReader::new(&segments);

        // Try to read 4 bytes when only 2 are in current segment
        let result = reader.take_bytes(4);
        assert!(matches!(
            result,
            Err(Error::DataSpansSegmentBoundary { .. })
        ));
    }

    #[test]
    fn test_eof_error() {
        let data = [1u8, 2];
        let segments = [data.as_slice()];
        let mut reader = SegmentedSliceReader::new(&segments);

        // Try to read more than available
        let result = reader.take_bytes(4);
        assert!(matches!(result, Err(Error::UnexpectedEof)));
    }
}
