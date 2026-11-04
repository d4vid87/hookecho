use crate::messages::loopback_test::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// A loopback test message (types 11 and 12) used to test the RDA/RPG wideband interface.
///
/// The message consists of a size field followed by a variable-length bit pattern of 0's and 1's.
/// Type 11 originates from the RDA and type 12 originates from the RPG. The RPG sends a type 12
/// message and expects to receive a matching type 11 message back from the RDA.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Message<'a> {
    header: Cow<'a, raw::Header>,
    bit_pattern_data: Cow<'a, [u8]>,
}

impl<'a> Message<'a> {
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<raw::Header>()?;

        // message_size is in halfwords. Halfword 1 is the size field itself, so the bit pattern
        // occupies (message_size - 1) halfwords = (message_size - 1) * 2 bytes.
        let halfwords = header.message_size.get() as usize;
        let pattern_bytes = halfwords.saturating_sub(1) * 2;
        let bit_pattern_data = reader.take_slice::<u8>(pattern_bytes)?;

        Ok(Self {
            header: Cow::Borrowed(header),
            bit_pattern_data: Cow::Borrowed(bit_pattern_data),
        })
    }

    /// The total message size in halfwords (range 2-1200), not including the message header.
    pub fn message_size(&self) -> u16 {
        self.header.message_size.get()
    }

    /// The bit pattern data used to test the interface.
    pub fn bit_pattern(&self) -> &[u8] {
        &self.bit_pattern_data
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: Cow::Owned(self.header.into_owned()),
            bit_pattern_data: Cow::Owned(self.bit_pattern_data.into_owned()),
        }
    }
}
