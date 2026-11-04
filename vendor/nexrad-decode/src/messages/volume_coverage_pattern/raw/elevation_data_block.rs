use std::fmt::Debug;

use crate::messages::primitive_aliases::{Code1, Code2, Integer1, Integer2, ScaledSInteger2};
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// A data block for a single elevation cut.
#[repr(C)]
#[derive(Clone, PartialEq, Debug, FromBytes, Immutable, KnownLayout)]
pub struct ElevationDataBlock {
    /// The elevation angle for this cut
    pub elevation_angle: Code2,

    /// The channel configuration for this cut
    /// 0 => Constant Phase
    /// 1 => Random Phase
    /// 2 => SZ2 Phase
    pub channel_configuration: Code1,

    /// The waveform type for this cut
    /// 1 => Contiguous Surveillance
    /// 2 => Contiguous Doppler w/ Ambiguity Resolution
    /// 3 => Contiguous Doppler w/o Ambiguity Resolution
    /// 4 => Batch
    /// 5 => Staggered Pulse Pair
    pub waveform_type: Code1,

    /// Super resolution control values for this cut
    /// Bit 0: 0.5 degree azimuth
    /// Bit 1: 1/4 km reflectivity
    /// Bit 2: Doppler to 300 km
    /// Bit 3: Dual polarization to 300 km
    pub super_resolution_control: Code1,

    /// The pulse repetition frequency number for surveillance cuts
    pub surveillance_prf_number: Integer1,

    /// The pulse count per radial for surveillance cuts
    pub surveillance_prf_pulse_count_radial: Integer2,

    /// The azimuth rate of the cut
    pub azimuth_rate: Code2,

    /// Signal to noise ratio (SNR) threshold for reflectivity
    pub reflectivity_threshold: ScaledSInteger2,

    /// Signal to noise ratio (SNR) threshold for velocity
    pub velocity_threshold: ScaledSInteger2,

    /// Signal to noise ratio (SNR) threshold for spectrum width
    pub spectrum_width_threshold: ScaledSInteger2,

    /// Signal to noise ratio (SNR) threshold for differential reflectivity
    pub differential_reflectivity_threshold: ScaledSInteger2,

    /// Signal to noise ratio (SNR) threshold for differential phase
    pub differential_phase_threshold: ScaledSInteger2,

    /// Signal to noise ratio (SNR) threshold for correlation coefficitn
    pub correlation_coefficient_threshold: ScaledSInteger2,

    /// Sector 1 Azimuth Clockwise Edge Angle (denotes start angle)
    pub sector_1_edge_angle: Code2,

    /// Sector 1 Doppler PRF Number
    pub sector_1_doppler_prf_number: Integer2,

    /// Sector 1 Doppler Pulse Count/Radial
    pub sector_1_doppler_prf_pulse_count_radial: Integer2,

    /// Supplemental Data
    /// Bit 0:    SAILS Cut
    /// Bits 1-3: SAILS Sequence Number
    /// Bit 4:    MRLE Cut
    /// Bits 5-7: MRLE Sequence Number
    /// Bit 8:    Spare
    /// Bit 9:    MPDA Cut
    /// Bit 10:   BASE TILT Cut
    pub supplemental_data: Code2,

    /// Sector 2 Azimuth Clockwise Edge Angle (denotes start angle)
    pub sector_2_edge_angle: Code2,

    /// Sector 2 Doppler PRF Number
    pub sector_2_doppler_prf_number: Integer2,

    /// Sector 2 Doppler Pulse Count/Radial
    pub sector_2_doppler_prf_pulse_count_radial: Integer2,

    /// The correction added to the elevation angle for this cut
    pub ebc_angle: Code2,

    /// Sector 3 Azimuth Clockwise Edge Angle (denotes start angle)
    pub sector_3_edge_angle: Code2,

    /// Sector 3 Doppler PRF Number
    pub sector_3_doppler_prf_number: Integer2,

    /// Sector 3 Doppler Pulse Count/Radial
    pub sector_3_doppler_prf_pulse_count_radial: Integer2,

    /// Reserved
    pub reserved: Integer2,
}

/// Decodes an angle as defined in table III-A of ICD 2620002AA
pub fn decode_angle(raw: Code2) -> f64 {
    let mut angle: f64 = 0.0;
    for i in 3..16 {
        if ((raw >> i) & 1) == 1 {
            angle += 180.0 * f64::powf(2.0, (i - 15) as f64);
        }
    }

    angle
}

/// Decodes an angular velocity as defined in table XI-D of ICD 2620002AA
pub fn decode_angular_velocity(raw: Code2) -> f64 {
    let mut angular_velocity: f64 = 0.0;

    for i in 3..15 {
        if ((raw >> i) & 1) == 1 {
            angular_velocity += 22.5 * f64::powf(2.0, (i - 14) as f64);
        }
    }

    if ((raw >> 15) & 1) == 1 {
        angular_velocity = -angular_velocity
    }

    angular_velocity
}
