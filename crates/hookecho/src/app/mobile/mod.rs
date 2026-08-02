//! Touch-first Android chrome, Material 3 map-first: a full-bleed map under a full-width color
//! scale, a slim top chip row, and one persistent bottom sheet — draggable between three snap
//! points — that carries the radar context and docks the five-action toolbar along its bottom
//! edge. Everything else opens over the map from there. Every control drives the shared desktop
//! data paths.

pub(crate) mod sheet;
mod toolbar;

use sheet::{SheetInfo, SheetSnap};

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
        } else if let Some(next) = self.mobile_snap.collapsed() {
            // The persistent sheet collapses a step at a time before back leaves the app.
            self.mobile_snap = next;
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

    pub(crate) fn mobile_chrome(&mut self, _root: &mut egui::Ui, ctx: &egui::Context) -> UiActions {
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

        // Hide/show all chrome (view the whole radar). This button is always drawn; when hidden it
        // is the only floating control, so the map is fully visible. It rides the top chip row —
        // parked lower it collided with the sheet once the sheet could reach full height.
        egui::Area::new(Id::new("m_chrome_toggle"))
            .anchor(Align2::RIGHT_TOP, vec2(-crate::ui::m3::SP_3, inset_top + 26.0))
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

        // ---------- TOP CHIPS ----------
        let chip_rect = self.mobile_top_chips(ctx, content, &site, &vcp_line);

        // ---------- PERSISTENT SHEET + DOCKED TOOLBAR ----------
        let info = SheetInfo {
            moment: cur_moment,
            srv,
            when,
            following,
            playing,
            nframes,
            playhead: self.views[active].timeline.playhead,
            n_tilt,
            cur_tilt,
            cur_angle,
            speed: self.views[active].timeline.speed,
            frame_labels: if self.mobile_snap == SheetSnap::Full {
                let tl = &self.views[active].timeline;
                tl.frames
                    .iter()
                    .map(|f| match f.date_time() {
                        Some(t) => crate::timefmt::fmt_date_clock(t, tz),
                        None => f.name().to_string(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        };
        let sheet_rect = self.mobile_sheet_surface(ctx, content, &info, alert_count, max_esc);

        // Two-finger gestures are read raw off the input state (see the pane input block), which
        // knows nothing about egui's layers — so without these rects a pinch on the sheet zoomed
        // the map underneath it.
        self.mobile_occlusion.push(sheet_rect);
        self.mobile_occlusion.push(chip_rect);

        // ---------- POPUPS / DRAWERS ----------
        match self.mobile_sheet {
            MobileSheet::Menu => self.mobile_menu_drawer(ctx, content, vr, &mut actions),
            MobileSheet::Alerts => {
                self.mobile_alerts_drawer(ctx, content, vr, &malerts, alert_count)
            }
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
                            let order_was = self.settings.layer_order.clone();
                            let chosen = crate::ui::layers_panel::body(
                                ui,
                                &entries,
                                &mut query,
                                accent,
                                content.height() * 0.5,
                                false,
                                &mut self.settings.layer_order,
                                // The phone keeps its knobs in the Advanced section below.
                                |_| {},
                            );
                            if self.settings.layer_order != order_was {
                                self.settings.save();
                            }
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
                                    &mut self.snow_hours,
                                    &self.show_tropical,
                                    &mut self.tropical_wind_kt,
                                    &mut self.tropical_surge,
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
