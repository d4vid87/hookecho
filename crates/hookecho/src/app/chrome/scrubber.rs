//! The scrubber pill: the transport, the clock, and a drawn time track, floating over the map's
//! bottom edge.
//!
//! The track is drawn rather than being an `egui::Slider` because it carries more than a value:
//! volume ticks, hour labels, the forecast tail and the live loop window all have to read off the
//! same axis.

use super::*;

impl HookEchoApp {
    pub(crate) fn scrubber(&mut self, ctx: &egui::Context) {
        crate::prof_scope!("scrubber");
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
            // Leading space: on the phone this label butts straight up against the clock, which
            // is the last thing in the opposite layout.
            format!(" ({} ago)", humanize(secs))
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
        // Wide enough for the track to be worth scrubbing, never so wide it spans a 4K map — and
        // never wider than the screen, which on a phone the 420 pt floor would otherwise be.
        let width = (self.chrome_rect.width() - 160.0)
            .clamp(420.0, 900.0)
            .min(self.chrome_rect.width() - 16.0);
        // The phone's pill drops the two extras: the readouts fit a desktop row, not a 400 pt one,
        // and rain arrival has its own chip lane.
        let (dvr, rain) = if cfg!(target_os = "android") {
            (0, None)
        } else {
            (dvr, rain)
        };
        let live_window = self.views[self.active].timeline.live_window;
        egui::Area::new(egui::Id::new("scrubber"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, crate::ui::style::LANE_BOTTOM_CHIP),
            )
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 238).show(ui, |ui| {
                ui.set_width(width);
                let t = &mut self.views[self.active].timeline;
                ui.horizontal(|ui| {
                    // The phone says the site in its search pill; a second copy here is 60 pt of
                    // a 400 pt row spent saying it twice, and the clock loses that argument.
                    if !cfg!(target_os = "android") {
                        ui.label(
                            egui::RichText::new(&site)
                                .size(crate::ui::style::FONT_BASE)
                                .strong()
                                .color(egui::Color32::from_gray(238)),
                        );
                    }
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
                    // The clock and the two status readouts ride the top row with the
                    // transport; the track gets a row to itself underneath.
                    let observed = t.frames.len();
                    if observed == 0 {
                        ui.weak(if t.listing {
                            "listing volumes\u{2026}"
                        } else {
                            "(no volumes)"
                        });
                    } else {
                        let readout = match t.forecast_hour() {
                            Some(h) => format!("F+{h}h"),
                            None => t
                                .current()
                                .and_then(|id| id.date_time())
                                .map(|d| crate::timefmt::fmt_clock(d, tz, true))
                                .unwrap_or_default(),
                        };
                        ui.label(
                            egui::RichText::new(readout)
                                .size(12.0)
                                .monospace()
                                .color(egui::Color32::from_gray(215)),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(age) = &age {
                            ui.label(
                                egui::RichText::new(age)
                                    .size(crate::ui::style::FONT_SM)
                                    .color(egui::Color32::from_gray(150)),
                            );
                        } else if loading {
                            ui.label(
                                egui::RichText::new("loading\u{2026}")
                                    .size(crate::ui::style::FONT_SM)
                                    .color(egui::Color32::from_gray(150)),
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
                    });
                });
                if t.slot_count() > 0 {
                    scrub_rect = Some(track(ui, t, tz, accent, live_window));
                }
                });
            });
        self.settings.live_loop_frames = loop_frames;
        if let Some(r) = scrub_rect {
            // The scrubber swallows two-finger gestures on the phone like any other surface.
            self.mobile_occlusion.push(r);
        }
        self.tour_anchors.timeline = scrub_rect;
        if go_head {
            self.views[self.active].timeline.go_head();
        }
    }
}

/// The time track: one tick per volume, hour labels, the forecast tail, the live loop window,
/// and a knob you can drag. Returns its rect for the tour's spotlight.
fn track(
    ui: &mut egui::Ui,
    t: &mut crate::timeline::Timeline,
    tz: Option<wxdata::tz::Tz>,
    accent: egui::Color32,
    live_window: usize,
) -> egui::Rect {
    let slots = t.slot_count();
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 34.0),
        egui::Sense::click_and_drag(),
    );
    let p = ui.painter_at(rect);
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - 12.0),
        egui::pos2(rect.right(), rect.bottom() - 6.0),
    );
    // Slot centres, so the first and last frames sit inside the track instead of half off it.
    let x_of = |i: usize| bar.left() + (i as f32 + 0.5) / slots as f32 * bar.width();
    p.rect_filled(bar, 3.0, egui::Color32::from_gray(60));

    let observed = t.frames.len();
    // The model tail is a different kind of time and says so, exactly as the old slider did.
    if slots > observed && observed > 0 {
        let x = x_of(observed);
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x, bar.top()),
                egui::pos2(bar.right(), bar.bottom()),
            ),
            3.0,
            egui::Color32::from_rgba_unmultiplied(120, 170, 240, 90),
        );
        p.line_segment(
            [egui::pos2(x, rect.top() + 6.0), egui::pos2(x, bar.bottom())],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 170, 240)),
        );
    }
    // The live loop window: the stretch ▶ actually cycles through when pinned to live. Without
    // it, pressing play on a day of frames looks like it jumped backwards for no reason.
    if t.following && observed > 0 {
        let from = observed.saturating_sub(live_window.max(1));
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x_of(from), bar.top()),
                egui::pos2(x_of(observed.saturating_sub(1)), bar.bottom()),
            ),
            3.0,
            accent.gamma_multiply(0.35),
        );
    }
    // Played-so-far fill, then one tick per volume with an hour label wherever the hour turns
    // over — the axis a chaser reads to find "the 22Z scan" without scrubbing for it.
    p.rect_filled(
        egui::Rect::from_min_max(bar.left_top(), egui::pos2(x_of(t.playhead), bar.bottom())),
        3.0,
        accent.gamma_multiply(0.8),
    );
    let mut last_hour = None;
    let mut last_label_x = f32::NEG_INFINITY;
    for (i, id) in t.frames.iter().enumerate() {
        let x = x_of(i);
        p.line_segment(
            [
                egui::pos2(x, bar.top() - 3.0),
                egui::pos2(x, bar.top() - 1.0),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(120)),
        );
        let Some(dt) = id.date_time() else { continue };
        let hour = hour_key(dt, tz);
        let turned = last_hour != Some(hour);
        last_hour = Some(hour);
        // Label the hour, not the volume: a five-minute clock on every tick is a smear. 96 px of
        // clearance, and both ends have to fit inside the track or the pill clips them.
        let half = 26.0;
        if !turned || x - last_label_x < 96.0 || x - half < rect.left() || x + half > rect.right() {
            continue;
        }
        p.text(
            egui::pos2(x, rect.top() + 1.0),
            egui::Align2::CENTER_TOP,
            hour_label(dt, tz),
            egui::FontId::proportional(crate::ui::style::FONT_SM),
            egui::Color32::from_gray(170),
        );
        p.line_segment(
            [
                egui::pos2(x, bar.top() - 6.0),
                egui::pos2(x, bar.top() - 1.0),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(150)),
        );
        last_label_x = x;
    }
    let knob = egui::pos2(x_of(t.playhead), bar.center().y);
    p.circle_filled(knob, 7.0, accent);
    p.circle_stroke(knob, 7.0, egui::Stroke::new(1.0, egui::Color32::BLACK));

    // Click anywhere on the track, or drag the knob: both are the same "put the playhead here".
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let frac = ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            let idx = ((frac * slots as f32) as usize).min(slots - 1);
            if idx != t.playhead {
                t.playhead = idx;
                t.playing = false;
                t.following = idx + 1 == observed;
            }
        }
    }
    rect
}

/// Which hour a frame falls in, in the radar's own zone — the axis the labels step along.
fn hour_key(dt: chrono::DateTime<Utc>, tz: Option<wxdata::tz::Tz>) -> (u32, u32) {
    use chrono::{Datelike, Timelike};
    match tz {
        Some(tz) => {
            let l = dt.with_timezone(&tz);
            (l.ordinal(), l.hour())
        }
        None => (dt.ordinal(), dt.hour()),
    }
}

/// Hour tick label: short enough to repeat across the track ("8 PM", or "20Z" in Zulu).
fn hour_label(dt: chrono::DateTime<Utc>, tz: Option<wxdata::tz::Tz>) -> String {
    match tz {
        Some(tz) => dt.with_timezone(&tz).format("%-I %p").to_string(),
        None => dt.format("%HZ").to_string(),
    }
}
