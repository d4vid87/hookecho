//! The left-dock Radar Toolbox: site, VCP, map style, products, tilts, threshold, overlays, timeline.

use crate::app::OverlayFilters;
use crate::settings::Settings;
use crate::view::MapView;
use wxdata::alerts::Category;
use wxdata::level2::Moment;

/// Signals the toolbox raises for the app to act on this frame.
#[derive(Default)]
pub struct ToolboxActions {
    pub open_site_dialog: bool,
    pub reload: bool,
    /// An overlay filter toggle changed; the app should reassemble the displayed set.
    pub overlays_changed: bool,
    /// Set the active view's storm motion from the SCIT storm-cell mean motion.
    pub srv_from_cells: bool,
    /// DVR: replay the buffered (in-RAM) frames from the earliest cached one.
    pub instant_replay: bool,
    /// The Day-1 outlook hazard changed; the app must clear + refetch that day's outlook.
    pub outlook_kind_changed: bool,
}

#[allow(clippy::too_many_arguments)] // one flat call per frame; a params struct adds churn for no reader gain
pub fn show(
    ui: &mut egui::Ui,
    view: &mut MapView,
    settings: &mut Settings,
    filters: &mut OverlayFilters,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    rotation_minutes: &mut u16,
    hrrr_fcst_hour: &mut u8,
    hrrr_valid: Option<chrono::DateTime<chrono::Utc>>,
    env_cape_ml: &mut bool,
    env_srh_km: &mut u8,
    l3grid_site: Option<&str>,
    show_sensors: &mut bool,
    show_hodo: &mut bool,
    show_alert_panel: &mut bool,
    show_storm_reports: &mut bool,
    show_spotters: &mut bool,
    show_probsevere: &mut bool,
    show_radar_sites: &mut bool,
    show_metar: &mut bool,
    show_tropical: &mut bool,
    show_aviation: &mut bool,
    show_range_rings: &mut bool,
) -> ToolboxActions {
    use crate::theme::section;
    let mut actions = ToolboxActions::default();
    egui::ScrollArea::vertical().show(ui, |ui| {
        section(ui, "Radar Site", |ui| site_section(ui, view, settings, &mut actions));
        section(ui, "Volume", |ui| vcp_section(ui, view));
        section(ui, "Map", |ui| map_section(ui, view, settings));
        section(ui, "Level 2", |ui| level2_section(ui, view, settings, &mut actions));
        section(ui, "National", |ui| national_section(ui, fields, rotation_minutes));
        section(ui, "Future Radar", |ui| hrrr_section(ui, fields, hrrr_fcst_hour, hrrr_valid));
        section(ui, "Environment", |ui| env_section(ui, fields, env_cape_ml, env_srh_km));
        section(ui, "Sensors", |ui| {
            ui.checkbox(show_sensors, "Sensor dashboard")
                .on_hover_text("Nearest NWS/METAR station: current conditions + 24h trends");
            ui.checkbox(show_hodo, "VAD hodograph")
                .on_hover_text("VAD wind profile (DS.48vwp): wind vectors by altitude");
        });
        section(ui, "Overlays", |ui| {
            ui.checkbox(show_alert_panel, "Active alerts panel")
                .on_hover_text("Right-dock list of alerts in view, sorted by severity (press A)");
            ui.checkbox(show_storm_reports, "SPC storm reports")
                .on_hover_text("Today's tornado (red) / wind (blue) / hail (green) reports; click a dot");
            ui.checkbox(show_spotters, "Spotter Network")
                .on_hover_text("Live spotters within 230 km of the active site (spotternetwork.org, 1-min refresh)");
            ui.checkbox(show_radar_sites, "Radar sites")
                .on_hover_text("Show all NEXRAD sites; click one on the map to switch radars");
            ui.checkbox(show_metar, "Surface obs (METAR)")
                .on_hover_text("Station plots: wind barbs + T/Td, flight-category colored (zoom in)");
            if ui.checkbox(show_tropical, "Tropical (NHC)")
                .on_hover_text("Active tropical cyclones: forecast cones, tracks, category-colored points")
                .changed()
            {
                actions.overlays_changed = true;
            }
            if ui.checkbox(show_probsevere, "ProbSevere")
                .on_hover_text("NOAA/CIMSS per-storm severe/tor/hail/wind probabilities; click a polygon")
                .changed()
            {
                actions.overlays_changed = true;
            }
            if ui.checkbox(show_aviation, "Aviation (SIGMET/AIRMET)")
                .on_hover_text("Convective/turbulence/icing/IFR hazard areas; click a polygon for the raw bulletin")
                .changed()
            {
                actions.overlays_changed = true;
            }
            ui.checkbox(show_range_rings, "Range rings")
                .on_hover_text("50/100/150/200 km rings + azimuth spokes around the pane's radar");
            overlays_section(ui, filters, &mut actions);
        });
        section(ui, "Level 3", |ui| level3_section(ui, fields, l3grid_site));
        section(ui, "Timeline", |ui| timeline_section(ui, view, settings, &mut actions));
    });
    actions
}

fn national_section(
    ui: &mut egui::Ui,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    rotation_minutes: &mut u16,
) {
    use crate::render::FieldLayer as FL;
    fn toggle(
        ui: &mut egui::Ui,
        fields: &mut std::collections::HashMap<FL, crate::app::FieldState>,
        layer: FL,
        label: &str,
        hover: &str,
    ) {
        if let Some(s) = fields.get_mut(&layer) {
            ui.checkbox(&mut s.show, label).on_hover_text(hover);
        }
    }
    toggle(ui, fields, FL::Mrms, "MRMS Mosaic", "MRMS national composite reflectivity (~2-min cadence)");
    toggle(ui, fields, FL::Rotation, "Rotation tracks", "Accumulated low-level azimuthal-shear max — tornado-track map");
    if fields.get(&FL::Rotation).is_some_and(|s| s.show) {
        ui.indent("rot_dur", |ui| {
            ui.horizontal(|ui| {
                ui.label("Window:");
                let mut changed = false;
                for m in [30u16, 60, 120] {
                    changed |= ui.selectable_value(rotation_minutes, m, format!("{m}m")).changed();
                }
                // Duration change → force an immediate refetch of the rotation grid.
                if changed {
                    if let Some(s) = fields.get_mut(&FL::Rotation) {
                        s.last_fetch = None;
                    }
                }
            });
        });
    }
    toggle(ui, fields, FL::Mesh, "MESH hail", "Max Estimated Size of Hail (MRMS)");
    toggle(ui, fields, FL::AzShear, "AzShear (0–2km)", "Instantaneous low-level azimuthal shear");
    toggle(ui, fields, FL::Lightning, "Lightning", "MRMS cloud-to-ground strike density, 5-min average (CONUS)");
    ui.separator();
    toggle(ui, fields, FL::Qpe1h, "QPE 1-hour", "MRMS multi-sensor 1-hour precip accumulation (mm)");
    toggle(ui, fields, FL::Qpe24h, "QPE 24-hour", "MRMS multi-sensor 24-hour precip accumulation (mm; storm total)");
    toggle(ui, fields, FL::PrecipType, "Precip type", "MRMS surface precipitation type (rain/snow/hail/convective)");
    toggle(ui, fields, FL::FlashFlood, "FLASH flood ARI", "MRMS FLASH flash-flood average recurrence interval (years)");
    toggle(ui, fields, FL::HailSwath, "Hail swaths (24 h)", "24-hour running max of MESH — hail damage tracks (≥0.75 in shown)");
}

fn hrrr_section(
    ui: &mut egui::Ui,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    hrrr_fcst_hour: &mut u8,
    hrrr_valid: Option<chrono::DateTime<chrono::Utc>>,
) {
    use crate::render::FieldLayer as FL;
    let on = if let Some(s) = fields.get_mut(&FL::Hrrr) {
        ui.checkbox(&mut s.show, "HRRR forecast reflectivity")
            .on_hover_text("Model composite reflectivity forecast (not observed) — scrub the forecast hour");
        s.show
    } else {
        false
    };
    if on {
        ui.add(egui::Slider::new(hrrr_fcst_hour, 0..=18).text("F+ hr"));
        match hrrr_valid {
            Some(v) => {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 170, 60),
                    format!("FORECAST +{}h — valid {}", hrrr_fcst_hour, v.format("%a %H:%MZ")),
                );
            }
            None => {
                ui.weak("loading forecast…");
            }
        }
    }
}

fn env_section(
    ui: &mut egui::Ui,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    env_cape_ml: &mut bool,
    env_srh_km: &mut u8,
) {
    use crate::render::FieldLayer as FL;
    // CAPE toggle + surface/mixed-layer parcel select.
    let cape_on = if let Some(s) = fields.get_mut(&FL::Cape) {
        ui.checkbox(&mut s.show, "CAPE")
            .on_hover_text("HRRR convective available potential energy (analysis f00)");
        s.show
    } else {
        false
    };
    if cape_on {
        ui.indent("cape_parcel", |ui| {
            ui.horizontal(|ui| {
                ui.label("Parcel:");
                let mut changed = ui.selectable_value(env_cape_ml, false, "SB").changed();
                changed |= ui.selectable_value(env_cape_ml, true, "ML").changed();
                if changed {
                    if let Some(s) = fields.get_mut(&FL::Cape) {
                        s.last_fetch = None;
                    }
                }
            });
        });
    }
    // SRH toggle + 0-1/0-3 km depth select.
    let srh_on = if let Some(s) = fields.get_mut(&FL::Srh) {
        ui.checkbox(&mut s.show, "Storm-relative helicity")
            .on_hover_text("HRRR SRH (analysis f00)");
        s.show
    } else {
        false
    };
    if srh_on {
        ui.indent("srh_depth", |ui| {
            ui.horizontal(|ui| {
                ui.label("Depth:");
                let mut changed = ui.selectable_value(env_srh_km, 1u8, "0–1 km").changed();
                changed |= ui.selectable_value(env_srh_km, 3u8, "0–3 km").changed();
                if changed {
                    if let Some(s) = fields.get_mut(&FL::Srh) {
                        s.last_fetch = None;
                    }
                }
            });
        });
    }
}

fn overlays_section(ui: &mut egui::Ui, filters: &mut OverlayFilters, actions: &mut ToolboxActions) {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("SPC Outlook:");
        for day in 0u8..=3 {
            let label = if day == 0 { "Off".to_string() } else { format!("D{day}") };
            changed |= ui.selectable_value(&mut filters.outlook_day, day, label).changed();
        }
    });
    // Day-1 hazard sub-select (probabilistic tornado/wind/hail); Days 2–3 are categorical only.
    if filters.outlook_day == 1 {
        ui.indent("outlook_kind", |ui| {
            ui.horizontal(|ui| {
                ui.label("Hazard:");
                for kind in wxdata::spc::OutlookKind::ALL {
                    if ui.selectable_value(&mut filters.outlook_kind, kind, kind.label()).changed() {
                        actions.outlook_kind_changed = true;
                        changed = true;
                    }
                }
            });
        });
    }
    changed |= ui.checkbox(&mut filters.show_mds, "Mesoscale Discussions").changed();
    ui.checkbox(&mut filters.show_cells, "Storm cells (L3)")
        .on_hover_text("Clickable storm tracking / hail / mesocyclone markers");
    if filters.show_cells {
        ui.indent("track_toggle", |ui| {
            ui.checkbox(&mut filters.show_tracks, "SCIT forecast tracks")
                .on_hover_text("15/30/45/60-min projected storm positions");
            ui.checkbox(&mut filters.show_arrival_cones, "Arrival-time cones")
                .on_hover_text("Project cell motion forward + ETA to your saved markers");
        });
    }
    ui.checkbox(&mut filters.show_nowcast, "Nowcast (echo extrapolation)")
        .on_hover_text("Advect the current reflectivity echo forward by the mean storm motion");
    if filters.show_nowcast {
        ui.indent("nowcast_lead", |ui| {
            ui.horizontal(|ui| {
                ui.label("Lead:");
                for m in [15u8, 30, 45] {
                    ui.selectable_value(&mut filters.nowcast_lead_min, m, format!("{m}m"));
                }
            });
        });
    }
    ui.checkbox(&mut filters.show_tds, "TDS detection")
        .on_hover_text("Auto-flag tornado debris signatures: low CC (ρhv) collocated with high reflectivity. Needs a dual-pol volume.");
    changed |= ui.checkbox(&mut filters.show_alerts, "NWS Alerts").changed();
    if filters.show_alerts {
        ui.indent("alert_cats", |ui| {
            for cat in Category::ALL {
                changed |= ui
                    .checkbox(&mut filters.alert_cats[cat.index()], cat.label())
                    .changed();
            }
        });
    }
    // |= — the Tropical/ProbSevere checkboxes above this call already set the flag.
    actions.overlays_changed |= changed;
}

fn site_section(ui: &mut egui::Ui, view: &mut MapView, settings: &mut Settings, actions: &mut ToolboxActions) {
    match &view.site {
        Some(id) => {
            let loc = wxdata::sites::site_by_id(id)
                .map(|s| format!("{}, {}", s.city, s.state))
                .unwrap_or_default();
            ui.strong(id);
            if !loc.is_empty() {
                ui.label(loc);
            }
        }
        None => {
            ui.weak("(none selected)");
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Select…").clicked() {
            actions.open_site_dialog = true;
        }
        if ui.button("Home").clicked() {
            view.site = Some(settings.default_site.clone());
        }
        if ui.button("None").clicked() {
            view.site = None;
        }
    });
    if !settings.presets.is_empty() {
        ui.horizontal(|ui| {
            ui.label("Preset:");
            egui::ComboBox::from_id_salt("preset_combo")
                .selected_text("choose")
                .show_ui(ui, |ui| {
                    let presets = settings.presets.clone();
                    for p in presets {
                        if ui.selectable_label(false, &p).clicked() {
                            view.site = Some(p);
                        }
                    }
                });
        });
    }
}

fn vcp_section(ui: &mut egui::Ui, view: &MapView) {
    ui.horizontal(|ui| {
        ui.label("VCP:");
        match &view.volume {
            Some(v) => ui.strong(&v.vcp),
            None => ui.weak("—"),
        };
    });
}

fn map_section(ui: &mut egui::Ui, view: &mut MapView, settings: &mut Settings) {
    use crate::settings::StartView;
    use crate::tiles::BasemapStyle;
    let (mb, mt) = (!settings.mapbox_key.is_empty(), !settings.maptiler_key.is_empty());
    egui::ComboBox::from_label("Background")
        .selected_text(view.basemap.label())
        .show_ui(ui, |ui| {
            // Only styles whose provider key is set are selectable.
            for s in BasemapStyle::ALL.into_iter().filter(|s| s.available(mb, mt)) {
                if ui.selectable_value(&mut view.basemap, s, s.label()).clicked() {
                    settings.basemap = s.slug().to_string(); // persist across restarts
                }
            }
        });
    if !mb || !mt {
        ui.weak("(add Mapbox/MapTiler keys in Settings for more)");
    }
    ui.weak("(press Z to cycle)");
    ui.checkbox(&mut view.smooth, "Smooth radar data");
    ui.add_enabled(false, egui::Checkbox::new(&mut false, "Track location"));

    // Startup view: remember this site + camera as the launch position.
    ui.separator();
    if ui.button("Save as startup view")
        .on_hover_text("Open here (site + map position) on next launch")
        .clicked()
    {
        if let Some(site) = &view.site {
            settings.start_view = Some(StartView {
                site: site.clone(),
                x: view.camera.center.0,
                y: view.camera.center.1,
                zoom: view.camera.zoom,
            });
        }
    }
    if let Some(site) = settings.start_view.as_ref().map(|sv| sv.site.clone()) {
        let mut clear = false;
        ui.horizontal(|ui| {
            ui.weak(format!("Starts at {site}"));
            clear = ui.small_button("Clear").clicked();
        });
        if clear {
            settings.start_view = None;
        }
    }
}

fn level2_section(ui: &mut egui::Ui, view: &mut MapView, settings: &mut Settings, actions: &mut ToolboxActions) {
    ui.horizontal_wrapped(|ui| {
        for m in Moment::ALL {
            ui.selectable_value(&mut view.moment, m, m.short_name());
        }
    });

    // Tilt buttons from the loaded volume.
    ui.label("Tilt:");
    if let Some(v) = &view.volume {
        let elevations = v.elevations.clone();
        ui.horizontal_wrapped(|ui| {
            for (i, angle) in elevations.iter().enumerate() {
                ui.selectable_value(&mut view.tilt, i, format!("{angle:.1}°"));
            }
        });
    } else {
        ui.weak("(no volume)");
    }

    // Storm-relative velocity (velocity moment only).
    if view.moment == Moment::Velocity {
        ui.checkbox(&mut settings.dealias_velocity, "Dealias")
            .on_hover_text("Unfold aliased velocity (region-based dealiasing)");
        ui.horizontal(|ui| {
            ui.checkbox(&mut view.srv, "Storm-relative");
        });
        if view.srv {
            ui.horizontal(|ui| {
                ui.label("Motion:");
                ui.add(egui::DragValue::new(&mut view.storm_dir_deg).range(0.0..=359.0).suffix("°"));
                ui.add(egui::DragValue::new(&mut view.storm_speed_kt).range(0.0..=150.0).suffix(" kt"));
            });
            if ui.button("From storm cells")
                .on_hover_text("Set motion to the SCIT storm-cell mean (needs L3 storm cells)")
                .clicked()
            {
                actions.srv_from_cells = true;
            }
        }
    }

    // Threshold for the active moment. The slider value stays internal (m/s for velocity);
    // display honors the Units setting.
    let i = view.moment.index();
    let (vmin, vmax) = view.moment.value_range();
    let (factor, label) = crate::app::display_units(view.moment, settings);
    let f = factor as f64;
    ui.horizontal(|ui| {
        ui.checkbox(&mut view.threshold_enabled[i], "Threshold");
        if view.threshold_enabled[i] {
            let t = view.thresholds[i].get_or_insert((vmin + vmax) * 0.5);
            ui.add(
                egui::Slider::new(t, vmin..=vmax)
                    .custom_formatter(move |v, _| format!("{:.0}", v * f))
                    .custom_parser(move |s| s.parse::<f64>().ok().map(|x| x / f))
                    .suffix(label),
            );
        }
    });
}

fn level3_section(
    ui: &mut egui::Ui,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    l3grid_site: Option<&str>,
) {
    use crate::render::FieldLayer as FL;
    ui.label("Storm cells: Storm Tracking, Hail (HDA), Mesocyclone");
    ui.weak("Toggle in Overlays ▸ Storm cells; click a marker to interrogate.");
    ui.separator();
    // Gridded L3 products for the active site (packet 16 digital radial arrays).
    if let Some(s) = fields.get_mut(&FL::Vil) {
        ui.checkbox(&mut s.show, "Digital VIL (DVL)")
            .on_hover_text("Gridded vertically-integrated liquid for the active site (kg/m²)");
    }
    if let Some(s) = fields.get_mut(&FL::EchoTops) {
        ui.checkbox(&mut s.show, "Echo tops (EET)")
            .on_hover_text("Enhanced echo tops for the active site (kft)");
    }
    if let Some(s) = fields.get_mut(&FL::Hca) {
        ui.checkbox(&mut s.show, "Hydrometeor class (HHC)")
            .on_hover_text("What the radar thinks it sees: rain / snow / hail / graupel / biological …");
    }
    if [FL::Vil, FL::EchoTops, FL::Hca].iter().any(|l| fields.get(l).is_some_and(|s| s.show)) {
        ui.weak(format!("Site: {}", l3grid_site.unwrap_or("—")));
    }
}

fn timeline_section(ui: &mut egui::Ui, view: &mut MapView, settings: &mut Settings, actions: &mut ToolboxActions) {
    let t = &mut view.timeline;

    ui.horizontal(|ui| {
        // Live indicator: green when pinned to a fresh head.
        let fresh = view
            .volume
            .as_ref()
            .map(|v| (chrono::Utc::now() - v.time).num_seconds() < 900)
            .unwrap_or(false);
        let color = if t.following && fresh {
            egui::Color32::from_rgb(0, 220, 0)
        } else if t.following {
            egui::Color32::from_rgb(220, 180, 0)
        } else {
            egui::Color32::from_gray(90)
        };
        ui.colored_label(color, "●");
        ui.label(if t.following { "Live" } else { "Archive" });
        if ui.button("Reload").clicked() {
            actions.reload = true;
        }
    });

    // Archive day picker (UTC). Prev/Next shift a day; a live jump re-pins to the head.
    ui.horizontal(|ui| {
        ui.label("Date:");
        if ui.button("◀").clicked() {
            if let Some(d) = t.date.pred_opt() {
                t.date = d;
                t.following = false;
            }
        }
        ui.monospace(t.date.format("%Y-%m-%d").to_string());
        let is_today = t.date >= chrono::Utc::now().date_naive();
        if ui.add_enabled(!is_today, egui::Button::new("▶")).clicked() {
            if let Some(d) = t.date.succ_opt() {
                t.date = d;
            }
        }
    });

    // Transport.
    ui.horizontal(|ui| {
        if ui.button("⏮").on_hover_text("First").clicked() {
            t.go_begin();
        }
        if ui.button("◀").on_hover_text("Step back").clicked() {
            t.step(-1);
        }
        let play_label = if t.playing { "⏸" } else { "▶" };
        if ui.button(play_label).on_hover_text("Play/Pause (live loops the newest frames)").clicked() {
            t.toggle_play();
        }
        if ui.button("▶|").on_hover_text("Step forward").clicked() {
            t.step(1);
        }
        if ui.button("⏭").on_hover_text("Live head").clicked() {
            t.go_head();
        }
        if ui.button("⟲ DVR").on_hover_text("Instant replay: loop the frames buffered in memory (R)").clicked() {
            actions.instant_replay = true;
        }
    });

    // Scrub bar over observed frames + the HRRR forecast tail.
    if !t.frames.is_empty() {
        let observed = t.frames.len();
        let last = t.slot_count().saturating_sub(1);
        let mut ph = t.playhead;
        let resp = ui.add(egui::Slider::new(&mut ph, 0..=last).show_value(false));
        if resp.changed() {
            t.playhead = ph;
            t.playing = false;
            t.following = ph + 1 == observed;
        }
        // Readout: observed frame time, or the forecast hour in the tail.
        match t.forecast_hour() {
            Some(h) => {
                ui.colored_label(egui::Color32::from_rgb(255, 170, 60), format!("▶ FORECAST  F+{h}h  (HRRR)"));
            }
            None => {
                let when = t.current().and_then(|id| id.date_time()).map(|d| d.format("%H:%M:%SZ").to_string());
                ui.label(format!("{} / {} observed  {}", t.playhead + 1, observed, when.unwrap_or_default()));
            }
        }
    } else if t.listing {
        ui.weak("listing volumes…");
    } else {
        ui.weak("(no volumes for this day)");
    }

    ui.horizontal(|ui| {
        ui.checkbox(&mut t.loop_enabled, "Loop");
        ui.label("Speed");
        ui.add(egui::Slider::new(&mut t.speed, 1.0..=15.0).suffix(" fps").show_value(true));
    });
    ui.horizontal(|ui| {
        ui.label("Live loop window");
        ui.add(egui::DragValue::new(&mut settings.live_loop_frames).range(2..=30).suffix(" frames"))
            .on_hover_text("How many of the newest volumes ▶ cycles through when live");
    });
}
