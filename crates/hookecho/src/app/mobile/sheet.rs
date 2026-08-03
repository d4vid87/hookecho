//! The persistent bottom sheet — the phone's primary surface.
//!
//! Material 3's map-first pattern: the map owns the screen, and everything about *this* radar
//! lives in one sheet anchored to the bottom edge that the user drags between three snap points.
//! Peek is a single summary line, half adds the scrubber and the product grid, full adds the
//! archive. The docked toolbar is the sheet's bottom row rather than a second floating bar,
//! because two stacked bars is exactly what M3 tells you not to build.
//!
//! Nothing here owns data: every control writes straight into the same `timeline`/`views` state
//! the desktop UI drives.

use egui::{pos2, vec2, Align, Color32, Id, Layout, Rect, RichText, Sense, Stroke};
use egui_phosphor::regular as ph;
use wxdata::level2::Moment;

use crate::ui::m3;
use crate::ui::style::{OMEGA_GREEN, OMEGA_ORANGE};

/// How far open the persistent sheet is.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SheetSnap {
    /// One summary row above the toolbar.
    #[default]
    Peek,
    /// Scrubber, products, tilt.
    Half,
    /// Everything, including the archive frame list.
    Full,
}

/// Sheet chrome that is always on screen, whatever the snap: handle + summary + toolbar.
const FIXED_H: f32 = 26.0 + m3::MIN_TARGET + m3::TOOLBAR_H + m3::SP_3;

impl SheetSnap {
    /// Pixel height for this snap inside `content`.
    fn height(self, content: Rect) -> f32 {
        match self {
            SheetSnap::Peek => FIXED_H,
            SheetSnap::Half => (content.height() * 0.45).max(FIXED_H + m3::SP_6),
            // Leave the top chips (and the map behind them) visible even at full.
            SheetSnap::Full => (content.height() - m3::TOOLBAR_H).max(FIXED_H),
        }
    }

    fn from_height(h: f32, content: Rect) -> Self {
        [SheetSnap::Peek, SheetSnap::Half, SheetSnap::Full]
            .into_iter()
            .min_by(|a, b| {
                let d = |s: SheetSnap| (s.height(content) - h).abs();
                d(*a).total_cmp(&d(*b))
            })
            .unwrap_or_default()
    }

    /// One step closed. `None` at Peek — the caller lets Android's back leave the app.
    pub(crate) fn collapsed(self) -> Option<Self> {
        match self {
            SheetSnap::Full => Some(SheetSnap::Half),
            SheetSnap::Half => Some(SheetSnap::Peek),
            SheetSnap::Peek => None,
        }
    }
}

/// Everything the sheet needs to read, copied out so its body doesn't borrow the app.
pub(crate) struct SheetInfo {
    pub moment: Moment,
    pub srv: bool,
    pub when: String,
    pub following: bool,
    pub playing: bool,
    pub nframes: usize,
    pub playhead: usize,
    pub n_tilt: usize,
    pub cur_tilt: usize,
    pub cur_angle: f32,
    pub speed: f32,
    pub frame_labels: Vec<String>,
}

impl crate::app::HookEchoApp {
    /// Draw the persistent sheet and return the rect it occupies (for gesture occlusion).
    pub(crate) fn mobile_sheet_surface(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        info: &SheetInfo,
        alert_count: usize,
        max_esc: u8,
    ) -> Rect {
        // Landscape has height to spare and no width: the sheet becomes a full-height rail down
        // the leading edge, with the same body and the same docked toolbar at its foot. No snaps
        // — a rail that is already floor-to-ceiling has nothing to expand into.
        let rail = m3::is_landscape(content);
        let rect = if rail {
            Rect::from_min_max(
                content.min,
                pos2(
                    content.left() + m3::RAIL_W.min(content.width() * 0.45),
                    content.bottom(),
                ),
            )
        } else {
            let target = self.mobile_snap.height(content);
            // While a drag is live the finger owns the height; otherwise it eases to the snap.
            let h = match self.mobile_sheet_drag {
                Some(dragged) => dragged,
                None => ctx.animate_value_with_time(Id::new("m_sheet_h"), target, m3::DUR_MED),
            }
            .clamp(FIXED_H, SheetSnap::Full.height(content));
            Rect::from_min_max(pos2(content.left(), content.bottom() - h), content.max)
        };
        let accent = crate::theme::accent(self.settings.theme);

        egui::Area::new(Id::new("m_sheet"))
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            // The body always lays out at full height and is clipped to the current snap, so the
            // Area's own bounds are taller than the sheet you can see. egui constrains an Area to
            // the screen by shifting it UP, which parked an invisible input-eating layer over the
            // middle of the map: `layer_id_at` saw the sheet where the map was, so pinch (and taps)
            // there went nowhere. Constrain to the sheet instead: bounds now match the visible rect.
            .constrain_to(rect)
            .show(ctx, |ui| {
                // Claim the whole sheet rect up front. An Area's interactive region is its
                // content's bounding box, so a sheet that painted itself and then laid out a
                // narrower column let taps fall straight through to the map behind it. This also
                // swallows stray taps on the sheet's empty space.
                //
                // `allocate_rect` and not `set_min_size`: the latter grows the Ui's min_rect,
                // which pushes the layout cursor past the bottom of the sheet, and everything
                // that followed got laid out off-screen.
                ui.allocate_rect(rect, egui::Sense::click_and_drag());
                ui.set_clip_rect(rect);
                let painter = ui.painter().clone();
                // Rounded on the edges that face the map, square where it meets the screen.
                let radius = if rail {
                    egui::CornerRadius {
                        ne: m3::R_XL as u8,
                        se: m3::R_XL as u8,
                        nw: 0,
                        sw: 0,
                    }
                } else {
                    egui::CornerRadius {
                        nw: m3::R_XL as u8,
                        ne: m3::R_XL as u8,
                        sw: 0,
                        se: 0,
                    }
                };
                painter.rect_filled(rect, radius, sheet_fill(ui));
                let edge = if rail {
                    [rect.right_top(), rect.right_bottom()]
                } else {
                    [rect.left_top(), rect.right_top()]
                };
                painter.line_segment(
                    edge,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20)),
                );
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(rect.shrink2(vec2(m3::SP_4, 0.0))),
                    |ui| {
                        ui.set_width(rect.width() - m3::SP_4 * 2.0);
                        ui.spacing_mut().item_spacing.y = m3::SP_2;
                        if rail {
                            ui.add_space(m3::SP_2);
                        } else {
                            self.sheet_handle(ui, content);
                        }
                        self.sheet_summary(ui, info, accent);
                        // The scrollable middle gets whatever is left between the summary and the
                        // docked toolbar; at Peek that is nothing and the section doesn't draw.
                        let free = rect.bottom() - ui.cursor().top() - m3::TOOLBAR_H - m3::SP_2;
                        if free > m3::SP_6 {
                            egui::ScrollArea::vertical()
                                .max_height(free)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.sheet_body(ui, info, accent);
                                });
                        }
                        let bar = Rect::from_min_max(
                            pos2(rect.left(), rect.bottom() - m3::TOOLBAR_H),
                            rect.max,
                        );
                        self.mobile_toolbar(ui, bar, alert_count, max_esc);
                    },
                );
            });
        rect
    }

    /// The drag pill. Dragging resizes the sheet; a tap cycles to the next snap.
    fn sheet_handle(&mut self, ui: &mut egui::Ui, content: Rect) {
        let resp = m3::drag_handle(ui);
        let live = self
            .mobile_sheet_drag
            .unwrap_or_else(|| self.mobile_snap.height(content));
        if resp.dragged() {
            // Dragging up (negative dy) grows the sheet.
            let h = (live - resp.drag_delta().y).clamp(FIXED_H, SheetSnap::Full.height(content));
            self.mobile_sheet_drag = Some(h);
        }
        if resp.drag_stopped() {
            let vel = -ui.input(|i| i.pointer.velocity().y);
            let snaps: Vec<f32> = [SheetSnap::Peek, SheetSnap::Half, SheetSnap::Full]
                .iter()
                .map(|s| s.height(content))
                .collect();
            // `snap_to` speaks "positive velocity closes", the opposite of the screen's y axis.
            let landed = m3::snap_to(live, -vel, &snaps);
            self.mobile_snap = SheetSnap::from_height(landed, content);
            self.mobile_sheet_drag = None;
            // Start the ease from where the finger left off instead of from the old snap.
            ui.ctx()
                .animate_value_with_time(Id::new("m_sheet_h"), live, 0.0);
        }
        if resp.clicked() {
            self.mobile_snap = match self.mobile_snap {
                SheetSnap::Peek => SheetSnap::Half,
                SheetSnap::Half => SheetSnap::Full,
                SheetSnap::Full => SheetSnap::Peek,
            };
        }
    }

    /// The always-visible line: play, product, tilt, time.
    fn sheet_summary(&mut self, ui: &mut egui::Ui, info: &SheetInfo, accent: Color32) {
        let active = self.active;
        ui.horizontal(|ui| {
            ui.set_height(m3::MIN_TARGET);
            let glyph = if info.playing { ph::PAUSE } else { ph::PLAY };
            if icon_button(ui, glyph, info.playing, accent).clicked() {
                self.views[active].timeline.toggle_play();
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    RichText::new(crate::products::name(info.moment, info.srv))
                        .size(m3::T_TITLE_SM)
                        .strong()
                        .color(accent),
                );
                ui.horizontal(|ui| {
                    let dot = if info.following {
                        OMEGA_GREEN
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, dot);
                    ui.label(
                        RichText::new(&info.when)
                            .size(m3::T_LABEL)
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if info.n_tilt > 0 {
                    // The tilt pill: tap steps to the next elevation, which is what the old
                    // product sheet's pill did, minus the sheet you had to open to reach it.
                    let label = format!("{:.1}°", info.cur_angle);
                    if m3::chip(ui, &label, false).clicked() {
                        self.views[active].tilt = (info.cur_tilt + 1) % info.n_tilt;
                    }
                }
            });
        });
    }

    /// Half + full content: scrubber, products, tilt, archive.
    fn sheet_body(&mut self, ui: &mut egui::Ui, info: &SheetInfo, accent: Color32) {
        let active = self.active;
        ui.add_space(m3::SP_1);
        // ---- Scrubber ----
        if info.nframes > 1 {
            let mut idx = info.playhead as f32;
            let slider = egui::Slider::new(&mut idx, 0.0..=(info.nframes - 1) as f32)
                .show_value(false)
                .handle_shape(egui::style::HandleShape::Circle);
            if ui
                .add_sized([ui.available_width(), m3::MIN_TARGET], slider)
                .changed()
            {
                let t = &mut self.views[active].timeline;
                t.playhead = idx.round() as usize;
                t.following = false;
                t.seek_target = None;
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Frame {}/{}", info.playhead + 1, info.nframes))
                        .size(m3::T_LABEL)
                        .color(ui.visuals().weak_text_color()),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for (label, mult) in [("2×", 2.0f32), ("1×", 1.0), ("½×", 0.5)] {
                        let on = (info.speed - mult).abs() < 0.01;
                        if m3::chip(ui, label, on).clicked() {
                            self.views[active].timeline.speed = mult;
                        }
                    }
                    if m3::chip(ui, "Live", info.following).clicked() {
                        self.views[active].timeline.go_head();
                    }
                });
            });
        }
        ui.add_space(m3::SP_2);
        section(ui, "Product");
        // ---- Products ----
        // Same rows the old modal picker had; the difference is that they are two taps closer.
        type Row = (Moment, bool, &'static str);
        let rows: [Row; 7] = [
            (Moment::Reflectivity, false, "Reflectivity"),
            (Moment::Velocity, false, "Velocity"),
            (Moment::Velocity, true, "Storm-Relative"),
            (Moment::SpectrumWidth, false, "Spectrum Width"),
            (Moment::CorrelationCoefficient, false, "CC"),
            (Moment::DifferentialReflectivity, false, "ZDR"),
            (Moment::DifferentialPhase, false, "KDP"),
        ];
        ui.horizontal_wrapped(|ui| {
            for (m, want_srv, label) in rows {
                let selected = info.moment == m && (m != Moment::Velocity || info.srv == want_srv);
                if m3::chip(ui, label, selected).clicked() {
                    self.views[active].moment = m;
                    if m == Moment::Velocity {
                        self.views[active].srv = want_srv;
                    }
                }
            }
        });
        // ---- Tilt ----
        if info.n_tilt > 1 {
            ui.add_space(m3::SP_2);
            section(ui, "Elevation");
            ui.horizontal_wrapped(|ui| {
                for t in 0..info.n_tilt {
                    let angle = self.views[active]
                        .volume
                        .as_ref()
                        .and_then(|v| v.elevations.get(t).copied())
                        .unwrap_or(0.0);
                    if m3::chip(ui, &format!("{angle:.1}°"), t == info.cur_tilt).clicked() {
                        self.views[active].tilt = t;
                    }
                }
            });
        }
        // ---- Archive (full only) ----
        if self.mobile_snap == SheetSnap::Full && !info.frame_labels.is_empty() {
            ui.add_space(m3::SP_2);
            section(ui, "Frames");
            for (i, label) in info.frame_labels.iter().enumerate() {
                if m3::list_row(ui, "", label, None, i == info.playhead).clicked() {
                    let t = &mut self.views[active].timeline;
                    t.playhead = i;
                    t.following = false;
                    t.seek_target = None;
                }
            }
        }
        let _ = accent;
        ui.add_space(m3::SP_4);
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(m3::T_LABEL)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
}

/// The sheet's own surface color: the theme's panel fill lifted by an M3 surface tint so the sheet
/// reads as a layer above the map rather than a hole in it.
pub(crate) fn sheet_fill(ui: &egui::Ui) -> Color32 {
    let base = ui.visuals().panel_fill;
    let lift = if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, m3::SURF_MED)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, m3::SURF_LOW)
    };
    m3::blend(base.to_opaque(), lift)
}

/// A 48pt round icon button with an M3 state layer.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    glyph: &str,
    active: bool,
    accent: Color32,
) -> egui::Response {
    let fg = if active {
        accent
    } else {
        ui.visuals().text_color()
    };
    let resp = ui.add(
        egui::Button::new(RichText::new(glyph).size(m3::T_HEADLINE).color(fg))
            .min_size(vec2(m3::MIN_TARGET, m3::MIN_TARGET))
            .corner_radius(m3::R_FULL)
            .fill(if active {
                accent.gamma_multiply(0.18)
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE),
    );
    if resp.is_pointer_button_down_on() {
        ui.painter().circle_filled(
            resp.rect.center(),
            m3::MIN_TARGET / 2.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, m3::A_PRESSED),
        );
    }
    resp
}

/// Reserved for the orange live/record semantics the toolbar keeps.
pub(crate) const LIVE_TINT: Color32 = OMEGA_ORANGE;

/// A modal bottom sheet: scrim, drag handle, title bar with a close button, scrollable body.
///
/// This is what the phone opens instead of a desktop drawer. It is the same shape as the
/// persistent sheet (top corners, handle, surface tint) so the two read as one system, but it is
/// modal: the scrim dims the map and a tap outside dismisses it.
///
/// Returns the rect it covers, for gesture occlusion.
///
/// ponytail: fixed at the "expanded" height rather than draggable. These sheets are lists you
/// came to read, not a summary you peek at — give them a snap ladder when one of them grows a
/// map-adjacent preview worth half-opening.
pub(crate) fn modal_sheet<R>(
    ctx: &egui::Context,
    content: Rect,
    id: &str,
    title: &str,
    close: &mut bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Rect {
    let h = content.height() * 0.88;
    let rect = Rect::from_min_max(pos2(content.left(), content.bottom() - h), content.max);

    // Scrim first, in its own layer under the sheet.
    egui::Area::new(Id::new(format!("{id}_scrim")))
        .order(egui::Order::Middle)
        .fixed_pos(content.min)
        .show(ctx, |ui| {
            let resp = ui.allocate_rect(content, Sense::click());
            ui.painter().rect_filled(
                content,
                egui::CornerRadius::ZERO,
                Color32::from_black_alpha(m3::A_SCRIM),
            );
            if resp.clicked()
                && !resp
                    .interact_pointer_pos()
                    .is_some_and(|p| rect.contains(p))
            {
                *close = true;
            }
        });

    egui::Area::new(Id::new(id))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            ui.allocate_rect(rect, Sense::click_and_drag());
            ui.set_clip_rect(rect);
            let painter = ui.painter().clone();
            painter.rect_filled(
                rect,
                egui::CornerRadius {
                    nw: m3::R_XL as u8,
                    ne: m3::R_XL as u8,
                    sw: 0,
                    se: 0,
                },
                sheet_fill(ui),
            );
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(rect.shrink2(vec2(m3::SP_4, 0.0))),
                |ui| {
                    ui.set_width(rect.width() - m3::SP_4 * 2.0);
                    if m3::drag_handle(ui).clicked() {
                        *close = true;
                    }
                    ui.horizontal(|ui| {
                        ui.set_height(m3::MIN_TARGET);
                        ui.label(
                            RichText::new(title)
                                .size(m3::T_TITLE)
                                .strong()
                                .color(ui.visuals().strong_text_color()),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let accent = ui.visuals().selection.bg_fill;
                            if icon_button(ui, ph::X, false, accent).clicked() {
                                *close = true;
                            }
                        });
                    });
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(rect.bottom() - ui.cursor().top() - m3::SP_2)
                        .show(ui, |ui| {
                            add(ui);
                            ui.add_space(m3::SP_6);
                        });
                },
            );
        });
    rect
}
