use super::{RDAStatusInfo, VCPInfo};
use crate::messages::MessageType;
use chrono::{DateTime, Utc};
use std::{
    collections::HashMap,
    fmt::{Display, Formatter, Result},
};

/// Summary of a single message or group of related messages.
#[derive(Clone, PartialEq, Debug)]
pub struct MessageGroupSummary {
    /// The type of message(s) in this group.
    pub message_type: MessageType,
    /// Start timestamp of the first message in the group.
    pub start_time: Option<DateTime<Utc>>,
    /// End timestamp of the last message in the group.
    pub end_time: Option<DateTime<Utc>>,
    /// Number of messages in this group.
    pub message_count: usize,

    /// Elevation number (for DigitalRadarData messages).
    pub elevation_number: Option<u8>,
    /// Elevation angle in degrees (for DigitalRadarData messages).
    pub elevation_angle: Option<f32>,
    /// Starting azimuth angle in degrees (for DigitalRadarData messages).
    pub start_azimuth: Option<f32>,
    /// Ending azimuth angle in degrees (for DigitalRadarData messages).
    pub end_azimuth: Option<f32>,
    /// Data types present and their counts (for DigitalRadarData messages).
    pub data_types: Option<HashMap<&'static str, usize>>,

    /// RDA status information (for RDAStatusData messages).
    pub rda_status_info: Option<RDAStatusInfo>,

    /// VCP information (for VolumeCoveragePattern messages).
    pub vcp_info: Option<VCPInfo>,

    /// Whether this group continues from a previous group.
    pub is_continued: bool,

    /// Index of the first message in this group.
    pub start_message_index: usize,
    /// Index of the last message in this group.
    pub end_message_index: usize,
}

impl Display for MessageGroupSummary {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if self.message_type == MessageType::RDADigitalRadarDataGenericFormat {
            write!(f, "Elevation: #{}", self.elevation_number.unwrap_or(0))?;

            if let Some(elev_angle) = self.elevation_angle {
                write!(f, " ({elev_angle:.2}°)")?;
            }

            write!(
                f,
                ", Azimuth: {:.1}° to {:.1}°",
                self.start_azimuth.unwrap_or(0.0),
                self.end_azimuth.unwrap_or(0.0)
            )?;

            if let Some(start) = self.start_time {
                write!(f, ", Time: {}", start.format("%H:%M:%S%.3f"))?;

                if let Some(end) = self.end_time {
                    if start != end {
                        write!(f, " to {}", end.format("%H:%M:%S%.3f"))?;
                        let duration = end.signed_duration_since(start);
                        write!(f, " ({:.2}s)", duration.num_milliseconds() as f64 / 1000.0)?;
                    }
                }
            }

            if let Some(data_types) = &self.data_types {
                if !data_types.is_empty() {
                    writeln!(f)?;
                    write!(f, "    Data types: ")?;
                    let data_types: Vec<_> = data_types.iter().collect();

                    for (i, (data_type, count)) in data_types.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }

                        // Use abbreviated names for common data types
                        let abbr = match **data_type {
                            "Reflectivity" => "REF",
                            "Velocity" => "VEL",
                            "Spectrum Width" => "SW",
                            "Differential Reflectivity" => "ZDR",
                            "Differential Phase" => "DP",
                            "Correlation Coefficient" => "CC",
                            "Clutter Filter Power" => "CFP",
                            _ => data_type,
                        };

                        write!(f, "{abbr} ({count})")?;
                    }
                }
            }
        } else if self.message_type == MessageType::RDAStatusData {
            if let Some(status_info) = &self.rda_status_info {
                write!(
                    f,
                    "RDA Status: {}, Operability: {}",
                    status_info.rda_status, status_info.operability_status
                )?;

                writeln!(f)?;
                write!(
                    f,
                    "    Control: {}, Mode: {}",
                    status_info.control_status, status_info.operational_mode
                )?;

                if let Some(vcp) = status_info.vcp_number {
                    let source = if status_info.vcp_is_local {
                        "local"
                    } else {
                        "remote"
                    };
                    write!(f, ", VCP: {vcp} ({source})")?;
                }

                writeln!(f)?;
                write!(
                    f,
                    "    Transmitter power: {} W, Reflectivity cal: {:.2} dB",
                    status_info.average_transmitter_power, status_info.reflectivity_calibration
                )?;

                writeln!(f)?;
                write!(
                    f,
                    "    Super resolution: {}",
                    status_info.super_resolution_status
                )?;

                // Data transmission
                if !status_info.data_transmission_enabled.is_empty() {
                    writeln!(f)?;
                    write!(f, "    Data enabled: ")?;
                    for (i, data_type) in status_info.data_transmission_enabled.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{data_type}")?;
                    }
                }

                // Scan flags
                if !status_info.scan_data_info.is_empty() {
                    writeln!(f)?;
                    write!(f, "    Scan settings: ")?;
                    for (i, flag) in status_info.scan_data_info.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{flag}")?;
                    }
                }

                // Alarms
                if status_info.has_alarms {
                    writeln!(f)?;
                    write!(f, "    Alarms: ")?;
                    for (i, alarm) in status_info.active_alarms.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{alarm}")?;
                    }
                }
            } else {
                write!(f, "RDA Status Data")?;
                if self.message_count > 1 {
                    write!(f, " ({})", self.message_count)?;
                }
            }
        } else if self.message_type == MessageType::RDAVolumeCoveragePattern {
            if let Some(vcp_info) = &self.vcp_info {
                write!(
                    f,
                    "VCP: #{}, Version: {}",
                    vcp_info.pattern_number, vcp_info.version
                )?;

                // Show general VCP information
                writeln!(f)?;
                write!(
                    f,
                    "    {} elevation cuts, Pulse width: {}",
                    vcp_info.number_of_elevation_cuts, vcp_info.pulse_width
                )?;

                if let Some(doppler_res) = vcp_info.doppler_velocity_resolution {
                    write!(f, ", Doppler resolution: {doppler_res:.1} m/s")?;
                }

                // VCP features
                if !vcp_info.vcp_features.is_empty() {
                    writeln!(f)?;
                    write!(f, "    Features: ")?;
                    for (i, feature) in vcp_info.vcp_features.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{feature}")?;
                    }
                }

                // Show all elevation cuts on separate lines
                if !vcp_info.elevations.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "    Elevation cuts:")?;

                    for (i, elev) in vcp_info.elevations.iter().enumerate() {
                        write!(f, "      Cut #{}: {:.2}°", i + 1, elev.elevation_angle)?;

                        if let Some(cut_type) = &elev.special_cut_type {
                            write!(f, " ({cut_type})")?;
                        }

                        // Show waveform and channel configuration
                        write!(
                            f,
                            ", {}, {}",
                            elev.waveform_type, elev.channel_configuration
                        )?;

                        // Show azimuth rate
                        write!(f, ", {:.1}°/s", elev.azimuth_rate)?;

                        // Show super resolution features if any
                        if !elev.super_resolution_features.is_empty() {
                            write!(f, ", Super res: ")?;
                            for (j, feature) in elev.super_resolution_features.iter().enumerate() {
                                if j > 0 {
                                    write!(f, ", ")?;
                                }
                                write!(f, "{feature}")?;
                            }
                        }

                        if i < vcp_info.elevations.len() - 1 {
                            writeln!(f)?;
                        }
                    }
                }
            } else {
                write!(f, "Volume Coverage Pattern")?;
                if self.message_count > 1 {
                    write!(f, " ({})", self.message_count)?;
                }
            }
        } else {
            write!(f, "{:?}", self.message_type)?;
            if self.message_count > 1 {
                write!(f, " ({})", self.message_count)?;
            }
            if let Some(start) = self.start_time {
                write!(f, ", Time: {}", start.format("%Y-%m-%d %H:%M:%S%.3f"))?;
                if let Some(end) = self.end_time {
                    if start != end {
                        write!(f, " to {}", end.format("%Y-%m-%d %H:%M:%S%.3f"))?;
                    }
                }
            }
        }
        Ok(())
    }
}
