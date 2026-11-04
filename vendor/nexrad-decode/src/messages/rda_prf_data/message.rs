use crate::messages::primitive_aliases::{Code2, Integer4};
use crate::messages::rda_prf_data::raw::Header;
use crate::messages::rda_prf_data::waveform_prf_data::WaveformPrfData;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// An RDA PRF data message (type 32) containing pulse repetition frequency data for each waveform
/// type used by the radar.
///
/// This message's contents correspond to ICD 2620002AA section 3.2.4.32 Table XVIII.
/// The message starts with a header indicating the number of waveform sections, followed by
/// a variable-length series of waveform type and PRF value entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Message<'a> {
    /// Decoded header information for this PRF data message.
    header: Cow<'a, Header>,

    /// PRF data for each waveform type included in this message.
    waveform_prf_data: Vec<WaveformPrfData>,
}

impl<'a> Message<'a> {
    /// Parse an RDA PRF data message from segmented input.
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<Header>()?;

        let waveform_count = header.number_of_waveforms.get() as usize;
        let mut waveform_prf_data = Vec::with_capacity(waveform_count);

        for _ in 0..waveform_count {
            let waveform_type = reader.take_ref::<Code2>()?.get();
            let prf_count = reader.take_ref::<Code2>()?.get() as usize;

            let mut prf_values = Vec::with_capacity(prf_count);
            for _ in 0..prf_count {
                let value = reader.take_ref::<Integer4>()?.get();
                prf_values.push(value);
            }

            waveform_prf_data.push(WaveformPrfData::new(waveform_type, prf_values));
        }

        Ok(Message {
            header: Cow::Borrowed(header),
            waveform_prf_data,
        })
    }

    /// The number of waveform types included in this message (1-5).
    pub fn number_of_waveforms(&self) -> u16 {
        self.header.number_of_waveforms.get()
    }

    /// The PRF data for each waveform type included in this message.
    pub fn waveform_prf_data(&self) -> &[WaveformPrfData] {
        &self.waveform_prf_data
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: Cow::Owned(self.header.into_owned()),
            waveform_prf_data: self.waveform_prf_data,
        }
    }
}
