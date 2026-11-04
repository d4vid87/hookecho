use super::raw;
use super::GenericDataBlockHeader;
use crate::binary_data::BinaryData;
use crate::result::Result;
use crate::slice_reader::SliceReader;
use std::borrow::Cow;

/// A generic data moment block.
#[derive(Clone, PartialEq, Debug)]
pub struct GenericDataBlock<'a> {
    /// The generic data block's header information.
    header: GenericDataBlockHeader<'a>,

    /// The generic data block's encoded moment data.
    encoded_data: BinaryData<Cow<'a, [u8]>>,
}

impl<'a> GenericDataBlock<'a> {
    /// Creates a new generic data moment block from the decoded header.
    pub(crate) fn parse(reader: &mut SliceReader<'a>) -> Result<Self> {
        let raw_header = reader.take_ref::<raw::GenericDataBlockHeader>()?;

        let word_size_bytes = raw_header.data_word_size as usize / 8;
        let encoded_data_size =
            raw_header.number_of_data_moment_gates.get() as usize * word_size_bytes;

        let encoded_data = reader.take_bytes(encoded_data_size)?;

        Ok(Self {
            header: GenericDataBlockHeader::new(raw_header),
            encoded_data: BinaryData::new(Cow::Borrowed(encoded_data)),
        })
    }

    /// Convert this data block to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> GenericDataBlock<'static> {
        GenericDataBlock {
            header: self.header.into_owned(),
            encoded_data: BinaryData::new(Cow::Owned(self.encoded_data.0.into_owned())),
        }
    }

    /// The generic data block's header information.
    pub fn header(&self) -> &GenericDataBlockHeader<'a> {
        &self.header
    }

    /// Raw gate values for this moment/radial ordered in ascending distance from the radar. These
    /// values are stored in a fixed-point representation using the header's offset and scale fields.
    /// Use `nexrad_model::data::MomentData` or `nexrad_model::data::CFPMomentData` to decode.
    pub fn encoded_values(&self) -> &[u8] {
        &self.encoded_data
    }

    /// Create a [`MomentData`](nexrad_model::data::MomentData) from this generic data block,
    /// cloning the underlying encoded data.
    #[cfg(feature = "nexrad-model")]
    pub fn moment_data(&self) -> nexrad_model::data::MomentData {
        nexrad_model::data::MomentData::from_fixed_point(
            self.header.number_of_data_moment_gates(),
            self.header.data_moment_range_raw(),
            self.header.data_moment_range_sample_interval_raw(),
            self.header.data_word_size_raw(),
            self.header.scale(),
            self.header.offset(),
            self.encoded_data.to_vec(),
        )
    }

    /// Convert this generic data block into a [`MomentData`](nexrad_model::data::MomentData),
    /// consuming the encoded data without copying.
    #[cfg(feature = "nexrad-model")]
    pub fn into_moment_data(self) -> nexrad_model::data::MomentData {
        nexrad_model::data::MomentData::from_fixed_point(
            self.header.number_of_data_moment_gates(),
            self.header.data_moment_range_raw(),
            self.header.data_moment_range_sample_interval_raw(),
            self.header.data_word_size_raw(),
            self.header.scale(),
            self.header.offset(),
            self.encoded_data.into_inner().into_owned(),
        )
    }

    /// Create a [`CFPMomentData`](nexrad_model::data::CFPMomentData) from this generic data
    /// block, cloning the underlying encoded data.
    #[cfg(feature = "nexrad-model")]
    pub fn cfp_moment_data(&self) -> nexrad_model::data::CFPMomentData {
        nexrad_model::data::CFPMomentData::from_fixed_point(
            self.header.number_of_data_moment_gates(),
            self.header.data_moment_range_raw(),
            self.header.data_moment_range_sample_interval_raw(),
            self.header.data_word_size_raw(),
            self.header.scale(),
            self.header.offset(),
            self.encoded_data.to_vec(),
        )
    }

    /// Convert this generic data block into a [`CFPMomentData`](nexrad_model::data::CFPMomentData),
    /// consuming the encoded data without copying.
    #[cfg(feature = "nexrad-model")]
    pub fn into_cfp_moment_data(self) -> nexrad_model::data::CFPMomentData {
        nexrad_model::data::CFPMomentData::from_fixed_point(
            self.header.number_of_data_moment_gates(),
            self.header.data_moment_range_raw(),
            self.header.data_moment_range_sample_interval_raw(),
            self.header.data_word_size_raw(),
            self.header.scale(),
            self.header.offset(),
            self.encoded_data.into_inner().into_owned(),
        )
    }
}
