use crate::templates::data_representation::{
    CCSDSDataRepresentationTemplate, ComplexPackingDataRepresentationTemplate,
    ComplexSpatialPackingDataRepresentationTemplate, DataRepresentationTemplate,
    SimplePackingDataRepresentationTemplate,
};
use crate::utils::{read_f32_from_bytes, read_u16_from_bytes, read_u32_from_bytes};

#[cfg(feature = "jpeg")]
use crate::templates::data_representation::JPEGDataRepresentationTemplate;
#[cfg(feature = "png")]
use crate::templates::data_representation::PNGDataRepresentationTemplate;

use super::grib_section::GribSection;

pub struct DataRepresentationSection<'a> {
    data: &'a [u8],
}

impl<'a> DataRepresentationSection<'a> {
    pub fn from_data(data: &'a [u8]) -> Self {
        DataRepresentationSection { data }
    }

    pub fn data_point_count(&self) -> usize {
        read_u32_from_bytes(self.data, 5).unwrap_or(0) as usize
    }

    pub fn data_representation_template_number(&self) -> u16 {
        read_u16_from_bytes(self.data, 9).unwrap_or(0)
    }

    /// Local patch: zero-bit simple/image/AEC fields are encoded constants, not absent data.
    pub(crate) fn constant_value(&self) -> Option<f64> {
        if self.data.get(19) != Some(&0)
            || !matches!(self.data_representation_template_number(), 0 | 40 | 41 | 42) {
            return None;
        }
        // NOAA g2c and ecCodes treat zero-bit constants as R itself, independent of scales.
        read_f32_from_bytes(self.data, 11).map(f64::from)
    }

    pub fn data_representation_template(&self) -> Option<Box<dyn DataRepresentationTemplate<f64>>> {
        let template_number = self.data_representation_template_number();
        match template_number {
            0 => Some(Box::new(SimplePackingDataRepresentationTemplate::new(
                self.data.to_vec(),
            ))),
            2 => Some(Box::new(ComplexPackingDataRepresentationTemplate::new(
                self.data.to_vec(),
            ))),
            3 => Some(Box::new(
                ComplexSpatialPackingDataRepresentationTemplate::new(self.data.to_vec()),
            )),
            #[cfg(feature = "jpeg")]
            40 => Some(Box::new(JPEGDataRepresentationTemplate::new(
                self.data.to_vec(),
            ))),
            #[cfg(feature = "png")]
            41 => Some(Box::new(PNGDataRepresentationTemplate::new(
                self.data.to_vec(),
            ))),
            42 => Some(Box::new(CCSDSDataRepresentationTemplate::new(
                self.data.to_vec(),
            ))),
            _ => None,
        }
    }
}

impl<'a> GribSection for DataRepresentationSection<'a> {
    fn len(&self) -> usize {
        read_u32_from_bytes(&self.data[0..4], 0).unwrap_or(0) as usize
    }

    fn number(&self) -> u8 {
        self.data[4]
    }
}
