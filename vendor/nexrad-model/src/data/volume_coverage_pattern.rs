use crate::data::{ElevationCut, PulseWidth, VCPNumber};
use std::fmt::Display;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Volume Coverage Pattern (VCP) configuration describing how the radar scans a volume.
///
/// The VCP defines the scanning strategy including the number of elevation cuts, the settings
/// for each cut, and various special scanning modes like SAILS, MRLE, and MPDA.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VolumeCoveragePattern {
    pattern_number: VCPNumber,
    version: u8,
    doppler_velocity_resolution: f32,
    pulse_width: PulseWidth,

    // SAILS (Supplemental Adaptive Intra-volume Low-level Scan)
    sails_enabled: bool,
    sails_cuts: u8,

    // MRLE (Mid-volume Rescan of Low Elevation)
    mrle_enabled: bool,
    mrle_cuts: u8,

    // Other capabilities
    mpda_enabled: bool,
    base_tilt_enabled: bool,
    base_tilt_count: u8,

    // Sequencing
    sequence_active: bool,
    truncated: bool,

    // Per-elevation configuration
    elevation_cuts: Vec<ElevationCut>,
}

impl VolumeCoveragePattern {
    /// Create a new volume coverage pattern with the given configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pattern_number: u16,
        version: u8,
        doppler_velocity_resolution: f32,
        pulse_width: PulseWidth,
        sails_enabled: bool,
        sails_cuts: u8,
        mrle_enabled: bool,
        mrle_cuts: u8,
        mpda_enabled: bool,
        base_tilt_enabled: bool,
        base_tilt_count: u8,
        sequence_active: bool,
        truncated: bool,
        elevation_cuts: Vec<ElevationCut>,
    ) -> Self {
        Self {
            pattern_number: VCPNumber::from_number(pattern_number),
            version,
            doppler_velocity_resolution,
            pulse_width,
            sails_enabled,
            sails_cuts,
            mrle_enabled,
            mrle_cuts,
            mpda_enabled,
            base_tilt_enabled,
            base_tilt_count,
            sequence_active,
            truncated,
            elevation_cuts,
        }
    }

    /// The volume coverage pattern identifying the scan strategy.
    pub fn pattern_number(&self) -> VCPNumber {
        self.pattern_number
    }

    /// The VCP version number.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Doppler velocity resolution in m/s (typically 0.5 or 1.0).
    pub fn doppler_velocity_resolution(&self) -> f32 {
        self.doppler_velocity_resolution
    }

    /// The pulse width configuration.
    pub fn pulse_width(&self) -> PulseWidth {
        self.pulse_width
    }

    /// Whether SAILS (Supplemental Adaptive Intra-volume Low-level Scan) is enabled.
    ///
    /// SAILS provides additional low-level scans during the volume scan to improve
    /// temporal resolution of the lowest elevations.
    pub fn sails_enabled(&self) -> bool {
        self.sails_enabled
    }

    /// The number of SAILS cuts in this VCP.
    pub fn sails_cuts(&self) -> u8 {
        self.sails_cuts
    }

    /// Whether MRLE (Mid-volume Rescan of Low Elevation) is enabled.
    ///
    /// MRLE provides additional rescans of low elevations during the middle of a volume scan.
    pub fn mrle_enabled(&self) -> bool {
        self.mrle_enabled
    }

    /// The number of MRLE cuts in this VCP.
    pub fn mrle_cuts(&self) -> u8 {
        self.mrle_cuts
    }

    /// Whether MPDA (Multiple PRF Dealiasing Algorithm) is enabled.
    pub fn mpda_enabled(&self) -> bool {
        self.mpda_enabled
    }

    /// Whether base tilt scanning is enabled.
    pub fn base_tilt_enabled(&self) -> bool {
        self.base_tilt_enabled
    }

    /// The number of base tilts in this VCP.
    pub fn base_tilt_count(&self) -> u8 {
        self.base_tilt_count
    }

    /// Whether the VCP sequence is currently active.
    pub fn sequence_active(&self) -> bool {
        self.sequence_active
    }

    /// Whether the VCP was truncated.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// The elevation cuts comprising this volume coverage pattern.
    pub fn elevation_cuts(&self) -> &[ElevationCut] {
        &self.elevation_cuts
    }

    /// The number of elevation cuts in this VCP.
    pub fn number_of_elevation_cuts(&self) -> usize {
        self.elevation_cuts.len()
    }
}

impl Display for VolumeCoveragePattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (v{}, {} cuts",
            self.pattern_number,
            self.version,
            self.elevation_cuts.len()
        )?;

        let mut features = Vec::new();
        if self.sails_enabled {
            features.push(format!("SAILS({})", self.sails_cuts));
        }
        if self.mrle_enabled {
            features.push(format!("MRLE({})", self.mrle_cuts));
        }
        if self.mpda_enabled {
            features.push("MPDA".to_string());
        }

        if !features.is_empty() {
            write!(f, ", {}", features.join(", "))?;
        }

        write!(f, ")")
    }
}
