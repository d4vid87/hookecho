use super::raw;
use super::{
    DataBlock, DataBlockId, ElevationDataBlock, GenericDataBlock, Header, RadialDataBlock,
    RadialStatus, VolumeDataBlock,
};
use crate::result::{Error, Result};
use crate::slice_reader::SliceReader;

/// The digital radar data message includes base radar data from a single radial for various
/// products.
#[derive(Debug, Clone, PartialEq)]
pub struct Message<'a> {
    /// The decoded digital radar data header.
    header: Header<'a>,

    /// Volume data if included in the message.
    volume_data_block: Option<DataBlock<'a, VolumeDataBlock<'a>>>,

    /// Elevation data if included in the message.
    elevation_data_block: Option<DataBlock<'a, ElevationDataBlock<'a>>>,

    /// Radial data if included in the message.
    radial_data_block: Option<DataBlock<'a, RadialDataBlock<'a>>>,

    /// Reflectivity data if included in the message.
    reflectivity_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,

    /// Velocity data if included in the message.
    velocity_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,

    /// Spectrum width data if included in the message.
    spectrum_width_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,

    /// Differential reflectivity data if included in the message.
    differential_reflectivity_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,

    /// Differential phase data if included in the message.
    differential_phase_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,

    /// Correlation coefficient data if included in the message.
    correlation_coefficient_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,

    /// Clutter filter power (CFP) data if included in the message.
    clutter_filter_power_data_block: Option<DataBlock<'a, GenericDataBlock<'a>>>,
}

impl<'a> Message<'a> {
    pub(crate) fn parse(reader: &mut SliceReader<'a>) -> Result<Self> {
        let start_position = reader.position();
        let raw_header = reader.take_ref::<raw::Header>()?;

        let pointers_space = raw_header.data_block_count.get() as usize * size_of::<u32>();
        let pointers_raw = reader.take_bytes(pointers_space)?;

        let pointers = pointers_raw
            .chunks_exact(size_of::<u32>())
            .map(|v| {
                v.try_into()
                    .map_err(|_| Error::Decoding("message pointers".to_string()))
                    .map(u32::from_be_bytes)
            })
            .collect::<Result<Vec<_>>>()?;

        let mut message = Self {
            header: Header::new(raw_header),
            volume_data_block: None,
            elevation_data_block: None,
            radial_data_block: None,
            reflectivity_data_block: None,
            velocity_data_block: None,
            spectrum_width_data_block: None,
            differential_reflectivity_data_block: None,
            differential_phase_data_block: None,
            correlation_coefficient_data_block: None,
            clutter_filter_power_data_block: None,
        };

        for pointer in pointers {
            let relative_position = reader.position() - start_position;
            let pointer_position = pointer as usize;

            if relative_position < pointer_position {
                reader.advance(pointer_position - relative_position);
            } else if relative_position > pointer_position {
                return Err(Error::InvalidDataBlockPointer {
                    bytes: relative_position - pointer_position,
                    position: reader.position(),
                });
            }

            let block_id = reader.take_ref::<raw::DataBlockId>()?;
            let id = DataBlockId::new(block_id);

            match &block_id.data_name {
                b"VOL" => {
                    // VOL expanded from 44 to 52 bytes at Build 20.0 (ICD 2620002U).
                    if reader.peek_u16().is_some_and(|lrtup| lrtup <= 44) {
                        let block = reader.take_ref::<raw::VolumeDataBlockLegacy>()?;
                        message.volume_data_block =
                            Some(DataBlock::new(id, VolumeDataBlock::new_legacy(block)));
                    } else {
                        let block = reader.take_ref::<raw::VolumeDataBlock>()?;
                        message.volume_data_block =
                            Some(DataBlock::new(id, VolumeDataBlock::new(block)));
                    }
                }
                b"ELV" => {
                    let elevation_block = reader.take_ref::<raw::ElevationDataBlock>()?;
                    message.elevation_data_block =
                        Some(DataBlock::new(id, ElevationDataBlock::new(elevation_block)));
                }
                b"RAD" => {
                    // RAD expanded from 20 to 28 bytes at Build 12.0 (ICD 2620002K).
                    if reader.peek_u16().is_some_and(|lrtup| lrtup <= 20) {
                        let block = reader.take_ref::<raw::RadialDataBlockLegacy>()?;
                        message.radial_data_block =
                            Some(DataBlock::new(id, RadialDataBlock::new_legacy(block)));
                    } else {
                        let block = reader.take_ref::<raw::RadialDataBlock>()?;
                        message.radial_data_block =
                            Some(DataBlock::new(id, RadialDataBlock::new(block)));
                    }
                }
                _ => {
                    let generic_block = GenericDataBlock::parse(reader)?;
                    match &block_id.data_name {
                        b"REF" => {
                            message.reflectivity_data_block =
                                Some(DataBlock::new(id, generic_block));
                        }
                        b"VEL" => {
                            message.velocity_data_block = Some(DataBlock::new(id, generic_block));
                        }
                        b"SW " => {
                            message.spectrum_width_data_block =
                                Some(DataBlock::new(id, generic_block));
                        }
                        b"ZDR" => {
                            message.differential_reflectivity_data_block =
                                Some(DataBlock::new(id, generic_block));
                        }
                        b"PHI" => {
                            message.differential_phase_data_block =
                                Some(DataBlock::new(id, generic_block));
                        }
                        b"RHO" => {
                            message.correlation_coefficient_data_block =
                                Some(DataBlock::new(id, generic_block));
                        }
                        b"CFP" => {
                            message.clutter_filter_power_data_block =
                                Some(DataBlock::new(id, generic_block));
                        }
                        _ => {
                            // Unknown block type - skip for forward compatibility with newer formats
                            log::warn!("Skipping unknown generic data block type: {:?}", block_id);
                        }
                    }
                }
            }
        }

        Ok(message)
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: self.header.into_owned(),
            volume_data_block: self
                .volume_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            elevation_data_block: self
                .elevation_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            radial_data_block: self
                .radial_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            reflectivity_data_block: self
                .reflectivity_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            velocity_data_block: self
                .velocity_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            spectrum_width_data_block: self
                .spectrum_width_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            differential_reflectivity_data_block: self
                .differential_reflectivity_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            differential_phase_data_block: self
                .differential_phase_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            correlation_coefficient_data_block: self
                .correlation_coefficient_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
            clutter_filter_power_data_block: self
                .clutter_filter_power_data_block
                .map(|b| b.into_owned_with(|inner| inner.into_owned())),
        }
    }

    /// The decoded digital radar data header.
    pub fn header(&self) -> &Header<'a> {
        &self.header
    }

    /// Volume data if included in the message.
    pub fn volume_data_block(&self) -> Option<&DataBlock<'a, VolumeDataBlock<'a>>> {
        self.volume_data_block.as_ref()
    }

    /// Elevation data if included in the message.
    pub fn elevation_data_block(&self) -> Option<&DataBlock<'a, ElevationDataBlock<'a>>> {
        self.elevation_data_block.as_ref()
    }

    /// Radial data if included in the message.
    pub fn radial_data_block(&self) -> Option<&DataBlock<'a, RadialDataBlock<'a>>> {
        self.radial_data_block.as_ref()
    }

    /// Reflectivity data if included in the message.
    pub fn reflectivity_data_block(&self) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.reflectivity_data_block.as_ref()
    }

    /// Velocity data if included in the message.
    pub fn velocity_data_block(&self) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.velocity_data_block.as_ref()
    }

    /// Spectrum width data if included in the message.
    pub fn spectrum_width_data_block(&self) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.spectrum_width_data_block.as_ref()
    }

    /// Differential reflectivity data if included in the message.
    pub fn differential_reflectivity_data_block(
        &self,
    ) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.differential_reflectivity_data_block.as_ref()
    }

    /// Differential phase data if included in the message.
    pub fn differential_phase_data_block(&self) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.differential_phase_data_block.as_ref()
    }

    /// Correlation coefficient data if included in the message.
    pub fn correlation_coefficient_data_block(
        &self,
    ) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.correlation_coefficient_data_block.as_ref()
    }

    /// Clutter filter power (CFP) data if included in the message.
    /// CFP represents the difference between clutter-filtered and unfiltered reflectivity.
    pub fn clutter_filter_power_data_block(&self) -> Option<&DataBlock<'a, GenericDataBlock<'a>>> {
        self.clutter_filter_power_data_block.as_ref()
    }

    /// Get a radial from this digital radar data message. This clones the underlying moment
    /// data; use [`into_radial`](Self::into_radial) to avoid the copy when the message is no
    /// longer needed.
    #[cfg(feature = "nexrad-model")]
    pub fn radial(&self) -> crate::result::Result<nexrad_model::data::Radial> {
        Self::build_radial(
            &self.header,
            self.radial_data_block.as_ref().map(|block| block.inner().nyquist_velocity_raw() as f32 * 0.01),
            self.reflectivity_data_block
                .as_ref()
                .map(|block| block.inner().moment_data()),
            self.velocity_data_block
                .as_ref()
                .map(|block| block.inner().moment_data()),
            self.spectrum_width_data_block
                .as_ref()
                .map(|block| block.inner().moment_data()),
            self.differential_reflectivity_data_block
                .as_ref()
                .map(|block| block.inner().moment_data()),
            self.differential_phase_data_block
                .as_ref()
                .map(|block| block.inner().moment_data()),
            self.correlation_coefficient_data_block
                .as_ref()
                .map(|block| block.inner().moment_data()),
            self.clutter_filter_power_data_block
                .as_ref()
                .map(|block| block.inner().cfp_moment_data()),
        )
    }

    /// Convert this digital radar data message into a common model radial, minimizing data copy.
    #[cfg(feature = "nexrad-model")]
    pub fn into_radial(self) -> crate::result::Result<nexrad_model::data::Radial> {
        Self::build_radial(
            &self.header,
            self.radial_data_block.as_ref().map(|block| block.inner().nyquist_velocity_raw() as f32 * 0.01),
            self.reflectivity_data_block
                .map(|block| block.into_inner().into_moment_data()),
            self.velocity_data_block
                .map(|block| block.into_inner().into_moment_data()),
            self.spectrum_width_data_block
                .map(|block| block.into_inner().into_moment_data()),
            self.differential_reflectivity_data_block
                .map(|block| block.into_inner().into_moment_data()),
            self.differential_phase_data_block
                .map(|block| block.into_inner().into_moment_data()),
            self.correlation_coefficient_data_block
                .map(|block| block.into_inner().into_moment_data()),
            self.clutter_filter_power_data_block
                .map(|block| block.into_inner().into_cfp_moment_data()),
        )
    }

    #[cfg(feature = "nexrad-model")]
    fn build_radial(
        header: &Header<'_>,
        nyquist_velocity_mps: Option<f32>,
        reflectivity: Option<nexrad_model::data::MomentData>,
        velocity: Option<nexrad_model::data::MomentData>,
        spectrum_width: Option<nexrad_model::data::MomentData>,
        differential_reflectivity: Option<nexrad_model::data::MomentData>,
        differential_phase: Option<nexrad_model::data::MomentData>,
        correlation_coefficient: Option<nexrad_model::data::MomentData>,
        clutter_filter_power: Option<nexrad_model::data::CFPMomentData>,
    ) -> crate::result::Result<nexrad_model::data::Radial> {
        use crate::result::Error;
        use nexrad_model::data::{Radial, RadialStatus as ModelRadialStatus};

        Ok(Radial::new(
            header
                .date_time()
                .ok_or(Error::MessageMissingDate)?
                .timestamp_millis(),
            header.azimuth_number(),
            header.azimuth_angle_raw(),
            header.azimuth_resolution_spacing_raw() as f32 * 0.5,
            match header.radial_status() {
                RadialStatus::ElevationStart => ModelRadialStatus::ElevationStart,
                RadialStatus::IntermediateRadialData => ModelRadialStatus::IntermediateRadialData,
                RadialStatus::ElevationEnd => ModelRadialStatus::ElevationEnd,
                RadialStatus::VolumeScanStart => ModelRadialStatus::ScanStart,
                RadialStatus::VolumeScanEnd => ModelRadialStatus::ScanEnd,
                RadialStatus::ElevationStartVCPFinal => ModelRadialStatus::ElevationStartVCPFinal,
                RadialStatus::Unknown(v) => ModelRadialStatus::Unknown(v),
            },
            header.elevation_number(),
            header.elevation_angle_raw(),
            reflectivity,
            velocity,
            spectrum_width,
            differential_reflectivity,
            differential_phase,
            correlation_coefficient,
            clutter_filter_power,
        ).with_nyquist_velocity_mps(nyquist_velocity_mps))
    }
}
