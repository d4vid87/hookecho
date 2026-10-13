//! Pane-local satellite selection. Runtime catalogs are deliberately not persisted.
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Selection {
    pub follow_radar: bool,
    pub time: Option<DateTime<Utc>>,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            follow_radar: true,
            time: None,
        }
    }
}
impl Selection {
    pub fn selected(
        &self,
        frames: &[DateTime<Utc>],
        radar: Option<DateTime<Utc>>,
    ) -> Option<DateTime<Utc>> {
        if self.follow_radar {
            match radar {
                Some(time) => nearest(frames, time),
                None => frames.last().copied(),
            }
        } else {
            self.time.filter(|time| frames.contains(time)).or_else(|| {
                if self.time.is_none() {
                    frames.last().copied()
                } else {
                    None
                }
            })
        }
    }
}

/// Reject imagery more than thirty minutes from the loaded radar scan.
pub fn nearest(frames: &[DateTime<Utc>], radar: DateTime<Utc>) -> Option<DateTime<Utc>> {
    frames
        .iter()
        .copied()
        .min_by_key(|frame| (*frame - radar).num_milliseconds().unsigned_abs())
        .filter(|frame| (*frame - radar).num_milliseconds().unsigned_abs() <= 1_800_000)
}

#[derive(Default)]
pub struct Catalog {
    pub times: Vec<DateTime<Utc>>,
    pub imagery_ready: bool,
    pub imagery_incomplete: bool,
    pub style: Option<crate::tiles::BasemapStyle>,
    pub hour: Option<i64>,
    pub requested_at: Option<wxdata::clock::Instant>,
    pub rx: Option<std::sync::mpsc::Receiver<Vec<DateTime<Utc>>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_cursors_and_missing_manual_frames() {
        let start = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let frames = [start, start + chrono::Duration::minutes(10)];
        let first = Selection {
            follow_radar: false,
            time: Some(frames[0]),
        };
        let second = Selection {
            follow_radar: false,
            time: Some(frames[1]),
        };
        assert_eq!(first.selected(&frames, Some(frames[1])), Some(frames[0]));
        assert_eq!(second.selected(&frames, Some(frames[0])), Some(frames[1]));
        assert_eq!(first.selected(&frames[1..], None), None);
        assert_eq!(
            Selection::default().selected(&frames, Some(start + chrono::Duration::minutes(8))),
            Some(frames[1])
        );
        assert_eq!(Selection::default().selected(&[], Some(start)), None);
        let saved = serde_json::to_string(&second).unwrap();
        assert_eq!(serde_json::from_str::<Selection>(&saved).unwrap(), second);
        assert_eq!(
            serde_json::from_str::<Selection>("{}").unwrap(),
            Selection::default()
        );
    }
}
