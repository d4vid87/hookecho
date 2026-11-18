use bitvec::prelude::*;

use crate::{error::GribberishError, utils::iter::ScaleGribValueIterator};
use itertools::izip;
use std::iter;

use crate::{
    templates::template::{Template, TemplateType},
    utils::{read_f32_from_bytes, read_u16_from_bytes, read_u32_from_bytes},
};

use super::{
    tables::{GroupSplittingMethod, MissingValueManagement, OriginalFieldValue},
    DataRepresentationTemplate,
};

pub struct ComplexPackingDataRepresentationTemplate {
    data: Vec<u8>,
}

impl Template for ComplexPackingDataRepresentationTemplate {
    fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    fn template_number(&self) -> u16 {
        2
    }

    fn template_type(&self) -> TemplateType {
        TemplateType::DataRepresentation
    }

    fn template_name(&self) -> &str {
        "grid point data - complex packing"
    }
}

impl ComplexPackingDataRepresentationTemplate {
    pub fn new(data: Vec<u8>) -> ComplexPackingDataRepresentationTemplate {
        ComplexPackingDataRepresentationTemplate { data }
    }

    pub fn reference_value(&self) -> f32 {
        read_f32_from_bytes(self.data.as_slice(), 11).unwrap_or(0.0)
    }

    pub fn binary_scale_factor(&self) -> i16 {
        as_signed!(
            read_u16_from_bytes(self.data.as_slice(), 15).unwrap_or(0),
            16,
            i16
        )
    }

    pub fn decimal_scale_factor(&self) -> i16 {
        as_signed!(
            read_u16_from_bytes(self.data.as_slice(), 17).unwrap_or(0),
            16,
            i16
        )
    }

    pub fn bit_count(&self) -> u8 {
        self.data[19]
    }

    pub fn original_field_value(&self) -> OriginalFieldValue {
        self.data[20].into()
    }

    pub fn group_splitting_method(&self) -> GroupSplittingMethod {
        self.data[21].into()
    }

    pub fn missing_value_management(&self) -> MissingValueManagement {
        self.data[22].into()
    }

    pub fn primary_missing_value_substitute(&self) -> f32 {
        read_f32_from_bytes(self.data.as_slice(), 23).unwrap_or(0.0)
    }

    pub fn secondary_missing_value_substitute(&self) -> f32 {
        read_f32_from_bytes(self.data.as_slice(), 27).unwrap_or(0.0)
    }

    pub fn number_of_groups(&self) -> u32 {
        read_u32_from_bytes(self.data.as_slice(), 31).unwrap()
    }

    pub fn group_width_reference(&self) -> u8 {
        self.data[35]
    }

    pub fn group_width_bits(&self) -> u8 {
        self.data[36]
    }

    pub fn group_length_reference(&self) -> u32 {
        read_u32_from_bytes(self.data.as_slice(), 37).unwrap()
    }

    pub fn group_length_increment(&self) -> u8 {
        self.data[41]
    }

    pub fn group_last_length(&self) -> u32 {
        read_u32_from_bytes(self.data.as_slice(), 42).unwrap()
    }

    pub fn group_length_bits(&self) -> u8 {
        self.data[46]
    }
}

impl DataRepresentationTemplate<f64> for ComplexPackingDataRepresentationTemplate {
    fn compression_type(&self) -> String {
        "Complex Grid Packing".into()
    }

    fn bit_count_per_datapoint(&self) -> usize {
        self.bit_count() as usize
    }

    fn unpack(&self, bits: &BitSlice<u8, Msb0>) -> Result<Vec<f64>, GribberishError> {
        let declared_groups = self.number_of_groups() as usize;
        let expected = read_u32_from_bytes(&self.data, 5).unwrap_or(0) as usize;
        // ecCodes' zero-group convention represents a constant field directly.
        if declared_groups == 0 { return Ok(vec![f64::from(self.reference_value()); expected]); }
        if self.group_splitting_method() == GroupSplittingMethod::RowByRow {
            return Err(GribberishError::DataRepresentationTemplateError(
                "row-by-row complex packing requires grid/bitmap row geometry".into()));
        }
        // Empty groups with no encoded descriptors can be skipped without iterating billions.
        let ng = if self.bit_count() == 0 && self.group_width_bits() == 0
            && self.group_length_bits() == 0 && self.group_length_reference() == 0 {
            1
        } else { declared_groups };
        for width in [self.bit_count(), self.group_width_bits(), self.group_length_bits()] {
            if width > 32 || ng.checked_mul(usize::from(width)).is_none_or(|length| length > bits.len()) {
                return Err(GribberishError::DataRepresentationTemplateError(
                    "complex descriptors exceed available packed data".into()));
            }
        }
        let nbits = self.bit_count() as usize;

        let group_references = (0..ng).map(|ig| {
            if nbits == 0 {
                0
            } else {
                let start = ig * nbits;
                bits[start..start + nbits].load_be::<u32>()
            }
        });

        let group_widths_start = ((ng * nbits) as f32 / 8.0).ceil() as usize * 8;
        let n_width_bits = self.group_width_bits() as usize;
        let group_widths = (0..ng).map(|ig| {
            if n_width_bits == 0 {
                u32::from(self.group_width_reference())
            } else {
                let start = group_widths_start + ig * n_width_bits;
                bits[start..start + n_width_bits].load_be::<u32>()
                    + self.group_width_reference() as u32
            }
        });

        let group_lengths_start =
            group_widths_start + (((n_width_bits * ng) as f32 / 8.0).ceil() as usize * 8);
        let n_length_bits = self.group_length_bits() as usize;
        let group_lengths = (0..ng.saturating_sub(1)).map(|ig| {
            if n_length_bits == 0 {
                u64::from(self.group_length_reference())
            } else {
                let start = group_lengths_start + ig * n_length_bits;
                u64::from(bits[start..start + n_length_bits].load_be::<u32>())
                    * u64::from(self.group_length_increment())
                    + u64::from(self.group_length_reference())
            }
        }).chain(iter::once(u64::from(self.group_last_length())));

        // Zero-width groups still emit every declared value; bound their expansion before collect.
        if group_lengths.clone().try_fold(0usize, |total, length|
            total.checked_add(usize::try_from(length).ok()?).filter(|total| *total <= expected)) != Some(expected) {
            return Err(GribberishError::DataRepresentationTemplateError(
                "complex group lengths disagree with packed point count".into()));
        }

        let mut pos =
            group_lengths_start + (((n_length_bits * ng) as f32 / 8.0).ceil() as usize * 8);
        let values = izip!(group_references, group_widths, group_lengths)
            .flat_map(|(reference, width, length)| {
                let length = length as usize; // validated against packed point count above
                let n_bits = width as usize * length;
                let group_values = (0..length).map(move |i| {
                    let value = if width == 0 {
                        0u32
                    } else {
                        bits[pos + i * width as usize
                            ..pos + i * width as usize + width as usize]
                            .load_be::<u32>()
                    };
                    let raw = as_signed!(value, 32, i32);
                    raw + reference as i32
                });

                pos += n_bits;

                group_values
            })
            .scale_value_by(
                self.binary_scale_factor(),
                self.decimal_scale_factor(),
                self.reference_value(),
            )
            .collect();

        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_descriptor_bits_keep_nonzero_width_and_length_references() {
        let mut section = vec![0u8; 47];
        section[5..9].copy_from_slice(&4u32.to_be_bytes());
        section[21] = 1;
        section[31..35].copy_from_slice(&2u32.to_be_bytes());
        section[35] = 8;
        section[37..41].copy_from_slice(&2u32.to_be_bytes());
        section[42..46].copy_from_slice(&2u32.to_be_bytes());
        let payload = [1u8, 2, 3, 4];
        assert_eq!(ComplexPackingDataRepresentationTemplate::new(section.clone())
            .unpack(payload.view_bits::<Msb0>()).unwrap(), [1.0, 2.0, 3.0, 4.0]);
        section[31..35].copy_from_slice(&0u32.to_be_bytes());
        section[11..15].copy_from_slice(&7f32.to_be_bytes());
        assert_eq!(ComplexPackingDataRepresentationTemplate::new(section)
            .unpack([].view_bits::<Msb0>()).unwrap(), [7.0; 4]);
    }

    #[test]
    fn zero_width_groups_cannot_expand_beyond_the_packed_count() {
        let mut section = vec![0u8; 47];
        section[5..9].copy_from_slice(&2u32.to_be_bytes());
        section[21] = 1;
        section[31..35].copy_from_slice(&1u32.to_be_bytes());
        section[42..46].copy_from_slice(&2u32.to_be_bytes());
        let bits = [].view_bits::<Msb0>();
        assert_eq!(ComplexPackingDataRepresentationTemplate::new(section.clone()).unpack(bits).unwrap(), [0.0, 0.0]);
        for length in [1u32, 3] {
            section[42..46].copy_from_slice(&length.to_be_bytes());
            assert!(ComplexPackingDataRepresentationTemplate::new(section.clone()).unpack(bits).is_err());
        }
        section[31..35].copy_from_slice(&3u32.to_be_bytes());
        section[37..41].copy_from_slice(&1u32.to_be_bytes());
        assert!(ComplexPackingDataRepresentationTemplate::new(section).unpack(bits).is_err());
    }
}
