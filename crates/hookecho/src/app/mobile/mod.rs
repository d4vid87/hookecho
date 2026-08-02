//! Touch-first Android chrome, Material 3 map-first: a full-bleed map under a full-width color
//! scale, a slim top chip row, and one persistent bottom sheet — draggable between three snap
//! points — that carries the radar context and docks the five-action toolbar along its bottom
//! edge. Everything else opens over the map from there. Every control drives the shared desktop
//! data paths.

pub(crate) mod sheet;
mod toolbar;

use sheet::{SheetInfo, SheetSnap};

use egui::{pos2, vec2, Align2, Color32, Id, Margin, Mesh, Rect, RichText, Sense, Shape, Stroke};
use egui_phosphor::regular as ph;
use wxdata::level2::Moment;

use crate::ui::layer_options::UiActions;

pub(crate) use crate::ui::style::{glass, square_btn, OMEGA_GREEN, OMEGA_ORANGE};

/// Which modal sheet is open over the map (`None` = just the persistent sheet and the chips).
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
    /// Whether back would dismiss something in-app rather than leaving the app.
    fn mobile_has_dismissable(&self) -> bool {
        self.marker_popup.is_some()
            || self.detail.is_some()
            || self.warning_popup.is_some()
            || self.cell_popup.is_some()
            || self.xsection.is_some()
            || self.site_dialog.is_some()
            || self.sounding_window.open
            || self.show_hodo
            || self.show_cappi
            || self.show_3d
            || self.show_sensors
            || self.climo_open
            || self.afd_open
            || self.digest_window.open
            || self.verify_window.open
            || self.event_window.open
            || self.marker_window.open
            || self.placefile_window.open
            || self.palette_editor.open
            || self.layer_window_open
            || self.show_cheatsheet
            || self.about_open
            || self.wizard.open
            || self.cells_window.open
            || self.forecast_open
            || self.settings_window.open
            || self.mobile_sheet != MobileSheet::None
            || self.mobile_snap.collapsed().is_some()
            || self.mobile_chrome_hidden
    }

    fn mobile_back(&mut self) {
        // Full-screen surfaces first, then the modal sheet, then the persistent sheet's snaps,
        // and only then nothing (which lets Android leave the app). Every surface the phone can
        // open is in here; one that isn't is a one-way door, because a full-screen surface hides
        // the map you'd otherwise tap to get out.
        macro_rules! close {
            ($($flag:expr),* $(,)?) => {
                $(if $flag { $flag = false; return; })*
            };
        }
        macro_rules! clear {
            ($($opt:expr),* $(,)?) => {
                $(if $opt.is_some() { $opt = None; return; })*
            };
        }
        clear!(
            self.marker_popup,
            self.detail,
            self.warning_popup,
            self.cell_popup,
            self.xsection,
            self.site_dialog,
        );
        close!(
            self.sounding_window.open,
            self.show_hodo,
            self.show_cappi,
            self.show_3d,
            self.show_sensors,
            self.climo_open,
            self.afd_open,
            self.digest_window.open,
            self.verify_window.open,
            self.event_window.open,
            self.marker_window.open,
            self.placefile_window.open,
            self.palette_editor.open,
            self.layer_window_open,
            self.show_cheatsheet,
            self.about_open,
            self.wizard.open,
            self.cells_window.open,
            self.forecast_open,
            self.settings_window.open,
        );
        if self.mobile_sheet != MobileSheet::None {
            self.mobile_sheet = MobileSheet::None;
        } else if let Some(next) = self.mobile_snap.collapsed() {
            // The persistent sheet collapses a step at a time before back leaves the app.
            self.mobile_snap = next;
        } else if self.mobile_chrome_hidden {
            self.mobile_chrome_hidden = false;
        }
    }

    pub(crate) fn mobile_chrome(&mut self, _root: &mut egui::Ui, ctx: &egui::Context) -> UiActions {
        let mut actions = UiActions::default();
        // Back arrives two ways: as a `BrowserBack` key event (the legacy path, which Android 16
        // stops delivering) and from the predictive-back callback in MainActivity. Either one
        // runs the same dismissal chain.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::BrowserBack))
            || crate::platform::take_back_pressed()
        {
            self.mobile_back();
        }
        // Tell Android whether we would consume the next gesture. With nothing open the callback
        // is disabled and the OS gets to draw its live home-screen preview as the user drags.
        crate::platform::set_back_consumed(self.mobile_has_dismissable());
        let active = self.active;
        let content = ctx.content_rect();
        let vr = ctx.viewport_rect();
        let inset_top = (content.top() - vr.top()).max(0.0);

        // Hide/show all chrome (view the whole radar). This button is always drawn; when hidden it
        // is the only floating control, so the map is fully visible. It rides the top chip row —
        // parked lower it collided with the sheet once the sheet could reach full height.
        egui::Area::new(Id::new("m_chrome_toggle"))
            .anchor(
                Align2::RIGHT_TOP,
                vec2(-crate::ui::m3::SP_3, inset_top + 26.0),
            )
            // Plain Middle order, so any surface opened afterwards (a full-screen window, a
            // modal sheet's scrim) covers it instead of leaving an eye floating over the
            // content — it was overlapping the site picker's buttons at Foreground.
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
        // Only the open Alerts sheet needs the owned rows; the dock badge just needs the count and
        // the top escalation, which cost nothing to compute. Building the full list every frame
        // meant several String clones per alert in view at 4-10 fps.
        let want_rows = self.mobile_sheet == MobileSheet::Alerts;
        let (malerts, alert_count, max_esc): (Vec<MAlert>, usize, u8) = {
            let feats = self.active_alert_features();
            let rows = crate::ui::alert_panel::rows_in_view(feats, bounds);
            let count = rows.len();
            let max_esc = rows.iter().map(|r| r.esc).max().unwrap_or(0);
            let owned = if want_rows {
                rows.into_iter()
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
            } else {
                Vec::new()
            };
            (owned, count, max_esc)
        };

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

        // ---------- MODAL SHEETS ----------
        let modal = match self.mobile_sheet {
            MobileSheet::Menu => Some(self.mobile_menu_sheet(ctx, content, &mut actions)),
            MobileSheet::Alerts => Some(self.mobile_alerts_sheet(ctx, content, &malerts)),
            MobileSheet::Capture => Some(self.mobile_capture(ctx, content)),
            MobileSheet::None => None,
        };
        if let Some(r) = modal {
            self.mobile_occlusion.push(r);
        }

        actions
    }

    /// The main menu, as a modal bottom sheet: the shared layer registry up top, the expert
    /// controls behind "Advanced", app rows below. Same content the desktop drawer shows — one
    /// list, not a phone copy of it that drifts.
    fn mobile_menu_sheet(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        actions: &mut UiActions,
    ) -> Rect {
        let accent = crate::theme::accent(self.settings.theme);
        let mut close = false;
        let mut open_capture = false;
        let rect = sheet::modal_sheet(ctx, content, "m_menu", "Layers & tools", &mut close, |ui| {
            // Pane mode: one row of chips instead of a combo box a thumb has to hit twice.
            let cur = self.views.len();
            ui.horizontal_wrapped(|ui| {
                for (n, label) in [(1usize, "Single"), (2, "Dual pane"), (4, "Quad pane")] {
                    if crate::ui::m3::chip(ui, label, cur == n).clicked() {
                        self.set_pane_count(n);
                    }
                }
            });
            ui.add_space(crate::ui::m3::SP_2);
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
            // Clear a search once it's been used, exactly as the desktop drawer does. Leaving it
            // set meant the next open showed a stale query.
            let searched = !query.trim().is_empty();
            self.mobile_drawer_query = query;
            if let Some(action) = chosen {
                self.apply_palette(action, ctx);
                self.mobile_sheet = MobileSheet::None;
                if searched {
                    self.mobile_drawer_query.clear();
                }
            }
            ui.add_space(crate::ui::m3::SP_2);
            ui.separator();
            egui::CollapsingHeader::new(
                RichText::new("Advanced")
                    .size(crate::ui::m3::T_TITLE_SM)
                    .strong()
                    .color(accent),
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
            ui.add_space(crate::ui::m3::SP_2);
            ui.separator();
            if ui.button("Capture\u{2026}").clicked() {
                open_capture = true;
            }
            self.app_rows(ui);
            ui.add_space(crate::ui::m3::SP_3);
            ui.label(
                RichText::new("\u{a9} Hook Echo-WX 2026")
                    .size(crate::ui::m3::T_LABEL)
                    .color(Color32::from_gray(110)),
            );
        });
        if close {
            self.mobile_sheet = MobileSheet::None;
        }
        if open_capture {
            self.mobile_sheet = MobileSheet::Capture;
        }
        rect
    }

    /// Active alerts, as a modal bottom sheet. Tapping a row flies the map to it and opens the
    /// full alert text, exactly as the desktop panel does.
    fn mobile_alerts_sheet(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        malerts: &[MAlert],
    ) -> Rect {
        let mut close = false;
        // The body has its own flag: `close` is already borrowed by the sheet shell for its
        // handle/scrim/✕ dismissals.
        let mut picked = false;
        let active = self.active;
        let title = format!("Alerts in view ({})", malerts.len());
        let rect = sheet::modal_sheet(ctx, content, "m_alerts", &title, &mut close, |ui| {
            if malerts.is_empty() {
                ui.add_space(crate::ui::m3::SP_4);
                ui.weak("No active alerts in view.");
            }
            for a in malerts {
                let col = if a.esc >= 2 {
                    Color32::from_rgb(255, 90, 90)
                } else {
                    a.color
                };
                let resp = egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                    .corner_radius(crate::ui::m3::R_MD)
                    .inner_margin(Margin::symmetric(
                        crate::ui::m3::SP_3 as i8,
                        crate::ui::m3::SP_3 as i8,
                    ))
                    // The severity stripe is the row's only color coding, so it stays a real
                    // 3px edge rather than a tint that a dark theme swallows.
                    .stroke(Stroke::new(3.0, col))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(&a.title)
                                .size(crate::ui::m3::T_BODY_LG)
                                .strong()
                                .color(col),
                        );
                        ui.label(
                            RichText::new(&a.sub)
                                .size(crate::ui::m3::T_LABEL)
                                .color(ui.visuals().weak_text_color()),
                        );
                    })
                    .response;
                ui.add_space(crate::ui::m3::SP_2);
                if resp.interact(Sense::click()).clicked() {
                    let cam = &mut self.views[active].camera;
                    cam.center = crate::render::mercator::lonlat_to_world(a.lon, a.lat);
                    cam.zoom = cam.zoom.max(8.0);
                    self.open_alert_popup(&a.id);
                    picked = true;
                }
            }
        });
        if close || picked {
            self.mobile_sheet = MobileSheet::None;
        }
        rect
    }

    /// Capture: screenshot, video, GIF — as a modal sheet with three big targets.
    fn mobile_capture(&mut self, ctx: &egui::Context, content: Rect) -> Rect {
        use super::ShotDest;
        let mut close = false;
        let mut picked = false;
        let rect = sheet::modal_sheet(ctx, content, "m_capture", "Capture", &mut close, |ui| {
            if crate::ui::m3::list_row(
                ui,
                ph::CAMERA,
                "Screenshot",
                Some("Save the current view as a PNG"),
                false,
            )
            .clicked()
            {
                if let Some(path) = crate::dialog::save_path("hookecho.png", "png") {
                    self.screenshot_pending = Some(ShotDest::File(path));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                }
                picked = true;
            }
            if crate::ui::m3::list_row(
                ui,
                ph::VIDEO_CAMERA,
                "Record video",
                Some("Loop the current frames to MP4"),
                false,
            )
            .clicked()
                && self.loop_export.is_none()
            {
                self.start_loop_export(crate::loopexport::LoopFormat::Mp4);
                picked = true;
            }
            if crate::ui::m3::list_row(
                ui,
                ph::GIF,
                "Record GIF",
                Some("Loop the current frames to an animated GIF"),
                false,
            )
            .clicked()
                && self.loop_export.is_none()
            {
                self.start_loop_export(crate::loopexport::LoopFormat::Gif);
                picked = true;
            }
        });
        if close || picked {
            self.mobile_sheet = MobileSheet::None;
        }
        rect
    }
}
