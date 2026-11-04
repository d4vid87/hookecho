use chrono::{DateTime, Utc};

use crate::messages::{digital_radar_data, MessageType};
use std::{
    collections::HashSet,
    fmt::{Display, Formatter, Result},
};

use super::MessageGroupSummary;

/// Summary of a set of messages.
#[derive(Clone, PartialEq, Debug)]
pub struct MessageSummary {
    /// The distinct volume coverage patterns found in these messages.
    pub volume_coverage_patterns: HashSet<digital_radar_data::VolumeCoveragePattern>,

    /// All messages in sequence, with related messages grouped together.
    pub message_groups: Vec<MessageGroupSummary>,

    /// Earliest collection timestamp across all messages.
    pub earliest_collection_time: Option<DateTime<Utc>>,
    /// Latest collection timestamp across all messages.
    pub latest_collection_time: Option<DateTime<Utc>>,
}

impl Display for MessageSummary {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        // Time information
        write!(f, "Scans from ")?;
        if let Some(start) = self.earliest_collection_time {
            write!(f, "{}", start.format("%Y-%m-%d %H:%M:%S%.3f UTC"))?;
        } else {
            write!(f, "unknown")?;
        }

        write!(f, " to ")?;
        if let Some(end) = self.latest_collection_time {
            write!(f, "{}", end.format("%Y-%m-%d %H:%M:%S%.3f UTC"))?;
            if let Some(start) = self.earliest_collection_time {
                let duration = end.signed_duration_since(start);
                write!(f, " ({:.2}m)", duration.num_milliseconds() as f64 / 60000.0)?;
            }
        } else {
            write!(f, "unknown")?;
        }
        writeln!(f)?;

        // Volume coverage patterns
        write!(f, "VCPs: ")?;
        if self.volume_coverage_patterns.is_empty() {
            writeln!(f, "none")?;
        } else {
            let vcps: Vec<_> = self.volume_coverage_patterns.iter().collect();
            for (i, vcp) in vcps.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{vcp:?}")?;
            }
            writeln!(f)?;
        }

        // Messages
        writeln!(f, "Messages:")?;
        for group in self.message_groups.iter() {
            let prefix = if group.message_type == MessageType::RDADigitalRadarDataGenericFormat {
                let msg_range = if group.start_message_index == group.end_message_index {
                    format!("  Msg {}", group.start_message_index + 1)
                } else {
                    format!(
                        "  Msg {}-{}",
                        group.start_message_index + 1,
                        group.end_message_index + 1
                    )
                };

                if group.is_continued {
                    format!("{msg_range} (cont.)")
                } else {
                    msg_range
                }
            } else if group.start_message_index == group.end_message_index {
                format!("  Msg {}", group.start_message_index + 1)
            } else {
                format!(
                    "  Msg {}-{}",
                    group.start_message_index + 1,
                    group.end_message_index + 1
                )
            };
            writeln!(f, "{prefix}: {group}")?;
        }

        Ok(())
    }
}
