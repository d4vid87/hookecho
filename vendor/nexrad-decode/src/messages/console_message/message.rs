use crate::messages::console_message::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// A console message (types 4 and 10) containing free-form text sent between the RDA and RPG.
///
/// The message consists of a size field followed by a variable-length text payload of up to 404
/// bytes. Type 4 messages originate from the RDA and type 10 messages originate from the RPG.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Message<'a> {
    header: Cow<'a, raw::Header>,
    text_data: Cow<'a, [u8]>,
}

impl<'a> Message<'a> {
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<raw::Header>()?;
        let size = header.message_size.get() as usize;
        let text_data = reader.take_slice::<u8>(size)?;

        Ok(Self {
            header: Cow::Borrowed(header),
            text_data: Cow::Borrowed(text_data),
        })
    }

    /// The size of the console message text in bytes/characters (range 2-404).
    pub fn message_size(&self) -> u16 {
        self.header.message_size.get()
    }

    /// The raw bytes of the console message text.
    pub fn text_bytes(&self) -> &[u8] {
        &self.text_data
    }

    /// The console message text as a UTF-8 string, if valid.
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.text_data).ok()
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: Cow::Owned(self.header.into_owned()),
            text_data: Cow::Owned(self.text_data.into_owned()),
        }
    }
}
