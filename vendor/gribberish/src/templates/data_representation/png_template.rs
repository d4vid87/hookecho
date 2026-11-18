use bitvec::prelude::*;

use super::{tables::OriginalFieldValue, DataRepresentationTemplate};
use crate::{
    error::GribberishError,
    templates::template::{Template, TemplateType},
    utils::{iter::ScaleGribValueIterator, read_f32_from_bytes, read_u16_from_bytes},
};
use png::Decoder;

pub struct PNGDataRepresentationTemplate {
    data: Vec<u8>,
}

impl Template for PNGDataRepresentationTemplate {
    fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    fn template_number(&self) -> u16 {
        40
    }

    fn template_type(&self) -> TemplateType {
        TemplateType::DataRepresentation
    }

    fn template_name(&self) -> &str {
        "grid point data - PNG compression"
    }
}

impl PNGDataRepresentationTemplate {
    pub fn new(data: Vec<u8>) -> PNGDataRepresentationTemplate {
        PNGDataRepresentationTemplate { data }
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
}

impl DataRepresentationTemplate<f64> for PNGDataRepresentationTemplate {
    fn compression_type(&self) -> String {
        "PNG".into()
    }

    fn bit_count_per_datapoint(&self) -> usize {
        self.bit_count() as usize
    }

    fn unpack(&self, bits: &BitSlice<u8, Msb0>) -> Result<Vec<f64>, GribberishError> {
        let bytes: Vec<u8> = bits.to_bitvec().into();

        let decoder = Decoder::new(bytes.as_slice());
        let decode_error = |error| GribberishError::DataRepresentationTemplateError(format!("PNG decode: {error}"));
        let mut reader = decoder.read_info().map_err(decode_error)?;

        // PNG's internal limits exclude this caller-owned buffer. Its dimensions must agree
        // with the packed GRIB count (which excludes points omitted by a bitmap).
        let expected = crate::utils::read_u32_from_bytes(&self.data, 5).unwrap_or(0) as usize;
        let bytes_per_datapoint = (self.bit_count_per_datapoint() / 8).max(1);
        let info = reader.info();
        if (info.width as usize).checked_mul(info.height as usize) != Some(expected)
            || expected.checked_mul(bytes_per_datapoint) != Some(reader.output_buffer_size()) {
            return Err(GribberishError::DataRepresentationTemplateError(
                "PNG dimensions disagree with packed point count".into()));
        }

        let mut image_data: Vec<u8> = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut image_data).map_err(decode_error)?;

        // Read each datapoint at its true width. The upstream 1.5.0 code always read a u16, which
        // overruns (and mis-decodes) 8-bit PNG grids like MRMS rotation-track / AzShear.
        let values = (0..image_data.len())
            .step_by(bytes_per_datapoint)
            .map(|ib| match bytes_per_datapoint {
                1 => image_data[ib] as u16,
                _ => read_u16_from_bytes(&image_data, ib).unwrap_or(0),
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
    fn png_pixels_must_match_packed_count_before_buffer_allocation() {
        let mut image = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut image, 2, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[5, 17]).unwrap();
        }
        let mut section = vec![0u8; 21];
        section[5..9].copy_from_slice(&2u32.to_be_bytes());
        section[19] = 8;
        assert_eq!(PNGDataRepresentationTemplate::new(section.clone())
            .unpack(image.view_bits::<Msb0>()).unwrap(), [5.0, 17.0]);
        for count in [1u32, 3] {
            section[5..9].copy_from_slice(&count.to_be_bytes());
            assert!(PNGDataRepresentationTemplate::new(section.clone())
                .unpack(image.view_bits::<Msb0>()).is_err());
        }
        let truncated = &image[..16];
        assert!(PNGDataRepresentationTemplate::new(section)
            .unpack(truncated.view_bits::<Msb0>()).is_err());
    }
}
