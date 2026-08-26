//! Driving the chase again, afterwards.
//!
//! The app already records where you went ([`crate::chaselog::Track`]) and can write it as GPX.
//! What it could not do is play it back: put the camera where the car was, with the radar at the
//! time it was there, and step forward. That is how a chase gets reviewed — "we were three
//! minutes from the wrong side of it and did not know" is only visible against the scan that was
//! on screen at the time.
//!
//! The page owns the playhead and hands the app one position per frame; the app owns the camera
//! and the timeline, exactly as the event library does.
//
// ponytail: linear playback at a speed multiplier, one track at a time, no interpolation between
// fixes. The ceiling is "watch the drive back". Smooth camera motion means tweening positions,
// which is the motion system's job, not this page's.

use crate::chaselog::Track;

/// What the page asks the app to do this frame.
pub enum ReplayAction {
    /// Put the camera here, with the radar at this time.
    Seek {
        lon: f64,
        lat: f64,
        time: chrono::DateTime<chrono::Utc>,
    },
    /// Load a GPX file the user picked.
    OpenFile,
}

pub struct ChaseReplay {
    pub open: bool,
    /// The track being replayed. Empty until the user loads one or copies the live session.
    pub track: Track,
    /// Where it came from, for the line under the title.
    pub source: String,
    /// Index into `track.points`.
    pub at: usize,
    pub playing: bool,
    /// Real-time multiplier. A chase is hours long; nobody replays one at 1x.
    pub speed: f64,
    /// App time the playhead last advanced, so speed means something regardless of frame rate.
    last_step: f64,
}

impl Default for ChaseReplay {
    fn default() -> Self {
        Self {
            open: false,
            track: Track::default(),
            source: String::new(),
            at: 0,
            playing: false,
            speed: 60.0,
            last_step: 0.0,
        }
    }
}

impl ChaseReplay {
    /// Take a copy of the live session's track. A copy, not a borrow: replaying yesterday's file
    /// must not disturb the log still being written today.
    pub fn load(&mut self, track: Track, source: impl Into<String>) {
        self.track = track;
        self.source = source.into();
        self.at = 0;
        self.playing = false;
    }

    /// The fix under the playhead.
    fn current(&self) -> Option<crate::chaselog::Fix> {
        self.track.points.get(self.at).copied()
    }

    /// Advance the playhead by wall-clock time times the speed multiplier. Returns true when it
    /// moved, which is what makes the app seek — a paused replay asks for nothing.
    fn advance(&mut self, now: f64) -> bool {
        if !self.playing || self.track.points.len() < 2 {
            return false;
        }
        let dt = now - self.last_step;
        self.last_step = now;
        // A frame that took a second (a decode, a tab coming back) would otherwise jump the
        // playhead a minute of track; clamp it to something a viewer can follow.
        let track_secs = (dt.clamp(0.0, 0.25) * self.speed) as i64;
        let Some(from) = self.current() else {
            return false;
        };
        let target = from.ts + track_secs.max(0);
        let next = self
            .track
            .points
            .iter()
            .position(|p| p.ts >= target)
            .unwrap_or(self.track.points.len() - 1);
        if next == self.at {
            return false;
        }
        self.at = next;
        if self.at + 1 >= self.track.points.len() {
            // Stop at the end rather than loop: a drive has a destination.
            self.playing = false;
        }
        true
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        live: &Track,
        drawer: &mut crate::ui::drawer::Drawer,
    ) -> Option<ReplayAction> {
        let mut open = self.open;
        let mut action = None;
        let Some(window) = drawer.page(
            ctx,
            "Chase Replay",
            &mut open,
            false,
            egui::Window::new("Chase Replay"),
        ) else {
            self.open = open;
            return None;
        };
        let mut load_live = false;
        let mut seek = false;
        window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button("Open GPX…")
                    .on_hover_text("Replay a drive you saved, or one another app recorded")
                    .clicked()
                {
                    action = Some(ReplayAction::OpenFile);
                }
                if ui
                    .add_enabled(
                        live.points.len() > 1,
                        egui::Button::new("This session"),
                    )
                    .on_hover_text("Replay the track being logged right now")
                    .clicked()
                {
                    load_live = true;
                }
            });
            if self.track.points.len() < 2 {
                ui.add_space(6.0);
                ui.weak(
                    "Nothing to replay yet. Turn on \u{201c}Log the chase\u{201d} before you \
                     drive, or open a GPX file.",
                );
                return;
            }
            ui.add_space(6.0);
            ui.weak(format!(
                "{} \u{2014} {} points, {:.0} miles",
                self.source,
                self.track.points.len(),
                self.track.miles()
            ));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let icon = if self.playing {
                    egui_phosphor::regular::PAUSE
                } else {
                    egui_phosphor::regular::PLAY
                };
                if ui.button(icon).clicked() {
                    self.playing = !self.playing;
                    // Playing from the end starts over; the alternative is a play button that
                    // looks broken.
                    if self.playing && self.at + 1 >= self.track.points.len() {
                        self.at = 0;
                    }
                    self.last_step = ui.input(|i| i.time);
                    seek = true;
                }
                let last = self.track.points.len() - 1;
                let mut at = self.at;
                if ui
                    .add(egui::Slider::new(&mut at, 0..=last).show_value(false))
                    .changed()
                {
                    self.at = at;
                    self.playing = false;
                    seek = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Speed");
                for s in [10.0, 60.0, 300.0] {
                    ui.selectable_value(&mut self.speed, s, format!("{s:.0}\u{00d7}"));
                }
            });
            if let Some(p) = self.current() {
                let when = chrono::DateTime::from_timestamp(p.ts, 0);
                ui.add_space(4.0);
                ui.label(match when {
                    Some(t) => t.format("%Y-%m-%d %H:%M:%SZ").to_string(),
                    // A file with no times still replays as a path; it just cannot drive the
                    // radar clock, and says so instead of showing 1970.
                    None => "no time recorded".to_string(),
                });
                let marks: Vec<&str> = self
                    .track
                    .waypoints
                    .iter()
                    .filter(|(i, _)| *i == self.at)
                    .map(|(_, l)| l.as_str())
                    .collect();
                if !marks.is_empty() {
                    ui.strong(marks.join(", "));
                }
            }
        });
        self.open = open;
        if load_live {
            self.load(live.clone(), "This session");
            seek = true;
        }
        if self.advance(ctx.input(|i| i.time)) {
            seek = true;
        }
        if self.playing {
            ctx.request_repaint();
        }
        if action.is_none() && seek {
            if let Some(p) = self.current() {
                // Timestamp 0 is "this file carried no times": drive the camera, leave the radar
                // clock where the user put it.
                if let Some(time) = chrono::DateTime::from_timestamp(p.ts, 0).filter(|_| p.ts > 0) {
                    action = Some(ReplayAction::Seek {
                        lon: p.lon,
                        lat: p.lat,
                        time,
                    });
                }
            }
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chaselog::Fix;

    fn track(n: usize) -> Track {
        let mut t = Track::default();
        for i in 0..n {
            t.points.push(Fix {
                lon: -97.5 + i as f64 * 0.01,
                lat: 35.3,
                ts: 1_700_000_000 + i as i64 * 60,
            });
        }
        t
    }

    #[test]
    fn playback_moves_with_the_clock_and_stops_at_the_end() {
        let mut r = ChaseReplay {
            track: track(5),
            playing: true,
            speed: 60.0,
            ..Default::default()
        };
        // One second of wall clock at 60x is a minute of track: one point.
        r.last_step = 0.0;
        assert!(r.advance(0.2));
        assert_eq!(r.at, 1);
        // Enough steps to run off the end: it stops there rather than wrapping or panicking.
        for i in 2..20 {
            r.advance(i as f64 * 0.2);
        }
        assert_eq!(r.at, 4);
        assert!(!r.playing);
    }

    #[test]
    fn a_paused_replay_asks_for_nothing() {
        let mut r = ChaseReplay {
            track: track(5),
            ..Default::default()
        };
        assert!(!r.advance(1.0));
        // …and neither does a one-point track that was somehow left playing.
        r.playing = true;
        r.track = track(1);
        assert!(!r.advance(2.0));
    }

    #[test]
    fn a_long_frame_does_not_launch_the_playhead() {
        let mut r = ChaseReplay {
            track: track(60),
            playing: true,
            speed: 60.0,
            ..Default::default()
        };
        // Ten seconds of stall at 60x would be ten minutes of track; the clamp keeps it to 15 s.
        assert!(r.advance(10.0));
        assert_eq!(r.at, 1);
    }
}
