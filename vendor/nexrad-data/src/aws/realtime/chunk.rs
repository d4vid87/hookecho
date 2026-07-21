use crate::result::aws::AWSError::UnrecognizedChunkFormat;
use crate::result::Error::AWS;
use crate::volume;

/// A chunk of real-time data within a volume. Chunks are ordered and when concatenated together
/// form a complete volume of radar data. All chunks contain an LDM record with radar data messages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Chunk<'a> {
    /// The start of a new volume. This chunk will begin with an Archive II volume header followed
    /// by a compressed LDM record.
    Start(volume::File),
    /// An intermediate or end chunk. This chunk will contain a compressed LDM record with radar
    /// data messages.
    IntermediateOrEnd(volume::Record<'a>),
}

impl Chunk<'_> {
    /// Creates a new chunk from the provided data. The data is expected to be in one of two formats:
    ///
    /// 1. An Archive II volume header followed by a compressed LDM record, or a "start" chunk.
    /// 2. A compressed LDM record, or an "intermediate" or "end" chunk.
    ///
    /// The chunk type is determined by the data's format.
    pub fn new(data: Vec<u8>) -> crate::result::Result<Self> {
        // Check if the data begins with an Archive II volume header, indicating a "start" chunk
        if data.len() >= 3 && data.get(0..3) == Some(b"AR2".as_slice()) {
            let file = volume::File::new(data);
            return Ok(Self::Start(file));
        }

        // Check if the data begins with a BZ compressed record, indicating an "intermediate" or "end" chunk
        if data.len() >= 6 && data.get(4..6) == Some(b"BZ".as_slice()) {
            let record = volume::Record::new(data);
            return Ok(Self::IntermediateOrEnd(record));
        }

        Err(AWS(UnrecognizedChunkFormat))
    }

    /// The data contained within this chunk.
    pub fn data(&self) -> &[u8] {
        match self {
            Self::Start(file) => file.data(),
            Self::IntermediateOrEnd(record) => record.data(),
        }
    }
}
