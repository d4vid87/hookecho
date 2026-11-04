use super::raw::{self, decode_angle, decode_angular_velocity};
use super::{ChannelConfiguration, WaveformType};
use std::borrow::Cow;

/// A data block for a single elevation cut.
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

    /// The elevation angle for this cut (raw encoded value)
    pub fn elevation_angle_raw(&self) -> u16 {
        self.inner.elevation_angle.get()
    }

    /// The channel configuration for this cut (raw value)
    /// 0 => Constant Phase
    /// 1 => Random Phase
    /// 2 => SZ2 Phase
    pub fn channel_configuration_raw(&self) -> u8 {
        self.inner.channel_configuration
    }

    /// The waveform type for this cut (raw value)
    /// 1 => Contiguous Surveillance
    /// 2 => Contiguous Doppler w/ Ambiguity Resolution
    /// 3 => Contiguous Doppler w/o Ambiguity Resolution
    /// 4 => Batch
    /// 5 => Staggered Pulse Pair
    pub fn waveform_type_raw(&self) -> u8 {
        self.inner.waveform_type
    }

    /// Super resolution control values for this cut (raw value)
    /// Bit 0: 0.5 degree azimuth
    /// Bit 1: 1/4 km reflectivity
    /// Bit 2: Doppler to 300 km
    /// Bit 3: Dual polarization to 300 km
    pub fn super_resolution_control(&self) -> u8 {
        self.inner.super_resolution_control
    }

    /// The pulse repetition frequency number for surveillance cuts
    pub fn surveillance_prf_number(&self) -> u8 {
        self.inner.surveillance_prf_number
    }

    /// The pulse count per radial for surveillance cuts
    pub fn surveillance_prf_pulse_count_radial(&self) -> u16 {
        self.inner.surveillance_prf_pulse_count_radial.get()
    }

    /// The azimuth rate of the cut (raw encoded value)
    pub fn azimuth_rate_raw(&self) -> u16 {
        self.inner.azimuth_rate.get()
    }

    /// Signal to noise ratio (SNR) threshold for reflectivity (raw scaled value)
    pub fn reflectivity_threshold_raw(&self) -> i16 {
        self.inner.reflectivity_threshold.get()
    }

    /// Signal to noise ratio (SNR) threshold for velocity (raw scaled value)
    pub fn velocity_threshold_raw(&self) -> i16 {
        self.inner.velocity_threshold.get()
    }

    /// Signal to noise ratio (SNR) threshold for spectrum width (raw scaled value)
    pub fn spectrum_width_threshold_raw(&self) -> i16 {
        self.inner.spectrum_width_threshold.get()
    }

    /// Signal to noise ratio (SNR) threshold for differential reflectivity (raw scaled value)
    pub fn differential_reflectivity_threshold_raw(&self) -> i16 {
        self.inner.differential_reflectivity_threshold.get()
    }

    /// Signal to noise ratio (SNR) threshold for differential phase (raw scaled value)
    pub fn differential_phase_threshold_raw(&self) -> i16 {
        self.inner.differential_phase_threshold.get()
    }

    /// Signal to noise ratio (SNR) threshold for correlation coefficient (raw scaled value)
    pub fn correlation_coefficient_threshold_raw(&self) -> i16 {
        self.inner.correlation_coefficient_threshold.get()
    }

    /// Sector 1 Azimuth Clockwise Edge Angle (raw encoded value, denotes start angle)
    pub fn sector_1_edge_angle_raw(&self) -> u16 {
        self.inner.sector_1_edge_angle.get()
    }

    /// Sector 1 Doppler PRF Number
    pub fn sector_1_doppler_prf_number(&self) -> u16 {
        self.inner.sector_1_doppler_prf_number.get()
    }

    /// Sector 1 Doppler Pulse Count/Radial
    pub fn sector_1_doppler_prf_pulse_count_radial(&self) -> u16 {
        self.inner.sector_1_doppler_prf_pulse_count_radial.get()
    }

    /// Supplemental Data (raw value)
    /// Bit 0:    SAILS Cut
    /// Bits 1-3: SAILS Sequence Number
    /// Bit 4:    MRLE Cut
    /// Bits 5-7: MRLE Sequence Number
    /// Bit 8:    Spare
    /// Bit 9:    MPDA Cut
    /// Bit 10:   BASE TILT Cut
    pub fn supplemental_data(&self) -> u16 {
        self.inner.supplemental_data.get()
    }

    /// Sector 2 Azimuth Clockwise Edge Angle (raw encoded value, denotes start angle)
    pub fn sector_2_edge_angle_raw(&self) -> u16 {
        self.inner.sector_2_edge_angle.get()
    }

    /// Sector 2 Doppler PRF Number
    pub fn sector_2_doppler_prf_number(&self) -> u16 {
        self.inner.sector_2_doppler_prf_number.get()
    }

    /// Sector 2 Doppler Pulse Count/Radial
    pub fn sector_2_doppler_prf_pulse_count_radial(&self) -> u16 {
        self.inner.sector_2_doppler_prf_pulse_count_radial.get()
    }

    /// The correction added to the elevation angle for this cut (raw encoded value)
    pub fn ebc_angle_raw(&self) -> u16 {
        self.inner.ebc_angle.get()
    }

    /// Sector 3 Azimuth Clockwise Edge Angle (raw encoded value, denotes start angle)
    pub fn sector_3_edge_angle_raw(&self) -> u16 {
        self.inner.sector_3_edge_angle.get()
    }

    /// Sector 3 Doppler PRF Number
    pub fn sector_3_doppler_prf_number(&self) -> u16 {
        self.inner.sector_3_doppler_prf_number.get()
    }

    /// Sector 3 Doppler Pulse Count/Radial
    pub fn sector_3_doppler_prf_pulse_count_radial(&self) -> u16 {
        self.inner.sector_3_doppler_prf_pulse_count_radial.get()
    }

    /// The elevation angle for this cut in degrees
    pub fn elevation_angle(&self) -> f64 {
        decode_angle(self.inner.elevation_angle)
    }

    /// The channel configuration for this cut
    pub fn channel_configuration(&self) -> ChannelConfiguration {
        match self.inner.channel_configuration {
            0 => ChannelConfiguration::ConstantPhase,
            1 => ChannelConfiguration::RandomPhase,
            2 => ChannelConfiguration::SZ2Phase,
            _ => ChannelConfiguration::UnknownPhase,
        }
    }

    /// The waveform type for this cut
    pub fn waveform_type(&self) -> WaveformType {
        match self.inner.waveform_type {
            1 => WaveformType::CS,
            2 => WaveformType::CDW,
            3 => WaveformType::CDWO,
            4 => WaveformType::B,
            5 => WaveformType::SPP,
            _ => WaveformType::Unknown,
        }
    }

    /// The azimuth rate of the cut in degrees/second
    pub fn azimuth_rate(&self) -> f64 {
        decode_angular_velocity(self.inner.azimuth_rate)
    }

    /// Signal to noise ratio (SNR) threshold for reflectivity in dB
    pub fn reflectivity_threshold(&self) -> f32 {
        self.inner.reflectivity_threshold.get() as f32 / 8.0
    }

    /// Signal to noise ratio (SNR) threshold for velocity in dB
    pub fn velocity_threshold(&self) -> f32 {
        self.inner.velocity_threshold.get() as f32 / 8.0
    }

    /// Signal to noise ratio (SNR) threshold for spectrum width in dB
    pub fn spectrum_width_threshold(&self) -> f32 {
        self.inner.spectrum_width_threshold.get() as f32 / 8.0
    }

    /// Signal to noise ratio (SNR) threshold for differential reflectivity in dB
    pub fn differential_reflectivity_threshold(&self) -> f32 {
        self.inner.differential_reflectivity_threshold.get() as f32 / 8.0
    }

    /// Signal to noise ratio (SNR) threshold for differential phase in dB
    pub fn differential_phase_threshold(&self) -> f32 {
        self.inner.differential_phase_threshold.get() as f32 / 8.0
    }

    /// Signal to noise ratio (SNR) threshold for correlation coefficient in dB
    pub fn correlation_coefficient_threshold(&self) -> f32 {
        self.inner.correlation_coefficient_threshold.get() as f32 / 8.0
    }

    /// Sector 1 Azimuth Clockwise Edge Angle in degrees (denotes start angle)
    pub fn sector_1_edge_angle(&self) -> f64 {
        decode_angle(self.inner.sector_1_edge_angle)
    }

    /// Sector 2 Azimuth Clockwise Edge Angle in degrees (denotes start angle)
    pub fn sector_2_edge_angle(&self) -> f64 {
        decode_angle(self.inner.sector_2_edge_angle)
    }

    /// Sector 3 Azimuth Clockwise Edge Angle in degrees (denotes start angle)
    pub fn sector_3_edge_angle(&self) -> f64 {
        decode_angle(self.inner.sector_3_edge_angle)
    }

    /// The correction added to the elevation angle for this cut in degrees
    pub fn ebc_angle(&self) -> f64 {
        decode_angle(self.inner.ebc_angle)
    }

    /// Whether 0.5 degree azimuth super resolution is enabled
    pub fn super_resolution_half_degree_azimuth(&self) -> bool {
        self.inner.super_resolution_control & 1 == 1
    }

    /// Whether 1/4 km reflectivity super resolution is enabled
    pub fn super_resolution_quarter_km_reflectivity(&self) -> bool {
        (self.inner.super_resolution_control >> 1) & 1 == 1
    }

    /// Whether Doppler to 300 km super resolution is enabled
    pub fn super_resolution_doppler_to_300km(&self) -> bool {
        (self.inner.super_resolution_control >> 2) & 1 == 1
    }

    /// Whether dual polarization to 300 km super resolution is enabled
    pub fn super_resolution_dual_pol_to_300km(&self) -> bool {
        (self.inner.super_resolution_control >> 3) & 1 == 1
    }

    /// Whether this is a SAILS cut
    pub fn is_sails_cut(&self) -> bool {
        self.inner.supplemental_data.get() & 1 == 1
    }

    /// SAILS sequence number
    pub fn sails_sequence_number(&self) -> u8 {
        ((self.inner.supplemental_data.get() >> 1) & 0x07) as u8
    }

    /// Whether this is an MRLE cut
    pub fn is_mrle_cut(&self) -> bool {
        (self.inner.supplemental_data.get() >> 4) & 1 == 1
    }

    /// MRLE sequence number
    pub fn mrle_sequence_number(&self) -> u8 {
        ((self.inner.supplemental_data.get() >> 5) & 0x07) as u8
    }

    /// Whether this is an MPDA cut
    pub fn is_mpda_cut(&self) -> bool {
        (self.inner.supplemental_data.get() >> 9) & 1 == 1
    }

    /// Whether this is a BASE TILT cut
    pub fn is_base_tilt_cut(&self) -> bool {
        (self.inner.supplemental_data.get() >> 10) & 1 == 1
    }
}
