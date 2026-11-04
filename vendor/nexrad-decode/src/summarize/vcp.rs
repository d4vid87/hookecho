/// Contains summary information from the Volume Coverage Pattern message.
#[derive(Clone, PartialEq, Debug)]
pub struct VCPInfo {
    /// VCP pattern number (e.g., 12, 31, 212).
    pub pattern_number: u16,
    /// VCP version number.
    pub version: u8,
    /// Number of elevation cuts in the VCP.
    pub number_of_elevation_cuts: u16,
    /// Pulse width description.
    pub pulse_width: String,
    /// Doppler velocity resolution in m/s, if available.
    pub doppler_velocity_resolution: Option<f64>,
    /// VCP features enabled (SAILS, MRLE, MPDA, etc.).
    pub vcp_features: Vec<String>,
    /// Information about each elevation cut.
    pub elevations: Vec<VCPElevationInfo>,
}

/// Summary information about a single elevation cut in a VCP.
#[derive(Clone, PartialEq, Debug)]
pub struct VCPElevationInfo {
    /// Elevation angle in degrees.
    pub elevation_angle: f64,
    /// Channel configuration description.
    pub channel_configuration: String,
    /// Waveform type description.
    pub waveform_type: String,
    /// Azimuth rotation rate in degrees per second.
    pub azimuth_rate: f64,
    /// Super resolution features enabled for this cut.
    pub super_resolution_features: Vec<String>,
    /// Special cut type (SAILS, MRLE, MPDA, Base tilt), if applicable.
    pub special_cut_type: Option<String>,
}

/// Helper function to extract Volume Coverage Pattern information from a message
pub fn extract_vcp_info(message: &crate::messages::volume_coverage_pattern::Message) -> VCPInfo {
    let mut vcp_features = Vec::new();

    if message.header().is_sails_vcp() {
        let sails_cuts = message.header().number_of_sails_cuts();
        vcp_features.push(format!("SAILS ({sails_cuts} cuts)"));
    }

    if message.header().is_mrle_vcp() {
        let mrle_cuts = message.header().number_of_mrle_cuts();
        vcp_features.push(format!("MRLE ({mrle_cuts} cuts)"));
    }

    if message.header().is_mpda_vcp() {
        vcp_features.push("MPDA".to_string());
    }

    if message.header().is_base_tilt_vcp() {
        let base_tilts = message.header().number_of_base_tilts();
        vcp_features.push(format!("Base tilts ({base_tilts} cuts)"));
    }

    if message.header().vcp_sequencing_sequence_active() {
        vcp_features.push("VCP sequence active".to_string());
    }

    if message.header().vcp_sequencing_truncated() {
        vcp_features.push("Truncated VCP".to_string());
    }

    let mut elevations = Vec::new();
    for elev in message.elevations().iter() {
        let mut super_res_features = Vec::new();
        if elev.super_resolution_half_degree_azimuth() {
            super_res_features.push("0.5° azimuth".to_string());
        }
        if elev.super_resolution_quarter_km_reflectivity() {
            super_res_features.push("0.25 km reflectivity".to_string());
        }
        if elev.super_resolution_doppler_to_300km() {
            super_res_features.push("Doppler to 300 km".to_string());
        }
        if elev.super_resolution_dual_pol_to_300km() {
            super_res_features.push("Dual pol to 300 km".to_string());
        }

        // Determine special cut type
        let mut special_cut_type = None;
        if elev.is_sails_cut() {
            let seq = elev.sails_sequence_number();
            special_cut_type = Some(format!("SAILS {seq}"));
        } else if elev.is_mrle_cut() {
            let seq = elev.mrle_sequence_number();
            special_cut_type = Some(format!("MRLE {seq}"));
        } else if elev.is_mpda_cut() {
            special_cut_type = Some("MPDA".to_string());
        } else if elev.is_base_tilt_cut() {
            special_cut_type = Some("Base tilt".to_string());
        }

        elevations.push(VCPElevationInfo {
            elevation_angle: elev.elevation_angle(),
            channel_configuration: format!("{:?}", elev.channel_configuration()),
            waveform_type: format!("{:?}", elev.waveform_type()),
            azimuth_rate: elev.azimuth_rate(),
            super_resolution_features: super_res_features,
            special_cut_type,
        });
    }

    let doppler_res = message.header().doppler_velocity_resolution();
    VCPInfo {
        pattern_number: message.header().pattern_number(),
        version: message.header().version(),
        number_of_elevation_cuts: message.header().number_of_elevation_cuts(),
        pulse_width: format!("{:?}", message.header().pulse_width()),
        doppler_velocity_resolution: if doppler_res > 0.0 {
            Some(doppler_res as f64)
        } else {
            None
        },
        vcp_features,
        elevations,
    }
}
