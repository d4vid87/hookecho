use crate::BinaryData;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "uom")]
use uom::si::{f64::Length, length::kilometer};

/// Common interface for types that provide gate-level moment data.
///
/// Both [`MomentData`] and [`CFPMomentData`] implement this trait, providing
/// access to shared gate metadata, scale/offset encoding parameters, and raw
/// gate values. Only the decoded value semantics differ between them.
pub trait DataMoment {
    /// The number of gates in this data moment.
    fn gate_count(&self) -> u16;

    /// The range to the center of the first gate in kilometers.
    fn first_gate_range_km(&self) -> f64;

    /// The range to the center of the first gate.
    #[cfg(feature = "uom")]
    fn first_gate_range(&self) -> Length;

    /// The range between the centers of consecutive gates in kilometers.
    fn gate_interval_km(&self) -> f64;

    /// Number of bits per gate (8 or 16).
    fn data_word_size(&self) -> u8;

    /// The range between the centers of consecutive gates.
    #[cfg(feature = "uom")]
    fn gate_interval(&self) -> Length;

    /// The scale factor used to decode raw gate values into floating-point values.
    /// A value of `0.0` means raw values are used directly without scaling.
    fn scale(&self) -> f32;

    /// The offset used to decode raw gate values into floating-point values.
    /// The decoded value is `(raw - offset) / scale`.
    fn offset(&self) -> f32;

    /// The raw encoded gate values as bytes. For 8-bit moments, each byte is one gate.
    /// For 16-bit moments, each pair of bytes is a big-endian `u16` gate value.
    fn raw_values(&self) -> &[u8];

    /// Iterator over raw gate values as `u16`, handling both 8-bit and 16-bit word sizes.
    fn raw_gate_values(&self) -> impl Iterator<Item = u16> + '_;
}

/// Implements [`DataMoment`] for a wrapper type that stores a `MomentDataBlock` as `self.inner`.
macro_rules! impl_data_moment {
    ($ty:ty) => {
        impl DataMoment for $ty {
            fn gate_count(&self) -> u16 {
                self.inner.gate_count()
            }
            fn first_gate_range_km(&self) -> f64 {
                self.inner.first_gate_range_km()
            }
            #[cfg(feature = "uom")]
            fn first_gate_range(&self) -> Length {
                self.inner.first_gate_range()
            }
            fn gate_interval_km(&self) -> f64 {
                self.inner.gate_interval_km()
            }
            fn data_word_size(&self) -> u8 {
                self.inner.data_word_size()
            }
            #[cfg(feature = "uom")]
            fn gate_interval(&self) -> Length {
                self.inner.gate_interval()
            }
            fn scale(&self) -> f32 {
                self.inner.scale()
            }
            fn offset(&self) -> f32 {
                self.inner.offset()
            }
            fn raw_values(&self) -> &[u8] {
                self.inner.raw_values()
            }
            fn raw_gate_values(&self) -> impl Iterator<Item = u16> + '_ {
                self.inner.raw_gate_values()
            }
        }
    };
}

/// CFP status codes for clutter filter power moments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum CFPStatus {
    /// Clutter filter not applied.
    FilterNotApplied,
    /// Point clutter filter applied.
    PointClutterFilterApplied,
    /// Dual-pol-only filter applied.
    DualPolOnlyFilterApplied,
    /// Reserved CFP status code.
    Reserved(u8),
}

/// Encoded moment data from a radial containing gate metadata and raw values.
///
/// This is an internal type providing gate metadata accessors (count, range, interval) shared
/// by both [`MomentData`] and [`CFPMomentData`]. Use those public wrapper types for decoded
/// gate values and access to gate metadata via the [`DataMoment`] trait.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub(crate) struct MomentDataBlock {
    gate_count: u16,
    first_gate_range: u16,
    gate_interval: u16,
    /// Bits per gate (8 or 16). Dual-pol moments often use 16-bit words.
    data_word_size: u8,
    scale: f32,
    offset: f32,
    values: BinaryData<Vec<u8>>,
}

impl MomentDataBlock {
    /// Create new moment data block from fixed-point encoding.
    pub(crate) fn from_fixed_point(
        gate_count: u16,
        first_gate_range: u16,
        gate_interval: u16,
        data_word_size: u8,
        scale: f32,
        offset: f32,
        values: Vec<u8>,
    ) -> Self {
        debug_assert!(
            data_word_size != 16 || values.len() % 2 == 0,
            "16-bit moment data must have an even number of bytes, got {}",
            values.len()
        );

        Self {
            gate_count,
            first_gate_range,
            gate_interval,
            data_word_size,
            scale,
            offset,
            values: BinaryData::new(values),
        }
    }

    /// The number of gates in this data moment.
    fn gate_count(&self) -> u16 {
        self.gate_count
    }

    /// The range to the center of the first gate in kilometers.
    fn first_gate_range_km(&self) -> f64 {
        self.first_gate_range as f64 * 0.001
    }

    /// The range to the center of the first gate.
    #[cfg(feature = "uom")]
    fn first_gate_range(&self) -> Length {
        Length::new::<kilometer>(self.first_gate_range as f64 * 0.001)
    }

    /// The range between the centers of consecutive gates in kilometers.
    fn gate_interval_km(&self) -> f64 {
        self.gate_interval as f64 * 0.001
    }

    /// Number of bits per gate (8 or 16).
    fn data_word_size(&self) -> u8 {
        self.data_word_size
    }

    /// The range between the centers of consecutive gates.
    #[cfg(feature = "uom")]
    fn gate_interval(&self) -> Length {
        Length::new::<kilometer>(self.gate_interval as f64 * 0.001)
    }

    /// The scale factor used to decode raw gate values into floating-point values.
    /// A value of `0.0` means raw values are used directly without scaling.
    fn scale(&self) -> f32 {
        self.scale
    }

    /// The offset used to decode raw gate values into floating-point values.
    /// The decoded value is `(raw - offset) / scale`.
    fn offset(&self) -> f32 {
        self.offset
    }

    /// The raw encoded gate values as bytes. For 8-bit moments, each byte is one gate.
    /// For 16-bit moments, each pair of bytes is a big-endian `u16` gate value.
    fn raw_values(&self) -> &[u8] {
        &self.values
    }

    /// Iterator over raw gate values as `u16`, handling both 8-bit and 16-bit word sizes.
    fn raw_gate_values(&self) -> impl Iterator<Item = u16> + '_ {
        let is_16bit = self.data_word_size == 16;
        let step = if is_16bit { 2 } else { 1 };
        self.values.chunks_exact(step).map(move |chunk| {
            if is_16bit {
                u16::from_be_bytes([chunk[0], chunk[1]])
            } else {
                chunk[0] as u16
            }
        })
    }
}

/// Moment data from a radial for a particular product where each value corresponds to a gate.
///
/// Gate metadata (count, range, interval) is available through the [`DataMoment`] trait —
/// see [`gate_count`](DataMoment::gate_count), [`first_gate_range_km`](DataMoment::first_gate_range_km),
/// and [`gate_interval_km`](DataMoment::gate_interval_km).
/// Use [`values`](MomentData::values) to decode gates with standard moment semantics
/// (below threshold, range folded, or numeric value).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct MomentData {
    inner: MomentDataBlock,
}

impl MomentData {
    /// Create new moment data from fixed-point encoding.
    pub fn from_fixed_point(
        gate_count: u16,
        first_gate_range: u16,
        gate_interval: u16,
        data_word_size: u8,
        scale: f32,
        offset: f32,
        values: Vec<u8>,
    ) -> Self {
        Self {
            inner: MomentDataBlock::from_fixed_point(
                gate_count,
                first_gate_range,
                gate_interval,
                data_word_size,
                scale,
                offset,
                values,
            ),
        }
    }

    /// Iterator over decoded gate values without allocating.
    pub fn iter(&self) -> impl Iterator<Item = MomentValue> + '_ {
        let scale = self.inner.scale;
        let offset = self.inner.offset;

        self.inner.raw_gate_values().map(move |raw_value| {
            // scale == 0.0 is an exact comparison; the value comes from a binary format
            // where IEEE 754 zero is stored literally.
            if scale == 0.0 {
                return MomentValue::Value(raw_value as f32);
            }

            match raw_value {
                0 => MomentValue::BelowThreshold,
                1 => MomentValue::RangeFolded,
                _ => MomentValue::Value((raw_value as f32 - offset) / scale),
            }
        })
    }

    /// Decoded gate values collected into a `Vec`. Prefer [`iter`](Self::iter) when
    /// processing values sequentially to avoid allocation.
    pub fn values(&self) -> Vec<MomentValue> {
        self.iter().collect()
    }
}

impl_data_moment!(MomentData);

/// The data moment value for a product in a radial's gate. The value may be a floating-point number
/// or a special case such as "below threshold" or "range folded".
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MomentValue {
    /// The data moment value for a gate.
    Value(f32),
    /// The value for this gate was below the signal threshold.
    BelowThreshold,
    /// The value for this gate exceeded the maximum unambiguous range.
    RangeFolded,
}

/// The validity status of a gate value in a field.
///
/// Used alongside `f32` values in field types ([`SweepField`](crate::data::SweepField),
/// [`CartesianField`](crate::data::CartesianField), [`VerticalField`](crate::data::VerticalField))
/// to indicate whether each gate contains a valid measurement or a special condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum GateStatus {
    /// The gate contains a valid floating-point value.
    Valid,
    /// The signal was below the detection threshold.
    BelowThreshold,
    /// The velocity was range-folded (aliased).
    RangeFolded,
    /// No data is available for this gate.
    NoData,
}

impl From<&MomentValue> for GateStatus {
    fn from(value: &MomentValue) -> Self {
        match value {
            MomentValue::Value(_) => GateStatus::Valid,
            MomentValue::BelowThreshold => GateStatus::BelowThreshold,
            MomentValue::RangeFolded => GateStatus::RangeFolded,
        }
    }
}

/// A decoded CFP gate value. Raw values 0–7 are status codes; values 8+ are numeric.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CFPMomentValue {
    /// A CFP status code (raw values 0–7).
    Status(CFPStatus),
    /// A decoded floating-point CFP value (raw values 8+).
    Value(f32),
}

/// Clutter filter power (CFP) moment data.
///
/// Gate metadata (count, range, interval) is available through the [`DataMoment`] trait —
/// see [`gate_count`](DataMoment::gate_count), [`first_gate_range_km`](DataMoment::first_gate_range_km),
/// and [`gate_interval_km`](DataMoment::gate_interval_km).
/// Use [`values`](CFPMomentData::values) to decode gates with CFP-specific semantics.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct CFPMomentData {
    inner: MomentDataBlock,
}

impl CFPMomentData {
    /// Create new CFP moment data from fixed-point encoding.
    pub fn from_fixed_point(
        gate_count: u16,
        first_gate_range: u16,
        gate_interval: u16,
        data_word_size: u8,
        scale: f32,
        offset: f32,
        values: Vec<u8>,
    ) -> Self {
        Self {
            inner: MomentDataBlock::from_fixed_point(
                gate_count,
                first_gate_range,
                gate_interval,
                data_word_size,
                scale,
                offset,
                values,
            ),
        }
    }

    /// Iterator over decoded CFP gate values without allocating.
    ///
    /// Raw values 0–7 are decoded as CFP status codes. Values 8+ are decoded as
    /// floating-point values using the moment's scale and offset.
    pub fn iter(&self) -> impl Iterator<Item = CFPMomentValue> + '_ {
        let scale = self.inner.scale;
        let offset = self.inner.offset;

        self.inner
            .raw_gate_values()
            .map(move |raw_value| match raw_value {
                0 => CFPMomentValue::Status(CFPStatus::FilterNotApplied),
                1 => CFPMomentValue::Status(CFPStatus::PointClutterFilterApplied),
                2 => CFPMomentValue::Status(CFPStatus::DualPolOnlyFilterApplied),
                3..=7 => CFPMomentValue::Status(CFPStatus::Reserved(raw_value as u8)),
                _ => {
                    if scale == 0.0 {
                        CFPMomentValue::Value(raw_value as f32)
                    } else {
                        CFPMomentValue::Value((raw_value as f32 - offset) / scale)
                    }
                }
            })
    }

    /// Decoded CFP gate values collected into a `Vec`. Prefer [`iter`](Self::iter) when
    /// processing values sequentially to avoid allocation.
    pub fn values(&self) -> Vec<CFPMomentValue> {
        self.iter().collect()
    }
}

impl_data_moment!(CFPMomentData);
