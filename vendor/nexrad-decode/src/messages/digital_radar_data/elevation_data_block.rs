use super::raw;
use std::borrow::Cow;

#[cfg(feature = "uom")]
use uom::si::f64::Information;
#[cfg(feature = "uom")]
use uom::si::information::byte;

/// An elevation data block.
#[derive(Clone, PartialEq, Debug)]
pub struct ElevationDataBlock<'a> {
    inner: Cow<'a, raw::ElevationDataBlock>,
}

impl<'a> ElevationDataBlock<'a> {
    /// Create a new ElevationDataBlock wrapper from a raw ElevationDataBlock reference.
    pub(crate) fn new(inner: &'a raw::ElevationDataBlock) -> Self {
        Self {
            inner: Cow::Borrowed(inner),
        }
    }

    /// Convert this elevation data block to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> ElevationDataBlock<'static> {
        ElevationDataBlock {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }

    /// Size of data block in bytes (raw value).
    pub fn lrtup_raw(&self) -> u16 {
        self.inner.lrtup.get()
    }

    /// Atmospheric attenuation factor in dB/km (raw scaled value).
    pub fn atmos_raw(&self) -> i16 {
        self.inner.atmos.get()
    }

    /// Scaling constant used by the signal processor for this elevation to calculate reflectivity
    /// in dB.
    pub fn calibration_constant(&self) -> f32 {
        self.inner.calibration_constant.get()
    }

    /// Size of data block.
    #[cfg(feature = "uom")]
    pub fn lrtup(&self) -> Information {
        Information::new::<byte>(self.inner.lrtup.get() as f64)
    }

    /// Atmospheric attenuation factor in dB/km.
    pub fn atmos(&self) -> f32 {
        self.inner.atmos.get() as f32 / 1000.0
    }
}
