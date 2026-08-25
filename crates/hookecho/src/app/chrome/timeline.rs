//! The bottom timeline panel: frame scrubber, play controls, archive picker.

use super::*;

impl HookEchoApp {
    pub(crate) fn timeline_bar(&mut self, root: &mut egui::Ui) {
        crate::prof_scope!("timeline_bar");
        use egui_phosphor::regular as ph;
        let accent = crate::theme::accent(self.settings.theme);
        let tz = self.active_tz();
        let fresh = self.views[self.active]
            .volume
            .as_ref()
            .is_some_and(|v| (chrono::Utc::now() - v.time).num_seconds() < 900);
        // Site and data age used to live in the docked status bar; the clock belongs with the clock.
        let site = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "no site".to_string());
        let age = self.views[self.active].volume.as_ref().map(|v| {
            let secs = (Utc::now() - v.time).num_seconds().max(0);
            format!("({} ago)", humanize(secs))
        });
        let loading = self.views[self.active].loading;
        let mut go_head = false;
        // Soonest rain arrival and the DVR buffer depth ride the pill: both are about time, and
        // both used to sit in an always-on chip in the opposite corner.
        let rain = self
            .rain_eta
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(name, min)| format!("\u{1f327} {name} ~{min:.0} min"));
        let dvr = self.dvr_depth();
        // Edited through a local so the pill closure keeps its single `&mut self.views` borrow.
        let mut loop_frames = self.settings.live_loop_frames;
        // Where the scrubber lands, for the tour's spotlight (same reason: no `self` in there).
        let mut scrub_rect = None;
        egui::Panel::bottom("timeline_bar")
            .exact_size(44.0)
            .show(root, |ui| {
                let t = &mut self.views[self.active].timeline;
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new(&site)
                            .size(crate::ui::style::FONT_BASE)
                            .strong()
                            .color(egui::Color32::from_gray(238)),
                    );
                    let btn = |ui: &mut egui::Ui, glyph: &str, on: bool| {
                        let fg = if on {
                            accent
                        } else {
                            egui::Color32::from_gray(225)
                        };
                        ui.add(
                            egui::Button::new(egui::RichText::new(glyph).size(18.0).color(fg))
                                .min_size(egui::vec2(30.0, 30.0))
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    };
                    if btn(ui, ph::SKIP_BACK, false) {
                        t.step(-1);
                    }
                    let playing = t.playing;
                    if btn(ui, if playing { ph::PAUSE } else { ph::PLAY }, playing) {
                        t.toggle_play();
                    }
                    if btn(ui, ph::SKIP_FORWARD, false) {
                        t.step(1);
                    }
                    // Live / archive badge: click to re-pin to the newest volume.
                    //
                    // Pinned to live but the newest volume is old means the site's feed has
                    // stopped, not that playback drifted — there is nothing newer to jump to.
                    // The colour said so already; saying "LIVE" over a volume hours old did not.
                    let (col, text, hint) = if t.following && fresh {
                        (
                            mobile::OMEGA_GREEN,
                            "LIVE".to_string(),
                            "Following the newest volume.",
                        )
                    } else if t.following {
                        (
                            egui::Color32::from_rgb(220, 180, 0),
                            "STALE".to_string(),
                            "Following the newest volume, but this site has not produced one \
                             recently — its feed has stopped. The age next to the clock is how \
                             far behind it is.",
                        )
                    } else {
                        (
                            egui::Color32::from_gray(150),
                            format!("ARCHIVE {}", t.date.format("%m/%d")),
                            "Scrubbed to an archive day. Click to jump back to live.",
                        )
                    };
                    let badge = ui.add(
                        egui::Button::new(
                            egui::RichText::new(text)
                                .size(12.0)
                                .strong()
                                .color(egui::Color32::BLACK),
                        )
                        .fill(col)
                        .corner_radius(9.0),
                    )
                    .on_hover_text(hint);
                    if badge.clicked() {
                        go_head = true;
                    }
                    // Right-click the badge for the knobs that used to sit in the toolbox's
                    // Timeline section: which archive day, and how playback loops.
                    egui::Popup::context_menu(&badge)
                        .align(egui::RectAlign::TOP_START)
                        .show(|ui| {
                            ui.set_min_width(240.0);
                            ui.horizontal(|ui| {
                                ui.label("Date:");
                                if ui.button(egui_phosphor::regular::CARET_LEFT).clicked() {
                                    if let Some(d) = t.date.pred_opt() {
                                        t.date = d;
                                        t.following = false;
                                    }
                                }
                                // The carets are still the fastest way to step a day, but they
                                // were the *only* way: reaching a storm from years back meant
                                // thousands of clicks. The archive runs to June 1991.
                                let today = chrono::Utc::now().date_naive();
                                if let Some(d) = archive_day_input(ui, t.date) {
                                    // Clamp rather than trust the input: neither a calendar
                                    // widget nor a typed string knows where the archive starts
                                    // or that the future is empty.
                                    t.date = d.clamp(wxdata::level2::ARCHIVE_START, today);
                                    t.following = t.date >= today;
                                }
                                let is_today = t.date >= chrono::Utc::now().date_naive();
                                if ui
                                    .add_enabled(
                                        !is_today,
                                        egui::Button::new(egui_phosphor::regular::CARET_RIGHT),
                                    )
                                    .clicked()
                                {
                                    if let Some(d) = t.date.succ_opt() {
                                        t.date = d;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.button("⏮").on_hover_text("First frame").clicked() {
                                    t.go_begin();
                                }
                                ui.checkbox(&mut t.loop_enabled, "Loop");
                            });
                            ui.add(
                                egui::Slider::new(&mut t.speed, 1.0..=15.0)
                                    .suffix(" fps")
                                    .show_value(true),
                            );
                            ui.add(
                                egui::DragValue::new(&mut loop_frames)
                                    .range(2..=30)
                                    .suffix(" frames"),
                            )
                            .on_hover_text(
                                "How many of the newest volumes ▶ cycles through when live",
                            );
                        });
                    // Scrub bar + readout fill the rest of the pill.
                    let observed = t.frames.len();
                    if observed == 0 {
                        ui.weak(if t.listing {
                            "listing volumes…"
                        } else {
                            "(no volumes)"
                        });
                        return;
                    }
                    let readout = match t.forecast_hour() {
                        Some(h) => format!("F+{h}h"),
                        None => t
                            .current()
                            .and_then(|id| id.date_time())
                            .map(|d| crate::timefmt::fmt_clock(d, tz, true))
                            .unwrap_or_default(),
                    };
                    let last = t.slot_count().saturating_sub(1);
                    let mut ph_idx = t.playhead;
                    let slider_w = (ui.available_width() - 92.0).max(80.0);
                    let resp = ui.add_sized(
                        [slider_w, 20.0],
                        egui::Slider::new(&mut ph_idx, 0..=last).show_value(false),
                    );
                    scrub_rect = Some(resp.rect);
                    if resp.changed() {
                        t.playhead = ph_idx;
                        t.playing = false;
                        t.following = ph_idx + 1 == observed;
                    }
                    // Mark where observed radar ends and the HRRR forecast tail begins, so a
                    // scrub past the head reads as a model run and not as more radar.
                    if t.slot_count() > observed {
                        let frac = observed as f32 / t.slot_count() as f32;
                        let r = resp.rect;
                        let x = r.left() + frac * r.width();
                        let p = ui.painter_at(r);
                        p.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(x, r.top()),
                                egui::pos2(r.right(), r.bottom()),
                            ),
                            0.0,
                            // Faint enough to stay behind the slider, solid enough to read on
                            // the dark bar — at alpha 36 it was invisible in a screenshot.
                            egui::Color32::from_rgba_unmultiplied(120, 170, 240, 90),
                        );
                        p.line_segment(
                            [egui::pos2(x, r.top()), egui::pos2(x, r.bottom())],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 170, 240)),
                        );
                    }
                    ui.label(
                        egui::RichText::new(readout)
                            .size(12.0)
                            .monospace()
                            .color(egui::Color32::from_gray(215)),
                    );
                    if let Some(r) = &rain {
                        ui.label(
                            egui::RichText::new(r)
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_rgb(110, 180, 240)),
                        )
                        .on_hover_text(
                            "Estimated from storm motion \u{2014} rough for backbuilding storms",
                        );
                    }
                    if dvr > 1 {
                        ui.label(
                            egui::RichText::new(format!("\u{27f2} {dvr}"))
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(150)),
                        )
                        .on_hover_text("Frames buffered in memory for instant replay (R)");
                    }
                    if let Some(age) = &age {
                        ui.label(
                            egui::RichText::new(age)
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(150)),
                        );
                    } else if loading {
                        ui.label(
                            egui::RichText::new("loading…")
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(150)),
                        );
                    }
                });
            });
        self.settings.live_loop_frames = loop_frames;
        self.tour_anchors.timeline = scrub_rect;
        if go_head {
            self.views[self.active].timeline.go_head();
        }
    }
}
