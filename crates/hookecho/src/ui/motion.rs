//! Easing, and the one brake that stops all of it.
//!
//! egui's `animate_bool_with_time` is a linear ramp. Linear is what "the panel moved" looks like;
//! a slight overshoot at the end is what "the panel arrived" looks like, and that difference is
//! most of what separates chrome that feels alive from chrome that feels like a slideshow. So the
//! durations stay in [`crate::ui::m3`] as tokens and the shapes live here.
//!
//! Two things turn motion off, and they share one switch. The user's `reduce_motion` setting is
//! the honest one. The other is the machine: a Pi drawing a 4-pane radar loop does not have the
//! frames to spare for a springy drawer, and an animation that stutters reads worse than no
//! animation at all. [`frame`] watches the frame time, engages the brake while the app is visibly
//! struggling, and releases it only after a sustained recovery.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set by [`frame`] from the user's setting; read by everything that animates.
static USER_OFF: AtomicBool = AtomicBool::new(false);
/// Set while sustained slow frames have the automatic visual-quality guard engaged.
static SLOW: AtomicBool = AtomicBool::new(false);

const SLOW_FRAME: f32 = 0.033;
const FAST_FRAME: f32 = 0.028;
const ENGAGE_FRAMES: u16 = 30;
const RECOVER_FRAMES: u16 = 300;

#[derive(Clone, Copy, Default)]
struct Guard {
    degraded: bool,
    run: u16,
}

impl Guard {
    fn observe(&mut self, dt: f32) -> Option<bool> {
        let qualifying = if self.degraded {
            dt < FAST_FRAME
        } else {
            dt > SLOW_FRAME
        };
        self.run = if qualifying {
            self.run.saturating_add(1)
        } else {
            0
        };
        let threshold = if self.degraded {
            RECOVER_FRAMES
        } else {
            ENGAGE_FRAMES
        };
        if self.run < threshold {
            return None;
        }
        self.degraded = !self.degraded;
        self.run = 0;
        Some(self.degraded)
    }
}

/// Whether the automatic visual-quality guard is engaged (independent of the user's setting).
pub fn degraded() -> bool {
    SLOW.load(Ordering::Relaxed)
}

/// Is motion off, for either reason?
pub fn reduced() -> bool {
    USER_OFF.load(Ordering::Relaxed) || SLOW.load(Ordering::Relaxed)
}

/// Call once per frame, before any chrome draws.
///
/// ponytail: the slow-frame test is a run of bad frames, not an average — a single 200 ms hitch is
/// a tile decode, not a slow machine, and averaging in the startup frames would brake every cold
/// start. Sustained badness is the thing worth reacting to.
pub fn frame(ctx: &egui::Context, user_reduce: bool) {
    USER_OFF.store(user_reduce, Ordering::Relaxed);
    // `stable_dt` substitutes the predicted frame interval after reactive-mode sleep. The raw
    // delta counts an idle 100 ms heartbeat as a slow frame and degrades a perfectly idle app.
    let dt = ctx.input(|i| i.stable_dt);
    let (slow, changed) = ctx.data_mut(|d| {
        let id = egui::Id::new("visual_quality_guard");
        let mut guard: Guard = d.get_temp(id).unwrap_or_default();
        let changed = guard.observe(dt);
        d.insert_temp(id, guard);
        (guard.degraded, changed)
    });
    SLOW.store(slow, Ordering::Relaxed);
    match changed {
        Some(true) => log::info!("performance: sustained slow frames, reducing visual quality"),
        Some(false) => log::info!("performance: frame rate recovered, restoring visual quality"),
        None => {}
    }
}

/// Overshoot-and-settle. Standard cubic back-out; `t` outside 0..=1 is clamped.
pub fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    let t = t.clamp(0.0, 1.0) - 1.0;
    1.0 + C3 * t * t * t + C1 * t * t
}

/// Decelerate into place, no overshoot. For anything that would look wrong bouncing — a fade, a
/// scrim, anything already touching an edge.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = 1.0 - t.clamp(0.0, 1.0);
    1.0 - t * t * t
}

/// A 0..1 progress for `on`, eased, or a hard 0/1 when motion is off.
///
/// `ease` is the shape; pass [`ease_out_back`] for arrivals and [`ease_out_cubic`] for the rest.
pub fn rise(ctx: &egui::Context, id: egui::Id, on: bool, secs: f32, ease: fn(f32) -> f32) -> f32 {
    if reduced() {
        return if on { 1.0 } else { 0.0 };
    }
    ease(ctx.animate_bool_with_time(id, on, secs))
}

/// How long the opening sweep lasts, in seconds.
const INTRO_SECS: f64 = 1.4;

/// The radar sweep that greets a cold start.
///
/// One beam around the map, fading as it goes, over the first second and a half. It costs four
/// draw calls and it is the only decoration in the app that has no job — that is the point of it.
/// Skipped entirely under reduced motion, and after the first pass it never draws again.
pub fn intro(ctx: &egui::Context, rect: egui::Rect, accent: egui::Color32) {
    let t = ctx.input(|i| i.time);
    if reduced() || t > INTRO_SECS {
        return;
    }
    ctx.request_repaint();
    let p = (t / INTRO_SECS) as f32;
    // Fades out over the back half rather than the whole sweep, so the beam is actually visible
    // while it crosses the screen.
    let fade = (1.0 - (p - 0.5).max(0.0) * 2.0).clamp(0.0, 1.0);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("intro_sweep"),
    ));
    let c = rect.center();
    let r = rect.size().length() * 0.5;
    let a = std::f32::consts::TAU * ease_out_cubic(p) - std::f32::consts::FRAC_PI_2;
    // The beam, plus a short trail behind it. Two segments read as a sweep; a full cone would
    // need a mesh and would cover the map it is meant to reveal.
    for (i, back) in [0.0_f32, 0.12, 0.24].iter().enumerate() {
        let alpha = (fade * 150.0 / (1.0 + i as f32 * 1.6)) as u8;
        let ang = a - back;
        painter.line_segment(
            [c, c + egui::vec2(ang.cos(), ang.sin()) * r],
            egui::Stroke::new(2.0, accent.gamma_multiply(alpha as f32 / 255.0)),
        );
    }
    // One range ring expanding with the sweep, so the beam has something to sweep over.
    painter.circle_stroke(
        c,
        r * ease_out_cubic(p),
        egui::Stroke::new(1.0, accent.gamma_multiply(fade * 0.25)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_start_at_zero_end_at_one_and_back_overshoots() {
        for f in [ease_out_back as fn(f32) -> f32, ease_out_cubic] {
            assert!(f(0.0).abs() < 1e-4);
            assert!((f(1.0) - 1.0).abs() < 1e-4);
            assert!((f(-3.0)).abs() < 1e-4, "clamped below");
            assert!((f(9.0) - 1.0).abs() < 1e-4, "clamped above");
        }
        // The overshoot is the whole reason ease_out_back exists.
        let peak = (0..100)
            .map(|i| ease_out_back(i as f32 / 100.0))
            .fold(0.0_f32, f32::max);
        assert!(peak > 1.02, "no overshoot: {peak}");
        // Cubic never does.
        let peak = (0..100)
            .map(|i| ease_out_cubic(i as f32 / 100.0))
            .fold(0.0_f32, f32::max);
        assert!(peak <= 1.0, "cubic overshot: {peak}");
    }

    #[test]
    fn guard_engages_and_recovers_with_hysteresis() {
        let mut guard = Guard::default();
        for _ in 0..ENGAGE_FRAMES - 1 {
            assert_eq!(guard.observe(0.040), None);
        }
        assert_eq!(guard.observe(0.040), Some(true));
        assert!(guard.degraded);

        for _ in 0..RECOVER_FRAMES - 1 {
            assert_eq!(guard.observe(0.020), None);
        }
        assert_eq!(guard.observe(0.020), Some(false));
        assert!(!guard.degraded);
    }

    #[test]
    fn one_nonqualifying_frame_resets_each_run() {
        let mut guard = Guard::default();
        for _ in 0..ENGAGE_FRAMES - 1 {
            guard.observe(0.040);
        }
        guard.observe(0.030);
        assert_eq!(guard.observe(0.040), None);

        guard.degraded = true;
        guard.run = 0;
        for _ in 0..RECOVER_FRAMES - 1 {
            guard.observe(0.020);
        }
        guard.observe(0.030);
        assert_eq!(guard.observe(0.020), None);
        assert!(guard.degraded);
    }
}
