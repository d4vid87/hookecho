//! Pane-local satellite selection. Runtime catalogs are deliberately not persisted.
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Selection {
    pub follow_radar: bool,
    pub time: Option<DateTime<Utc>>,
    /// Stable basemap slug that supplied `time`; older saved selections have no source.
    pub source_slug: Option<String>,
}
impl Default for Selection {
    fn default() -> Self {
        Self {
            follow_radar: true,
            time: None,
            source_slug: None,
        }
    }
}
impl Selection {
    /// A manual time from another product's cadence is not a valid selection here. Reset once
    /// for older saved selections with no source identity as well.
    pub fn bind_source(&mut self, slug: &str) {
        if self.source_slug.as_deref() == Some(slug) { return; }
        self.follow_radar = true;
        self.time = None;
        self.source_slug = Some(slug.to_owned());
    }

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

    /// Forecast maps lead with the next advertised valid period, rather than the most distant
    /// frame or the radar-time nearest-neighbor rule used for observed imagery.
    pub fn selected_forecast(
        &self,
        frames: &[DateTime<Utc>],
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        if self.follow_radar || self.time.is_none() {
            frames.iter().copied().find(|t| *t >= now).or_else(|| frames.last().copied())
        } else {
            self.time.filter(|time| frames.contains(time))
        }
    }
}

/// Reject imagery more than thirty minutes from the loaded radar scan.
pub fn nearest(frames: &[DateTime<Utc>], radar: DateTime<Utc>) -> Option<DateTime<Utc>> {
    nearest_within(frames, radar, 1_800_000)
}

fn nearest_within(frames: &[DateTime<Utc>], target: DateTime<Utc>, max_offset_ms: u64) -> Option<DateTime<Utc>> {
    frames
        .iter()
        .copied()
        .min_by_key(|frame| (*frame - target).num_milliseconds().unsigned_abs())
        .filter(|frame| (*frame - target).num_milliseconds().unsigned_abs() <= max_offset_ms)
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
    /// Pane-local imagery playback, separate from the radar-volume transport.
    pub playing: bool,
    pub last_advance: Option<wxdata::clock::Instant>,
}

/// Advance one advertised frame, wrapping to the oldest only after the newest is displayed.
pub fn next_loop_frame(frames: &[DateTime<Utc>], current: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    if frames.len() < 2 { return None; }
    let i = current.and_then(|time| frames.iter().position(|frame| *frame == time));
    Some(frames[i.map_or(0, |i| (i + 1) % frames.len())])
}

/// Keep automatic playback inside the current catalog window. A live catalog may contain 48
/// frames; wrapping to its oldest can cross the four-hour archive boundary and discard itself.
/// Archive catalogs loop inside their selected hour for the same reason.
pub fn loop_window(
    frames: &[DateTime<Utc>],
    now: DateTime<Utc>,
    archive_hour: Option<i64>,
) -> &[DateTime<Utc>] {
    let (start, end) = if let Some(hour) = archive_hour {
        (
            frames.partition_point(|frame| frame.timestamp().div_euclid(3600) < hour),
            frames.partition_point(|frame| frame.timestamp().div_euclid(3600) <= hour),
        )
    } else {
        let cutoff = now - chrono::Duration::hours(3);
        (frames.partition_point(|frame| *frame < cutoff), frames.len())
    };
    let start = start.max(end.saturating_sub(8));
    &frames[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forecast_selection_starts_with_next_valid_period_and_honors_manual_step() {
        let now = DateTime::parse_from_rfc3339("2026-10-15T05:00:00Z").unwrap().with_timezone(&Utc);
        let frames = [
            DateTime::parse_from_rfc3339("2026-10-15T00:00:00Z").unwrap().with_timezone(&Utc),
            DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z").unwrap().with_timezone(&Utc),
            DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z").unwrap().with_timezone(&Utc),
        ];
        let mut selected = Selection::default();
        assert_eq!(selected.selected_forecast(&frames, now), Some(frames[1]));
        selected.follow_radar = false;
        selected.time = Some(frames[2]);
        assert_eq!(selected.selected_forecast(&frames, now), Some(frames[2]));
    }
    #[test]
    fn independent_cursors_and_missing_manual_frames() {
        let start = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let frames = [start, start + chrono::Duration::minutes(10)];
        let first = Selection {
            follow_radar: false,
            time: Some(frames[0]),
            source_slug: None,
        };
        let second = Selection {
            follow_radar: false,
            time: Some(frames[1]),
            source_slug: None,
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

    #[test]
    fn imagery_loop_advances_only_through_advertised_frames() {
        let first = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let frames = [first, first + chrono::Duration::minutes(5), first + chrono::Duration::minutes(10)];
        assert_eq!(next_loop_frame(&frames, Some(frames[0])), Some(frames[1]));
        assert_eq!(next_loop_frame(&frames, Some(frames[2])), Some(frames[0]));
        assert_eq!(next_loop_frame(&frames, Some(first - chrono::Duration::hours(1))), Some(frames[0]));
        assert_eq!(next_loop_frame(&frames[..1], Some(first)), None);
    }

    #[test]
    fn imagery_loop_stays_inside_live_or_archive_catalog_window() {
        let start: DateTime<Utc> = "2026-10-14T20:00:00Z".parse().unwrap();
        let frames: Vec<_> = (0..60).map(|i| start + chrono::Duration::minutes(i * 5)).collect();
        let now = start + chrono::Duration::hours(5);
        let live = loop_window(&frames, now, None);
        assert_eq!(live.len(), 8);
        assert_eq!(live[0], frames[52]);
        assert_eq!(next_loop_frame(live, live.last().copied()), Some(live[0]));
        let archive_hour = start.timestamp().div_euclid(3600) + 2;
        let archive = loop_window(&frames, now, Some(archive_hour));
        assert_eq!(archive.len(), 8);
        assert!(archive.iter().all(|frame| frame.timestamp().div_euclid(3600) == archive_hour));
    }

    #[test]
    fn manual_time_never_snaps_to_a_different_source_cadence() {
        let dwd_time: DateTime<Utc> = "2026-10-15T00:05:00Z".parse().unwrap();
        let eccc_frames = [
            "2026-10-15T00:00:00Z".parse().unwrap(),
            "2026-10-15T00:06:00Z".parse().unwrap(),
        ];
        let manual = Selection { follow_radar: false, time: Some(dwd_time), source_slug: None };
        assert_eq!(manual.selected(&eccc_frames, None), None);
        let expired = Selection { time: Some(dwd_time - chrono::Duration::hours(2)), ..manual };
        assert_eq!(expired.selected(&eccc_frames, None), None);
    }

    #[test]
    fn changing_imagery_source_discards_foreign_manual_time() {
        let old: DateTime<Utc> = "2026-10-15T00:05:00Z".parse().unwrap();
        let mut selection = Selection { follow_radar: false, time: Some(old), source_slug: Some("dwd-radar-wn".into()) };
        selection.bind_source("eccc-radar-rain");
        assert!(selection.follow_radar);
        assert_eq!(selection.time, None);
        assert_eq!(selection.source_slug.as_deref(), Some("eccc-radar-rain"));
        selection.follow_radar = false;
        selection.time = Some(old);
        selection.bind_source("eccc-radar-rain");
        assert_eq!(selection.time, Some(old), "same-source manual selection is preserved");
        let mut legacy: Selection = serde_json::from_str(r#"{"follow_radar":false,"time":"2026-10-15T00:05:00Z"}"#).unwrap();
        legacy.bind_source("eccc-radar-rain");
        assert!(legacy.follow_radar);
        assert_eq!(legacy.time, None);
    }
}
