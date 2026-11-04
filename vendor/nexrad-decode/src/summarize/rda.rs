/// Contains summary information from the RDA Status Data message.
#[derive(Clone, PartialEq, Debug)]
pub struct RDAStatusInfo {
    /// Current RDA status (e.g., "Operate", "Standby").
    pub rda_status: String,
    /// System operability status.
    pub operability_status: String,
    /// Control status (local/remote).
    pub control_status: String,
    /// Operational mode (operational/maintenance).
    pub operational_mode: String,
    /// Active VCP number, if any.
    pub vcp_number: Option<i16>,
    /// Whether the VCP was set locally.
    pub vcp_is_local: bool,
    /// Average transmitter power in watts.
    pub average_transmitter_power: u16,
    /// Reflectivity calibration correction in dB.
    pub reflectivity_calibration: f32,
    /// Super resolution status.
    pub super_resolution_status: String,
    /// List of enabled data transmission types.
    pub data_transmission_enabled: Vec<String>,
    /// Whether any alarms are active.
    pub has_alarms: bool,
    /// List of active alarm descriptions.
    pub active_alarms: Vec<String>,
    /// Scan and data configuration flags.
    pub scan_data_info: Vec<String>,
}

/// Helper function to extract RDA status information from a message
pub fn extract_rda_status_info(
    message: &crate::messages::rda_status_data::Message,
) -> RDAStatusInfo {
    let mut info = RDAStatusInfo {
        rda_status: format!("{:?}", message.rda_status()),
        operability_status: format!("{:?}", message.operability_status()),
        control_status: format!("{:?}", message.control_status()),
        operational_mode: format!("{:?}", message.operational_mode()),
        vcp_number: message.volume_coverage_pattern().map(|vcp| vcp.number()),
        vcp_is_local: message
            .volume_coverage_pattern()
            .map(|vcp| vcp.local())
            .unwrap_or(false),
        average_transmitter_power: message.average_transmitter_power(),
        reflectivity_calibration: message.horizontal_reflectivity_calibration_correction(),
        super_resolution_status: format!("{:?}", message.super_resolution_status()),
        data_transmission_enabled: Vec::new(),
        has_alarms: !message.rda_alarm_summary().none(),
        active_alarms: Vec::new(),
        scan_data_info: Vec::new(),
    };

    // Data transmission enabled
    let data_enabled = message.data_transmission_enabled();
    if data_enabled.none() {
        info.data_transmission_enabled.push("None".to_string());
    } else {
        if data_enabled.reflectivity() {
            info.data_transmission_enabled
                .push("Reflectivity".to_string());
        }
        if data_enabled.velocity() {
            info.data_transmission_enabled.push("Velocity".to_string());
        }
        if data_enabled.spectrum_width() {
            info.data_transmission_enabled
                .push("Spectrum Width".to_string());
        }
    }

    // Scan data flags
    let flags = message.rda_scan_and_data_flags();
    if flags.avset_enabled() {
        info.scan_data_info.push("AVSET enabled".to_string());
    } else {
        info.scan_data_info.push("AVSET disabled".to_string());
    }
    if flags.ebc_enabled() {
        info.scan_data_info.push("EBC enabled".to_string());
    }
    if flags.rda_log_data_enabled() {
        info.scan_data_info.push("Log data enabled".to_string());
    }
    if flags.time_series_data_recording_enabled() {
        info.scan_data_info
            .push("Time series recording enabled".to_string());
    }

    // Alarms
    let alarms = message.rda_alarm_summary();
    if alarms.tower_utilities() {
        info.active_alarms.push("Tower/utilities".to_string());
    }
    if alarms.pedestal() {
        info.active_alarms.push("Pedestal".to_string());
    }
    if alarms.transmitter() {
        info.active_alarms.push("Transmitter".to_string());
    }
    if alarms.receiver() {
        info.active_alarms.push("Receiver".to_string());
    }
    if alarms.rda_control() {
        info.active_alarms.push("RDA control".to_string());
    }
    if alarms.communication() {
        info.active_alarms.push("Communication".to_string());
    }
    if alarms.signal_processor() {
        info.active_alarms.push("Signal processor".to_string());
    }

    info
}
