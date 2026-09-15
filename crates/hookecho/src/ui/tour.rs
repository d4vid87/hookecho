//! The optional 60-second guide: a stable glass walkthrough, four stops long.
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

/// The live product state used to make the guide copy specific to the current view.
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
}

impl Tour {
    /// Start (or restart) at the first stop.
    pub fn start(&mut self) {
        self.open = true;
        self.step = 0;
    }

    fn next(&mut self) {
        self.step += 1;
        if self.step >= TITLES.len() {
            self.open = false;
        }
    }

    fn back(&mut self) {
        self.step = self.step.saturating_sub(1);
    }

    /// The guide no longer moves app chrome underneath itself.
    pub fn wants_sheet(&self) -> bool {
        false
    }

    /// Keep the desktop panel stable while the centered guide is open.
    pub fn wants_panel(&self) -> bool {
        false
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

    /// Draw the current stop as one stable, theme-aware glass card.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        _anchors: &TourAnchors,
        sig: Signals,
        accent: egui::Color32,
    ) {
        if !self.open {
            return;
        }
        let step = self.step.min(TITLES.len() - 1);
        let screen = ctx.viewport_rect();
        let mut p = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("tour_dim"),
        ));
        p.set_clip_rect(screen);
        p.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(176));

        let card_w = 420.0_f32.min(screen.width() - 32.0);
        let body = self.body(sig);
        let mut act = 0_i8;
        let window = egui::Window::new("60-second guide")
            .id(egui::Id::new("tour_card"))
            .frame(style::window(ctx))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO);
        crate::ui::phone_surface(ctx, window).show(ctx, |ui| {
                    ui.set_width(card_w);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("60-second guide")
                                .size(style::FONT_TITLE)
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_sized([44.0, 44.0], egui::Button::new("×"))
                                .on_hover_text("Close guide")
                                .clicked()
                            {
                                act = -2;
                            }
                        });
                    });
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(TITLES[step])
                            .size(style::FONT_LG)
                            .strong()
                            .color(accent),
                    );
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(body).size(style::FONT_BASE));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 72.0).max(0.0) / 2.0);
                        for i in 0..TITLES.len() {
                            ui.label(
                                egui::RichText::new(if i == step { "●" } else { "○" }).color(
                                    if i == step {
                                        accent
                                    } else {
                                        ui.visuals().weak_text_color()
                                    },
                                ),
                            );
                        }
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if step > 0
                            && ui
                                .add_sized([88.0, 44.0], egui::Button::new("Back"))
                                .clicked()
                        {
                            act = -1;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let last = step + 1 == TITLES.len();
                            if ui
                                .add_sized(
                                    [112.0, 44.0],
                                    egui::Button::new(if last { "Done" } else { "Next" })
                                        .fill(accent),
                                )
                                .clicked()
                            {
                                act = 1;
                            }
                        });
                    });
        });
        match act {
            1 => self.next(),
            -1 => self.back(),
            -2 => self.open = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_navigation_is_bounded() {
        let mut t = Tour::default();
        t.start();
        t.back();
        assert_eq!(t.step, 0);
        t.next();
        t.back();
        assert_eq!(t.step, 0);
        for _ in 0..TITLES.len() {
            t.next();
        }
        assert!(!t.open);
    }

    #[test]
    fn start_resets_a_finished_tour() {
        let mut t = Tour::default();
        t.start();
        t.step = 3;
        t.start();
        assert!(t.open && t.step == 0);
    }
}
