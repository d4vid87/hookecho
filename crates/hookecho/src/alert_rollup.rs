//! Outbreak mode: when alerts arrive faster than anyone can read them, stop delivering one push
//! per warning and deliver one rolling summary instead.
//!
//! On a big day a single office can issue a dozen warnings in ten minutes, and every one of them
//! is a phone buzz. Past a threshold the buzzes stop carrying information — you already know it
//! is a bad day, and the individual pushes are what makes people mute the app during exactly the
//! event they installed it for.
//!
//! This is only about *delivery*. Banners, the alert list, sounds and the map are untouched; so
//! is the escalated tier, which always pushes on its own.

use std::collections::VecDeque;
use std::time::Duration;
use wxdata::clock::Instant;

/// What the caller should do with an alert it was about to push.
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Push it as itself.
    Send,
    /// Hold it and push this summary instead (already includes everything rolled up so far).
    Rollup(String),
    /// Hold it and push nothing — the summary is already current for this minute.
    Hold,
}

/// Rolling window of recently pushed alert titles.
#[derive(Default)]
pub struct Rollup {
    /// (arrival, short title) within the window, oldest first.
    recent: VecDeque<(Instant, String)>,
    /// Everything folded into the current summary, in arrival order.
    folded: Vec<String>,
    /// When the summary last went out, so it updates at most once a minute.
    last_summary: Option<Instant>,
}

/// Don't re-push the summary more often than this, however fast the alerts land.
const SUMMARY_EVERY: Duration = Duration::from_secs(60);

impl Rollup {
    /// Offer an alert. `threshold` alerts inside `window` turns rollup on; it turns off by itself
    /// when the window drains back below the threshold.
    pub fn offer(
        &mut self,
        now: Instant,
        title: &str,
        threshold: usize,
        window: Duration,
    ) -> Decision {
        while self
            .recent
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > window)
        {
            self.recent.pop_front();
        }
        self.recent.push_back((now, title.to_string()));
        if self.recent.len() < threshold.max(2) {
            // Out of outbreak mode: the next one starts a fresh summary.
            self.folded.clear();
            self.last_summary = None;
            return Decision::Send;
        }
        self.folded.push(title.to_string());
        let due = self
            .last_summary
            .is_none_or(|t| now.duration_since(t) >= SUMMARY_EVERY);
        if !due {
            return Decision::Hold;
        }
        self.last_summary = Some(now);
        Decision::Rollup(self.summary(window))
    }

    /// One line per alert, newest first, capped — a notification nobody scrolls.
    fn summary(&self, window: Duration) -> String {
        const MAX_LINES: usize = 8;
        let mins = window.as_secs().div_ceil(60);
        let mut s = format!(
            "{} alerts in the last {mins} min:\n",
            self.recent.len().max(self.folded.len())
        );
        for t in self.folded.iter().rev().take(MAX_LINES) {
            s.push_str("• ");
            s.push_str(t);
            s.push('\n');
        }
        if self.folded.len() > MAX_LINES {
            s.push_str(&format!("…and {} more", self.folded.len() - MAX_LINES));
        }
        s.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> Duration {
        Duration::from_secs(600)
    }

    #[test]
    fn under_the_threshold_everything_sends_itself() {
        let mut r = Rollup::default();
        let t0 = Instant::now();
        for i in 0..4 {
            let d = r.offer(
                t0 + Duration::from_secs(i * 10),
                "Severe Thunderstorm",
                5,
                win(),
            );
            assert_eq!(d, Decision::Send, "alert {i}");
        }
    }

    #[test]
    fn the_fifth_alert_rolls_up_and_the_sixth_is_held() {
        let mut r = Rollup::default();
        let t0 = Instant::now();
        for i in 0..4 {
            r.offer(
                t0 + Duration::from_secs(i * 10),
                "Severe Thunderstorm",
                5,
                win(),
            );
        }
        let Decision::Rollup(text) =
            r.offer(t0 + Duration::from_secs(50), "Tornado Warning", 5, win())
        else {
            panic!("fifth alert inside the window should roll up");
        };
        assert!(text.starts_with("5 alerts in the last 10 min:"), "{text}");
        assert!(text.contains("Tornado Warning"));
        // Same minute: the summary is current, so nothing goes out.
        assert_eq!(
            r.offer(
                t0 + Duration::from_secs(60),
                "Flash Flood Warning",
                5,
                win()
            ),
            Decision::Hold
        );
        // A minute later it refreshes, now carrying the held one too.
        let Decision::Rollup(text) =
            r.offer(t0 + Duration::from_secs(120), "Special Marine", 5, win())
        else {
            panic!("summary should refresh after a minute");
        };
        assert!(text.contains("Flash Flood Warning"), "{text}");
    }

    #[test]
    fn the_window_drains_and_normal_pushes_resume() {
        let mut r = Rollup::default();
        let t0 = Instant::now();
        for i in 0..5 {
            r.offer(
                t0 + Duration::from_secs(i * 10),
                "Severe Thunderstorm",
                5,
                win(),
            );
        }
        // Eleven minutes later the window holds only this one.
        let d = r.offer(
            t0 + Duration::from_secs(11 * 60),
            "Tornado Warning",
            5,
            win(),
        );
        assert_eq!(d, Decision::Send);
    }
}
