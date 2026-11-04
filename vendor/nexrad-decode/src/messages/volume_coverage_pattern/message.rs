use super::raw;
use super::{ElevationDataBlock, Header};
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;

/// The volume coverage pattern message describes the current scan pattern. This is sent on
/// wideband connection and the start of each volume scan.
///
/// This message's contents correspond to ICD 2620002AA section 3.2.4.12 Table XI.
#[derive(Debug, Clone, PartialEq)]
pub struct Message<'a> {
    /// The decoded volume coverage pattern header.
    header: Header<'a>,

    /// The decoded elevation data blocks.
    elevations: Vec<ElevationDataBlock<'a>>,
}

impl<'a> Message<'a> {
    /// Parse a volume coverage pattern message from the input.
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let raw_header = reader.take_ref::<raw::Header>()?;

        let elevation_cuts = raw_header.number_of_elevation_cuts.get() as usize;
        let raw_elevations = reader.take_slice::<raw::ElevationDataBlock>(elevation_cuts)?;

        let header = Header::new(raw_header);
        let elevations = raw_elevations.iter().map(ElevationDataBlock::new).collect();

        Ok(Self { header, elevations })
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: self.header.into_owned(),
            elevations: self
                .elevations
                .into_iter()
                .map(ElevationDataBlock::into_owned)
                .collect(),
        }
    }

    /// The decoded volume coverage pattern header.
    pub fn header(&self) -> &Header<'a> {
        &self.header
    }

    /// The decoded elevation data blocks.
    pub fn elevations(&self) -> &[ElevationDataBlock<'a>] {
        &self.elevations
    }
}
