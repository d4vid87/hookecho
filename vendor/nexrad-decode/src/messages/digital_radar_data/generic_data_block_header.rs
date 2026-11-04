use super::raw;
use super::{ControlFlags, DataWordSize};
use std::borrow::Cow;

#[cfg(feature = "uom")]
use uom::si::f64::{Information, Length};
#[cfg(feature = "uom")]
use uom::si::information::byte;
#[cfg(feature = "uom")]
use uom::si::length::kilometer;

/// A generic data moment block's decoded header.
#[derive(Clone, PartialEq, Debug)]
pub struct GenericDataBlockHeader<'a> {
    inner: Cow<'a, raw::GenericDataBlockHeader>,
}

impl<'a> GenericDataBlockHeader<'a> {
    /// Create a new GenericDataBlockHeader wrapper from a raw GenericDataBlockHeader reference.
    pub(crate) fn new(inner: &'a raw::GenericDataBlockHeader) -> Self {
        Self {
            inner: Cow::Borrowed(inner),
        }
    }

    /// Convert this generic data block header to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> GenericDataBlockHeader<'static> {
        GenericDataBlockHeader {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }

    /// Reserved.
    pub fn reserved(&self) -> u32 {
        self.inner.reserved.get()
    }

    /// Number of data moment gates for current radial, from 0 to 1840.
    pub fn number_of_data_moment_gates(&self) -> u16 {
        self.inner.number_of_data_moment_gates.get()
    }

    /// Range to center of first range gate in 0.000-scaled kilometers (raw value).
    pub fn data_moment_range_raw(&self) -> u16 {
        self.inner.data_moment_range.get()
    }

    /// Size of data moment sample interval in 0.000-scaled kilometers (raw value).
    pub fn data_moment_range_sample_interval_raw(&self) -> u16 {
        self.inner.data_moment_range_sample_interval.get()
    }

    /// Threshold parameter specifying the minimum difference in echo power between two resolution
    /// gates in dB for them to not be labeled as "overlayed" (raw scaled value).
    pub fn tover_raw(&self) -> u16 {
        self.inner.tover.get()
    }

    /// Signal-to-noise ratio threshold for valid data from -12 to 20 dB (raw scaled value).
    pub fn snr_threshold_raw(&self) -> i16 {
        self.inner.snr_threshold.get()
    }

    /// Flags indicating special control features (raw value).
    pub fn control_flags_raw(&self) -> u8 {
        self.inner.control_flags
    }

    /// Number of bits (8 or 16) used for storing data for each data moment gate (raw value).
    pub fn data_word_size_raw(&self) -> u8 {
        self.inner.data_word_size
    }

    /// Scale factor for converting data moments to floating-point representation.
    pub fn scale(&self) -> f32 {
        self.inner.scale.get()
    }

    /// Offset value for converting data moments to floating-point representation.
    pub fn offset(&self) -> f32 {
        self.inner.offset.get()
    }

    /// Range to center of first range gate in kilometers.
    pub fn data_moment_range_km(&self) -> f64 {
        self.inner.data_moment_range.get() as f64 * 0.001
    }

    /// Size of data moment sample interval in kilometers.
    pub fn data_moment_range_sample_interval_km(&self) -> f64 {
        self.inner.data_moment_range_sample_interval.get() as f64 * 0.001
    }

    /// Number of bits used for storing data for each data moment gate.
    pub fn data_word_size(&self) -> DataWordSize {
        match self.inner.data_word_size {
            8 => DataWordSize::EightBit,
            16 => DataWordSize::SixteenBit,
            other => DataWordSize::Unknown(other),
        }
    }

    /// Range to center of first range gate.
    #[cfg(feature = "uom")]
    pub fn data_moment_range(&self) -> Length {
        Length::new::<kilometer>(self.inner.data_moment_range.get() as f64 * 0.001)
    }

    /// Size of data moment sample interval.
    #[cfg(feature = "uom")]
    pub fn data_moment_range_sample_interval(&self) -> Length {
        Length::new::<kilometer>(self.inner.data_moment_range_sample_interval.get() as f64 * 0.001)
    }

    /// Flags indicating special control features.
    pub fn control_flags(&self) -> ControlFlags {
        match self.inner.control_flags {
            0 => ControlFlags::None,
            1 => ControlFlags::RecombinedAzimuthalRadials,
            2 => ControlFlags::RecombinedRangeGates,
            3 => ControlFlags::RecombinedRadialsAndRangeGatesToLegacyResolution,
            other => ControlFlags::Unknown(other),
        }
    }

    /// Size of the data moment block in bytes.
    #[cfg(feature = "uom")]
    pub fn moment_size(&self) -> Information {
        Information::new::<byte>(
            self.inner.number_of_data_moment_gates.get() as f64 * self.inner.data_word_size as f64
                / 8.0,
        )
    }
}
