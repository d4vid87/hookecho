//! Touch-first Android chrome. A compact top bar (site · volume age · alerts · settings), a
//! persistent bottom dock (moment chips + tilt cycler + sheet toggles), and one slide-up sheet
//! that hosts the full radar toolbox, a tools grid, or the active-alerts list. The map itself,
//! every floating window, and all data paths are shared with desktop — only this chrome differs.

use egui::{vec2, Align, Color32, Layout, RichText};
use wxdata::level2::Moment;

use super::humanize;
use crate::ui::toolbox::ToolboxActions;

/// Which slide-up sheet is open (mutually exclusive; `None` = just the dock).
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MobileSheet {
    #[default]
    None,
    Layers,
    Tools,
    Alerts,
}

/// One alert row, owned so the sheet renders without holding a borrow of `self`.
struct MAlert {
    id: String,
    title: String,
    sub: String,
    color: Color32,
    lon: f64,
    lat: f64,
    esc: u8,
}

/// A full-width touch button, ~44 px tall, tinted when active.
fn tbtn(ui: &mut egui::Ui, label: &str, active: bool, accent: Color32) -> egui::Response {
    let (fg, bg) = if active {
        (Color32::BLACK, accent)
    } else {
        (Color32::from_gray(225), Color32::from_gray(46))
    };
    ui.add(
        egui::Button::new(RichText::new(label).size(14.0).color(fg))
            .min_size(vec2(0.0, 44.0))
            .fill(bg)
            .corner_radius(8.0),
    )
}

impl super::HookEchoApp {
    /// Render the whole Android chrome and return the toolbox actions the shared code processes.
    pub(crate) fn mobile_chrome(&mut self, root: &mut egui::Ui, ctx: &egui::Context) -> ToolboxActions {
        let mut actions = ToolboxActions::default();
        let active = self.active;
        let accent = crate::theme::accent(self.settings.theme);

        // Owned alert rows (decoupled from `&self`) so the sheet can both list and fly-to them.
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

        // Copies read by the dock closures, taken up front so no live borrow of `self.views`
        // straddles a mutation (moment/tilt taps write back into the active view).
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

        // ---- Top bar: site · volume age · alerts bell · settings ----
        egui::Panel::top("m_top").exact_size(54.0).show(root, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let site = self.views[active].site.clone().unwrap_or_else(|| "—".into());
                if ui
                    .add(
                        egui::Button::new(RichText::new(format!("◎ {site}")).size(18.0).strong())
                            .min_size(vec2(0.0, 42.0))
                            .corner_radius(8.0),
                    )
                    .clicked()
                    && self.site_dialog.is_none()
                {
                    self.site_dialog = Some(Default::default());
                }
                let v = &self.views[active];
                if let Some(vol) = &v.volume {
                    let age = (chrono::Utc::now() - vol.time).num_seconds().max(0);
                    ui.label(
                        RichText::new(format!("{} · {} ago", vol.time.format("%H:%MZ"), humanize(age)))
                            .size(13.0)
                            .color(Color32::from_gray(190)),
                    );
                } else if v.loading {
                    ui.label(RichText::new("loading…").size(13.0).color(Color32::from_gray(170)));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(6.0);
                    if ui
                        .add(egui::Button::new(RichText::new("⚙").size(20.0)).min_size(vec2(42.0, 42.0)).corner_radius(8.0))
                        .clicked()
                    {
                        self.settings_window.open = true;
                    }
                    let (bell, col) = if alert_count > 0 {
                        (format!("🔔 {alert_count}"), Color32::from_rgb(255, 120, 120))
                    } else {
                        ("🔔".to_string(), Color32::from_gray(150))
                    };
                    let on = self.mobile_sheet == MobileSheet::Alerts;
                    if ui
                        .add(
                            egui::Button::new(RichText::new(bell).size(16.0).color(if on { Color32::BLACK } else { col }))
                                .min_size(vec2(48.0, 42.0))
                                .fill(if on { accent } else { Color32::from_gray(46) })
                                .corner_radius(8.0),
                        )
                        .clicked()
                    {
                        self.mobile_sheet = if on { MobileSheet::None } else { MobileSheet::Alerts };
                    }
                });
            });
        });

        // ---- Bottom dock: moment chips + tilt cycler + Layers/Tools toggles ----
        egui::Panel::bottom("m_dock").exact_size(112.0).show(root, |ui| {
            ui.add_space(7.0);
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    for m in Moment::ALL {
                        let sel = cur_moment == m;
                        let (fg, bg) = if sel {
                            (Color32::BLACK, accent)
                        } else {
                            (Color32::from_gray(220), Color32::from_gray(46))
                        };
                        if ui
                            .add(
                                egui::Button::new(RichText::new(m.short_name()).size(15.0).color(fg))
                                    .min_size(vec2(54.0, 40.0))
                                    .fill(bg)
                                    .corner_radius(8.0),
                            )
                            .clicked()
                        {
                            self.views[active].moment = m;
                        }
                    }
                    ui.add_space(6.0);
                });
            });
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let layers_on = self.mobile_sheet == MobileSheet::Layers;
                if tbtn(ui, "☰ Layers", layers_on, accent).clicked() {
                    self.mobile_sheet = if layers_on { MobileSheet::None } else { MobileSheet::Layers };
                }
                let tools_on = self.mobile_sheet == MobileSheet::Tools;
                if tbtn(ui, "⛭ Tools", tools_on, accent).clicked() {
                    self.mobile_sheet = if tools_on { MobileSheet::None } else { MobileSheet::Tools };
                }
                if n_tilt > 0 {
                    if tbtn(ui, &format!("⤒ {cur_angle:.1}°"), false, accent).clicked() {
                        self.views[active].tilt = (cur_tilt + 1) % n_tilt.max(1);
                    }
                } else {
                    tbtn(ui, "⤒ —", false, accent);
                }
                ui.add_space(6.0);
            });
        });

        // ---- Slide-up sheet ----
        if self.mobile_sheet != MobileSheet::None {
            let sheet_h = (ctx.content_rect().height() * 0.46).clamp(220.0, 620.0);
            let frame = egui::Frame::new()
                .fill(ctx.global_style().visuals.panel_fill)
                .inner_margin(egui::Margin::symmetric(10, 8));
            egui::Panel::bottom("m_sheet").exact_size(sheet_h).frame(frame).show(root, |ui| {
                ui.horizontal(|ui| {
                    let title = match self.mobile_sheet {
                        MobileSheet::Layers => "Layers & data",
                        MobileSheet::Tools => "Tools",
                        MobileSheet::Alerts => "Active alerts",
                        MobileSheet::None => "",
                    };
                    ui.label(RichText::new(title).size(16.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("✕").size(16.0)).min_size(vec2(38.0, 32.0)).corner_radius(8.0))
                            .clicked()
                        {
                            self.mobile_sheet = MobileSheet::None;
                        }
                    });
                });
                ui.separator();
                match self.mobile_sheet {
                    MobileSheet::Layers => {
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
                    }
                    MobileSheet::Tools => self.mobile_tools(ui),
                    MobileSheet::Alerts => {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if malerts.is_empty() {
                                ui.add_space(8.0);
                                ui.weak("No active alerts in view.");
                            }
                            for a in &malerts {
                                let mut txt = RichText::new(&a.title).size(15.0).strong().color(a.color);
                                if a.esc >= 2 {
                                    txt = txt.color(Color32::from_rgb(255, 90, 90));
                                }
                                let resp = ui.add(egui::Button::new(txt).min_size(vec2(ui.available_width(), 40.0)).corner_radius(8.0));
                                ui.label(RichText::new(&a.sub).size(12.0).color(Color32::from_gray(170)));
                                ui.add_space(6.0);
                                if resp.clicked() {
                                    let cam = &mut self.views[active].camera;
                                    cam.center = crate::render::mercator::lonlat_to_world(a.lon, a.lat);
                                    cam.zoom = cam.zoom.max(8.0);
                                    self.open_alert_popup(&a.id);
                                    self.mobile_sheet = MobileSheet::None;
                                }
                            }
                        });
                    }
                    MobileSheet::None => {}
                }
            });
        }

        actions
    }

    /// The Tools sheet: map-click tools plus one-tap openers for the analysis windows.
    fn mobile_tools(&mut self, ui: &mut egui::Ui) {
        use super::{MapTool, ShotDest};
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new("Map tap tool").size(13.0).strong());
            ui.horizontal_wrapped(|ui| {
                for (tool, label) in [
                    (MapTool::Interrogate, "Interrogate"),
                    (MapTool::Measure, "Measure"),
                    (MapTool::Marker, "Drop marker"),
                    (MapTool::CrossSection, "Cross-section"),
                    (MapTool::Sounding, "Sounding"),
                    (MapTool::Climatology, "Tornado climo"),
                ] {
                    let sel = self.tool == tool;
                    if tbtn(ui, label, sel, crate::theme::accent(self.settings.theme)).clicked() {
                        self.tool = tool;
                    }
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.label(RichText::new("Analysis").size(13.0).strong());
            let accent = crate::theme::accent(self.settings.theme);
            egui::Grid::new("m_tools_grid").num_columns(2).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
                if tbtn(ui, "3D volume", false, accent).clicked() {
                    self.build_volume3d();
                }
                if tbtn(ui, "CAPPI slice", false, accent).clicked() {
                    self.show_cappi = true;
                    self.cappi_key = None;
                }
                ui.end_row();
                if tbtn(ui, "Storm digest", false, accent).clicked() {
                    self.digest_window.open = true;
                    self.generate_digest();
                }
                if tbtn(ui, "Forecast (AFD)", false, accent).clicked() {
                    self.afd_open = true;
                    self.fetch_afd();
                }
                ui.end_row();
                if tbtn(ui, "Event library", false, accent).clicked() {
                    self.event_window.open = true;
                }
                if tbtn(ui, "Markers", false, accent).clicked() {
                    self.marker_window.open = true;
                }
                ui.end_row();
                if tbtn(ui, "Placefiles", false, accent).clicked() {
                    self.placefile_window.open = true;
                }
                if tbtn(ui, "Color tables", false, accent).clicked() {
                    self.palette_editor.open = true;
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.separator();
            ui.label(RichText::new("Panes").size(13.0).strong());
            ui.horizontal(|ui| {
                for count in [1usize, 2, 4] {
                    let sel = self.views.len() == count;
                    if tbtn(ui, &format!("{count}"), sel, accent).clicked() {
                        self.set_pane_count(count);
                    }
                }
                let linked = self.link_cameras;
                if tbtn(ui, "Link", linked, accent).clicked() {
                    self.link_cameras = !self.link_cameras;
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.label(RichText::new("Capture & help").size(13.0).strong());
            egui::Grid::new("m_cap_grid").num_columns(2).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
                if tbtn(ui, "Screenshot", false, accent).clicked() {
                    if let Some(path) = crate::dialog::save_path("hookecho.png", "png") {
                        self.screenshot_pending = Some(ShotDest::File(path));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                    }
                }
                if tbtn(ui, "Export GIF", self.loop_export.is_some(), accent).clicked() && self.loop_export.is_none() {
                    self.start_loop_export(crate::loopexport::LoopFormat::Gif);
                }
                ui.end_row();
                if tbtn(ui, "OBS mode", self.obs_mode, accent).clicked() {
                    self.obs_mode = true;
                }
                if tbtn(ui, "Setup wizard", false, accent).clicked() {
                    self.wizard.start();
                }
                ui.end_row();
            });
        });
    }
}
