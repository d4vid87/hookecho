//! Transient chrome over the map: toasts, warning banners, info and error chips.

use super::*;

impl HookEchoApp {
    /// Show a warning banner, or refresh the matching one already on screen — a repeated event
    /// bumps its clock instead of stacking a duplicate card over the radar.
    pub(crate) fn banner(&mut self, event: String, area: String) {
        // ponytail: linear scan; the lane holds a handful of cards at most.
        if let Some(b) = self
            .warning_banners
            .iter_mut()
            .find(|(e, _, _)| *e == event)
        {
            b.1 = area;
            b.2 = Instant::now();
        } else {
            self.warning_banners.push((event, area, Instant::now()));
        }
    }

    /// Say something happened. Operation results used to reach the user only through the log.
    pub(crate) fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
    }

    /// Say something once, ever. A hint is for the thing that is discoverable only if somebody
    /// tells you it is there — the ⓘ links, the fact that panes are independent — and it earns
    /// exactly one appearance per install.
    pub(crate) fn hint(&mut self, id: &str, text: &str) {
        if self.settings.hints_seen.iter().any(|s| s == id) {
            return;
        }
        self.settings.hints_seen.push(id.to_string());
        self.settings.save();
        self.toast(ToastKind::Info, text);
    }

    /// Toast stack, below the right-hand control column. Expires after ~4 s, fading out over the
    /// last second; click to dismiss.
    /// ponytail: clock-based fade, no per-toast animation ids.
    pub(crate) fn show_toasts(&mut self, ctx: &egui::Context) {
        const LIFE: f32 = 4.0;
        self.toasts.retain(|t| t.at.elapsed().as_secs_f32() < LIFE);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let accent = crate::theme::accent(self.settings.theme);
        let mut dismiss = None;
        egui::Area::new(egui::Id::new("toasts"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(
                    crate::ui::style::LANE_RIGHT_BADGE_X,
                    crate::ui::style::lane_right_badge_y(6),
                ),
            )
            .show(ctx, |ui| {
                for (i, t) in self.toasts.iter().enumerate() {
                    let left = LIFE - t.at.elapsed().as_secs_f32();
                    let alpha = left.min(1.0).clamp(0.0, 1.0);
                    let stripe = match t.kind {
                        ToastKind::Info => accent,
                        ToastKind::Success => crate::ui::style::OMEGA_GREEN,
                        ToastKind::Error => egui::Color32::from_rgb(220, 90, 90),
                    };
                    let resp = crate::ui::style::glass(ui, (220.0 * alpha) as u8)
                        .stroke(egui::Stroke::new(1.0, stripe.gamma_multiply(alpha)))
                        .show(ui, |ui| {
                            ui.set_max_width(320.0);
                            ui.label(
                                egui::RichText::new(&t.text)
                                    .size(crate::ui::style::FONT_BASE)
                                    .color(egui::Color32::from_gray(235).gamma_multiply(alpha)),
                            );
                        })
                        .response;
                    if resp.interact(egui::Sense::click()).clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

    /// Draw new-warning banners at top-center (auto-expire ~45s; click to dismiss all).
    pub(crate) fn show_warning_banners(&mut self, ctx: &egui::Context) {
        self.warning_banners
            .retain(|(_, _, at)| at.elapsed().as_secs() < 45);
        if self.warning_banners.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("warning_banners"))
            .constrain_to(self.chrome_rect)
            // Read-only banner that self-expires in 45 s. Interactable layers occlude the map's
            // pinch test (`layer_id_at`), and a banner across the top of a phone screen is exactly
            // where a two-finger gesture lands, so it must not take input.
            .interactable(false)
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(0.0, crate::ui::style::LANE_TOP_BANNER),
            )
            .show(ctx, |ui| {
                for (event, area, at) in &self.warning_banners {
                    // Fade out over the last two seconds instead of vanishing mid-read.
                    let a = ((45.0 - at.elapsed().as_secs_f32()) / 2.0).clamp(0.0, 1.0);
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(150, 20, 20).gamma_multiply(a))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(255, 120, 120).gamma_multiply(a),
                        ))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(egui_phosphor::regular::WARNING)
                                        .size(16.0)
                                        .color(egui::Color32::WHITE.gamma_multiply(a)),
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("New {event}"))
                                            .strong()
                                            .color(egui::Color32::WHITE.gamma_multiply(a)),
                                    );
                                    if !area.is_empty() {
                                        ui.label(egui::RichText::new(area).small().color(
                                            egui::Color32::from_gray(230).gamma_multiply(a),
                                        ));
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
            });
        // Fast enough for the fade-out; the banner is only up for 45 s.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    pub(crate) fn info_chip(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use egui_phosphor::regular as ph;
        // Icon, tool name, and what to do with it — an armed tool should look armed, not leave a
        // sentence of instructions as the only sign anything changed.
        let (glyph, name, hint) = match self.tool {
            MapTool::Measure => (ph::RULER, "Measure", "click two points"),
            MapTool::Marker => (ph::MAP_PIN, "Drop marker", "click the map"),
            MapTool::CrossSection => (ph::CHART_LINE, "Cross-section", "click two points"),
            MapTool::Sounding => (ph::THERMOMETER_SIMPLE, "Sounding", "click a point"),
            MapTool::Forecast => (ph::CLOUD_SUN, "Forecast", "click a point"),
            MapTool::Chase => (ph::CROSSHAIR, "Chase", "click your location"),
            MapTool::Climatology => (ph::TORNADO, "Climatology", "click a point"),
            MapTool::Draw => (ph::PENCIL_SIMPLE, "Draw", "drag to scribble"),
            MapTool::AlertZone => (
                ph::POLYGON,
                "Watch zone",
                "click each corner · double-click to close",
            ),
            MapTool::Interrogate => ("", "", ""),
        };
        // Nothing armed, nothing to say. The chip used to be a permanent readout of the cursor's
        // lat/lon and the zoom level — numbers nobody was reading, in the corner where the map is.
        if hint.is_empty() {
            return;
        }
        let accent = crate::theme::accent(self.settings.theme);
        // An armed tool changes what a click means, so the cursor says so over the whole map.
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        // Escape disarms, the same way it closes every other transient thing.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tool = MapTool::Interrogate;
            return;
        }
        egui::Area::new(egui::Id::new("info_chip"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-14.0, style::LANE_BOTTOM_CHIP),
            )
            // The draw tool's chip carries buttons; the rest are read-only hints that must never
            // swallow a click meant for the map underneath.
            .interactable(self.tool == MapTool::Draw)
            .show(ctx, |ui| {
                style::glass(ui, 238).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(glyph).size(15.0).color(accent));
                        ui.label(
                            egui::RichText::new(name)
                                .size(style::FONT_BASE)
                                .color(accent)
                                .strong(),
                        );
                        ui.label(egui::RichText::new(hint).size(style::FONT_SM).weak());
                        ui.label(
                            egui::RichText::new("· Esc cancels")
                                .size(style::FONT_SM)
                                .weak(),
                        );
                        if self.tool != MapTool::Draw {
                            return;
                        }
                        ui.separator();
                        for c in DRAW_COLORS {
                            let sel = self.draw_color == c;
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                            let p = ui.painter_at(rect);
                            p.circle_filled(rect.center(), 7.0, c);
                            if sel {
                                p.circle_stroke(
                                    rect.center(),
                                    8.5,
                                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                                );
                            }
                            if resp.clicked() {
                                self.draw_color = c;
                            }
                        }
                        ui.separator();
                        if ui
                            .add_enabled(!self.strokes.is_empty(), egui::Button::new("Undo"))
                            .clicked()
                        {
                            self.strokes.pop();
                        }
                        if ui
                            .add_enabled(!self.strokes.is_empty(), egui::Button::new("Clear"))
                            .clicked()
                        {
                            self.strokes.clear();
                        }
                    });
                });
            });
    }

    /// Bottom-center error chip: the active pane's fetch error, auto-hiding after ~6 seconds.
    ///
    /// ponytail: one chip, newest error wins, no toast queue — add a queue if overlapping errors
    /// from different panes turn out to matter.
    pub(crate) fn error_chip(&mut self, ctx: &egui::Context) {
        const HOLD_SECS: f64 = 6.0;
        let now = ctx.input(|i| i.time);
        if let Some(e) = self.views[self.active].error.clone() {
            if self.error_chip.as_ref().is_none_or(|(prev, _)| *prev != e) {
                self.error_chip = Some((e, now));
            }
        }
        let Some((msg, since)) = self.error_chip.clone() else {
            return;
        };
        if now - since > HOLD_SECS {
            self.error_chip = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let chip = egui::Area::new(egui::Id::new("error_chip"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, crate::ui::style::LANE_BOTTOM_CHASE),
            )
            // Interactable so the message can be read and copied: six seconds is not long enough
            // to transcribe a URL out of a failure. The chip is small and sits in the bottom lane,
            // so what it costs the map underneath is that strip.
            .interactable(true)
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 246)
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(230, 100, 100).gamma_multiply(0.8),
                    ))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&msg)
                                    .size(crate::ui::style::FONT_BASE)
                                    .color(egui::Color32::from_rgb(240, 150, 150)),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click to copy")
                    })
            });
        let resp = &chip.inner.inner;
        if resp.hovered() {
            // Hold while it is being read.
            self.error_chip = Some((msg.clone(), now));
        }
        if resp.clicked() {
            ctx.copy_text(msg);
            self.toast(ToastKind::Success, "Error copied");
            self.error_chip = None;
        }
    }
}
