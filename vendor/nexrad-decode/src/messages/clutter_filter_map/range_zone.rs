use crate::messages::clutter_filter_map::raw;
use crate::messages::clutter_filter_map::OpCode;
use std::borrow::Cow;

#[cfg(feature = "uom")]
use uom::si::f64::Length;
#[cfg(feature = "uom")]
use uom::si::length::kilometer;

/// Defines a range segment of a particular elevation and azimuth with an operation type describing
/// the clutter filter map behavior for the segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RangeZone<'a> {
    inner: Cow<'a, raw::RangeZone>,
}

impl<'a> RangeZone<'a> {
    /// Create a new RangeZone wrapper from a reference to a raw RangeZone.
    pub(crate) fn new(inner: &'a raw::RangeZone) -> Self {
        Self {
            inner: Cow::Borrowed(inner),
        }
    }

    /// Operation code for the range zone.
    pub fn raw_op_code(&self) -> u16 {
        self.inner.op_code.get()
    }

    /// Stop range per zone in km. There are 20 possible zones and not all need to be defined. The
    /// last zone must have an end range of 511km.
    pub fn end_range(&self) -> u16 {
        self.inner.end_range.get()
    }

    /// Operation code for the range zone.
    pub fn op_code(&self) -> OpCode {
        match self.inner.op_code.get() {
            0 => OpCode::BypassFilter,
            1 => OpCode::BypassMapInControl,
            2 => OpCode::ForceFilter,
            other => OpCode::Unknown(other),
        }
    }

    /// Stop range per zone. There are 20 possible zones and not all need to be defined. The last
    /// zone must have an end range of 511km.
    #[cfg(feature = "uom")]
    pub fn end_range_uom(&self) -> Length {
        Length::new::<kilometer>(self.inner.end_range.get() as f64)
    }

    /// Convert this range zone to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> RangeZone<'static> {
        RangeZone {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }
}
