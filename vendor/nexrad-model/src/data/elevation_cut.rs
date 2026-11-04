use crate::data::{ChannelConfiguration, WaveformType};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Configuration for a single elevation cut in a volume coverage pattern.
///
/// Each elevation cut defines the radar settings used when scanning at a particular elevation
/// angle, including waveform type, PRF settings, and various thresholds.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ElevationCut {
    elevation_angle_degrees: f64,
    channel_configuration: ChannelConfiguration,
    waveform_type: WaveformType,
    azimuth_rate_degrees_per_second: f64,

    // Super resolution flags
    super_resolution_half_degree_azimuth: bool,
    super_resolution_quarter_km_reflectivity: bool,
    super_resolution_doppler_to_300km: bool,
    super_resolution_dual_pol_to_300km: bool,

    // PRF settings
    surveillance_prf_number: u8,
    surveillance_prf_pulse_count: u16,

    // SNR thresholds (dB)
    reflectivity_threshold_db: f32,
    velocity_threshold_db: f32,
    spectrum_width_threshold_db: f32,
    differential_reflectivity_threshold_db: f32,
    differential_phase_threshold_db: f32,
    correlation_coefficient_threshold_db: f32,

    // Special cut flags
    is_sails_cut: bool,
    sails_sequence_number: u8,
    is_mrle_cut: bool,
    mrle_sequence_number: u8,
    is_mpda_cut: bool,
    is_base_tilt_cut: bool,
}

impl ElevationCut {
    /// Create a new elevation cut with the given configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        elevation_angle_degrees: f64,
        channel_configuration: ChannelConfiguration,
        waveform_type: WaveformType,
        azimuth_rate_degrees_per_second: f64,
        super_resolution_half_degree_azimuth: bool,
        super_resolution_quarter_km_reflectivity: bool,
        super_resolution_doppler_to_300km: bool,
        super_resolution_dual_pol_to_300km: bool,
        surveillance_prf_number: u8,
        surveillance_prf_pulse_count: u16,
        reflectivity_threshold_db: f32,
        velocity_threshold_db: f32,
        spectrum_width_threshold_db: f32,
        differential_reflectivity_threshold_db: f32,
        differential_phase_threshold_db: f32,
        correlation_coefficient_threshold_db: f32,
        is_sails_cut: bool,
        sails_sequence_number: u8,
        is_mrle_cut: bool,
        mrle_sequence_number: u8,
        is_mpda_cut: bool,
        is_base_tilt_cut: bool,
    ) -> Self {
        Self {
            elevation_angle_degrees,
            channel_configuration,
            waveform_type,
            azimuth_rate_degrees_per_second,
            super_resolution_half_degree_azimuth,
            super_resolution_quarter_km_reflectivity,
            super_resolution_doppler_to_300km,
            super_resolution_dual_pol_to_300km,
            surveillance_prf_number,
            surveillance_prf_pulse_count,
            reflectivity_threshold_db,
            velocity_threshold_db,
            spectrum_width_threshold_db,
            differential_reflectivity_threshold_db,
            differential_phase_threshold_db,
            correlation_coefficient_threshold_db,
            is_sails_cut,
            sails_sequence_number,
            is_mrle_cut,
            mrle_sequence_number,
            is_mpda_cut,
            is_base_tilt_cut,
        }
    }

    /// The elevation angle for this cut in degrees.
    pub fn elevation_angle_degrees(&self) -> f64 {
        self.elevation_angle_degrees
    }

    /// The channel configuration (phase coding) for this cut.
    pub fn channel_configuration(&self) -> ChannelConfiguration {
        self.channel_configuration
    }

    /// The waveform type used for this cut.
    pub fn waveform_type(&self) -> WaveformType {
        self.waveform_type
    }

    /// The azimuth rotation rate in degrees per second.
    pub fn azimuth_rate_degrees_per_second(&self) -> f64 {
        self.azimuth_rate_degrees_per_second
    }

    /// Whether 0.5 degree azimuth super resolution is enabled.
    pub fn super_resolution_half_degree_azimuth(&self) -> bool {
        self.super_resolution_half_degree_azimuth
    }

    /// Whether 1/4 km reflectivity super resolution is enabled.
    pub fn super_resolution_quarter_km_reflectivity(&self) -> bool {
        self.super_resolution_quarter_km_reflectivity
    }

    /// Whether Doppler to 300 km super resolution is enabled.
    pub fn super_resolution_doppler_to_300km(&self) -> bool {
        self.super_resolution_doppler_to_300km
    }

    /// Whether dual polarization to 300 km super resolution is enabled.
    pub fn super_resolution_dual_pol_to_300km(&self) -> bool {
        self.super_resolution_dual_pol_to_300km
    }

    /// The pulse repetition frequency number for surveillance cuts.
    pub fn surveillance_prf_number(&self) -> u8 {
        self.surveillance_prf_number
    }

    /// The pulse count per radial for surveillance cuts.
    pub fn surveillance_prf_pulse_count(&self) -> u16 {
        self.surveillance_prf_pulse_count
    }

    /// Signal to noise ratio threshold for reflectivity in dB.
    pub fn reflectivity_threshold_db(&self) -> f32 {
        self.reflectivity_threshold_db
    }

    /// Signal to noise ratio threshold for velocity in dB.
    pub fn velocity_threshold_db(&self) -> f32 {
        self.velocity_threshold_db
    }

    /// Signal to noise ratio threshold for spectrum width in dB.
    pub fn spectrum_width_threshold_db(&self) -> f32 {
        self.spectrum_width_threshold_db
    }

    /// Signal to noise ratio threshold for differential reflectivity in dB.
    pub fn differential_reflectivity_threshold_db(&self) -> f32 {
        self.differential_reflectivity_threshold_db
    }

    /// Signal to noise ratio threshold for differential phase in dB.
    pub fn differential_phase_threshold_db(&self) -> f32 {
        self.differential_phase_threshold_db
    }

    /// Signal to noise ratio threshold for correlation coefficient in dB.
    pub fn correlation_coefficient_threshold_db(&self) -> f32 {
        self.correlation_coefficient_threshold_db
    }

    /// Whether this is a SAILS (Supplemental Adaptive Intra-volume Low-level Scan) cut.
    pub fn is_sails_cut(&self) -> bool {
        self.is_sails_cut
    }

    /// The SAILS sequence number for this cut.
    pub fn sails_sequence_number(&self) -> u8 {
        self.sails_sequence_number
    }

    /// Whether this is an MRLE (Mid-volume Rescan of Low Elevation) cut.
    pub fn is_mrle_cut(&self) -> bool {
        self.is_mrle_cut
    }

    /// The MRLE sequence number for this cut.
    pub fn mrle_sequence_number(&self) -> u8 {
        self.mrle_sequence_number
    }

    /// Whether this is an MPDA (Multiple PRF Dealiasing Algorithm) cut.
    pub fn is_mpda_cut(&self) -> bool {
        self.is_mpda_cut
    }

    /// Whether this is a base tilt cut.
    pub fn is_base_tilt_cut(&self) -> bool {
        self.is_base_tilt_cut
    }
}
