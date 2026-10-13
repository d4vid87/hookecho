//! Bounded frame observations and reversible quality decisions.
//! Idle repaint intervals are excluded; saved display preferences are never rewritten.
use std::collections::VecDeque;

#[derive(Default)]
pub struct AdaptiveQuality {
    samples: VecDeque<f32>,
    cadence_samples: VecDeque<f32>,
    slow: u16,
    healthy: u16,
    pub reduced: bool,
}

impl AdaptiveQuality {
    /// UI CPU time, measured within the frame rather than from the idle repaint cadence.
    pub fn observe(&mut self, ms: f32, enabled: bool) {
        if !enabled { self.reduced = false; self.slow = 0; self.healthy = 0; }
        if !ms.is_finite() || !(0.0..=500.0).contains(&ms) { return; }
        if self.samples.len() == 120 { self.samples.pop_front(); }
        self.samples.push_back(ms);
        if !enabled { return; }
        if ms > 25.0 {
            self.slow = self.slow.saturating_add(1);
            self.healthy = 0;
            if self.slow >= 30 { self.reduced = true; }
        } else if ms < 12.0 {
            self.healthy = self.healthy.saturating_add(1);
            self.slow = 0;
            if self.healthy >= 240 { self.reduced = false; }
        } else {
            self.slow = 0;
            self.healthy = 0;
        }
    }

    /// Frame cadence is meaningful during continuous gestures, not scheduled idle or playback.
    pub fn observe_cadence(&mut self, ms: f32, continuous: bool) {
        if !continuous || !ms.is_finite() || !(0.1..100.0).contains(&ms) { return; }
        if self.cadence_samples.len() == 120 { self.cadence_samples.pop_front(); }
        self.cadence_samples.push_back(ms);
    }

    pub fn cadence_p95_ms(&self) -> Option<f32> {
        let mut values: Vec<_> = self.cadence_samples.iter().copied().collect();
        if values.is_empty() { return None; }
        values.sort_by(f32::total_cmp);
        Some(values[(values.len() * 95).div_ceil(100).saturating_sub(1)])
    }

    pub fn p95_ms(&self) -> Option<f32> {
        let mut values: Vec<_> = self.samples.iter().copied().collect();
        if values.is_empty() { return None; }
        values.sort_by(f32::total_cmp);
        Some(values[(values.len() * 95).div_ceil(100).saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheduled_idle_intervals_are_excluded_from_gesture_cadence() {
        let mut quality = AdaptiveQuality::default();
        quality.observe_cadence(500.0, true);
        quality.observe_cadence(16.0, false);
        assert_eq!(quality.cadence_p95_ms(), None);
        quality.observe_cadence(16.0, true);
        assert_eq!(quality.cadence_p95_ms(), Some(16.0));
    }

    #[test]
    fn reduction_requires_sustained_work_and_recovers_without_flapping() {
        let mut quality = AdaptiveQuality::default();
        for _ in 0..29 { quality.observe(30.0, true); }
        assert!(!quality.reduced);
        quality.observe(30.0, true);
        assert!(quality.reduced);
        for _ in 0..239 { quality.observe(5.0, true); }
        assert!(quality.reduced);
        quality.observe(5.0, true);
        assert!(!quality.reduced);
        assert_eq!(quality.samples.len(), 120);
        assert_eq!(quality.p95_ms(), Some(5.0));
    }
    #[test]
    fn disabled_and_invalid_observations_do_not_select_reduced_quality() {
        let mut quality = AdaptiveQuality::default();
        for _ in 0..40 { quality.observe(30.0, true); }
        quality.observe(f32::NAN, false);
        assert!(!quality.reduced);
        for _ in 0..40 { quality.observe(1000.0, true); }
        assert!(!quality.reduced);
    }
}
