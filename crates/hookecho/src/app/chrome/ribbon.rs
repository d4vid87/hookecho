//! The WSV3 layout's docked chrome: a top ribbon of labelled control groups, the colour scale
//! docked under it, and a bottom status bar. Drawn only on desktop/web and only when
//! `Settings::layout` is `Wsv3`; Android keeps its own touch chrome and the minimal layout keeps
//! the floating pill + control column in `overlay.rs`.
//!
//! Every control here funnels into the same `PaletteAction` / `UiActions` path the minimal chrome
//! and the command palette use, so the two layouts can never drift apart.

use super::*;
use crate::ui::wsv3;
use egui::{vec2, Align, Color32, Layout, RichText};

/// One fixed-width ribbon group laid out top-down, followed by a hairline divider. Fixed width
/// because `horizontal_wrapped` inside a group otherwise claims the whole remaining ribbon width
/// as its own and strands every group after it far to the right.
fn ribbon_group(ui: &mut egui::Ui, w: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        vec2(w, wsv3::RIBBON_H - 6.0),
        Layout::top_down(Align::Min),
        add,
    );
    wsv3::vsep(ui);
}

impl HookEchoApp {
    /// The docked ribbon. Call on the eframe root `Ui`, before `chrome_rect` is captured, so the
    /// floating windows constrain to the map area below it.
    pub(crate) fn wsv3_ribbon(&mut self, root: &mut egui::Ui, ctx: &egui::Context) {
        let accent = wsv3::WSV3_BLUE;
        let mut actions = ui::layer_options::UiActions::default();

        // --- reads, before the panel borrows `self` in a closure ---
        let (moment, srv, tilt) = {
            let v = &self.views[self.active];
            (v.moment, v.srv, v.tilt)
        };
        let site = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "Pick site".to_string());
        let city_state = wxdata::sites::site_by_id(&site)
            .map(|s| format!("{}, {}", s.city, s.state))
            .unwrap_or_default();
        let elevations = self.views[self.active]
            .volume
            .as_ref()
            .map(|v| v.elevations.clone())
            .unwrap_or_default();
        let vcp = self.views[self.active]
            .volume
            .as_ref()
            .map(|v| v.vcp.split(" (").next().unwrap_or_default().to_string())
            .unwrap_or_default();
        let health = self.radar_health();
        let (health_txt, health_col) = ui::layers_panel::health_look(health.state());
        let panes = self.views.len();
        let cur_tool = self.tool;
        let mut smooth = self.settings.smooth_radar;
        let mut legend_on = self.views[self.active].show_legend;
        let layers_on = self.panel_open && !self.show_alert_panel;
        let alerts_on = self.panel_open && self.show_alert_panel;
        let basemap_on = self.basemap_open;
        let hrrr_on = self.views[self.active]
            .fields_on
            .contains(&crate::render::FieldLayer::Hrrr);
        let hrrr_valid = self.hrrr_valid;
        let tz_l = self.active_tz();

        let (disp_f, disp_l) = display_units(moment, &self.settings);
        let table = self.palettes.table(moment).clone();

        let mut pick_tilt: Option<usize> = None;
        let mut pick_panes: Option<usize> = None;
        let mut hrrr_hour = self.hrrr_fcst_hour;
        let mut all_tilts = false;

        egui::Panel::top("wsv3_ribbon")
            .exact_size(wsv3::RIBBON_H + wsv3::COLORBAR_H)
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                let full = ui.max_rect();
                let ribbon_rect =
                    egui::Rect::from_min_max(full.min, egui::pos2(full.right(), full.top() + wsv3::RIBBON_H));
                wsv3::ribbon_gradient(ui.painter(), ribbon_rect);

                ui.spacing_mut().item_spacing = vec2(6.0, 3.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);

                    // ---- RADAR ----
                    ribbon_group(ui, 208.0, |ui| {
                        wsv3::group_label(ui, "Radar");
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(egui_phosphor::regular::BROADCAST)
                                            .size(20.0)
                                            .color(accent),
                                    )
                                    .min_size(vec2(34.0, 34.0))
                                    .fill(wsv3::PILL_BG)
                                    .corner_radius(17.0),
                                )
                                .clicked()
                            {
                                actions.open_site_dialog = true;
                            }
                            ui.vertical(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(&site)
                                                .size(16.0)
                                                .strong()
                                                .color(wsv3::INK),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    actions.open_site_dialog = true;
                                }
                                ui.label(
                                    RichText::new(&city_state)
                                        .size(10.0)
                                        .color(Color32::from_white_alpha(130)),
                                );
                            });
                        });
                        ui.add_space(1.0);
                        for chunk in crate::products::PRODUCTS.chunks(4) {
                            ui.horizontal(|ui| {
                                for p in chunk {
                                    if wsv3::pill_sized(
                                        ui,
                                        p.short,
                                        p.moment == moment,
                                        accent,
                                        40.0,
                                    )
                                    .on_hover_text(p.blurb)
                                    .clicked()
                                    {
                                        actions.palette =
                                            Some(PaletteAction::SetMoment(p.moment, srv));
                                    }
                                }
                            });
                        }
                    });

                    // ---- TILT ----
                    ribbon_group(ui, 250.0, |ui| {
                        wsv3::group_label(ui, "Tilt angle");
                        if elevations.is_empty() {
                            ui.label(
                                RichText::new("loading\u{2026}")
                                    .size(11.0)
                                    .color(wsv3::STATUS_FG),
                            );
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                for (i, a) in elevations.iter().enumerate() {
                                    if wsv3::pill_sized(
                                        ui,
                                        &format!("{a:.1}\u{b0}"),
                                        i == tilt,
                                        accent,
                                        42.0,
                                    )
                                    .clicked()
                                    {
                                        pick_tilt = Some(i);
                                    }
                                }
                                if wsv3::pill(ui, "All \u{2317}", false, accent)
                                    .on_hover_text(
                                        "Four panes, one product, four tilts, cameras linked",
                                    )
                                    .clicked()
                                {
                                    all_tilts = true;
                                }
                            });
                        }
                    });

                    // ---- VIEW ----
                    ribbon_group(ui, 148.0, |ui| {
                        wsv3::group_label(ui, "View");
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("\u{25cf} {health_txt}"))
                                    .size(11.0)
                                    .color(health_col),
                            );
                            if !vcp.is_empty() {
                                ui.label(
                                    RichText::new(&vcp).size(11.0).color(wsv3::STATUS_FG),
                                );
                            }
                        });
                        wsv3::check(ui, "Smoothing", &mut smooth);
                        wsv3::check(ui, "Map legend", &mut legend_on);
                        ui.add_space(1.0);
                        ui.horizontal(|ui| {
                            for n in [1usize, 2, 4] {
                                if wsv3::pill_sized(
                                    ui,
                                    &format!("{n}\u{d7}"),
                                    panes == n,
                                    accent,
                                    30.0,
                                )
                                .on_hover_text(format!("{n}-pane layout"))
                                .clicked()
                                {
                                    pick_panes = Some(n);
                                }
                            }
                        });
                    });

                    // ---- MODEL ----
                    ribbon_group(ui, 128.0, |ui| {
                        wsv3::group_label(ui, "Model");
                        if wsv3::pill(ui, "HRRR future", hrrr_on, accent)
                            .on_hover_text(
                                "HRRR composite-reflectivity forecast — future radar out to 18 h",
                            )
                            .clicked()
                        {
                            actions.palette = Some(PaletteAction::ToggleField(
                                crate::render::FieldLayer::Hrrr,
                            ));
                        }
                        if hrrr_on {
                            ui.horizontal(|ui| {
                                if wsv3::pill(ui, "\u{2039}", false, accent).clicked() {
                                    hrrr_hour = hrrr_hour.saturating_sub(1).max(1);
                                }
                                ui.label(
                                    RichText::new(format!("F+{hrrr_hour}h"))
                                        .size(12.0)
                                        .strong()
                                        .color(wsv3::INK),
                                );
                                if wsv3::pill(ui, "\u{203a}", false, accent).clicked() {
                                    hrrr_hour = (hrrr_hour + 1).min(18);
                                }
                            });
                            if let Some(v) = hrrr_valid {
                                ui.label(
                                    RichText::new(crate::timefmt::fmt_clock(v, tz_l, false))
                                        .size(10.0)
                                        .color(wsv3::STATUS_FG),
                                );
                            }
                        }
                    });

                    // ---- OVERLAYS ----
                    ribbon_group(ui, 92.0, |ui| {
                        wsv3::group_label(ui, "Overlays");
                        if wsv3::pill(ui, "Layers", layers_on, accent).clicked() {
                            self.panel_open = !layers_on;
                            self.show_alert_panel = false;
                        }
                        if wsv3::pill(ui, "Alerts", alerts_on, accent).clicked() {
                            self.panel_open = !alerts_on;
                            self.show_alert_panel = true;
                        }
                        if wsv3::pill(ui, "Basemap", basemap_on, accent).clicked() {
                            self.basemap_open = !basemap_on;
                        }
                    });

                    // ---- TOOLS ----
                    ribbon_group(ui, 224.0, |ui| {
                        wsv3::group_label(ui, "Tools");
                        ui.horizontal_wrapped(|ui| {
                            for (tool, label) in [
                                (MapTool::Interrogate, "Explore"),
                                (MapTool::Measure, "Measure"),
                                (MapTool::Marker, "Marker"),
                                (MapTool::CrossSection, "X-section"),
                                (MapTool::Sounding, "Sounding"),
                                (MapTool::Forecast, "Forecast"),
                            ] {
                                if wsv3::pill(ui, label, cur_tool == tool, accent).clicked() {
                                    actions.palette = Some(PaletteAction::Tool(tool));
                                }
                            }
                            if wsv3::pill(ui, "Cells", false, accent).clicked() {
                                actions.palette =
                                    Some(PaletteAction::OpenWindow(AppWindow::StormTable));
                            }
                            if wsv3::pill(ui, "3D", false, accent).clicked() {
                                actions.palette =
                                    Some(PaletteAction::OpenWindow(AppWindow::Volume3d));
                            }
                        });
                    });

                    // ---- CAPTURE ----
                    ribbon_group(ui, 118.0, |ui| {
                        wsv3::group_label(ui, "Capture");
                        if wsv3::pill(ui, "Share view", false, accent).clicked() {
                            actions.palette = Some(PaletteAction::CopyViewLink);
                        }
                        if wsv3::pill(ui, "Open in Windy", false, accent).clicked() {
                            actions.palette = Some(PaletteAction::OpenInWindy);
                        }
                        if wsv3::pill(ui, "Settings", false, accent).clicked() {
                            actions.palette = Some(PaletteAction::OpenWindow(AppWindow::Settings));
                        }
                        if wsv3::pill(ui, "Help", false, accent).clicked() {
                            actions.palette = Some(PaletteAction::OpenWindow(AppWindow::Help));
                        }
                    });
                });

                // ---- docked colour scale ----
                let cb = egui::Rect::from_min_max(
                    egui::pos2(full.left(), full.top() + wsv3::RIBBON_H),
                    full.max,
                );
                wsv3::colorbar(ui.painter(), cb, &table, disp_f, disp_l);
            });

        // --- apply ---
        if let Some(i) = pick_tilt {
            self.views[self.active].tilt = i;
        }
        if let Some(n) = pick_panes {
            self.apply_palette(PaletteAction::SetPanes(n), ctx);
        }
        if all_tilts {
            self.apply_palette(PaletteAction::AllTilts, ctx);
        }
        if self.settings.smooth_radar != smooth {
            self.settings.smooth_radar = smooth;
            self.settings.save();
        }
        self.views[self.active].show_legend = legend_on;
        // The per-frame sync in `update` refetches when `hrrr_fcst_hour` changes; a no-op write
        // when the steppers weren't touched costs nothing.
        self.hrrr_fcst_hour = hrrr_hour;
        self.apply_ui_actions(actions, ctx);
    }

    /// The bottom status bar: connection on the left, the valid time in the middle, the active
    /// camera's centre and zoom on the right.
    pub(crate) fn wsv3_status_bar(&mut self, root: &mut egui::Ui) {
        let site = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "\u{2014}".to_string());
        let age = self.views[self.active].volume.as_ref().map(|v| {
            let secs = (Utc::now() - v.time).num_seconds().max(0);
            humanize(secs)
        });
        let valid_dt = self.views[self.active]
            .timeline
            .current()
            .and_then(|id| id.date_time());
        let tz = self.active_tz();
        let valid = valid_dt
            .map(|d| crate::timefmt::fmt_clock(d, tz, false))
            .unwrap_or_default();
        let cam = self.views[self.active].camera;
        let (lon, lat) = crate::render::mercator::world_to_lonlat(cam.center.0, cam.center.1);
        let zoom = cam.zoom;

        egui::Panel::bottom("wsv3_status")
            .exact_size(wsv3::STATUS_H)
            .frame(egui::Frame::NONE.fill(wsv3::STATUS_BG))
            .show(root, |ui| {
                let lab = |ui: &mut egui::Ui, s: String| {
                    ui.label(RichText::new(s).size(11.0).color(wsv3::STATUS_FG));
                };
                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    lab(
                        ui,
                        match &age {
                            Some(a) => format!("{site}  \u{b7}  scan {a} ago"),
                            None => format!("{site}  \u{b7}  live only"),
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        lab(ui, format!("{lat:.3}, {lon:.3}   \u{7c}   z{zoom:.1}"));
                        if !valid.is_empty() {
                            ui.separator();
                            lab(ui, valid);
                        }
                    });
                });
            });
    }

    /// The blue timestamp pill in the active pane's top-left corner.
    pub(crate) fn wsv3_timestamp(&mut self, ctx: &egui::Context) {
        let Some(d) = self.views[self.active]
            .timeline
            .current()
            .and_then(|id| id.date_time())
        else {
            return;
        };
        let text = crate::timefmt::fmt_date_clock(d, self.active_tz());
        egui::Area::new(egui::Id::new("wsv3_timestamp"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, vec2(10.0, 10.0))
            .interactable(false)
            .show(ctx, |ui| {
                // Give the label room so it lays out on one line instead of wrapping to the
                // Area's shrink-wrapped width.
                ui.set_max_width(360.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x22, 0x35, 0x5e))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0x4a, 0x63, 0x9a)))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).size(12.5).color(Color32::WHITE));
                    });
            });
    }
}
