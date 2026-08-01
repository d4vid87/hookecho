//! Touch-first Android chrome, RadarOmega-style: a full-bleed map under a full-width color scale,
//! floating chrome — a top bar (menu · VCP/site pill · 3D · alert badge), a bottom card (frames ·
//! product · timestamp · elevation) with a Phosphor icon tool dock, a categorized product picker,
//! a capture menu, and slide-in drawers. Every control drives the shared desktop data paths.

use egui::{
    pos2, vec2, Align, Align2, Color32, Frame, Id, Layout, Margin, Mesh, Rect, RichText, Sense,
    Shape, Stroke,
};
use egui_phosphor::regular as ph;
use wxdata::level2::Moment;

use crate::ui::layer_options::UiActions;

pub(crate) use crate::ui::style::{glass, square_btn, OMEGA_BLUE, OMEGA_GREEN, OMEGA_ORANGE};

/// Drawer navy (RadarOmega's menu background).
const DRAWER_BG: Color32 = Color32::from_rgb(0x0E, 0x18, 0x28);
/// RadarOmega's blue brand title color.
const OMEGA_TITLE: Color32 = Color32::from_rgb(0x38, 0xB6, 0xFF);

/// Which slide-in surface is open (`None` = just the floating chrome over the map).
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MobileSheet {
    #[default]
    None,
    Menu,
    Alerts,
    Products,
    Capture,
}

/// One alert row, owned so a drawer can list + fly to it without borrowing `self`.
struct MAlert {
    id: String,
    title: String,
    sub: String,
    color: Color32,
    lon: f64,
    lat: f64,
    esc: u8,
}

/// A labeled dock slot: glyph over a small caption. Ten unlabeled glyphs asked the user to guess
/// which one was the sounding tool and which was range rings; five captioned ones don't.
fn dock_slot(ui: &mut egui::Ui, glyph: &str, label: &str, active: bool, width: f32) -> bool {
    let fg = if active {
        OMEGA_ORANGE
    } else {
        Color32::from_gray(205)
    };
    ui.allocate_ui_with_layout(vec2(width, 46.0), Layout::top_down(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let resp = ui.add(
            egui::Button::new(RichText::new(glyph).size(21.0).color(fg))
                .min_size(vec2(width, 26.0))
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::NONE),
        );
        ui.label(RichText::new(label).size(10.0).color(fg));
        resp.clicked()
    })
    .inner
}

/// A flat tool-dock icon (transparent, tinted when active) — RadarOmega's bottom toolbar.
fn dock_icon(ui: &mut egui::Ui, glyph: &str, active: bool) -> egui::Response {
    let fg = if active {
        OMEGA_ORANGE
    } else {
        Color32::from_gray(205)
    };
    ui.add(
        egui::Button::new(RichText::new(glyph).size(23.0).color(fg))
            .min_size(vec2(34.0, 38.0))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    )
}

/// A slim full-width strip for a gridded field layer, with its name and range inline.
fn paint_field_strip(painter: &egui::Painter, rect: Rect, layer: crate::render::FieldLayer) {
    use crate::render::field_ramps::{ramp_for, FieldScale};
    let Some(r) = ramp_for(layer) else { return };
    let col = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
    match r.scale {
        FieldScale::Ramp { lo, hi, stops, .. } => {
            let mut mesh = Mesh::default();
            for w in stops.windows(2) {
                let (t0, c0) = w[0];
                let (t1, c1) = w[1];
                let q = Rect::from_min_max(
                    pos2(rect.left() + t0 * rect.width(), rect.top()),
                    pos2(rect.left() + t1 * rect.width(), rect.bottom()),
                );
                let i = mesh.vertices.len() as u32;
                for (p, c) in [
                    (q.left_top(), col(c0)),
                    (q.right_top(), col(c1)),
                    (q.right_bottom(), col(c1)),
                    (q.left_bottom(), col(c0)),
                ] {
                    mesh.colored_vertex(p, c);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(Shape::mesh(mesh));
            let label = if r.units.is_empty() {
                format!("{}  {lo:.0}\u{2013}{hi:.0}", r.label)
            } else {
                format!("{}  {lo:.0}\u{2013}{hi:.0} {}", r.label, r.units)
            };
            painter.text(
                pos2(rect.left() + 4.0, rect.bottom() + 1.0),
                Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(9.0),
                Color32::from_gray(215),
            );
        }
        FieldScale::Categorical(cats) => {
            // Equal slices, one per class, with the layer name beneath.
            let w = rect.width() / cats.len() as f32;
            for (i, (_, rgb, _)) in cats.iter().enumerate() {
                let x = rect.left() + i as f32 * w;
                painter.rect_filled(
                    Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height())),
                    0.0,
                    col(*rgb),
                );
            }
            painter.text(
                pos2(rect.left() + 4.0, rect.bottom() + 1.0),
                Align2::LEFT_TOP,
                r.label,
                egui::FontId::proportional(9.0),
                Color32::from_gray(215),
            );
        }
    }
}

/// Paint the active product's color table as a full-width gradient strip (the top scale bar).
fn paint_colorbar(
    painter: &egui::Painter,
    rect: Rect,
    moment: Moment,
    table: &crate::colormap::ColorTable,
) {
    let (vmin, vmax) = moment.value_range();
    let span = (vmax - vmin).max(f32::EPSILON);
    let x_of = |v: f32| rect.left() + ((v - vmin) / span).clamp(0.0, 1.0) * rect.width();
    let col = |c: [u8; 4]| Color32::from_rgb(c[0], c[1], c[2]);
    let mut mesh = Mesh::default();
    let mut quad = |x0: f32, x1: f32, c0: Color32, c1: Color32| {
        if x1 <= x0 {
            return;
        }
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x0, rect.top()), c0);
        mesh.colored_vertex(pos2(x1, rect.top()), c1);
        mesh.colored_vertex(pos2(x1, rect.bottom()), c1);
        mesh.colored_vertex(pos2(x0, rect.bottom()), c0);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    for (i, s) in table.stops.iter().enumerate() {
        let x0 = x_of(s.value);
        match table.stops.get(i + 1) {
            Some(n) => {
                let x1 = x_of(n.value);
                if s.solid {
                    quad(x0, x1, col(s.rgba), col(s.rgba));
                } else {
                    quad(x0, x1, col(s.rgba), col(s.end.unwrap_or(n.rgba)));
                }
            }
            None => quad(
                x0,
                rect.right(),
                col(s.end.unwrap_or(s.rgba)),
                col(s.end.unwrap_or(s.rgba)),
            ),
        }
    }
    painter.add(Shape::mesh(mesh));
}

impl super::HookEchoApp {
    /// Render the whole Android chrome (color scale + floating bars + drawers/popups) and return
    /// the UI actions the shared code processes.
    /// What Android's back button dismisses, innermost first. Returns without doing anything when
    /// nothing is open, which lets the OS handle it (leave the app).
    fn mobile_back(&mut self) {
        if self.mobile_sheet != MobileSheet::None {
            self.mobile_sheet = MobileSheet::None;
        } else if self.marker_popup.is_some() {
            self.marker_popup = None;
        } else if self.site_dialog.is_some() {
            self.site_dialog = None;
        } else if self.cells_window.open {
            self.cells_window.open = false;
        } else if self.forecast_open {
            self.forecast_open = false;
        } else if self.settings_window.open {
            self.settings_window.open = false;
        } else if self.mobile_chrome_hidden {
            self.mobile_chrome_hidden = false;
        }
    }

    pub(crate) fn mobile_chrome(
        &mut self,
        _root: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> UiActions {
        let mut actions = UiActions::default();
        // Android's back button arrives as `BrowserBack`. Without this every sheet and drawer was
        // a one-way door — back did nothing and the only exit was the ✕, which a sheet's own
        // content covers on a small screen.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::BrowserBack)) {
            self.mobile_back();
        }
        let active = self.active;
        let content = ctx.content_rect();
        let vr = ctx.viewport_rect();
        let inset_top = (content.top() - vr.top()).max(0.0);
        let inset_bottom = (vr.bottom() - content.bottom()).max(0.0);

        // Hide/show all chrome (view the whole radar). This button is always drawn; when hidden it
        // is the only floating control, so the map is fully visible.
        egui::Area::new(Id::new("m_chrome_toggle"))
            .anchor(Align2::RIGHT_TOP, vec2(-10.0, inset_top + 66.0))
            .order(egui::Order::Foreground) // above the drawer scrim, so it stays tappable + undimmed
            .show(ctx, |ui| {
                let g = if self.mobile_chrome_hidden {
                    ph::EYE
                } else {
                    ph::EYE_SLASH
                };
                if square_btn(ui, g, self.mobile_chrome_hidden, OMEGA_ORANGE).clicked() {
                    self.mobile_chrome_hidden = !self.mobile_chrome_hidden;
                    self.mobile_sheet = MobileSheet::None;
                }
            });
        if self.mobile_chrome_hidden {
            return actions;
        }

        // Owned alert rows (decoupled from `&self`) so the Alerts drawer can list + fly to them.
        let bounds = self.view_bounds();
        let malerts: Vec<MAlert> = {
            let feats = self.active_alert_features();
            crate::ui::alert_panel::rows_in_view(feats, bounds)
                .into_iter()
                .map(|r| MAlert {
                    id: r.info.id.clone(),
                    title: r.info.event.clone(),
                    sub: r.info.area.clone(),
                    color: Color32::from_rgb(r.color[0], r.color[1], r.color[2]),
                    lon: r.center.0,
                    lat: r.center.1,
                    esc: r.esc,
                })
                .collect()
        };
        let alert_count = malerts.len();
        let max_esc = malerts.iter().map(|a| a.esc).max().unwrap_or(0);

        // Read-only copies used by closures that also write back (avoids borrow conflicts).
        let site = self.views[active]
            .site
            .clone()
            .unwrap_or_else(|| "—".into());
        let cur_moment = self.views[active].moment;
        let srv = self.views[active].srv;
        let (n_tilt, cur_tilt, cur_angle): (usize, usize, f32) = {
            let v = &self.views[active];
            match &v.volume {
                Some(vol) => {
                    let n = vol.elevations.len();
                    let t = v.tilt.min(n.saturating_sub(1));
                    (n, t, vol.elevations.get(t).copied().unwrap_or(0.0))
                }
                None => (0, 0, 0.0),
            }
        };
        let tz = self.settings.tz_for(self.views[active].site.as_deref());
        let (vcp_line, when, following) = {
            let v = &self.views[active];
            let following = v.timeline.following;
            match &v.volume {
                Some(vol) => {
                    // "VCP 35 (Clear air, SZ-2)" -> "VCP 35: Clear air, SZ-2".
                    let vcp = vol
                        .vcp
                        .replacen(" (", ": ", 1)
                        .trim_end_matches(')')
                        .to_string();
                    let when = crate::timefmt::fmt_date_clock(vol.time, tz);
                    (vcp, when, following)
                }
                None => ("no volume".to_string(), String::new(), following),
            }
        };
        let nframes = self.views[active].timeline.frames.len();
        let playing = self.views[active].timeline.playing;

        // ---------- FULL-WIDTH COLOR SCALE (top edge, under the status bar) ----------
        if self.views[active].volume.is_some() {
            let table = self.palettes.table(cur_moment);
            let strip = Rect::from_min_size(
                pos2(content.left(), content.top()),
                vec2(content.width(), 9.0),
            );
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                Id::new("m_colorbar"),
            ));
            paint_colorbar(&painter, strip, cur_moment, table);

            // Second strip for the topmost gridded layer — the phone has no room for the desktop
            // legend box, but an unlabeled MESH/QPE wash is just as cryptic here.
            if let Some(top) = crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .rev()
                .find(|l| self.fields.get(l).is_some_and(|s| s.show))
            {
                paint_field_strip(&painter, strip.translate(vec2(0.0, 11.0)), *top);
            }
        }

        // ---------- TOP BAR ----------
        egui::Area::new(Id::new("m_topbar"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, inset_top + 14.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // Hamburger (its own rounded-square fill; no extra glass wrapper).
                    let on = self.mobile_sheet == MobileSheet::Menu;
                    if square_btn(ui, ph::LIST, on, OMEGA_ORANGE).clicked() {
                        self.mobile_sheet = if on {
                            MobileSheet::None
                        } else {
                            MobileSheet::Menu
                        };
                    }
                    // Center pill: VCP line + search/site line + 3D button. Its inner content width
                    // is the screen minus the two 44px squares, the 8px gaps, and the pill's own
                    // 12px×2 frame margins — so the row never overflows the edges.
                    let pill_inner = (content.width() - 44.0 - 44.0 - 32.0 - 24.0).max(110.0);
                    glass(ui, 238).show(ui, |ui| {
                        ui.set_width(pill_inner);
                        ui.horizontal(|ui| {
                            let text_w = (pill_inner - 52.0).max(60.0);
                            ui.vertical(|ui| {
                                ui.set_width(text_w);
                                ui.label(
                                    RichText::new(&vcp_line)
                                        .size(11.0)
                                        .color(Color32::from_gray(160)),
                                );
                                let site_line = ui
                                    .horizontal(|ui| {
                                        ui.label(
                                            RichText::new(ph::MAGNIFYING_GLASS)
                                                .size(13.0)
                                                .color(Color32::from_gray(180)),
                                        );
                                        ui.label(
                                            RichText::new(&site)
                                                .size(16.0)
                                                .strong()
                                                .color(Color32::from_gray(240)),
                                        )
                                    })
                                    .inner;
                                if site_line.interact(Sense::click()).clicked()
                                    && self.site_dialog.is_none()
                                {
                                    self.site_dialog = Some(Default::default());
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let b = egui::Button::new(
                                    RichText::new("3D")
                                        .size(14.0)
                                        .strong()
                                        .color(Color32::BLACK),
                                )
                                .fill(OMEGA_ORANGE)
                                .corner_radius(9.0)
                                .min_size(vec2(40.0, 34.0));
                                if ui.add(b).clicked() {
                                    self.build_volume3d();
                                }
                            });
                        });
                    });
                    // Alert badge (bare rounded square).
                    let (bg, fg) = if alert_count == 0 {
                        (
                            Color32::from_rgba_unmultiplied(255, 255, 255, 22),
                            Color32::from_gray(180),
                        )
                    } else if max_esc >= 2 {
                        (Color32::from_rgb(220, 40, 40), Color32::WHITE)
                    } else {
                        (OMEGA_ORANGE, Color32::BLACK)
                    };
                    let label = if alert_count == 0 {
                        ph::BELL.to_string()
                    } else {
                        alert_count.to_string()
                    };
                    let on = self.mobile_sheet == MobileSheet::Alerts;
                    let badge =
                        egui::Button::new(RichText::new(label).size(17.0).strong().color(fg))
                            .fill(bg)
                            .corner_radius(13.0)
                            .min_size(vec2(44.0, 44.0));
                    if ui.add(badge).clicked() {
                        self.mobile_sheet = if on {
                            MobileSheet::None
                        } else {
                            MobileSheet::Alerts
                        };
                    }
                });
            });

        // ---------- BOTTOM INFO CARD + TOOL DOCK ----------
        egui::Area::new(Id::new("m_bottom"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -(inset_bottom + 8.0)))
            .show(ctx, |ui| {
                let cwi = (content.width() - 56.0).max(200.0); // card inner content width (leaves side margins)
                glass(ui, 246).show(ui, |ui| {
                    ui.set_width(cwi);
                    // Row A: frames + history · product name.
                    ui.horizontal(|ui| {
                        ui.set_width(cwi);
                        let frames = egui::Button::new(
                            RichText::new(format!("{} Frames", nframes.max(1)))
                                .size(14.0)
                                .strong()
                                .color(OMEGA_ORANGE),
                        )
                        .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 14))
                        .corner_radius(10.0)
                        .min_size(vec2(0.0, 34.0));
                        if ui.add(frames).clicked() {
                            self.views[active].timeline.toggle_play();
                        }
                        if dock_icon(ui, ph::CLOCK_COUNTER_CLOCKWISE, playing).clicked() {
                            self.views[active].timeline.toggle_play();
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let prod = egui::Button::new(
                                RichText::new(crate::products::name(cur_moment, srv))
                                    .size(14.0)
                                    .strong()
                                    .color(OMEGA_ORANGE),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE);
                            if ui.add(prod).clicked() {
                                self.mobile_sheet = if self.mobile_sheet == MobileSheet::Products {
                                    MobileSheet::None
                                } else {
                                    MobileSheet::Products
                                };
                            }
                        });
                    });
                    // Row B: live dot + timestamp · elevation.
                    ui.horizontal(|ui| {
                        ui.set_width(cwi);
                        let dot = if following {
                            OMEGA_GREEN
                        } else {
                            Color32::from_gray(150)
                        };
                        let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().circle_filled(rect.center(), 5.0, dot);
                        ui.label(
                            RichText::new(&when)
                                .size(13.0)
                                .color(Color32::from_gray(210)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if n_tilt > 0 {
                                ui.label(
                                    RichText::new(format!("{cur_angle:.1}°"))
                                        .size(13.0)
                                        .strong()
                                        .color(OMEGA_ORANGE),
                                );
                                ui.label(
                                    RichText::new("Elevation: ")
                                        .size(13.0)
                                        .color(Color32::from_gray(190)),
                                );
                            }
                        });
                    });
                    ui.add_space(3.0);
                    // Row C: the dock. Five labeled slots; everything else is one tap into More.
                    ui.horizontal(|ui| {
                        ui.set_width(cwi);
                        // Zero the inter-slot spacing so five even slots fill the row exactly.
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let iw = cwi / 5.0;
                        let sheet = self.mobile_sheet;
                        if dock_slot(
                            ui,
                            if playing { ph::PAUSE } else { ph::PLAY },
                            if playing { "Pause" } else { "Play" },
                            playing,
                            iw,
                        ) {
                            self.views[active].timeline.toggle_play();
                        }
                        // Layers and More are the same destination: the Menu sheet leads with the
                        // layer registry and keeps everything else one scroll below it. Two slots,
                        // one surface — a phone doesn't have room for two lists of the same thing.
                        // ponytail: fold the two slots into one if the dock ever needs the width.
                        if dock_slot(ui, ph::STACK, "Layers", sheet == MobileSheet::Menu, iw) {
                            self.mobile_sheet = if sheet == MobileSheet::Menu {
                                MobileSheet::None
                            } else {
                                MobileSheet::Menu
                            };
                        }
                        if dock_slot(
                            ui,
                            ph::DROP_HALF,
                            "Products",
                            sheet == MobileSheet::Products,
                            iw,
                        ) {
                            self.mobile_sheet = if sheet == MobileSheet::Products {
                                MobileSheet::None
                            } else {
                                MobileSheet::Products
                            };
                        }
                        if dock_slot(ui, ph::RADIO_BUTTON, "Site", self.site_dialog.is_some(), iw)
                            && self.site_dialog.is_none()
                        {
                            self.site_dialog = Some(Default::default());
                        }
                        if dock_slot(ui, ph::DOTS_THREE, "More", sheet == MobileSheet::Menu, iw) {
                            self.mobile_sheet = if sheet == MobileSheet::Menu {
                                MobileSheet::None
                            } else {
                                MobileSheet::Menu
                            };
                        }
                    });
                });
            });

        // ---------- ARMED TOOL HINT ----------
        // The desktop status bar (which tells you a tap-tool is armed and needs 2 points) is hidden
        // on mobile, so measure/cross-section read as "broken". Float the guidance above the card.
        let hint = match self.tool {
            super::MapTool::Measure => Some("Tap two points to measure"),
            super::MapTool::Marker => Some("Tap the map to drop a marker"),
            super::MapTool::CrossSection => Some("Tap two points for a cross-section"),
            super::MapTool::Sounding => Some("Tap a point for a sounding"),
            super::MapTool::Climatology => Some("Tap a point for tornado climatology"),
            _ => None,
        };
        if let Some(text) = hint {
            egui::Area::new(Id::new("m_toolhint"))
                .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -(inset_bottom + 172.0)))
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(242, 160, 51, 236))
                        .corner_radius(14.0)
                        .inner_margin(Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(text)
                                    .size(14.0)
                                    .strong()
                                    .color(Color32::BLACK),
                            );
                        });
                });
        }

        // ---------- POPUPS / DRAWERS ----------
        match self.mobile_sheet {
            MobileSheet::Menu => self.mobile_menu_drawer(ctx, content, vr, &mut actions),
            MobileSheet::Alerts => {
                self.mobile_alerts_drawer(ctx, content, vr, &malerts, alert_count)
            }
            MobileSheet::Products => self.mobile_products(
                ctx, content, vr, cur_moment, srv, n_tilt, cur_tilt, cur_angle,
            ),
            MobileSheet::Capture => self.mobile_capture(ctx, content, vr),
            MobileSheet::None => {}
        }

        actions
    }

    /// A dimming scrim behind a popup; a tap outside `keep` closes the sheet.
    fn mobile_scrim(&mut self, ctx: &egui::Context, vr: Rect, keep: Rect) {
        egui::Area::new(Id::new("m_scrim"))
            .fixed_pos(vr.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let r = ui.allocate_response(vr.size(), Sense::click());
                ui.painter().rect_filled(
                    r.rect,
                    egui::CornerRadius::ZERO,
                    Color32::from_black_alpha(150),
                );
                if r.clicked() && !r.interact_pointer_pos().is_some_and(|p| keep.contains(p)) {
                    self.mobile_sheet = MobileSheet::None;
                }
            });
    }

    /// Left navigation drawer (RadarOmega-style navy panel), populated with our toolbox.
    fn mobile_menu_drawer(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        vr: Rect,
        actions: &mut UiActions,
    ) {
        let dw = (content.width() * 0.88).min(440.0);
        let drawer_rect = Rect::from_min_size(
            pos2(content.left(), content.top()),
            vec2(dw, content.height()),
        );
        self.mobile_scrim(ctx, vr, drawer_rect);
        let accent = crate::theme::accent(self.settings.theme);
        // Set inside the drawer body, acted on after it closes its borrow of `self`.
        let mut open_capture = false;
        egui::Area::new(Id::new("m_drawer"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(vr.left(), vr.top()))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(DRAWER_BG)
                    .stroke(Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 18),
                    ))
                    .inner_margin(Margin {
                        left: 14,
                        right: 14,
                        top: content.top() as i8 + 6,
                        bottom: 10,
                    })
                    .show(ui, |ui| {
                        ui.set_width(dw - 28.0);
                        ui.set_height(content.bottom() - content.top() - 12.0);
                        // Header.
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(ph::CROSSHAIR_SIMPLE)
                                    .size(22.0)
                                    .color(OMEGA_ORANGE),
                            );
                            ui.label(
                                RichText::new("Hook Echo")
                                    .size(20.0)
                                    .strong()
                                    .color(OMEGA_TITLE),
                            );
                            ui.label(
                                RichText::new("WX")
                                    .size(20.0)
                                    .strong()
                                    .color(Color32::from_gray(220)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if square_btn(ui, ph::X, false, accent).clicked() {
                                    self.mobile_sheet = MobileSheet::None;
                                }
                            });
                        });
                        ui.add_space(6.0);
                        // Pane-mode selector (RadarOmega "Single Site").
                        let modes = [(1usize, "Single Site"), (2, "Dual Pane"), (4, "Quad Pane")];
                        let cur = self.views.len();
                        let cur_label = modes
                            .iter()
                            .find(|(n, _)| *n == cur)
                            .map(|(_, l)| *l)
                            .unwrap_or("Single Site");
                        egui::ComboBox::from_id_salt("m_panemode")
                            .selected_text(RichText::new(cur_label).strong())
                            .width(dw - 40.0)
                            .show_ui(ui, |ui| {
                                for (n, label) in modes {
                                    if ui.selectable_label(cur == n, label).clicked() {
                                        self.set_pane_count(n);
                                    }
                                }
                            });
                        ui.add_space(8.0);
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // The drawer used to embed the entire desktop toolbox — twelve dense
                            // sections of desktop-density widgets on a phone. It now leads with the
                            // same categorized, described registry the Layers sheet uses, and keeps
                            // every expert control one tap further in under "Advanced".
                            let entries = self.palette_entries();
                            let mut query = std::mem::take(&mut self.mobile_drawer_query);
                            let chosen = crate::ui::layers_panel::body(
                                ui,
                                &entries,
                                &mut query,
                                accent,
                                content.height() * 0.5,
                                false,
                            );
                            // Clear a search once it's been used, exactly as the desktop drawer
                            // does. Leaving it set meant the next open showed a stale query — and
                            // since the soft keyboard appends, the search after that was garbage.
                            let searched = !query.trim().is_empty();
                            self.mobile_drawer_query = query;
                            if let Some(action) = chosen {
                                self.apply_palette(action, ctx);
                                self.mobile_sheet = MobileSheet::None;
                                if searched {
                                    self.mobile_drawer_query.clear();
                                }
                            }
                            ui.add_space(8.0);
                            ui.separator();
                            egui::CollapsingHeader::new(
                                RichText::new("Advanced").size(14.0).strong().color(accent),
                            )
                            .default_open(false)
                            .show(ui, |ui| {
                                let l3_site = self.l3grid_site.clone();
                                let tz = self.active_tz();
                                let mosaic = self.mosaic_status();
                                crate::ui::layer_options::show(
                                    ui,
                                    &mut self.filters,
                                    &mut self.fields,
                                    &mut self.rotation_minutes,
                                    &mut self.hrrr_fcst_hour,
                                    self.hrrr_valid,
                                    tz,
                                    &mut self.env_cape_ml,
                                    &mut self.env_srh_km,
                                    &mut self.env_model,
                                    &mut self.contour_kind,
                                    &mut self.settings.etop_dbz,
                                    l3_site.as_deref(),
                                    Some(mosaic.as_str()),
                                    actions,
                                );
                                ui.separator();
                                self.map_rows(ui, actions);
                            });
                            ui.add_space(8.0);
                            ui.separator();
                            if ui.button("Capture…").clicked() {
                                open_capture = true;
                            }
                            // The same App rows the desktop drawer shows — one list, not a phone
                            // copy of it that drifts.
                            self.app_rows(ui);
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("© Hook Echo-WX 2026")
                                    .size(12.0)
                                    .color(Color32::from_gray(110)),
                            );
                        });
                    });
            });
        if open_capture {
            self.mobile_sheet = MobileSheet::Capture;
        }
    }

    /// Right alerts drawer, RadarOmega "Active Weather Alerts" styling.
    fn mobile_alerts_drawer(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        vr: Rect,
        malerts: &[MAlert],
        alert_count: usize,
    ) {
        let dw = (content.width() * 0.9).min(460.0);
        let drawer_rect = Rect::from_min_size(
            pos2(content.right() - dw, content.top()),
            vec2(dw, content.height()),
        );
        self.mobile_scrim(ctx, vr, drawer_rect);
        let active = self.active;
        egui::Area::new(Id::new("m_alerts"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(content.right() - dw, content.top()))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(10, 12, 16))
                    .corner_radius(16.0)
                    .stroke(Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 18),
                    ))
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(dw - 28.0);
                        ui.set_height(content.height() - 20.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ph::WARNING).size(22.0).color(OMEGA_ORANGE));
                            ui.label(
                                RichText::new("Active Weather Alerts")
                                    .size(18.0)
                                    .strong()
                                    .color(OMEGA_ORANGE),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if square_btn(ui, ph::X, false, OMEGA_ORANGE).clicked() {
                                    self.mobile_sheet = MobileSheet::None;
                                }
                            });
                        });
                        ui.horizontal(|ui| {
                            let pill = egui::Frame::new()
                                .fill(Color32::from_rgba_unmultiplied(242, 160, 51, 40))
                                .corner_radius(10.0)
                                .inner_margin(Margin::symmetric(10, 3));
                            pill.show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("{alert_count}"))
                                        .strong()
                                        .color(OMEGA_ORANGE),
                                );
                                ui.label(RichText::new(" active").color(Color32::from_gray(200)));
                            });
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                            ui.painter().circle_filled(
                                rect.center(),
                                5.0,
                                Color32::from_rgb(220, 60, 60),
                            );
                            ui.label(
                                RichText::new("LIVE")
                                    .size(13.0)
                                    .strong()
                                    .color(Color32::from_rgb(220, 60, 60)),
                            );
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if malerts.is_empty() {
                                ui.add_space(10.0);
                                ui.weak("No active alerts in view.");
                            }
                            for a in malerts {
                                let col = if a.esc >= 2 {
                                    Color32::from_rgb(255, 90, 90)
                                } else {
                                    a.color
                                };
                                let row = egui::Frame::new()
                                    .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                                    .corner_radius(12.0)
                                    .inner_margin(Margin::symmetric(12, 10))
                                    .stroke(Stroke::new(3.0, col));
                                let resp = row
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new(&a.title).size(15.0).strong().color(col),
                                        );
                                        ui.label(
                                            RichText::new(&a.sub)
                                                .size(12.0)
                                                .color(Color32::from_gray(170)),
                                        );
                                    })
                                    .response;
                                ui.add_space(7.0);
                                if resp.interact(Sense::click()).clicked() {
                                    let cam = &mut self.views[active].camera;
                                    cam.center =
                                        crate::render::mercator::lonlat_to_world(a.lon, a.lat);
                                    cam.zoom = cam.zoom.max(8.0);
                                    self.open_alert_popup(&a.id);
                                    self.mobile_sheet = MobileSheet::None;
                                }
                            }
                        });
                    });
            });
    }

    /// Categorized product picker (RadarOmega "Reflectivity / Velocity / Dual-Polarization").
    #[allow(clippy::too_many_arguments)]
    fn mobile_products(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        vr: Rect,
        cur_moment: Moment,
        srv: bool,
        n_tilt: usize,
        cur_tilt: usize,
        cur_angle: f32,
    ) {
        let pw = (content.width() - 20.0).min(560.0);
        let panel = Rect::from_min_size(
            pos2(content.center().x - pw / 2.0, content.top() + 70.0),
            vec2(pw, content.height() * 0.6),
        );
        self.mobile_scrim(ctx, vr, panel);
        let active = self.active;
        egui::Area::new(Id::new("m_products"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel.min)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(10, 12, 16))
                    .corner_radius(16.0)
                    .stroke(Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 18),
                    ))
                    .inner_margin(Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(pw - 32.0);
                        // Grouped by category, but the names come from `products::PRODUCTS` so
                        // this sheet says the same thing as the Layers list and the bottom card.
                        type Row = (Moment, bool, &'static str, &'static str);
                        let p = |m: Moment| -> Row {
                            let i = crate::products::info(m);
                            (m, false, i.name, i.blurb)
                        };
                        let groups: [(&str, Vec<Row>); 3] = [
                            ("Reflectivity", vec![p(Moment::Reflectivity)]),
                            (
                                "Velocity",
                                vec![
                                    p(Moment::Velocity),
                                    (
                                        Moment::Velocity,
                                        true,
                                        "Storm-Relative Velocity",
                                        "Velocity with the storm's own motion subtracted out",
                                    ),
                                    p(Moment::SpectrumWidth),
                                ],
                            ),
                            (
                                "Dual-Polarization",
                                vec![
                                    p(Moment::CorrelationCoefficient),
                                    p(Moment::DifferentialReflectivity),
                                    p(Moment::DifferentialPhase),
                                ],
                            ),
                        ];
                        egui::ScrollArea::vertical()
                            .max_height(content.height() * 0.62)
                            .show(ui, |ui| {
                                for (title, rows) in groups.iter() {
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(*title)
                                            .size(15.0)
                                            .strong()
                                            .color(Color32::from_gray(235)),
                                    );
                                    ui.add_space(2.0);
                                    for (m, want_srv, label, blurb) in rows.iter().copied() {
                                        let selected = cur_moment == m
                                            && (m != Moment::Velocity || srv == want_srv);
                                        ui.horizontal(|ui| {
                                            let fg = if selected {
                                                OMEGA_ORANGE
                                            } else {
                                                Color32::from_gray(220)
                                            };
                                            // Inside a horizontal layout `available_width` is
                                            // effectively unbounded, so the blurb won't wrap
                                            // unless the row is boxed to a real width first.
                                            let row_w = (ui.available_width() - 100.0).max(120.0);
                                            let clicked = ui
                                                .allocate_ui(vec2(row_w, 0.0), |ui| {
                                                    ui.set_max_width(row_w);
                                                    let b = egui::Button::new(
                                                        RichText::new(format!("{label}\n{blurb}"))
                                                            .size(15.0)
                                                            .color(fg),
                                                    )
                                                    .fill(Color32::TRANSPARENT)
                                                    .stroke(Stroke::NONE)
                                                    // egui defaults to Extend inside a horizontal
                                                    // layout, which ran the blurb off the panel.
                                                    .wrap_mode(egui::TextWrapMode::Wrap)
                                                    .min_size(vec2(row_w, 44.0));
                                                    ui.add(b).clicked()
                                                })
                                                .inner;
                                            if clicked {
                                                self.views[active].moment = m;
                                                if m == Moment::Velocity {
                                                    self.views[active].srv = want_srv;
                                                }
                                                self.mobile_sheet = MobileSheet::None;
                                            }
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    if selected && n_tilt > 0 {
                                                        let tb = egui::Button::new(
                                                            RichText::new(format!(
                                                                "Tilt {}  {:.1}°",
                                                                cur_tilt + 1,
                                                                cur_angle
                                                            ))
                                                            .size(13.0)
                                                            .color(OMEGA_BLUE),
                                                        )
                                                        .fill(Color32::from_rgba_unmultiplied(
                                                            45, 156, 219, 30,
                                                        ))
                                                        .corner_radius(8.0);
                                                        if ui.add(tb).clicked() {
                                                            self.views[active].tilt =
                                                                (cur_tilt + 1) % n_tilt.max(1);
                                                        }
                                                    }
                                                },
                                            );
                                        });
                                    }
                                    ui.add_space(4.0);
                                    ui.separator();
                                }
                            });
                    });
            });
    }

    /// Capture menu (Screenshot / Record GIF / Record Video) — RadarOmega's blue tri-icon sheet.
    fn mobile_capture(&mut self, ctx: &egui::Context, content: Rect, vr: Rect) {
        use super::ShotDest;
        let pw = (content.width() - 24.0).min(520.0);
        let panel = Rect::from_min_size(
            pos2(content.center().x - pw / 2.0, content.center().y - 60.0),
            vec2(pw, 130.0),
        );
        self.mobile_scrim(ctx, vr, panel);
        egui::Area::new(Id::new("m_capture"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel.min)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgb(24, 30, 42))
                    .corner_radius(16.0)
                    .inner_margin(Margin::symmetric(14, 14))
                    .show(ui, |ui| {
                        ui.set_width(pw - 28.0);
                        ui.columns(3, |cols| {
                            let cap = |ui: &mut egui::Ui, glyph: &str, label: &str| -> bool {
                                ui.vertical_centered(|ui| {
                                    let clicked = ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(glyph).size(34.0).color(OMEGA_BLUE),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .stroke(Stroke::NONE),
                                        )
                                        .clicked();
                                    ui.label(
                                        RichText::new(label).size(13.0).strong().color(OMEGA_BLUE),
                                    );
                                    clicked
                                })
                                .inner
                            };
                            if cap(&mut cols[0], ph::CAMERA, "Screenshot") {
                                if let Some(path) = crate::dialog::save_path("hookecho.png", "png")
                                {
                                    self.screenshot_pending = Some(ShotDest::File(path));
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                                        egui::UserData::default(),
                                    ));
                                }
                                self.mobile_sheet = MobileSheet::None;
                            }
                            if cap(&mut cols[1], ph::VIDEO_CAMERA, "Record Video")
                                && self.loop_export.is_none()
                            {
                                self.start_loop_export(crate::loopexport::LoopFormat::Mp4);
                                self.mobile_sheet = MobileSheet::None;
                            }
                            if cap(&mut cols[2], ph::GIF, "Record Gif")
                                && self.loop_export.is_none()
                            {
                                self.start_loop_export(crate::loopexport::LoopFormat::Gif);
                                self.mobile_sheet = MobileSheet::None;
                            }
                        });
                    });
            });
    }
}

