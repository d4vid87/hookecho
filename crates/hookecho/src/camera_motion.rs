//! Time-based wheel zoom and drag inertia. Touch pinch remains a direct camera operation.
use crate::render::mercator::Camera;

#[derive(Debug, Default)]
pub struct Motion {
    zoom_impulses: Vec<ZoomImpulse>,
    velocity: (f32, f32),
    dragging: bool,
    fresh_input: bool,
    last_scroll: Option<(f64, bool)>,
    last_drag_move: Option<f64>,
    expected: Option<Camera>,
    viewport: Option<(f32, f32)>,
}

#[derive(Debug)]
struct ZoomImpulse {
    remaining: f64,
    velocity: f64,
    anchor: (f32, f32),
}

impl Motion {
    pub fn active(&self) -> bool {
        self.zoom_impulses.iter().any(|impulse| impulse.remaining.abs() > 1e-5)
            || (!self.dragging && self.velocity.0.hypot(self.velocity.1) >= 8.0)
    }

    pub fn cancel(&mut self) {
        self.zoom_impulses.clear();
        self.velocity = (0.0, 0.0);
        self.dragging = false;
        self.fresh_input = false;
        self.last_drag_move = None;
        self.expected = None;
    }

    /// An externally placed camera, projection change, or resized pane takes precedence.
    pub fn sync(&mut self, cam: &Camera, vp: (f32, f32)) {
        if self
            .expected
            .is_some_and(|e| e.center != cam.center || e.zoom != cam.zoom || e.globe != cam.globe || e.pitch_deg != cam.pitch_deg || e.bearing_deg != cam.bearing_deg)
            || self.viewport.is_some_and(|old| old != vp)
        {
            self.cancel();
        }
        self.viewport = Some(vp);
        self.expected = Some(*cam);
    }

    /// Wheel units identify discrete input; pixel input uses size and cadence as a fallback.
    pub fn scroll_delta(&mut self, unit: egui::MouseWheelUnit, delta: f64, now: f64) -> f64 {
        if !delta.is_finite() || !now.is_finite() || delta == 0.0 {
            return 0.0;
        }
        let wheel = match unit {
            egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => true,
            egui::MouseWheelUnit::Point if delta.abs() < 4.0 => false,
            egui::MouseWheelUnit::Point => match self.last_scroll {
                Some((last, previous)) if now >= last && now - last < 0.4 => {
                    if now == last { previous } else { (now - last) * 1000.0 * delta.abs() >= 200.0 }
                }
                _ => true,
            },
        };
        self.last_scroll = Some((now, wheel));
        let points = delta * match unit {
            egui::MouseWheelUnit::Point => 1.0,
            egui::MouseWheelUnit::Line => 40.0,
            egui::MouseWheelUnit::Page => 120.0,
        };
        let rate = if wheel { 0.004444 } else { 0.02 };
        (2.0 / (1.0 + (-points.abs() * rate).exp())).log2() * points.signum()
    }

    pub fn wheel(
        &mut self,
        cam: &mut Camera,
        delta: f64,
        anchor: (f32, f32),
        vp: (f32, f32),
        reduced: bool,
    ) {
        if !delta.is_finite() {
            return;
        }
        // Some browsers report a Page wheel unit as a whole viewport per event. A multi-page
        // burst can otherwise jump from a regional map to world view in a single frame.
        let delta = delta.clamp(-2.0, 2.0);
        self.sync(cam, vp);
        self.velocity = (0.0, 0.0);
        self.dragging = false;
        if reduced {
            self.zoom_impulses.clear();
            cam.zoom_at(delta, anchor, vp);
            self.expected = Some(*cam);
        } else {
            self.fresh_input |= self.zoom_impulses.is_empty();
            // A new wheel event can arrive before the previous ease has settled. Keep each
            // unfinished part at its own cursor anchor; reanchoring the old part to the latest
            // cursor makes the geography slide sideways during an otherwise smooth zoom.
            if self.zoom_impulses.len() >= 32 {
                let oldest = self.zoom_impulses.remove(0);
                cam.zoom_at(oldest.remaining, oldest.anchor, vp);
                self.expected = Some(*cam);
            }
            let queued: f64 = self.zoom_impulses.iter().map(|impulse| impulse.remaining).sum();
            let accepted = (cam.zoom + queued + delta).clamp(-2.0, 20.0) - cam.zoom - queued;
            if let Some(last) = self.zoom_impulses.last_mut().filter(|last| last.anchor == anchor) {
                last.remaining += accepted;
                // Reversals must not carry momentum past the new target.
                let limit = 30.0 * last.remaining.abs();
                last.velocity = if last.velocity * last.remaining < 0.0 { 0.0 } else { last.velocity.clamp(-limit, limit) };
            } else if accepted.abs() > 1e-5 {
                self.zoom_impulses.push(ZoomImpulse { remaining: accepted, velocity: 0.0, anchor });
            }
        }
    }

    /// A diagonal trackpad gesture can contain pan and zoom in the same frame. Keep its queued
    /// zoom when applying horizontal movement; treating the pan as an external jump loses it.
    pub fn pan_input(&mut self, cam: &mut Camera, delta: (f32, f32), vp: (f32, f32)) {
        self.sync(cam, vp);
        self.velocity = (0.0, 0.0);
        cam.pan_pixels(delta.0, delta.1, vp);
        self.expected = Some(*cam);
    }

    pub fn begin_drag(&mut self) {
        self.cancel();
        self.dragging = true;
    }

    pub fn drag(&mut self, cam: &mut Camera, delta: (f32, f32), dt: f32, vp: (f32, f32), now: f64) {
        self.dragging = true;
        cam.pan_pixels(delta.0, delta.1, vp);
        if delta.0 != 0.0 || delta.1 != 0.0 {
            self.last_drag_move = Some(now);
        }
        let dt = dt.clamp(0.001, 0.05);
        let weight = 1.0 - (-dt / 0.035).exp();
        let x = (delta.0 / dt).clamp(-2000.0, 2000.0);
        let y = (delta.1 / dt).clamp(-2000.0, 2000.0);
        self.velocity.0 += (x - self.velocity.0) * weight;
        self.velocity.1 += (y - self.velocity.1) * weight;
        self.expected = Some(*cam);
    }

    pub fn drag_at(&mut self, cam: &mut Camera, delta: (f32, f32), to: (f32, f32), dt: f32, vp: (f32, f32), now: f64) {
        let mut anchored = *cam;
        anchored.pan_at(delta, to, vp);
        self.drag(cam, delta, dt, vp, now);
        *cam = anchored;
        self.expected = Some(*cam);
    }

    pub fn release(&mut self, reduced: bool, now: f64) {
        self.dragging = false;
        if reduced || !now.is_finite() || self.last_drag_move.is_none_or(|last| now - last > 0.08) {
            self.velocity = (0.0, 0.0);
        }
    }

    /// Returns true while more camera frames are required. A stall finishes requested zoom,
    /// but cancels pan inertia instead of applying a large catch-up jump on resume.
    pub fn advance(&mut self, cam: &mut Camera, dt: f32, vp: (f32, f32), reduced: bool) -> bool {
        self.sync(cam, vp);
        if !dt.is_finite() || (dt > 0.25 && !self.fresh_input) || reduced {
            if reduced || dt.is_finite() {
                for impulse in &self.zoom_impulses {
                    cam.zoom_at(impulse.remaining, impulse.anchor, vp);
                }
            }
            self.cancel();
            self.expected = Some(*cam);
            return false;
        }
        // A new ease must not consume the reactive idle interval preceding its input.
        let dt = if self.fresh_input {
            dt.clamp(0.0, 1.0 / 60.0)
        } else {
            dt.max(0.0)
        } as f64;
        self.fresh_input = false;
        for impulse in &mut self.zoom_impulses {
            let delta = if impulse.remaining.abs() < 1e-4 {
                impulse.remaining
            } else {
                // Analytic critical damping keeps velocity continuous across wheel events.
                let c = 30.0 * impulse.remaining - impulse.velocity;
                let decay = (-30.0 * dt).exp();
                let remaining = (impulse.remaining + c * dt) * decay;
                impulse.velocity = (impulse.velocity + 30.0 * c * dt) * decay;
                impulse.remaining - remaining
            };
            cam.zoom_at(delta, impulse.anchor, vp);
            impulse.remaining -= delta;
        }
        self.zoom_impulses.retain(|impulse| impulse.remaining.abs() > 1e-5);
        if !self.dragging && self.velocity.0.hypot(self.velocity.1) >= 8.0 {
            let decay = (-dt.min(0.05) * 7.5).exp() as f32;
            let distance = (1.0 - decay) / 7.5;
            cam.pan_pixels(self.velocity.0 * distance, self.velocity.1 * distance, vp);
            self.velocity.0 *= decay;
            self.velocity.1 *= decay;
        } else if !self.dragging {
            self.velocity = (0.0, 0.0);
        }
        self.expected = Some(*cam);
        self.active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VP: (f32, f32) = (1000.0, 800.0);
    #[test]
    fn wheel_preserves_anchor_and_settles_at_requested_zoom() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut m = Motion::default();
        let anchor = (700.0, 300.0);
        let world = cam.screen_to_world(anchor, VP);
        m.wheel(&mut cam, 1.25, anchor, VP, false);
        assert_eq!(cam.zoom, 7.0);
        let mut frames = 0;
        while m.advance(&mut cam, 1.0 / 60.0, VP, false) {
            frames += 1;
            assert!(frames < 120);
        }
        assert!(frames > 5);
        assert!((cam.zoom - 8.25).abs() < 1e-4);
        let at = cam.world_to_screen(world, VP);
        assert!((at.0 - anchor.0).abs() < 0.01 && (at.1 - anchor.1).abs() < 0.01);
    }
    #[test]
    fn overlapping_wheel_events_keep_their_original_anchors() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut motion = Motion::default();
        let first = (250.0, 400.0);
        let second = (750.0, 400.0);
        let mut left = cam;
        left.zoom_at(1.0, first, VP);
        left.zoom_at(1.0, second, VP);
        let mut right = cam;
        right.zoom_at(1.0, second, VP);
        right.zoom_at(1.0, first, VP);
        motion.wheel(&mut cam, 1.0, first, VP, false);
        motion.advance(&mut cam, 1.0 / 60.0, VP, false);
        motion.wheel(&mut cam, 1.0, second, VP, false);
        assert_eq!(motion.zoom_impulses.len(), 2);
        assert_eq!(motion.zoom_impulses[0].anchor, first);
        assert_eq!(motion.zoom_impulses[1].anchor, second);
        for _ in 0..60 {
            motion.advance(&mut cam, 1.0 / 60.0, VP, false);
        }
        assert!(!motion.active());
        assert!((cam.zoom - 9.0).abs() < 1e-4);
        assert!(cam.center.0 >= left.center.0.min(right.center.0) - 1e-9);
        assert!(cam.center.0 <= left.center.0.max(right.center.0) + 1e-9);
        assert!((cam.center.1 - left.center.1).abs() < 1e-9);
    }
    #[test]
    fn oversized_page_wheel_stays_regional() {
        let mut cam = Camera::at_lonlat(-120.4, 34.8, 7.0);
        let mut motion = Motion::default();
        motion.wheel(&mut cam, -13.0, (570.0, 400.0), VP, false);
        for _ in 0..90 { motion.advance(&mut cam, 1.0 / 60.0, VP, false); }
        assert!((cam.zoom - 5.0).abs() < 1e-4);
        let (_, lat) = crate::render::mercator::world_to_lonlat(cam.center.0, cam.center.1);
        assert!((lat - 34.8).abs() < 5.0);
    }
    #[test]
    fn external_camera_and_resize_cancel_old_motion() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut m = Motion::default();
        m.wheel(&mut cam, 1.0, (500.0, 400.0), VP, false);
        cam = Camera::at_lonlat(-90.0, 40.0, 8.0);
        assert!(!m.advance(&mut cam, 0.016, VP, false));
        assert_eq!(cam.zoom, 8.0);
        m.wheel(&mut cam, 1.0, (500.0, 400.0), VP, false);
        assert!(!m.advance(&mut cam, 0.016, (800.0, 600.0), false));
        assert_eq!(cam.zoom, 8.0);
    }
    #[test]
    fn reduced_motion_is_immediate_and_has_no_coast() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut m = Motion::default();
        m.wheel(&mut cam, 1.0, (500.0, 400.0), VP, true);
        assert_eq!(cam.zoom, 8.0);
        assert!(!m.active());
        m.begin_drag();
        m.drag(&mut cam, (20.0, 0.0), 0.016, VP, 1.0);
        m.release(true, 1.016);
        let center = cam.center;
        m.advance(&mut cam, 0.016, VP, true);
        assert_eq!(cam.center, center);
    }
    #[test]
    fn drag_coasts_only_after_release_and_stalls_cancel() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut m = Motion::default();
        m.begin_drag();
        m.drag(&mut cam, (20.0, 0.0), 0.016, VP, 1.0);
        let center = cam.center;
        assert!(!m.advance(&mut cam, 0.016, VP, false));
        assert_eq!(cam.center, center);
        m.release(false, 1.016);
        assert!(m.advance(&mut cam, 0.016, VP, false));
        assert_ne!(cam.center, center);
        let center = cam.center;
        assert!(!m.advance(&mut cam, 1.0, VP, false));
        assert_eq!(cam.center, center);
    }
    #[test]
    fn zoom_curve_is_independent_of_refresh_rate_and_reverses_without_overshoot() {
        let sample = |dt: f32, frames: usize| {
            let mut c = Camera::at_lonlat(-97.0, 35.0, 7.0);
            let mut m = Motion::default();
            m.wheel(&mut c, 1.0, (600.0, 300.0), VP, false);
            for _ in 0..frames {
                m.advance(&mut c, dt, VP, false);
            }
            c.zoom
        };
        assert!((sample(1.0 / 60.0, 12) - sample(1.0 / 120.0, 24)).abs() < 1e-7);
        let mut c = Camera::at_lonlat(-97.0, 35.0, 19.8);
        let mut m = Motion::default();
        m.wheel(&mut c, 100.0, (500.0, 400.0), VP, false);
        for _ in 0..60 {
            m.advance(&mut c, 1.0 / 60.0, VP, false);
            assert!(c.zoom <= 20.0);
        }
        m.wheel(&mut c, -1.0, (500.0, 400.0), VP, false);
        for _ in 0..60 {
            m.advance(&mut c, 1.0 / 60.0, VP, false);
            assert!(c.zoom >= 19.0 - 1e-9);
        }
        assert!((c.zoom - 19.0).abs() < 1e-4);
    }

    #[test]
    fn slow_frame_finishes_requested_zoom_at_its_anchor() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 8.0);
        let anchor = (810.0, 350.0);
        let mut expected = cam;
        expected.zoom_at(1.0, anchor, VP);
        let mut motion = Motion::default();
        motion.wheel(&mut cam, 1.0, anchor, VP, false);
        assert!(motion.advance(&mut cam, 0.016, VP, false));
        assert!(!motion.advance(&mut cam, 0.284, VP, false));
        assert!((cam.zoom - expected.zoom).abs() < 1e-9);
        assert!((cam.center.0 - expected.center.0).abs() < 1e-9);
        assert!((cam.center.1 - expected.center.1).abs() < 1e-9);
    }

    #[test]
    fn a_new_wheel_event_after_reactive_sleep_is_not_discarded() {
        for idle in [0.1, 0.5] {
            let mut c = Camera::at_lonlat(-97.0, 35.0, 7.0);
            let mut m = Motion::default();
            m.wheel(&mut c, 0.5, (500.0, 400.0), VP, false);
            assert!(m.advance(&mut c, idle, VP, false));
            assert!(c.zoom > 7.0 && c.zoom < 7.1, "idle time must not jump through the ease");
            assert!(!m.advance(&mut c, 0.5, VP, false));
            assert!((c.zoom - 7.5).abs() < 1e-9);
        }
    }

    #[test]
    fn holding_a_drag_still_before_release_does_not_throw_the_map() {
        let mut c = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut m = Motion::default();
        m.begin_drag();
        m.drag(&mut c, (20.0, 0.0), 0.016, VP, 1.0);
        m.release(false, 1.2);
        let center = c.center;
        assert!(!m.advance(&mut c, 0.016, VP, false));
        assert_eq!(c.center, center);
    }

    #[test]
    fn diagonal_scroll_does_not_discard_its_zoom_component() {
        let mut c = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut m = Motion::default();
        let center = c.center;
        m.wheel(&mut c, 0.5, (500.0, 400.0), VP, false);
        m.pan_input(&mut c, (20.0, 0.0), VP);
        assert_ne!(c.center, center);
        for _ in 0..60 {
            m.advance(&mut c, 1.0 / 60.0, VP, false);
        }
        assert!((c.zoom - 7.5).abs() < 1e-4);
    }
    #[test]
    fn changing_orientation_cancels_old_inertia() {
        let mut c=Camera::at_lonlat(-97.0,35.0,8.0);
        let mut m=Motion::default();m.wheel(&mut c,1.0,(500.0,400.0),VP,false);
        c.set_orientation(45.0,90.0);
        assert!(!m.advance(&mut c,0.016,VP,false));
        assert_eq!(c.zoom,8.0);
    }

    #[test]
    fn scroll_curve_distinguishes_wheel_and_precision_input_and_bounds_large_deltas() {
        use egui::MouseWheelUnit::{Line, Page, Point};
        let mut motion = Motion::default();
        let wheel = motion.scroll_delta(Point, 120.0, 1.0);
        assert!((wheel - 0.333991).abs() < 1e-6);
        assert!((motion.scroll_delta(Point, -120.0, 2.0) + wheel).abs() < 1e-9);
        assert!((motion.scroll_delta(Line, 3.0, 3.0) - wheel).abs() < 1e-9);
        assert!((motion.scroll_delta(Page, 1.0, 4.0) - wheel).abs() < 1e-9);
        let precision = motion.scroll_delta(Point, 2.0, 5.0);
        assert!(precision > 0.02 && precision < 0.04);
        let rapid = motion.scroll_delta(Point, 12.0, 5.008);
        let discrete = motion.scroll_delta(Point, 12.0, 6.0);
        assert!(rapid > discrete * 3.0, "precision scroll must not feel like tiny wheel ticks");
        assert!(motion.scroll_delta(Page, f64::MAX, 7.0) <= 1.0);
        assert!(motion.scroll_delta(Page, -f64::MAX, 8.0) >= -1.0);
        assert_eq!(motion.scroll_delta(Point, f64::NAN, 9.0), 0.0);
        assert_eq!(motion.scroll_delta(Point, 120.0, f64::INFINITY), 0.0);
    }

    #[test]
    fn repeated_input_keeps_velocity_and_reversal_does_not_overshoot() {
        let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut motion = Motion::default();
        let anchor = (650.0, 300.0);
        motion.wheel(&mut cam, 0.334, anchor, VP, false);
        motion.advance(&mut cam, 0.05, VP, false);
        let velocity = motion.zoom_impulses[0].velocity;
        assert!(velocity > 0.0);
        motion.wheel(&mut cam, 0.334, anchor, VP, false);
        assert_eq!(motion.zoom_impulses[0].velocity, velocity);
        motion.advance(&mut cam, 0.00001, VP, false);
        let before = cam.zoom;
        motion.wheel(&mut cam, -1.0, anchor, VP, false);
        let target = cam.zoom + motion.zoom_impulses[0].remaining;
        let mut previous = before;
        for _ in 0..60 {
            motion.advance(&mut cam, 1.0 / 60.0, VP, false);
            assert!(cam.zoom <= previous && cam.zoom >= target - 1e-9);
            previous = cam.zoom;
        }
        assert!((cam.zoom - target).abs() < 1e-4);
    }

    #[test]
    fn slow_zoom_frames_use_elapsed_time_instead_of_stretching_the_animation() {
        let sample = |dt, frames| {
            let mut cam = Camera::at_lonlat(-97.0, 35.0, 7.0);
            let mut motion = Motion::default();
            motion.wheel(&mut cam, 1.0, (500.0, 400.0), VP, false);
            motion.advance(&mut cam, 1.0 / 60.0, VP, false);
            for _ in 0..frames {
                motion.advance(&mut cam, dt, VP, false);
            }
            cam.zoom
        };
        let fast = sample(1.0 / 120.0, 24);
        assert!((fast - sample(0.1, 2)).abs() < 1e-7);
        assert!(fast > 7.98 && fast < 8.0);
    }

    #[test]
    fn reduced_motion_changes_animation_without_changing_precision_scroll_gain() {
        let mut normal = Motion::default();
        let mut reduced = Motion::default();
        let mut a = Camera::at_lonlat(-97.0, 35.0, 7.0);
        let mut b = a;
        for (now, delta) in [(1.0, 2.0), (1.008, 12.0)] {
            let eased = normal.scroll_delta(egui::MouseWheelUnit::Point, delta, now);
            let immediate = reduced.scroll_delta(egui::MouseWheelUnit::Point, delta, now);
            assert_eq!(eased, immediate);
            normal.wheel(&mut a, eased, (500.0, 400.0), VP, false);
            reduced.wheel(&mut b, immediate, (500.0, 400.0), VP, true);
            normal.advance(&mut a, 0.008, VP, false);
            reduced.advance(&mut b, 0.008, VP, true);
        }
    }

}
