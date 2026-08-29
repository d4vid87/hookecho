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
//! animation at all. [`frame`] watches the frame time and latches the brake when the app is
//! visibly struggling.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set by [`frame`] from the user's setting; read by everything that animates.
static USER_OFF: AtomicBool = AtomicBool::new(false);
/// Latched when frames get slow. Never unlatches: a machine that fell behind once will do it
/// again, and a brake that flickers on and off is worse than either state.
static SLOW: AtomicBool = AtomicBool::new(false);

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
    if SLOW.load(Ordering::Relaxed) {
        return;
    }
    let dt = ctx.input(|i| i.unstable_dt);
    // A frame budget of 33 ms is 30 fps: below that the springs stop reading as springs.
    let run: u32 = ctx.data_mut(|d| {
        let id = egui::Id::new("motion_slow_run");
        let n: u32 = d.get_temp(id).unwrap_or(0);
        let n = if dt > 1.0 / 30.0 { n + 1 } else { 0 };
        d.insert_temp(id, n);
        n
    });
    if run >= 30 {
        SLOW.store(true, Ordering::Relaxed);
        log::info!("motion: frames sustained under 30 fps, reducing animation");
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
}
