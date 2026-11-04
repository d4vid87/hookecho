use crate::messages::request_for_data::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// A request for data message (type 9) sent by the RPG to request specific data from the RDA.
///
/// The message contains a single bitfield halfword indicating which types of data are being
/// requested. Each request type is identified by a combination of bit 7 and one of bits 0-5.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Message<'a> {
    inner: Cow<'a, raw::Message>,
}

impl<'a> Message<'a> {
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let inner = reader.take_ref::<raw::Message>()?;
        Ok(Self {
            inner: Cow::Borrowed(inner),
        })
    }

    /// The raw data request type bitfield value.
    pub fn raw_data_request_type(&self) -> u16 {
        self.inner.data_request_type.get()
    }

    /// Whether a summary RDA status is requested (bits 0 and 7 set, value 129).
    pub fn requests_rda_status(&self) -> bool {
        self.inner.data_request_type.get() & 0x81 == 0x81
    }

    /// Whether RDA performance/maintenance data is requested (bits 1 and 7 set, value 130).
    pub fn requests_performance_maintenance_data(&self) -> bool {
        self.inner.data_request_type.get() & 0x82 == 0x82
    }

    /// Whether the clutter filter bypass map is requested (bits 2 and 7 set, value 132).
    pub fn requests_clutter_filter_bypass_map(&self) -> bool {
        self.inner.data_request_type.get() & 0x84 == 0x84
    }

    /// Whether the clutter filter map is requested (bits 3 and 7 set, value 136).
    pub fn requests_clutter_filter_map(&self) -> bool {
        self.inner.data_request_type.get() & 0x88 == 0x88
    }

    /// Whether RDA adaptation data is requested (bits 4 and 7 set, value 144).
    pub fn requests_adaptation_data(&self) -> bool {
        self.inner.data_request_type.get() & 0x90 == 0x90
    }

    /// Whether volume coverage pattern data is requested (bits 5 and 7 set, value 160).
    pub fn requests_volume_coverage_pattern(&self) -> bool {
        self.inner.data_request_type.get() & 0xA0 == 0xA0
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }
}
