//! The left-dock Radar Toolbox: site, map style, products and tilts, overlays, timeline.

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
    /// Start an offline chase-pack download of the current view's basemap.
    pub download_chasepack: bool,
    /// Cancel the in-progress chase-pack download.
    pub cancel_chasepack: bool,
    /// A row in the embedded layers registry was clicked; the app applies it.
    pub(crate) palette: Option<crate::app::PaletteAction>,
}

/// Read-only chase-pack state the app feeds the toolbox each frame: the current-view estimate and,
/// while a download runs, its progress `(done, total, errors, mb)`.
pub struct ChasePackUi {
    pub tiles: u64,
    pub mb: f64,
    /// The active basemap can be pre-downloaded (raster with a URL, or vector once its template loads).
    pub packable: bool,
    pub z_lo: u8,
    pub z_hi: u8,
    pub progress: Option<(u64, u64, u64, f64)>,
}

#[allow(clippy::too_many_arguments)] // one flat call per frame; a params struct adds churn for no reader gain
pub(crate) fn show(
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
    contour_kind: &mut crate::app::ContourKind,
    l3grid_site: Option<&str>,
    // The layer registry to embed, or `None` to omit the Layers section entirely — the mobile
    // drawer already leads with the same list and nests this panel under "Advanced".
    layers: Option<&[crate::app::PaletteEntry]>,
    layers_query: &mut String,
    accent: egui::Color32,
    chasepack: &ChasePackUi,
) -> ToolboxActions {
    use crate::theme::section;
    let mut actions = ToolboxActions::default();
    let tz = settings.tz_for(view.site.as_deref());
    egui::ScrollArea::vertical().show(ui, |ui| {
        section(ui, "Site", |ui| {
            site_section(ui, view, settings, &mut actions)
        });
        section(ui, "Map", |ui| {
            map_section(ui, view, settings, chasepack, &mut actions)
        });
        section(ui, "Product", |ui| {
            product_section(ui, view, settings, &mut actions)
        });
        if let Some(entries) = layers {
            section(ui, "Layers", |ui| {
                if let Some(a) =
                    crate::ui::layers_panel::body(ui, entries, layers_query, accent, 420.0, false)
                {
                    actions.palette = Some(a);
                }
            });
        }
        section(ui, "Layer Options", |ui| {
            layer_options_section(
                ui,
                filters,
                fields,
                rotation_minutes,
                hrrr_fcst_hour,
                hrrr_valid,
                tz,
                env_cape_ml,
                env_srh_km,
                contour_kind,
                l3grid_site,
                &mut actions,
            )
        });
        section(ui, "Timeline", |ui| {
            timeline_section(ui, view, settings, &mut actions)
        });
    });
    actions
}

/// Settings that shape a layer you already turned on. Only the ones whose layer is active render,
/// so this section is short — usually empty — instead of a wall of controls for things you can't see.
#[allow(clippy::too_many_arguments)]
fn layer_options_section(
    ui: &mut egui::Ui,
    filters: &mut OverlayFilters,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    rotation_minutes: &mut u16,
    hrrr_fcst_hour: &mut u8,
    hrrr_valid: Option<chrono::DateTime<chrono::Utc>>,
    tz: Option<wxdata::tz::Tz>,
    env_cape_ml: &mut bool,
    env_srh_km: &mut u8,
    contour_kind: &mut crate::app::ContourKind,
    l3grid_site: Option<&str>,
    actions: &mut ToolboxActions,
) {
    use crate::render::FieldLayer as FL;
    let mut changed = false;

    // SPC outlook: a four-way day selector whose own "Off" is the off-state, so it can't wear the
    // registry's ON/OFF pill. It lives here rather than in the layer list.
    ui.horizontal(|ui| {
        ui.label("SPC Outlook:");
        for day in 0u8..=3 {
            let label = if day == 0 {
                "Off".to_string()
            } else {
                format!("D{day}")
            };
            changed |= ui
                .selectable_value(&mut filters.outlook_day, day, label)
                .changed();
        }
    });
    // Day-1 hazard sub-select (probabilistic tornado/wind/hail); Days 2–3 are categorical only.
    if filters.outlook_day == 1 {
        ui.indent("outlook_kind", |ui| {
            ui.horizontal(|ui| {
                ui.label("Hazard:");
                for kind in wxdata::spc::OutlookKind::ALL {
                    if ui
                        .selectable_value(&mut filters.outlook_kind, kind, kind.label())
                        .changed()
                    {
                        actions.outlook_kind_changed = true;
                        changed = true;
                    }
                }
            });
        });
    }

    // HRRR model contours (isolines) — MSLP / 2 m temp / dewpoint / SB-CAPE / 0-3 km SRH.
    egui::ComboBox::from_label("Contours")
        .selected_text(contour_kind.label())
        .show_ui(ui, |ui| {
            for k in crate::app::ContourKind::ALL {
                ui.selectable_value(contour_kind, k, k.label());
            }
        })
        .response
        .on_hover_text("Draw an HRRR surface field as labeled contour lines (analysis f00)");

    // Everything below belongs to a layer that has to be on for it to mean anything.
    let header = |ui: &mut egui::Ui, text: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).small().strong());
    };

    if fields.get(&FL::Rotation).is_some_and(|s| s.show) {
        header(ui, "Rotation tracks");
        ui.horizontal(|ui| {
            ui.label("Window:");
            let mut dur = false;
            for m in [30u16, 60, 120] {
                dur |= ui
                    .selectable_value(rotation_minutes, m, format!("{m}m"))
                    .changed();
            }
            // Duration change → force an immediate refetch of the rotation grid.
            if dur {
                if let Some(s) = fields.get_mut(&FL::Rotation) {
                    s.last_fetch = None;
                }
            }
        });
    }

    if fields.get(&FL::Hrrr).is_some_and(|s| s.show) {
        header(ui, "Future radar");
        ui.add(egui::Slider::new(hrrr_fcst_hour, 0..=18).text("F+ hr"));
        match hrrr_valid {
            Some(v) => {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 170, 60),
                    format!(
                        "FORECAST +{}h — valid {}",
                        hrrr_fcst_hour,
                        crate::timefmt::fmt_date_clock(v, tz)
                    ),
                );
            }
            None => {
                ui.weak("loading forecast…");
            }
        }
    }

    if fields.get(&FL::Cape).is_some_and(|s| s.show) {
        header(ui, "CAPE");
        ui.horizontal(|ui| {
            ui.label("Parcel:");
            let mut c = ui.selectable_value(env_cape_ml, false, "SB").changed();
            c |= ui.selectable_value(env_cape_ml, true, "ML").changed();
            if c {
                if let Some(s) = fields.get_mut(&FL::Cape) {
                    s.last_fetch = None;
                }
            }
        });
    }

    if fields.get(&FL::Srh).is_some_and(|s| s.show) {
        header(ui, "Storm-relative helicity");
        ui.horizontal(|ui| {
            ui.label("Depth:");
            let mut c = ui.selectable_value(env_srh_km, 1u8, "0–1 km").changed();
            c |= ui.selectable_value(env_srh_km, 3u8, "0–3 km").changed();
            if c {
                if let Some(s) = fields.get_mut(&FL::Srh) {
                    s.last_fetch = None;
                }
            }
        });
    }

    if filters.show_cells {
        header(ui, "Storm cells");
        ui.checkbox(&mut filters.show_tracks, "SCIT forecast tracks")
            .on_hover_text("15/30/45/60-min projected storm positions");
        ui.checkbox(&mut filters.show_arrival_cones, "Arrival-time cones")
            .on_hover_text("Project cell motion forward + ETA to your saved markers");
    }

    if filters.show_nowcast {
        header(ui, "Nowcast");
        ui.horizontal(|ui| {
            ui.label("Lead:");
            for m in [15u8, 30, 45] {
                ui.selectable_value(&mut filters.nowcast_lead_min, m, format!("{m}m"));
            }
        });
    }

    if filters.show_alerts {
        header(ui, "NWS Alerts");
        for cat in Category::ALL {
            changed |= ui
                .checkbox(&mut filters.alert_cats[cat.index()], cat.label())
                .changed();
        }
    }

    if [FL::Vil, FL::EchoTops, FL::Hca]
        .iter()
        .any(|l| fields.get(l).is_some_and(|s| s.show))
    {
        header(ui, "Level 3 grids");
        ui.weak(format!("Site: {}", l3grid_site.unwrap_or("—")));
    }

    actions.overlays_changed |= changed;
}
fn site_section(
    ui: &mut egui::Ui,
    view: &mut MapView,
    settings: &mut Settings,
    actions: &mut ToolboxActions,
) {
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
    // The scan pattern the radar is currently running — one line, not its own section.
    if let Some(v) = &view.volume {
        ui.weak(format!("Scanning: {}", v.vcp));
    }
}

fn map_section(
    ui: &mut egui::Ui,
    view: &mut MapView,
    settings: &mut Settings,
    chasepack: &ChasePackUi,
    actions: &mut ToolboxActions,
) {
    use crate::settings::StartView;
    use crate::tiles::BasemapStyle;
    let (mb, mt) = (
        !settings.mapbox_key.is_empty(),
        !settings.maptiler_key.is_empty(),
    );
    egui::ComboBox::from_label("Background")
        .selected_text(view.basemap.label())
        .show_ui(ui, |ui| {
            // Only styles whose provider key is set are selectable.
            for s in BasemapStyle::ALL
                .into_iter()
                .filter(|s| s.available(mb, mt))
            {
                if ui
                    .selectable_value(&mut view.basemap, s, s.label())
                    .clicked()
                {
                    settings.basemap = s.slug().to_string(); // persist across restarts
                }
            }
        });
    ui.weak(if mb && mt {
        "Z cycles backgrounds".to_string()
    } else {
        "Z cycles backgrounds · more styles with Mapbox/MapTiler keys in Settings".to_string()
    });
    ui.checkbox(&mut view.smooth, "Smooth radar data");

    // A download in flight stays above the disclosure — progress you can't find reads as a hang.
    if let Some((done, total, errors, mb)) = chasepack.progress {
        ui.separator();
        let frac = if total > 0 {
            done as f32 / total as f32
        } else {
            1.0
        };
        ui.add(egui::ProgressBar::new(frac).text(format!("{done}/{total} tiles · {mb:.0} MB")));
        if errors > 0 {
            ui.colored_label(
                egui::Color32::from_rgb(230, 120, 60),
                format!("{errors} failed"),
            );
        }
        if ui.button("Cancel download").clicked() {
            actions.cancel_chasepack = true;
        }
        return;
    }

    ui.collapsing("Startup & offline", |ui| {
        // Startup view: remember this site + camera as the launch position.
        if ui
            .button("Save as startup view")
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

        // Offline chase pack: pre-cache this view's basemap tiles so it renders with no signal.
        ui.separator();
        if !chasepack.packable {
            ui.weak("Offline pack: pick a raster or vector basemap");
        } else {
            ui.weak(format!(
                "Offline pack: {} tiles ≈ {:.0} MB (z{}–{}, current view)",
                chasepack.tiles, chasepack.mb, chasepack.z_lo, chasepack.z_hi
            ));
            let too_big = chasepack.mb > 2000.0;
            if ui
                .add_enabled(!too_big, egui::Button::new("⬇ Download offline pack"))
                .on_hover_text(
                    "Cache this view's basemap tiles to disk for offline use in the field",
                )
                .clicked()
            {
                actions.download_chasepack = true;
            }
            if too_big {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 90, 90),
                    "Too large (>2 GB) — zoom in or narrow the view",
                );
            }
        }
    });
}

fn product_section(
    ui: &mut egui::Ui,
    view: &mut MapView,
    settings: &mut Settings,
    actions: &mut ToolboxActions,
) {
    ui.horizontal_wrapped(|ui| {
        for p in &crate::products::PRODUCTS {
            ui.selectable_value(&mut view.moment, p.moment, p.short)
                .on_hover_text(format!("{} — {}", p.name, p.blurb));
        }
    });

    // Tilt buttons from the loaded volume.
    ui.label("Tilt:")
        .on_hover_text("How high the radar beam is aimed — 0.5° is closest to the ground");
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

    // Expert knobs behind a disclosure: picking a product and a tilt is the whole job for most
    // people, and burying these keeps the panel readable without losing them.
    ui.collapsing("Options", |ui| {
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
                    ui.add(
                        egui::DragValue::new(&mut view.storm_dir_deg)
                            .range(0.0..=359.0)
                            .suffix("°"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut view.storm_speed_kt)
                            .range(0.0..=150.0)
                            .suffix(" kt"),
                    );
                });
                if ui
                    .button("From storm cells")
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
            ui.checkbox(&mut view.threshold_enabled[i], "Threshold")
                .on_hover_text(
                    "Hide everything below a value — cuts light rain out of the picture",
                );
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
    });
}

fn timeline_section(
    ui: &mut egui::Ui,
    view: &mut MapView,
    settings: &mut Settings,
    actions: &mut ToolboxActions,
) {
    let tz = settings.tz_for(view.site.as_deref());
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
        ui.monospace(t.date.format("%Y-%m-%d").to_string())
            .on_hover_text("Archive days are UTC days — the S3 buckets are bucketed that way");
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
        if ui
            .button(play_label)
            .on_hover_text("Play/Pause (live loops the newest frames)")
            .clicked()
        {
            t.toggle_play();
        }
        if ui.button("▶|").on_hover_text("Step forward").clicked() {
            t.step(1);
        }
        if ui.button("⏭").on_hover_text("Live head").clicked() {
            t.go_head();
        }
        if ui
            .button("⟲ DVR")
            .on_hover_text("Instant replay: loop the frames buffered in memory (R)")
            .clicked()
        {
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
                ui.colored_label(
                    egui::Color32::from_rgb(255, 170, 60),
                    format!("▶ FORECAST  F+{h}h  (HRRR)"),
                );
            }
            None => {
                let when = t
                    .current()
                    .and_then(|id| id.date_time())
                    .map(|d| crate::timefmt::fmt_clock(d, tz, true));
                ui.label(format!(
                    "{} / {} observed  {}",
                    t.playhead + 1,
                    observed,
                    when.unwrap_or_default()
                ));
            }
        }
    } else if t.listing {
        ui.weak("listing volumes…");
    } else {
        ui.weak("(no volumes for this day)");
    }

    ui.horizontal(|ui| {
        ui.checkbox(&mut t.loop_enabled, "Loop");
        ui.add(
            egui::Slider::new(&mut t.speed, 1.0..=15.0)
                .suffix(" fps")
                .show_value(true),
        );
        ui.add(
            egui::DragValue::new(&mut settings.live_loop_frames)
                .range(2..=30)
                .suffix(" frames"),
        )
        .on_hover_text("How many of the newest volumes ▶ cycles through when live");
    });
}
