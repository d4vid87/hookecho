//! The optional 60-second tour: a spotlight on the app's own chrome, four stops long.
//!
//! It replaces three hardcoded coach-mark cards that pointed at pixel offsets and died on the
//! first stray click. Here the highlighted rects are the real `Response` rects the chrome
//! registered this frame (see [`TourAnchors`]), so the tour cannot drift out of sync with the
//! layout, and the dimming is painted by a `layer_painter` — a painter takes no input, so the
//! control under the spotlight stays clickable. That is what lets two of the four stops be
//! "do the thing" rather than "read the card".
//!
//! Nothing here auto-starts. The first-run card offers it, and it is re-runnable from the command
//! palette, the panel's App section, and Settings → General.

use wxdata::level2::Moment;

use crate::ui::style;

/// Where the chrome actually drew the four things the tour points at, this frame.
///
/// Cleared before chrome runs and re-registered by the draw sites, desktop and mobile alike; a
/// stop whose anchor is `None` (hidden chrome, obs mode, a collapsed sheet) still shows its card,
/// centered and without a hole.
#[derive(Default, Clone, Copy)]
pub struct TourAnchors {
    pub timeline: Option<egui::Rect>,
    pub product: Option<egui::Rect>,
    pub menu: Option<egui::Rect>,
    pub alerts: Option<egui::Rect>,
}

/// The bits of app state the two hands-on stops watch. Compared against a snapshot taken when
/// the step opened: any difference means the user did the thing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Signals {
    pub moment: Moment,
    pub srv: bool,
    pub playhead: usize,
    pub following: bool,
}

/// Stop titles, and the one source of truth for how many stops there are.
const TITLES: [&str; 4] = [
    "Time travel",
    "The products",
    "Everything else",
    "What's out there",
]; // len = the "(n/4)" denominator

#[derive(Default)]
pub struct Tour {
    pub open: bool,
    step: usize,
    /// Signals as they were when the current step opened.
    base: Option<Signals>,
}

impl Tour {
    /// Start (or restart) at the first stop.
    pub fn start(&mut self) {
        self.open = true;
        self.step = 0;
        self.base = None;
    }

    fn next(&mut self) {
        self.step += 1;
        self.base = None;
        if self.step >= TITLES.len() {
            self.open = false;
        }
    }

    /// Advance the two hands-on stops when the user does the thing. Pure — the tests drive it
    /// without an egui context.
    pub fn advance_if_done(&mut self, sig: Signals) {
        if !self.open {
            return;
        }
        let base = *self.base.get_or_insert(sig);
        let done = match self.step {
            // Scrubbed, or jumped back to live.
            0 => sig.playhead != base.playhead || sig.following != base.following,
            // Switched product (storm-relative velocity counts — it's a different picture).
            1 => sig.moment != base.moment || sig.srv != base.srv,
            // The rest are read-and-Next.
            _ => false,
        };
        if done {
            self.next();
        }
    }

    /// The first two stops need the phone's sheet open — that is where the scrubber and the
    /// product chips live.
    pub fn wants_sheet(&self) -> bool {
        self.open && self.step < 2
    }

    /// The desktop equivalent: the product stop points at the site and tilt rows, which live in
    /// the slide-over panel. Spotlighting a control the user has to find first is not a tour.
    pub fn wants_panel(&self) -> bool {
        self.open && self.step == 1
    }

    fn body(&self, sig: Signals) -> String {
        let android = cfg!(target_os = "android");
        match self.step {
            0 => if android {
                "Drag the scrubber to walk back through the storm, or tap Live to snap to the \
                 newest scan. Every overlay — warnings, reports, lightning — follows you back in \
                 time."
            } else {
                "Drag the scrubber along the bottom to walk back through the storm, or click LIVE \
                 to snap to the newest scan. Every overlay — warnings, reports, lightning — \
                 follows you back in time."
            }
            .to_string(),
            1 => {
                let p = crate::products::info(sig.moment);
                let how = if android {
                    "Tap another product chip"
                } else {
                    "Pick another product, or press its number key"
                };
                format!(
                    "You're looking at {}: {}.\n\n{} — the tilt row beside them is how high above \
                     the ground the beam is aimed.",
                    p.name, p.blurb, how
                )
            }
            2 => if android {
                "Layers opens everything else: overlays, tools, windows, the radar site. It has a \
                 search box — type what you want in plain English (\"hail\", \"sounding\", a town \
                 name) and it's one tap away."
            } else {
                "This pill opens the panel, and the panel holds everything else: products, \
                 overlays, tools, settings. Ctrl+K jumps straight to its search — type what you \
                 want in plain English (\"hail\", \"sounding\", a town name) and Enter runs the \
                 top match. Tools you read rather than watch open as pages in a drawer down the \
                 left edge; the buttons down the right edge are the layers, the background map \
                 and the alert bell."
            }
            .to_string(),
            _ => {
                let tap = if android { "Tap" } else { "Click" };
                format!(
                    "{tap} a storm on the map to interrogate it — what the beam sees there, which \
                     warnings cover it, how far away it is.\n\nThe bell counts warnings in view; \
                     open it for the list, worst first. Alerts on your saved places work with the \
                     app closed.\n\nPress ? any time for the keyboard map."
                )
            }
        }
    }

    fn anchor(&self, a: &TourAnchors) -> Option<egui::Rect> {
        match self.step {
            0 => a.timeline,
            1 => a.product,
            2 => a.menu,
            _ => a.alerts,
        }
    }

    /// Draw the current stop. Call after the chrome has registered its anchors.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        anchors: &TourAnchors,
        sig: Signals,
        accent: egui::Color32,
    ) {
        if !self.open {
            return;
        }
        let step = self.step.min(TITLES.len() - 1);
        // Dim the whole window: `content_rect` stops at the docked panels, which left the sidebar
        // and the legend at full brightness and the spotlight looking like a stray shadow.
        let screen = ctx.viewport_rect();
        let hole = self
            .anchor(anchors)
            .map(|r| r.expand(6.0).intersect(screen));
        // Dim everything but the hole. Four rects rather than a mask because a painter has no
        // clip stack worth fighting, and four rects is four lines.
        let mut p = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("tour_dim"),
        ));
        // A layer painter is clipped to `available_rect` — what's left after the docked panels —
        // so without this the sidebar and the legend never dim.
        p.set_clip_rect(screen);
        let dim = egui::Color32::from_black_alpha(160);
        match hole {
            Some(h) => {
                let (l, r, t, b) = (h.left(), h.right(), h.top(), h.bottom());
                for rect in [
                    egui::Rect::from_min_max(screen.min, egui::pos2(screen.right(), t)),
                    egui::Rect::from_min_max(
                        egui::pos2(screen.left(), b),
                        egui::pos2(screen.right(), screen.bottom()),
                    ),
                    egui::Rect::from_min_max(egui::pos2(screen.left(), t), egui::pos2(l, b)),
                    egui::Rect::from_min_max(egui::pos2(r, t), egui::pos2(screen.right(), b)),
                ] {
                    p.rect_filled(rect, 0.0, dim);
                }
                p.rect_stroke(
                    h,
                    6.0,
                    egui::Stroke::new(2.0, accent),
                    egui::StrokeKind::Middle,
                );
            }
            None => {
                p.rect_filled(screen, 0.0, dim);
            }
        }

        let card_w = 320.0_f32.min(screen.width() - 32.0);
        // Below the spotlight if it fits, above otherwise; centered when there is no spotlight.
        let (pivot, pos) = match hole {
            Some(h) if screen.bottom() - h.bottom() > 220.0 => (
                egui::Align2::CENTER_TOP,
                egui::pos2(h.center().x, h.bottom() + 12.0),
            ),
            Some(h) => (
                egui::Align2::CENTER_BOTTOM,
                egui::pos2(h.center().x, h.top() - 12.0),
            ),
            None => (egui::Align2::CENTER_CENTER, ctx.content_rect().center()),
        };
        let body = self.body(sig);
        let mut act = None;
        egui::Area::new(egui::Id::new("tour_card"))
            .order(egui::Order::Foreground)
            .constrain_to(ctx.content_rect())
            .pivot(pivot)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                style::glass(ui, 245)
                    .stroke(egui::Stroke::new(1.5, accent))
                    .show(ui, |ui| {
                        ui.set_width(card_w);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(TITLES[step])
                                    .size(style::FONT_BASE)
                                    .strong()
                                    .color(accent),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{}/{}",
                                            step + 1,
                                            TITLES.len()
                                        ))
                                        .size(style::FONT_SM)
                                        .color(egui::Color32::from_gray(150)),
                                    );
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(body)
                                .size(style::FONT_BASE)
                                .color(egui::Color32::from_gray(238)),
                        );
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.small_button("Skip tour").clicked() {
                                act = Some(false);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let last = step + 1 == TITLES.len();
                                    if ui.button(if last { "Done" } else { "Next" }).clicked() {
                                        act = Some(true);
                                    }
                                },
                            );
                        });
                    });
            });
        match act {
            Some(true) => self.next(),
            Some(false) => self.open = false,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig() -> Signals {
        Signals {
            moment: Moment::Reflectivity,
            srv: false,
            playhead: 3,
            following: true,
        }
    }

    #[test]
    fn scrubbing_finishes_the_timeline_stop() {
        let mut t = Tour::default();
        t.start();
        t.advance_if_done(sig()); // snapshots
        t.advance_if_done(sig());
        assert_eq!(t.step, 0, "nothing happened, so nothing advances");
        t.advance_if_done(Signals {
            playhead: 4,
            following: false,
            ..sig()
        });
        assert_eq!(t.step, 1);
    }

    #[test]
    fn switching_product_finishes_the_product_stop() {
        let mut t = Tour::default();
        t.start();
        t.step = 1;
        t.advance_if_done(sig());
        t.advance_if_done(Signals {
            moment: Moment::Velocity,
            ..sig()
        });
        assert_eq!(t.step, 2);
        // Storm-relative counts as a switch too.
        let mut t = Tour::default();
        t.start();
        t.step = 1;
        t.advance_if_done(sig());
        t.advance_if_done(Signals { srv: true, ..sig() });
        assert_eq!(t.step, 2);
    }

    #[test]
    fn the_passive_stops_only_move_on_next() {
        let mut t = Tour::default();
        t.start();
        t.step = 2;
        t.advance_if_done(sig());
        t.advance_if_done(Signals {
            moment: Moment::Velocity,
            playhead: 99,
            ..sig()
        });
        assert_eq!(t.step, 2);
        t.next();
        assert_eq!(t.step, 3);
    }

    #[test]
    fn the_last_next_closes_the_tour() {
        let mut t = Tour::default();
        t.start();
        for _ in 0..TITLES.len() {
            assert!(t.open);
            t.next();
        }
        assert!(!t.open);
    }

    #[test]
    fn start_resets_a_finished_tour() {
        let mut t = Tour::default();
        t.start();
        t.step = 3;
        t.advance_if_done(sig());
        t.start();
        assert!(t.open && t.step == 0 && t.base.is_none());
    }

    #[test]
    fn a_closed_tour_ignores_signals() {
        let mut t = Tour::default();
        t.advance_if_done(sig());
        t.advance_if_done(Signals {
            playhead: 9,
            ..sig()
        });
        assert_eq!(t.step, 0);
        assert!(!t.open);
    }
}
