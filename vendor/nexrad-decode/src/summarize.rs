//! # Summarize Module
//!
//! The `summarize` module provides functionality for generating human-readable summaries
//! of NEXRAD radar data messages. It processes raw radar messages and organizes them into
//! logical groups based on their type and content, making it easier to understand the
//! structure and content of radar data files.
//!
//! The primary function in this module is `messages()`, which takes a slice of `Message` objects
//! and returns a `MessageSummary` containing organized information about those messages.
//! The summary includes:
//!
//! * Volume coverage patterns found in the messages
//! * Logical groupings of related messages
//! * Time range of the data collection
//! * Detailed information about radar status, scan strategies, and data types

use crate::messages::{Message, MessageContents, MessageType};
use std::collections::{HashMap, HashSet};

mod message;
pub use message::MessageSummary;

mod group;
pub use group::MessageGroupSummary;

mod rda;
pub use rda::RDAStatusInfo;

mod vcp;
pub use vcp::{VCPElevationInfo, VCPInfo};

/// Processes a collection of NEXRAD messages and generates a comprehensive summary.
///
/// This function analyzes the provided messages and organizes them into logical groups
/// based on their type and content. It extracts key information such as:
///
/// * Volume coverage patterns
/// * Time range of data collection
/// * Radar operational status
/// * Scan strategy details
/// * Data types present in each elevation cut
///
/// The function handles various message types including:
/// * Digital Radar Data messages (reflectivity, velocity, etc.)
/// * RDA Status Data messages
/// * Volume Coverage Pattern messages
/// * Other message types
///
/// Messages of the same type and characteristics are grouped together to provide
/// a more concise and understandable summary.
pub fn messages(messages: &[Message]) -> MessageSummary {
    let mut summary = MessageSummary {
        volume_coverage_patterns: HashSet::new(),
        message_groups: Vec::new(),
        earliest_collection_time: None,
        latest_collection_time: None,
    };

    let mut current_group: Option<MessageGroupSummary> = None;
    for (i, message) in messages.iter().enumerate() {
        let message_type = message.header().message_type();
        let message_time = message.header().date_time();

        match message.contents() {
            MessageContents::DigitalRadarData(radar_data) => {
                if let Some(time) = message_time {
                    if (summary.earliest_collection_time.is_none()
                        || summary.earliest_collection_time > Some(time))
                        && time.timestamp_millis() > 0
                    {
                        summary.earliest_collection_time = Some(time);
                    }
                    if summary.latest_collection_time.is_none()
                        || summary.latest_collection_time < Some(time)
                    {
                        summary.latest_collection_time = Some(time);
                    }
                }

                let elevation_number = radar_data.header().elevation_number();

                let can_continue = if let Some(group) = &current_group {
                    group.message_type == MessageType::RDADigitalRadarDataGenericFormat
                        && group.elevation_number == Some(elevation_number)
                } else {
                    false
                };

                if !can_continue {
                    if let Some(group) = current_group.take() {
                        summary.message_groups.push(group);
                    }

                    current_group = Some(MessageGroupSummary {
                        message_type: MessageType::RDADigitalRadarDataGenericFormat,
                        start_time: message_time,
                        end_time: message_time,
                        message_count: 1,
                        elevation_number: Some(elevation_number),
                        elevation_angle: Some(radar_data.header().elevation_angle_raw()),
                        start_azimuth: Some(radar_data.header().azimuth_angle_raw()),
                        end_azimuth: Some(radar_data.header().azimuth_angle_raw()),
                        data_types: Some(HashMap::new()),
                        rda_status_info: None,
                        vcp_info: None,
                        is_continued: !summary.message_groups.is_empty()
                            && summary.message_groups.iter().rev().any(|g| {
                                g.message_type == MessageType::RDADigitalRadarDataGenericFormat
                                    && g.elevation_number == Some(elevation_number)
                            }),
                        start_message_index: i,
                        end_message_index: i,
                    });
                } else if let Some(group) = &mut current_group {
                    group.end_time = message_time;
                    group.message_count += 1;
                    group.end_azimuth = Some(radar_data.header().azimuth_angle_raw());
                    group.end_message_index = i;
                }

                if let Some(group) = &mut current_group {
                    if let Some(data_types) = group.data_types.as_mut() {
                        let mut increment_count = |data_type: &'static str| {
                            let count = data_types.get(data_type).unwrap_or(&0) + 1;
                            data_types.insert(data_type, count);
                        };

                        if radar_data.reflectivity_data_block().is_some() {
                            increment_count("Reflectivity");
                        }
                        if radar_data.velocity_data_block().is_some() {
                            increment_count("Velocity");
                        }
                        if radar_data.spectrum_width_data_block().is_some() {
                            increment_count("Spectrum Width");
                        }
                        if radar_data.differential_reflectivity_data_block().is_some() {
                            increment_count("Differential Reflectivity");
                        }
                        if radar_data.differential_phase_data_block().is_some() {
                            increment_count("Differential Phase");
                        }
                        if radar_data.correlation_coefficient_data_block().is_some() {
                            increment_count("Correlation Coefficient");
                        }
                        if radar_data.clutter_filter_power_data_block().is_some() {
                            increment_count("Clutter Filter Power");
                        }
                    }
                }

                if let Some(volume_data) = radar_data.volume_data_block() {
                    summary
                        .volume_coverage_patterns
                        .insert(volume_data.volume_coverage_pattern());
                }
            }
            MessageContents::RDAStatusData(status_data) => {
                if let Some(time) = message_time {
                    if (summary.earliest_collection_time.is_none()
                        || summary.earliest_collection_time > Some(time))
                        && time.timestamp_millis() > 0
                    {
                        summary.earliest_collection_time = Some(time);
                    }
                    if summary.latest_collection_time.is_none()
                        || summary.latest_collection_time < Some(time)
                    {
                        summary.latest_collection_time = Some(time);
                    }
                }

                // Each RDA status message is treated individually
                if let Some(group) = current_group.take() {
                    summary.message_groups.push(group);
                }

                current_group = Some(MessageGroupSummary {
                    message_type: MessageType::RDAStatusData,
                    start_time: message_time,
                    end_time: message_time,
                    message_count: 1,
                    elevation_number: None,
                    elevation_angle: None,
                    start_azimuth: None,
                    end_azimuth: None,
                    data_types: None,
                    rda_status_info: Some(rda::extract_rda_status_info(status_data)),
                    vcp_info: None,
                    is_continued: false,
                    start_message_index: i,
                    end_message_index: i,
                });
            }
            MessageContents::VolumeCoveragePattern(vcp_data) => {
                // Each Volume Coverage Pattern message is treated individually
                if let Some(group) = current_group.take() {
                    summary.message_groups.push(group);
                }

                current_group = Some(MessageGroupSummary {
                    message_type: MessageType::RDAVolumeCoveragePattern,
                    start_time: message_time,
                    end_time: message_time,
                    message_count: 1,
                    elevation_number: None,
                    elevation_angle: None,
                    start_azimuth: None,
                    end_azimuth: None,
                    data_types: None,
                    rda_status_info: None,
                    vcp_info: Some(vcp::extract_vcp_info(vcp_data)),
                    is_continued: false,
                    start_message_index: i,
                    end_message_index: i,
                });
            }
            _ => {
                let can_combine = if let Some(group) = &current_group {
                    group.message_type == message_type
                } else {
                    false
                };

                if !can_combine {
                    if let Some(group) = current_group.take() {
                        summary.message_groups.push(group);
                    }

                    current_group = Some(MessageGroupSummary {
                        message_type,
                        start_time: message_time,
                        end_time: message_time,
                        message_count: 1,
                        elevation_number: None,
                        elevation_angle: None,
                        start_azimuth: None,
                        end_azimuth: None,
                        data_types: None,
                        rda_status_info: None,
                        vcp_info: None,
                        is_continued: false,
                        start_message_index: i,
                        end_message_index: i,
                    });
                } else if let Some(group) = &mut current_group {
                    group.end_time = message_time;
                    group.message_count += 1;
                    group.end_message_index = i;
                }
            }
        }
    }

    if let Some(group) = current_group {
        summary.message_groups.push(group);
    }

    summary
}
