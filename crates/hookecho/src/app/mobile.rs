//! Touch-first Android chrome, RadarOmega-style: a full-bleed map under floating translucent
//! controls. A top bar (menu · site/VCP pill · settings · alert badge), a bottom card (playback ·
//! frames/product/elevation · moment chips + tilt), and a slide-in left drawer that holds every
//! feature in sections. The map, floating windows, and all data paths are shared with desktop.

use egui::{pos2, vec2, Align, Align2, Color32, Frame, Id, Layout, Margin, RichText, Sense, Stroke};
use wxdata::level2::Moment;

use super::humanize;
use crate::ui::toolbox::ToolboxActions;

/// Which slide-in drawer is open (`None` = just the floating bars over the map).
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MobileSheet {
    #[default]
    None,
    Menu,
    Alerts,
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

/// Translucent "glass" card used by every floating bar and drawer.
fn glass(alpha: u8) -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(16, 18, 24, alpha))
        .corner_radius(18.0)
        .inner_margin(Margin::symmetric(12, 8))
        .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 24)))
}

/// A round-ish 42px control button (hamburger, gear, playback…).
fn round_btn(ui: &mut egui::Ui, glyph: &str, active: bool, accent: Color32) -> egui::Response {
    let (fg, bg) = if active {
        (Color32::BLACK, accent)
    } else {
        (Color32::from_gray(230), Color32::from_rgba_unmultiplied(255, 255, 255, 22))
    };
    ui.add(
        egui::Button::new(RichText::new(glyph).size(18.0).color(fg))
            .min_size(vec2(42.0, 42.0))
            .fill(bg)
            .corner_radius(13.0),
    )
}

/// A labelled pill/chip, ~40px tall, tinted when active.
fn chip(ui: &mut egui::Ui, label: &str, w: f32, active: bool, accent: Color32) -> egui::Response {
    let (fg, bg) = if active {
        (Color32::BLACK, accent)
    } else {
        (Color32::from_gray(225), Color32::from_rgba_unmultiplied(255, 255, 255, 20))
    };
    ui.add_sized(
        [w, 38.0],
        egui::Button::new(RichText::new(label).size(14.0).color(fg)).fill(bg).corner_radius(12.0),
    )
}

/// Long product name for the info line.
fn product_name(m: Moment) -> &'static str {
    match m {
        Moment::Reflectivity => "Reflectivity",
        Moment::Velocity => "Velocity",
        Moment::SpectrumWidth => "Spectrum Width",
        Moment::DifferentialReflectivity => "Differential Refl.",
        Moment::DifferentialPhase => "Differential Phase",
        Moment::CorrelationCoefficient => "Correlation Coef.",
    }
}

impl super::HookEchoApp {
    /// Render the whole Android chrome (floating bars + drawers) and return the toolbox actions
    /// the shared code processes.
    pub(crate) fn mobile_chrome(&mut self, _root: &mut egui::Ui, ctx: &egui::Context) -> ToolboxActions {
        let mut actions = ToolboxActions::default();
        let active = self.active;
        let accent = crate::theme::accent(self.settings.theme);
        let content = ctx.content_rect();
        let vr = ctx.viewport_rect();
        let inset_top = (content.top() - vr.top()).max(0.0);
        let inset_bottom = (vr.bottom() - content.bottom()).max(0.0);

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
        let site = self.views[active].site.clone().unwrap_or_else(|| "—".into());
        let cur_moment = self.views[active].moment;
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
        let (vcp, age_str, loading) = {
            let v = &self.views[active];
            match &v.volume {
                Some(vol) => {
                    let age = (chrono::Utc::now() - vol.time).num_seconds().max(0);
                    (Some(vol.vcp.clone()), Some(humanize(age)), false)
                }
                None => (None, None, v.loading),
            }
        };
        let nframes = self.views[active].timeline.frames.len();
        let following = self.views[active].timeline.following;
        let playing = self.views[active].timeline.playing;

        // ---------- TOP BAR ----------
        egui::Area::new(Id::new("m_topbar"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, inset_top + 8.0))
            .show(ctx, |ui| {
                // Fixed inner width computed from the safe content width, minus the frame margins,
                // so the row (hamburger · pill · gear · badge) never overflows the screen edges.
                let inner_w = content.width() - 36.0;
                glass(210).show(ui, |ui| {
                    ui.set_width(inner_w);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let menu_on = self.mobile_sheet == MobileSheet::Menu;
                        if round_btn(ui, "☰", menu_on, accent).clicked() {
                            self.mobile_sheet = if menu_on { MobileSheet::None } else { MobileSheet::Menu };
                        }
                        // Pill width = inner width minus the three fixed buttons and the gaps.
                        let pill_w = (inner_w - 42.0 - 42.0 - 52.0 - 4.0 * 6.0).max(80.0);
                        // Short VCP ("VCP 35 (Clear air, SZ-2)" -> "VCP 35") so the pill fits.
                        let mode = vcp
                            .as_deref()
                            .map(|s| s.split(" (").next().unwrap_or(s).to_string())
                            .unwrap_or_else(|| "no volume".into());
                        let pill = egui::Button::new(
                            RichText::new(format!("◎  {site}  ·  {mode}")).size(15.0).strong().color(Color32::from_gray(235)),
                        )
                        .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 16))
                        .corner_radius(11.0);
                        if ui.add_sized([pill_w, 42.0], pill).clicked() && self.site_dialog.is_none() {
                            self.site_dialog = Some(Default::default());
                        }
                        if round_btn(ui, "⚙", false, accent).clicked() {
                            self.settings_window.open = true;
                        }
                        // Alert badge: colored by max escalation, count inside.
                        let (bg, fg) = if alert_count == 0 {
                            (Color32::from_rgba_unmultiplied(255, 255, 255, 22), Color32::from_gray(160))
                        } else if max_esc >= 2 {
                            (Color32::from_rgb(220, 40, 40), Color32::WHITE)
                        } else {
                            (Color32::from_rgb(230, 170, 40), Color32::BLACK)
                        };
                        let label = if alert_count == 0 { "🔔".to_string() } else { alert_count.to_string() };
                        let on = self.mobile_sheet == MobileSheet::Alerts;
                        let badge = egui::Button::new(RichText::new(label).size(16.0).strong().color(fg))
                            .fill(if on { accent } else { bg })
                            .corner_radius(21.0);
                        if ui.add_sized([52.0, 42.0], badge).clicked() {
                            self.mobile_sheet = if on { MobileSheet::None } else { MobileSheet::Alerts };
                        }
                    });
                });
            });

        // ---------- BOTTOM BAR ----------
        egui::Area::new(Id::new("m_bottombar"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -(inset_bottom + 8.0)))
            .show(ctx, |ui| {
                glass(216).show(ui, |ui| {
                    ui.set_width(content.width() - 36.0);
                    // Info + playback line.
                    ui.horizontal(|ui| {
                        if round_btn(ui, if playing { "⏸" } else { "▶" }, playing, accent).clicked() {
                            self.views[active].timeline.toggle_play();
                        }
                        if round_btn(ui, "⏮", false, accent).clicked() {
                            self.views[active].timeline.step(-1);
                        }
                        if round_btn(ui, "⏭", false, accent).clicked() {
                            self.views[active].timeline.step(1);
                        }
                        let live_col = if following { Color32::from_rgb(90, 220, 120) } else { Color32::from_gray(150) };
                        if ui
                            .add(
                                egui::Button::new(RichText::new("LIVE").size(13.0).strong().color(if following { Color32::BLACK } else { live_col }))
                                    .fill(if following { Color32::from_rgb(90, 220, 120) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 18) })
                                    .corner_radius(10.0)
                                    .min_size(vec2(56.0, 40.0)),
                            )
                            .clicked()
                        {
                            self.views[active].timeline.go_head();
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some(age) = &age_str {
                                ui.label(RichText::new(format!("{age} ago")).size(12.0).color(Color32::from_gray(170)));
                            } else if loading {
                                ui.label(RichText::new("loading…").size(12.0).color(Color32::from_gray(170)));
                            }
                        });
                    });
                    // Product / frames / elevation line.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{}   ·   {} frames", product_name(cur_moment), nframes.max(1)))
                                .size(13.0)
                                .strong()
                                .color(Color32::from_rgb(120, 210, 150)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if n_tilt > 0 {
                                ui.label(RichText::new(format!("Elev {cur_angle:.1}°")).size(12.0).color(Color32::from_gray(190)));
                            }
                        });
                    });
                    ui.add_space(4.0);
                    // Moment chips + tilt cycler.
                    egui::ScrollArea::horizontal().id_salt("m_moments").show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for m in Moment::ALL {
                                if chip(ui, m.short_name(), 52.0, cur_moment == m, accent).clicked() {
                                    self.views[active].moment = m;
                                }
                            }
                            if n_tilt > 0 && chip(ui, &format!("Tilt {cur_angle:.1}°"), 92.0, false, accent).clicked() {
                                self.views[active].tilt = (cur_tilt + 1) % n_tilt.max(1);
                            }
                        });
                    });
                });
            });

        // ---------- DRAWERS ----------
        if self.mobile_sheet != MobileSheet::None {
            let dw = (content.width() * 0.86).min(430.0);
            // The opaque drawer rect (left for Menu, right for Alerts) — taps inside it must not
            // close the drawer via the scrim.
            let drawer_rect = if self.mobile_sheet == MobileSheet::Alerts {
                egui::Rect::from_min_size(pos2(content.right() - dw, content.top()), vec2(dw, content.height()))
            } else {
                egui::Rect::from_min_size(pos2(content.left(), content.top()), vec2(dw, content.height()))
            };
            // Scrim: dims the map + bars; a tap *outside* the drawer closes it.
            egui::Area::new(Id::new("m_scrim")).fixed_pos(vr.min).show(ctx, |ui| {
                let r = ui.allocate_response(vr.size(), Sense::click());
                ui.painter().rect_filled(r.rect, egui::CornerRadius::ZERO, Color32::from_black_alpha(130));
                if r.clicked() && !r.interact_pointer_pos().is_some_and(|p| drawer_rect.contains(p)) {
                    self.mobile_sheet = MobileSheet::None;
                }
            });

            let drawer = Frame::new()
                .fill(Color32::from_rgba_unmultiplied(14, 16, 22, 246))
                .inner_margin(Margin::symmetric(12, 10))
                .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20)));

            match self.mobile_sheet {
                MobileSheet::Menu => {
                    egui::Area::new(Id::new("m_drawer"))
                        .fixed_pos(pos2(content.left(), content.top()))
                        .show(ctx, |ui| {
                            drawer.show(ui, |ui| {
                                ui.set_width(dw - 24.0);
                                ui.set_height(content.height() - 20.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Hook Echo-WX").size(19.0).strong().color(accent));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if round_btn(ui, "✕", false, accent).clicked() {
                                            self.mobile_sheet = MobileSheet::None;
                                        }
                                    });
                                });
                                ui.separator();
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let l3_site = self.l3grid_site.clone();
                                    actions = crate::ui::toolbox::show(
                                        ui,
                                        &mut self.views[active],
                                        &mut self.settings,
                                        &mut self.filters,
                                        &mut self.fields,
                                        &mut self.rotation_minutes,
                                        &mut self.hrrr_fcst_hour,
                                        self.hrrr_valid,
                                        &mut self.env_cape_ml,
                                        &mut self.env_srh_km,
                                        l3_site.as_deref(),
                                        &mut self.show_sensors,
                                        &mut self.show_hodo,
                                        &mut self.show_alert_panel,
                                        &mut self.show_storm_reports,
                                        &mut self.show_spotters,
                                        &mut self.show_probsevere,
                                        &mut self.show_radar_sites,
                                        &mut self.show_metar,
                                        &mut self.show_tropical,
                                        &mut self.show_aviation,
                                        &mut self.show_range_rings,
                                    );
                                    ui.add_space(8.0);
                                    ui.separator();
                                    self.mobile_tools(ui);
                                });
                            });
                        });
                }
                MobileSheet::Alerts => {
                    egui::Area::new(Id::new("m_alerts"))
                        .fixed_pos(pos2(content.right() - dw, content.top()))
                        .show(ctx, |ui| {
                            drawer.show(ui, |ui| {
                                ui.set_width(dw - 24.0);
                                ui.set_height(content.height() - 20.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("Active alerts ({alert_count})")).size(18.0).strong());
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if round_btn(ui, "✕", false, accent).clicked() {
                                            self.mobile_sheet = MobileSheet::None;
                                        }
                                    });
                                });
                                ui.separator();
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    if malerts.is_empty() {
                                        ui.add_space(10.0);
                                        ui.weak("No active alerts in view.");
                                    }
                                    for a in &malerts {
                                        let col = if a.esc >= 2 { Color32::from_rgb(255, 90, 90) } else { a.color };
                                        let resp = ui.add(
                                            egui::Button::new(RichText::new(&a.title).size(15.0).strong().color(col))
                                                .min_size(vec2(ui.available_width(), 40.0))
                                                .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 14))
                                                .corner_radius(10.0),
                                        );
                                        ui.label(RichText::new(&a.sub).size(12.0).color(Color32::from_gray(165)));
                                        ui.add_space(7.0);
                                        if resp.clicked() {
                                            let cam = &mut self.views[active].camera;
                                            cam.center = crate::render::mercator::lonlat_to_world(a.lon, a.lat);
                                            cam.zoom = cam.zoom.max(8.0);
                                            self.open_alert_popup(&a.id);
                                            self.mobile_sheet = MobileSheet::None;
                                        }
                                    }
                                });
                            });
                        });
                }
                MobileSheet::None => {}
            }
        }

        actions
    }

    /// The drawer's Tools block: map-tap tools plus one-tap openers for the analysis windows.
    fn mobile_tools(&mut self, ui: &mut egui::Ui) {
        use super::{MapTool, ShotDest};
        let accent = crate::theme::accent(self.settings.theme);
        let w = (ui.available_width() - 16.0) / 3.0;

        ui.label(RichText::new("Map tap tool").size(13.0).strong().color(accent));
        ui.horizontal_wrapped(|ui| {
            for (tool, label) in [
                (MapTool::Interrogate, "Interrogate"),
                (MapTool::Measure, "Measure"),
                (MapTool::Marker, "Marker"),
                (MapTool::CrossSection, "X-section"),
                (MapTool::Sounding, "Sounding"),
                (MapTool::Climatology, "Tor climo"),
            ] {
                if chip(ui, label, w, self.tool == tool, accent).clicked() {
                    self.tool = tool;
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Analysis").size(13.0).strong().color(accent));
        ui.horizontal_wrapped(|ui| {
            if chip(ui, "3D volume", w, false, accent).clicked() {
                self.build_volume3d();
            }
            if chip(ui, "CAPPI", w, false, accent).clicked() {
                self.show_cappi = true;
                self.cappi_key = None;
            }
            if chip(ui, "Digest", w, false, accent).clicked() {
                self.digest_window.open = true;
                self.generate_digest();
            }
            if chip(ui, "AFD", w, false, accent).clicked() {
                self.afd_open = true;
                self.fetch_afd();
            }
            if chip(ui, "Events", w, false, accent).clicked() {
                self.event_window.open = true;
            }
            if chip(ui, "Markers", w, false, accent).clicked() {
                self.marker_window.open = true;
            }
            if chip(ui, "Placefiles", w, false, accent).clicked() {
                self.placefile_window.open = true;
            }
            if chip(ui, "Palettes", w, false, accent).clicked() {
                self.palette_editor.open = true;
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Panes & capture").size(13.0).strong().color(accent));
        ui.horizontal_wrapped(|ui| {
            for count in [1usize, 2, 4] {
                if chip(ui, &format!("{count}⊞"), w * 0.6, self.views.len() == count, accent).clicked() {
                    self.set_pane_count(count);
                }
            }
            if chip(ui, "Link", w * 0.7, self.link_cameras, accent).clicked() {
                self.link_cameras = !self.link_cameras;
            }
            if chip(ui, "Screenshot", w, false, accent).clicked() {
                if let Some(path) = crate::dialog::save_path("hookecho.png", "png") {
                    self.screenshot_pending = Some(ShotDest::File(path));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                }
            }
            if chip(ui, "GIF", w * 0.7, self.loop_export.is_some(), accent).clicked() && self.loop_export.is_none() {
                self.start_loop_export(crate::loopexport::LoopFormat::Gif);
            }
            if chip(ui, "OBS", w * 0.7, self.obs_mode, accent).clicked() {
                self.obs_mode = true;
            }
            if chip(ui, "Wizard", w, false, accent).clicked() {
                self.wizard.start();
            }
        });
    }
}
