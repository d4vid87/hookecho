//! Bounded frame observations and reversible quality decisions.
//! Idle repaint intervals are excluded; saved display preferences are never rewritten.
use std::collections::VecDeque;

#[derive(Default)]
pub struct AdaptiveQuality {
    samples: VecDeque<f32>,
    setup_samples: VecDeque<f32>,
    chrome_samples: VecDeque<f32>,
    sync_samples: VecDeque<f32>,
    map_samples: VecDeque<f32>,
    tail_samples: VecDeque<f32>,
    cadence_samples: VecDeque<f32>,
    previous_motion: bool,
    cadence_pressure: f32,
    slow: u16,
    healthy: u16,
    pub reduced: bool,
}

impl AdaptiveQuality {
    /// Bounded CPU timing windows for each part of the UI frame.
    pub fn observe_phases(&mut self, ms: [f32; 5]) {
        for (samples, ms) in [
            (&mut self.setup_samples, ms[0]),
            (&mut self.chrome_samples, ms[1]),
            (&mut self.sync_samples, ms[2]),
            (&mut self.map_samples, ms[3]),
            (&mut self.tail_samples, ms[4]),
        ] {
            if !ms.is_finite() || ms < 0.0 { continue; }
            if samples.len() == 120 { samples.pop_front(); }
            samples.push_back(ms.min(500.0));
        }
    }

    fn p95(samples: &VecDeque<f32>) -> Option<f32> {
        let mut values: Vec<_> = samples.iter().copied().collect();
        if values.is_empty() { return None; }
        values.sort_by(f32::total_cmp);
        Some(values[(values.len() * 95).div_ceil(100).saturating_sub(1)])
    }

    pub fn setup_p95_ms(&self) -> Option<f32> { Self::p95(&self.setup_samples) }
    pub fn chrome_p95_ms(&self) -> Option<f32> { Self::p95(&self.chrome_samples) }
    pub fn sync_p95_ms(&self) -> Option<f32> { Self::p95(&self.sync_samples) }
    pub fn map_p95_ms(&self) -> Option<f32> { Self::p95(&self.map_samples) }
    pub fn tail_p95_ms(&self) -> Option<f32> { Self::p95(&self.tail_samples) }

    /// UI CPU time, measured within the frame rather than from the idle repaint cadence.
    pub fn observe(&mut self, ms: f32, enabled: bool) {
        if !enabled { self.reduced = false; self.slow = 0; self.healthy = 0; }
        if !ms.is_finite() || ms < 0.0 { return; }
        let ms = ms.min(500.0);
        if self.samples.len() == 120 { self.samples.pop_front(); }
        self.samples.push_back(ms);
        if !enabled { return; }
        let pressure = ms.max(self.cadence_pressure);
        if pressure > 25.0 {
            self.slow = self.slow.saturating_add(1);
            self.healthy = 0;
            if self.slow >= 30 { self.reduced = true; }
        } else if ms < 12.0 && self.cadence_pressure < 20.0 {
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
        let measured = continuous && self.previous_motion && ms.is_finite() && ms >= 0.1;
        self.previous_motion = continuous;
        self.cadence_pressure = if measured { ms.min(500.0) } else { 0.0 };
        if !measured { return; }
        if self.cadence_samples.len() == 120 { self.cadence_samples.pop_front(); }
        self.cadence_samples.push_back(self.cadence_pressure);
    }

    pub fn cadence_p95_ms(&self) -> Option<f32> {
        Self::p95(&self.cadence_samples)
    }

    pub fn p95_ms(&self) -> Option<f32> {
        Self::p95(&self.samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sustained_slow_motion_cadence_reduces_quality_even_with_fast_cpu() {
        let mut quality = AdaptiveQuality::default();
        for _ in 0..31 { quality.observe_cadence(50.0, true); quality.observe(2.0, true); }
        assert!(quality.reduced);
        // Recovery must work during normal 60 Hz motion, not require the user to stop panning.
        for _ in 0..239 { quality.observe_cadence(16.7, true); quality.observe(2.0, true); }
        assert!(quality.reduced);
        quality.observe_cadence(16.7, true);
        quality.observe(2.0, true);
        assert!(!quality.reduced);
    }

    #[test]
    fn very_slow_cpu_frames_still_trigger_reduction() {
        let mut quality = AdaptiveQuality::default();
        for _ in 0..30 { quality.observe(1000.0, true); }
        assert!(quality.reduced);
        assert_eq!(quality.p95_ms(), Some(500.0));
    }

    #[test]
    fn scheduled_idle_intervals_are_excluded_from_gesture_cadence() {
        let mut quality = AdaptiveQuality::default();
        quality.observe_cadence(500.0, false);
        quality.observe_cadence(16.0, false);
        assert_eq!(quality.cadence_p95_ms(), None);
        quality.observe_cadence(16.0, true);
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
        for _ in 0..40 { quality.observe(-1.0, true); }
        assert!(!quality.reduced);
    }
}
