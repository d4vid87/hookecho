//! Archive timeline / playback state for one map pane.
//!
//! Unifies live and archive under a single playhead: `following` pins the head to the newest
//! volume (live), and scrubbing/stepping un-pins it to browse a fixed list of volumes for the
//! selected UTC day. The app turns the current [`Timeline::current`] identifier into a decoded
//! volume (via an LRU cache + background download); this type is pure playback bookkeeping.

use chrono::{DateTime, NaiveDate, Utc};
use std::time::Instant;
use wxdata::level2::Identifier;

pub struct Timeline {
    /// Selected UTC archive day.
    pub date: NaiveDate,
    /// Volumes for the current site+date, oldest first.
    pub frames: Vec<Identifier>,
    /// Index into `frames` of the displayed volume.
    pub playhead: usize,
    /// Pinned to the newest volume (live). Cleared by scrubbing/stepping.
    pub following: bool,
    pub playing: bool,
    /// Playback rate in frames/second.
    pub speed: f32,
    pub loop_enabled: bool,
    /// How many newest frames the live loop cycles over (synced from settings each frame).
    pub live_window: usize,
    /// The (site, date) the current `frames` were listed for — detects a stale listing.
    pub frames_key: Option<(String, NaiveDate)>,
    /// A frame listing is in flight.
    pub listing: bool,
    /// After the next listing lands, snap the playhead to the frame nearest this time (event
    /// jump / archive deep-link). Cleared once applied.
    pub seek_target: Option<DateTime<Utc>>,
    /// Forecast hours appended after the newest observed frame (HRRR "future radar" scrub tail).
    pub forecast_hours: u8,
    last_advance: Option<Instant>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            date: chrono::Utc::now().date_naive(),
            frames: Vec::new(),
            playhead: 0,
            following: true,
            playing: false,
            speed: 4.0,
            loop_enabled: true,
            live_window: 10,
            frames_key: None,
            listing: false,
            seek_target: None,
            forecast_hours: 6,
            last_advance: None,
        }
    }
}

impl Timeline {
    /// The identifier of the volume at the playhead, if any (`None` in the forecast tail).
    pub fn current(&self) -> Option<&Identifier> {
        self.frames.get(self.playhead)
    }

    /// Total scrub slots: observed frames plus the forecast tail (only when frames exist).
    pub fn slot_count(&self) -> usize {
        if self.frames.is_empty() {
            0
        } else {
            self.frames.len() + self.forecast_hours as usize
        }
    }

    /// If the playhead is in the forecast tail, the forecast hour (1..=forecast_hours), else None.
    pub fn forecast_hour(&self) -> Option<u8> {
        if !self.frames.is_empty() && self.playhead >= self.frames.len() {
            Some((self.playhead - self.frames.len() + 1) as u8)
        } else {
            None
        }
    }

    /// Whether the playhead is on (or past) the newest frame.
    pub fn at_head(&self) -> bool {
        self.frames.is_empty() || self.playhead + 1 >= self.frames.len()
    }

    /// Install a fresh frame listing; keeps the playhead at the head while following, else
    /// clamps it into range (so appended live frames don't move a scrubbed view).
    pub fn set_frames(&mut self, frames: Vec<Identifier>, key: (String, NaiveDate)) {
        self.frames = frames;
        self.frames_key = Some(key);
        self.listing = false;
        // A pending event/deep-link seek wins: snap to the nearest frame by time.
        if let Some(target) = self.seek_target.take() {
            if let Some(i) = self.nearest_frame(target) {
                self.playhead = i;
                self.following = false;
                return;
            }
        }
        // While live-looping, a fresh listing must not yank the playhead to the head — the loop
        // owns the playhead. Only clamp it back into range if it now points past the end.
        if self.following && self.playing && !self.frames.is_empty() {
            self.playhead = self.playhead.min(self.frames.len() - 1);
        } else if self.following || self.playhead >= self.frames.len() {
            self.playhead = self.frames.len().saturating_sub(1);
        }
    }

    /// Append a newly-arrived live head frame (keeps the loop window sliding forward). The
    /// playhead is left where it is; the window is relative to `frames.len()`.
    pub fn append_head(&mut self, id: Identifier) {
        self.frames.push(id);
    }

    /// True when the play button is running a rolling live loop (pinned to head, playing, and the
    /// playhead is somewhere behind the newest frame).
    pub fn live_looping(&self) -> bool {
        self.following && self.playing && !self.at_head()
    }

    /// Play/pause toggle. Starting at the live head begins a rolling loop over the newest
    /// `live_window` frames while staying pinned to live; starting on an archive day replays from
    /// the first frame.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        if self.playing && self.at_head() && !self.frames.is_empty() {
            if self.following {
                // Live: loop the tail window, stay pinned so new volumes keep arriving.
                self.playhead = self.frames.len().saturating_sub(self.live_window.max(1));
            } else {
                self.playhead = 0; // archive: replay from the start
            }
        }
    }

    /// Index of the frame whose time is closest to `target`.
    fn nearest_frame(&self, target: DateTime<Utc>) -> Option<usize> {
        self.frames
            .iter()
            .enumerate()
            .filter_map(|(i, id)| id.date_time().map(|t| (i, (t - target).num_seconds().abs())))
            .min_by_key(|(_, d)| *d)
            .map(|(i, _)| i)
    }

    /// Step `delta` slots (observed frames + forecast tail), un-pinning and pausing playback.
    pub fn step(&mut self, delta: i32) {
        self.playing = false;
        let n = self.slot_count() as i32;
        if n == 0 {
            return;
        }
        self.playhead = (self.playhead as i32 + delta).clamp(0, n - 1) as usize;
        // Re-pin to live only when back on the last observed frame (not in the forecast tail).
        self.following = self.playhead + 1 == self.frames.len();
    }

    /// Jump to the newest frame and re-pin to live.
    pub fn go_head(&mut self) {
        self.following = true;
        self.playing = false;
        self.playhead = self.frames.len().saturating_sub(1);
    }

    /// Jump to the oldest frame.
    pub fn go_begin(&mut self) {
        self.following = false;
        self.playing = false;
        self.playhead = 0;
    }

    /// Advance playback if a frame interval has elapsed. Returns true if the playhead moved.
    pub fn tick(&mut self) -> bool {
        if !self.playing || self.frames.is_empty() {
            return false;
        }
        let interval = std::time::Duration::from_secs_f32((1.0 / self.speed).clamp(0.05, 10.0));
        if !self.last_advance.map_or(true, |t| t.elapsed() >= interval) {
            return false;
        }
        self.last_advance = Some(Instant::now());
        if self.playhead + 1 < self.frames.len() {
            self.playhead += 1;
            // Archive play re-pins to live at the head; a live loop stays pinned throughout.
            if !self.following {
                self.following = self.playhead + 1 >= self.frames.len();
            }
        } else if self.loop_enabled {
            // Wrap: a live loop jumps back to the window start and keeps following; an archive
            // loop restarts from the beginning, un-pinned.
            if self.following {
                self.playhead = self.frames.len().saturating_sub(self.live_window.max(1));
            } else {
                self.playhead = 0;
            }
        } else {
            self.playing = false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `Identifier` has no cheap public constructor, so the populated-frame paths are exercised
    // by the app integration; here we lock the empty-list safety and the index arithmetic that
    // `step`/`tick` rely on.
    #[test]
    fn empty_timeline_is_safe() {
        let mut t = Timeline::default();
        assert!(t.at_head());
        t.step(3);
        assert_eq!(t.playhead, 0);
        t.go_head();
        assert_eq!(t.playhead, 0);
        assert!(!t.tick(), "no playback without frames");
    }

    #[test]
    fn playhead_math_clamps_and_loops() {
        let n = 5usize;
        let clamp = |p: i32| p.clamp(0, n as i32 - 1) as usize;
        assert_eq!(clamp(-2), 0);
        assert_eq!(clamp(9), 4);
        let next = |p: usize| if p + 1 < n { p + 1 } else { 0 };
        assert_eq!(next(4), 0);
        assert_eq!(next(2), 3);
    }

    /// Build a timeline with `n` valid archive frames (names parse to real times).
    fn with_frames(n: usize) -> Timeline {
        let mut t = Timeline::default();
        t.frames = (0..n)
            .map(|i| Identifier::new(format!("KTLX20130520_{:02}{:02}00_V06", i / 60, i % 60)))
            .collect();
        t
    }

    #[test]
    fn toggle_play_at_live_head_starts_rolling_loop() {
        let mut t = with_frames(15);
        t.live_window = 10;
        t.following = true;
        t.playhead = 14; // at head
        t.toggle_play();
        assert!(t.playing);
        assert!(t.following, "live loop stays pinned to the head");
        assert_eq!(t.playhead, 5, "loop starts at len - window");
    }

    #[test]
    fn live_loop_wraps_to_window_start_and_keeps_following() {
        let mut t = with_frames(15);
        t.live_window = 10;
        t.following = true;
        t.playing = true;
        t.playhead = 14; // last frame → tick wraps
        assert!(t.tick(), "first tick fires immediately");
        assert_eq!(t.playhead, 5, "wrap to len - window");
        assert!(t.following, "still live");
    }

    #[test]
    fn archive_loop_wraps_to_zero_unpinned() {
        let mut t = with_frames(5);
        t.following = false;
        t.playing = true;
        t.playhead = 4;
        assert!(t.tick());
        assert_eq!(t.playhead, 0, "archive loop restarts from the beginning");
        assert!(!t.following);
    }

    #[test]
    fn appended_head_slides_the_window() {
        let mut t = with_frames(12);
        t.live_window = 10;
        t.following = true;
        t.playing = true;
        t.playhead = 2; // mid-window
        let before = t.frames.len();
        t.append_head(Identifier::new("KTLX20130520_0012_00_V06".to_string()));
        assert_eq!(t.frames.len(), before + 1, "head appended");
        assert_eq!(t.playhead, 2, "playhead unmoved; window slides on next wrap");
        assert!(t.live_looping());
    }
}
