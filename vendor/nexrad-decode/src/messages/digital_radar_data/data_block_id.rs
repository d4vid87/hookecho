use super::raw;
use super::{DataBlockName, DataBlockType};
use std::borrow::Cow;

/// A digital radar data block's identifier.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DataBlockId<'a> {
    inner: Cow<'a, raw::DataBlockId>,
}

impl<'a> DataBlockId<'a> {
    /// Create a new DataBlockId wrapper from a raw DataBlockId reference.
    pub(crate) fn new(inner: &'a raw::DataBlockId) -> Self {
        Self {
            inner: Cow::Borrowed(inner),
        }
    }

    /// Convert this data block ID to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> DataBlockId<'static> {
        DataBlockId {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }

    /// Data block type (raw byte), e.g. b'R'.
    pub fn data_block_type_raw(&self) -> u8 {
        self.inner.data_block_type
    }

    /// Data block name (raw bytes), e.g. b"VOL".
    pub fn data_name_raw(&self) -> &[u8; 3] {
        &self.inner.data_name
    }

    /// Data block type category.
    pub fn data_block_type(&self) -> DataBlockType {
        match self.inner.data_block_type {
            b'R' => DataBlockType::Radial,
            b'D' => DataBlockType::DataMoment,
            other => DataBlockType::Unknown(other),
        }
    }

    /// Data block type as a character, e.g. 'R'.
    pub fn data_block_type_char(&self) -> char {
        self.inner.data_block_type as char
    }

    /// Data block name.
    pub fn data_block_name(&self) -> DataBlockName {
        match &self.inner.data_name {
            b"VOL" => DataBlockName::Volume,
            b"ELV" => DataBlockName::Elevation,
            b"RAD" => DataBlockName::Radial,
            b"REF" => DataBlockName::Reflectivity,
            b"VEL" => DataBlockName::Velocity,
            b"SW\0" => DataBlockName::SpectrumWidth,
            b"ZDR" => DataBlockName::DifferentialReflectivity,
            b"PHI" => DataBlockName::DifferentialPhase,
            b"RHO" => DataBlockName::CorrelationCoefficient,
            b"CFP" => DataBlockName::ClutterFilterPower,
            other => DataBlockName::Unknown(*other),
        }
    }

    /// Data block name as a string, e.g. "VOL".
    ///
    /// The name is always 3 bytes of ASCII from the binary format. If the bytes are
    /// not valid UTF-8 (e.g. from a malformed file), invalid sequences are replaced
    /// with the Unicode replacement character.
    pub fn data_block_name_str(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.inner.data_name)
    }
}
