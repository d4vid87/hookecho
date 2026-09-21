//! Settings that shape a layer you already turned on, plus the action struct the chrome raises.
//!
//! This is what survived the Advanced toolbox: the toolbox's Site / Map / Product / Timeline
//! sections were all fourth copies of controls the site dialog, the drawer, the product pill and
//! the timeline pill already own. Only the per-layer knobs had no other home, so they moved into
//! the drawer's "Layer options" section — and, like before, a knob only renders when its layer is
//! actually on, so the section is short (usually empty) instead of a wall of dead controls.

use crate::app::OverlayFilters;
use wxdata::alerts::Category;

/// Signals the chrome (drawer, pills, mobile sheets) raises for the app to act on this frame.
#[derive(Default)]
pub struct UiActions {
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
    /// The WSSI day changed; the app must clear + refetch it.
    pub wssi_day_changed: bool,
    /// The Excessive Rainfall Outlook day changed; the app must clear + refetch it.
    pub ero_day_changed: bool,
    /// Start an offline chase-pack download of the current view's basemap.
    pub download_chasepack: bool,
    /// Cancel the in-progress chase-pack download.
    pub cancel_chasepack: bool,
    /// A row in the embedded layers registry was clicked; the app applies it.
    pub(crate) palette: Option<crate::app::PaletteAction>,
    pub trail_changed: bool,
    pub export_trail: bool,
    pub export_local_tracks_csv: bool,
    pub export_local_tracks_json: bool,
}

/// Read-only chase-pack state the app feeds the UI each frame: the current-view estimate and,
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

/// Can STP be computed from this source? It needs an LCL height, which only the HRRR surface
/// file publishes — the RAP analysis and the NAM nest both leave it out.
fn stp_source(model: wxdata::hrrr::Model) -> bool {
    matches!(model, wxdata::hrrr::Model::Hrrr)
}

#[allow(clippy::too_many_arguments)] // one flat call per frame; a params struct adds churn for no reader gain
pub(crate) fn show(
    ui: &mut egui::Ui,
    filters: &mut OverlayFilters,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    // Which layers the active pane draws. Visibility is per-pane now; the map above is the shared
    // fetch state, which is what `fields` is still needed for (clearing a refetch clock).
    on: &std::collections::HashSet<crate::render::FieldLayer>,
    rotation_minutes: &mut u16,
    trail_minutes: &mut u16,
    trail_threshold: &mut f32,
    hail_minutes: &mut u16,
    hrrr_fcst_hour: &mut u8,
    refs_fcst_hour: &mut u8,
    hrrr_valid: Option<chrono::DateTime<chrono::Utc>>,
    tz: Option<wxdata::tz::Tz>,
    env_cape_ml: &mut bool,
    env_srh_km: &mut u8,
    env_model: &mut wxdata::hrrr::Model,
    contour_kind: &mut crate::app::ContourKind,
    etop_dbz: &mut f32,
    snow_hours: &mut u16,
    show_tropical: &bool,
    tropical_wind_kt: &mut Option<u8>,
    tropical_surge: &mut bool,
    l3grid_site: Option<&str>,
    // Global models: which one, and how far into its run.
    global_model: &mut wxdata::global::GlobalModel,
    global_fcst_hour: &mut u16,
    // Model difference: which field, and the two valid times the last fetch actually compared.
    diff_field: &mut crate::fielddiff::DiffField,
    diff_valid: Option<&(String, String)>,
    // Lightning: NLDN averaging window, and whether GLM also polls GOES-West.
    lightning_minutes: &mut u16,
    show_glm: bool,
    glm_goes_west: &mut bool,
    abi_scene: &mut wxdata::abi::Scene,
    // Spotter Network dots: on-state, and how far from the radar to draw them (0 = whole feed).
    show_spotters: bool,
    show_local_tracks: bool,
    spotter_range_km: &mut f64,
    // Where the signature detectors draw their lines; only rendered for the ones that are on.
    detectors: &mut crate::settings::DetectorTuning,
    // One line about the live composite: contributing sites and the age of its oldest scan, or
    // why there isn't one. Radars scan on their own schedules, so a composite is always a little
    // ragged in time and the honest thing is to show by how much.
    mosaic: Option<&str>,
    actions: &mut UiActions,
) {
    use crate::render::FieldLayer as FL;
    let mut changed = false;

    let global_on = [
        FL::GlobalMslp,
        FL::GlobalHeight500,
        FL::GlobalTemp2m,
        FL::GlobalDewpoint2m,
        FL::GlobalWind10m,
        FL::GlobalPrecip,
    ]
    .iter()
    .any(|l| on.contains(l));
    let satellite_on = on.iter().any(|layer| {
        layer.descriptor().is_some_and(|descriptor| {
            descriptor.family == wxdata::field::FieldFamily::Satellite
        })
    });
    let sections = [
        ("Storm cells", filters.show_cells),
        ("Alerts", filters.show_alerts),
        ("Tropical", *show_tropical),
        ("Outlooks", true),
        ("Environment", true),
        ("Global forecast", global_on),
        ("Model comparison", on.contains(&FL::ModelDiff)),
        ("Lightning", show_glm || on.contains(&FL::Lightning)),
        ("Satellite", satellite_on),
        ("Spotters", show_spotters),
        ("Local cell tracks", show_local_tracks),
        ("Rotation tracks", on.contains(&FL::Rotation)),
        ("Reflectivity trail", on.contains(&FL::MrmsReflectivityTrail)),
        ("Hail swaths", on.contains(&FL::HailSwath)),
        ("Radar mosaic", on.contains(&FL::Mosaic)),
        (
            "Data details",
            crate::render::FieldLayer::DRAW_ORDER.iter().any(|layer| {
                on.contains(layer)
                    && fields
                        .get(layer)
                        .and_then(|state| state.frame.as_ref())
                        .is_some()
            }),
        ),
        ("Future radar", on.contains(&FL::Hrrr)),
        ("Ensemble forecast", on.contains(&FL::RefsReflectivityProb)),
        ("Nowcast", filters.show_nowcast),
        ("Snowfall", on.contains(&FL::SnowAnalysis)),
        (
            "Derived radar",
            [FL::VilLocal, FL::VilDensity, FL::EtopLocal]
                .iter()
                .any(|l| on.contains(l)),
        ),
        ("Detectors", filters.show_tbss || filters.show_zdr_columns),
        (
            "Level 3 grids",
            [FL::Vil, FL::EchoTops, FL::Hca]
                .iter()
                .any(|l| on.contains(l)),
        ),
    ];
    let id = ui.id().with("layer_settings_section");
    let remembered = ui.ctx().data_mut(|d| d.get_temp::<&'static str>(id));
    let mut section = remembered
        .filter(|name| sections.iter().any(|(s, on)| s == name && *on))
        .unwrap_or(sections.iter().find(|(_, on)| *on).unwrap().0);
    ui.spacing_mut().item_spacing.y = 8.0;
    egui::ComboBox::from_id_salt("settings_for")
        .width(ui.available_width() - 8.0)
        .selected_text(section)
        .show_ui(ui, |ui| {
            for (name, visible) in sections {
                if visible {
                    ui.selectable_value(&mut section, name, name);
                }
            }
        })
        .response
        .on_hover_text("Choose a layer to adjust");
    ui.ctx().data_mut(|d| d.insert_temp(id, section));
    ui.add_space(4.0);
    if section == "Data details" {
        if let Some((descriptor, grid, stamp)) = crate::render::FieldLayer::DRAW_ORDER
            .iter()
            .rev()
            .find(|layer| on.contains(layer))
            .and_then(|layer| {
                let descriptor = layer.descriptor()?;
                let (grid, stamp) = fields.get(layer)?.metadata.as_ref()?;
                Some((descriptor, grid, stamp))
            })
        {
            ui.label(egui::RichText::new(descriptor.display_name).strong());
            egui::Grid::new("field_provenance")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.weak("Source");
                    ui.label(descriptor.source);
                    ui.end_row();
                    ui.weak("Valid");
                    ui.label(stamp.valid_time.format("%Y-%m-%d %H:%M UTC").to_string());
                    ui.end_row();
                    ui.weak("Issue");
                    ui.label(
                        stamp
                            .issue_time
                            .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_else(|| "Unknown".into()),
                    );
                    ui.end_row();
                    ui.weak("Run");
                    ui.label(
                        stamp
                            .run_time
                            .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
                            .unwrap_or_else(|| "Unknown".into()),
                    );
                    ui.end_row();
                    ui.weak("Received");
                    ui.label(
                        stamp
                            .received_time
                            .format("%Y-%m-%d %H:%M:%S UTC")
                            .to_string(),
                    );
                    ui.end_row();
                    ui.weak("Class");
                    ui.label(stamp.class.label());
                    ui.end_row();
                    ui.weak("Quality");
                    ui.label(stamp.quality.label());
                    ui.end_row();
                    if let Some(members) = stamp.available_members {
                        ui.weak("Members");
                        ui.label(members.to_string());
                        ui.end_row();
                    }
                    ui.weak("Units");
                    ui.label(descriptor.units);
                    ui.end_row();
                    ui.weak("Grid");
                    ui.label(format!("{} × {} · {}", grid.nx, grid.ny, grid.projection));
                    ui.end_row();
                    ui.weak("Resolution");
                    ui.label(
                        grid.native_resolution_m
                            .map(|metres| format!("{metres:.0} m"))
                            .unwrap_or_else(|| "Unknown".into()),
                    );
                    ui.end_row();
                    ui.weak("Sampling");
                    ui.label(descriptor.sampling.label());
                    ui.end_row();
                    ui.weak("Object");
                    ui.label(&stamp.source_identity);
                    ui.end_row();
                });
        }
    }
    if section == "Global forecast" && global_on {
        ui.horizontal(|ui| {
            ui.label("Global model:");
            for m in [
                wxdata::global::GlobalModel::Gfs,
                wxdata::global::GlobalModel::GefsMean,
                wxdata::global::GlobalModel::GefsSpread,
                wxdata::global::GlobalModel::Ecmwf,
            ] {
                changed |= ui.selectable_value(global_model, m, m.label()).changed();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Forecast hour:");
            changed |= ui
                .add(
                    egui::Slider::new(global_fcst_hour, 0..=120)
                        .step_by(3.0)
                        .suffix(" h"),
                )
                .on_hover_text("Three-hourly out to five days, from the newest complete cycle")
                .changed();
        });
    }

    if section == "Model comparison" && on.contains(&FL::ModelDiff) {
        let (a, b) = diff_field.pair();
        ui.horizontal_wrapped(|ui| {
            ui.label("Difference:");
            for f in crate::fielddiff::DiffField::ALL {
                changed |= ui.selectable_value(diff_field, f, f.label()).changed();
            }
        });
        ui.weak(format!(
            "{a} minus {b}, in {}. Red = {a} higher, blue = {b} higher; where they agree, nothing is drawn.",
            diff_field.units()
        ));
        match diff_valid {
            // The two models rarely share a cycle, and a difference between two instants is only
            // honest if it says which two.
            Some((va, vb)) if va != vb => {
                ui.weak(format!(
                    "⚠ {a} valid {va}, {b} valid {vb} — not the same time."
                ));
            }
            Some((va, _)) => {
                ui.weak(format!("Both valid {va}."));
            }
            None => {}
        }
    }

    if section == "Lightning" && on.contains(&FL::Lightning) {
        ui.horizontal(|ui| {
            ui.label("CG density window:");
            for m in [1u16, 5, 15, 30] {
                changed |= ui
                    .selectable_value(lightning_minutes, m, format!("{m}m"))
                    .changed();
            }
        });
        ui.weak("NLDN = cloud-to-ground only; GLM = total lightning (optical, in-cloud included).");
    }

    if section == "Lightning" && show_glm {
        changed |= crate::ui::style::toggle(ui, glm_goes_west, "Include GOES-West")
            .on_hover_text("Adds GOES-18 so the Pacific and the west coast are covered too")
            .changed();
    }

    if section == "Satellite" {
        let before = *abi_scene;
        ui.horizontal_wrapped(|ui| {
            ui.label("ABI sector:");
            for (scene, label) in [
                (wxdata::abi::Scene::Conus, "CONUS"),
                (wxdata::abi::Scene::Mesoscale1, "Mesoscale 1"),
                (wxdata::abi::Scene::Mesoscale2, "Mesoscale 2"),
            ] {
                changed |= ui.selectable_value(abi_scene, scene, label).changed();
            }
        });
        ui.weak("Mesoscale sectors update every minute and move with active weather.");
        if *abi_scene != before {
            for (layer, state) in fields.iter_mut() {
                if layer.descriptor().is_some_and(|descriptor| {
                    descriptor.family == wxdata::field::FieldFamily::Satellite
                }) {
                    state.last_fetch = None;
                }
            }
        }
    }

    if section == "Spotters" && show_spotters {
        ui.horizontal(|ui| {
            ui.label("Spotters within:");
            ui.add(
                egui::DragValue::new(spotter_range_km)
                    .range(0.0..=5000.0)
                    .speed(10.0)
                    .max_decimals(0)
                    .suffix(" km"),
            )
            .on_hover_text("Distance from the active radar. 0 draws the whole national feed.");
        });
    }

    if section == "Outlooks" {
        let accent = ui.visuals().selection.stroke.color;
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().window_stroke)
            .corner_radius(crate::ui::style::RADIUS_LG)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(egui_phosphor::regular::WARNING)
                            .size(22.0)
                            .color(accent),
                    );
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("SPC Convective Outlook").strong());
                        ui.weak("NOAA Storm Prediction Center");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(
                            if filters.outlook_day == 0 {
                                ui.visuals().weak_text_color()
                            } else {
                                accent
                            },
                            if filters.outlook_day == 0 {
                                "OFF"
                            } else {
                                "ON"
                            },
                        );
                    });
                });
                ui.separator();
                ui.label(egui::RichText::new("Forecast day").small().strong());
                ui.horizontal_wrapped(|ui| {
                    for day in 1u8..=8 {
                        if ui
                            .selectable_label(filters.outlook_day == day, format!("Day {day}"))
                            .clicked()
                        {
                            filters.outlook_day = if filters.outlook_day == day { 0 } else { day };
                            changed = true;
                        }
                    }
                });

                // Day 1 is the only outlook with separate tornado, wind and hail probabilities.
                if filters.outlook_day == 1 {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Layer").small().strong());
                    ui.horizontal_wrapped(|ui| {
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
                } else if filters.outlook_day >= 4 {
                    ui.weak("Experimental severe-weather probability");
                }

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (label, color) in [
                        ("TSTM", egui::Color32::from_rgb(85, 170, 85)),
                        ("MRGL", egui::Color32::from_rgb(65, 145, 75)),
                        ("SLGT", egui::Color32::from_rgb(235, 210, 45)),
                        ("ENH", egui::Color32::from_rgb(235, 145, 45)),
                        ("MDT", egui::Color32::from_rgb(220, 60, 55)),
                        ("HIGH", egui::Color32::from_rgb(220, 70, 190)),
                    ] {
                        ui.colored_label(color, egui::RichText::new(format!("● {label}")).small());
                    }
                });
            });

        // Excessive Rainfall Outlook: the flood half of the day, directly under the severe half.
        ui.label("Rainfall outlook");
        egui::ComboBox::from_id_salt("ero_day")
            .width(ui.available_width() - 8.0)
            .selected_text(if filters.ero_day == 0 {
                "Off".to_string()
            } else {
                format!("Day {}", filters.ero_day)
            })
            .show_ui(ui, |ui| {
                for day in 0u8..=3 {
                    let label = if day == 0 {
                        "Off".to_string()
                    } else {
                        format!("Day {day}")
                    };
                    if ui
                        .selectable_value(&mut filters.ero_day, day, label)
                        .changed()
                    {
                        actions.ero_day_changed = true;
                        changed = true;
                    }
                }
            });

        // Winter Storm Severity Index: same off-plus-three-days shape as the outlook selector.
        ui.label("Winter impacts");
        egui::ComboBox::from_id_salt("wssi_day")
            .width(ui.available_width() - 8.0)
            .selected_text(if filters.wssi_day == 0 {
                "Off".to_string()
            } else {
                format!("Day {}", filters.wssi_day)
            })
            .show_ui(ui, |ui| {
                for day in 0u8..=3 {
                    let label = if day == 0 {
                        "Off".to_string()
                    } else {
                        format!("Day {day}")
                    };
                    if ui
                        .selectable_value(&mut filters.wssi_day, day, label)
                        .changed()
                    {
                        actions.wssi_day_changed = true;
                        changed = true;
                    }
                }
            });
    }

    if section == "Environment" {
        // Where the environment fields and contours come from. RAP f00 is an analysis of what the
        // atmosphere is doing now (assimilated obs, 13 km) rather than an HRRR forecast at hour zero —
        // the thing people mean by "mesoanalysis". Labelled honestly, coarser grid and all.
        let env_before = *env_model;
        ui.label("Model");
        egui::ComboBox::from_id_salt("environment_model")
            .width(ui.available_width() - 8.0)
            .selected_text(env_model.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(env_model, wxdata::hrrr::Model::Hrrr, "HRRR 3 km")
                    .on_hover_text("HRRR forecast model, 3 km grid (analysis at F+0)");
                ui.selectable_value(env_model, wxdata::hrrr::Model::Rap, "RAP analysis")
            .on_hover_text(
                "RAP f00 observed analysis, 13 km grid — coarser, but what is, not what's forecast",
            );
                ui.selectable_value(env_model, wxdata::hrrr::Model::Rrfs, "RRFS v1 parallel")
                    .on_hover_text(
                        "NOAA's 3 km RRFS pre-implementation parallel — experimental until the operational promotion",
                    );
                ui.selectable_value(env_model, wxdata::hrrr::Model::NamNest, "NAM 3 km nest")
            .on_hover_text(
                "The NAM's 3 km CONUS nest — a second convection-allowing opinion on its own \
                 dynamical core, run every six hours",
            );
            });
        if *env_model != env_before {
            // Both sources feed CAPE/SRH and the contours; drop their clocks so the next frame refetches.
            for l in [FL::Cape, FL::Srh] {
                if let Some(s) = fields.get_mut(&l) {
                    s.last_fetch = None;
                }
            }
            // STP needs an LCL height the RAP file doesn't carry (see wxdata::severe::fetch_grid).
            if !stp_source(*env_model) && *contour_kind == crate::app::ContourKind::Stp {
                *contour_kind = crate::app::ContourKind::Off;
            }
            changed = true;
        }

        // Model contours (isolines) — MSLP / 2 m temp / dewpoint / SB-CAPE / 0-3 km SRH.
        ui.label("Contours");
        egui::ComboBox::from_id_salt("environment_contours")
            .width(ui.available_width() - 8.0)
            .selected_text(contour_kind.label())
            .show_ui(ui, |ui| {
                for k in crate::app::ContourKind::ALL {
                    if k == crate::app::ContourKind::Stp && !stp_source(*env_model) {
                        continue; // no LCL height in these files
                    }
                    ui.selectable_value(contour_kind, k, k.label());
                }
            })
            .response
            .on_hover_text("Draw a surface field as labeled contour lines (f00)");
    }

    // Everything below belongs to a layer that has to be on for it to mean anything.
    let header = |ui: &mut egui::Ui, text: &str| {
        if text != section && !(section == "Alerts" && text == "NWS Alerts") {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(text).small().strong());
        }
    };

    if section == "Reflectivity trail" && on.contains(&FL::MrmsReflectivityTrail) {
        header(ui, "MRMS maximum reflectivity trail");
        ui.horizontal(|ui| {
            ui.label("Window:");
            for minutes in [15u16, 30, 60, 120] {
                actions.trail_changed |= ui
                    .selectable_value(trail_minutes, minutes, format!("{minutes}m"))
                    .changed();
            }
        });
        actions.trail_changed |= ui
            .add(egui::Slider::new(trail_threshold, 5.0..=70.0).text("Threshold").suffix(" dBZ"))
            .changed();
        if ui.button("Reset trail now").clicked() {
            actions.trail_changed = true;
        }
        actions.export_trail |= ui.button("Export trail values…").clicked();
        ui.weak("Keeps each cell's strongest reflectivity and its contributing frame age.");
    }

    if section == "Local cell tracks" && show_local_tracks {
        header(ui, "Radar-derived storm history");
        ui.weak("Exports every tracked centroid, time, direction, and speed currently held in the radar loop.");
        ui.horizontal(|ui| {
            actions.export_local_tracks_csv |= ui.button("Export CSV…").clicked();
            actions.export_local_tracks_json |= ui.button("Export JSON…").clicked();
        });
    }

    if section == "Rotation tracks" && on.contains(&FL::Rotation) {
        header(ui, "Rotation tracks");
        ui.horizontal_wrapped(|ui| {
            ui.label("Window:");
            let mut dur = false;
            for (m, label) in [
                (30u16, "30m"),
                (60, "1h"),
                (120, "2h"),
                (240, "4h"),
                (360, "6h"),
                (1440, "24h"),
            ] {
                dur |= ui
                    .selectable_value(rotation_minutes, m, label)
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

    if section == "Hail swaths" && on.contains(&FL::HailSwath) {
        header(ui, "Hail swaths");
        ui.horizontal(|ui| {
            ui.label("Window:");
            let mut dur = false;
            for (m, label) in [
                (30u16, "30m"),
                (60, "1h"),
                (120, "2h"),
                (360, "6h"),
                (1440, "24h"),
            ] {
                dur |= ui.selectable_value(hail_minutes, m, label).changed();
            }
            if dur {
                if let Some(s) = fields.get_mut(&FL::HailSwath) {
                    s.last_fetch = None;
                }
            }
        });
    }

    if section == "Radar mosaic" && on.contains(&FL::Mosaic) {
        header(ui, "Radar mosaic");
        if let Some(m) = mosaic {
            ui.weak(m);
        }
    }

    if section == "Future radar" && on.contains(&FL::Hrrr) {
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

    if section == "Ensemble forecast" && on.contains(&FL::RefsReflectivityProb) {
        header(ui, "REFS storm probability");
        ui.add(egui::Slider::new(refs_fcst_hour, 1..=60).text("F+ hr"));
        ui.weak("Neighborhood probability of composite reflectivity above 40 dBZ.");
    }

    if section == "Environment" && on.contains(&FL::Cape) {
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

    if section == "Environment" && on.contains(&FL::Srh) {
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

    if section == "Storm cells" && filters.show_cells {
        header(ui, "Storm cells");
        crate::ui::style::toggle(ui, &mut filters.show_tracks, "Forecast tracks")
            .on_hover_text("15/30/45/60-min projected storm positions");
        crate::ui::style::toggle(ui, &mut filters.show_arrival_cones, "Arrival-time cones")
            .on_hover_text("Project cell motion forward + ETA to your saved markers");
    }

    if section == "Nowcast" && filters.show_nowcast {
        header(ui, "Nowcast");
        ui.horizontal(|ui| {
            ui.label("Lead:");
            for m in [15u8, 30, 45, 60, 90, 120] {
                ui.selectable_value(&mut filters.nowcast_lead_min, m, format!("{m}m"));
            }
        });
        if filters.nowcast_lead_min > 45 {
            ui.small(
                "Past 45 minutes this is extrapolation, not forecasting — it moves the echo \
                 that exists and cannot grow or decay it. HRRR future radar is the model \
                 answer for an hour or more.",
            );
        }
    }

    if section == "Alerts" && filters.show_alerts {
        header(ui, "NWS Alerts");
        for cat in Category::ALL {
            changed |=
                crate::ui::style::toggle(ui, &mut filters.alert_cats[cat.index()], cat.label())
                    .changed();
        }
    }

    if section == "Tropical" && *show_tropical {
        header(ui, "Tropical");
        ui.label("Wind field");
        egui::ComboBox::from_id_salt("tropical_wind")
            .width(ui.available_width() - 8.0)
            .selected_text(
                tropical_wind_kt.map_or_else(|| "Off".to_string(), |kt| format!("{kt} kt")),
            )
            .show_ui(ui, |ui| {
                changed |= ui.selectable_value(tropical_wind_kt, None, "Off").changed();
                for kt in [34u8, 50, 64] {
                    changed |= ui
                        .selectable_value(tropical_wind_kt, Some(kt), format!("{kt} kt"))
                        .on_hover_text("How far out the forecast wind of that strength reaches")
                        .changed();
                }
            });
        changed |= crate::ui::style::toggle(ui, tropical_surge, "Potential storm surge")
            .on_hover_text(
                "How deep water could get above ground if the peak surge arrives at high tide",
            )
            .changed();
    }

    if section == "Snowfall" && on.contains(&FL::SnowAnalysis) {
        header(ui, "Snowfall analysis");
        ui.horizontal(|ui| {
            ui.label("Window:");
            for h in wxdata::nohrsc::DURATIONS {
                changed |= ui
                    .selectable_value(snow_hours, h, format!("{h}h"))
                    .changed();
            }
        });
    }

    if section == "Derived radar"
        && [FL::VilLocal, FL::VilDensity, FL::EtopLocal]
            .iter()
            .any(|l| on.contains(l))
    {
        header(ui, "Derived products");
        ui.horizontal(|ui| {
            ui.label("Echo top:");
            changed |= ui
                .add(egui::Slider::new(etop_dbz, 5.0..=50.0).suffix(" dBZ"))
                .on_hover_text("Reflectivity that counts as the storm top (18.5 = NWS EET)")
                .changed();
        });
    }

    // Detector thresholds. Each block only appears with its own detector on, and the defaults are
    // what the detectors shipped with — the reset button is there because a slider you can't get
    // back from is worse than no slider.
    if section == "Detectors" && filters.show_tbss {
        header(ui, "Hail spike (TBSS)");
        ui.add(
            egui::Slider::new(&mut detectors.tbss_core_dbz, 50.0..=70.0)
                .text("Core")
                .suffix(" dBZ"),
        )
        .on_hover_text("How strong the core must be before a spike behind it is looked for");
    }
    if section == "Detectors" && filters.show_zdr_columns {
        header(ui, "ZDR columns");
        ui.add(
            egui::Slider::new(&mut detectors.zdr_min_db, 0.5..=3.0)
                .text("Minimum ZDR")
                .suffix(" dB"),
        );
        ui.add(
            egui::Slider::new(&mut detectors.zdr_min_depth_km, 0.5..=3.0)
                .text("Depth above freezing")
                .suffix(" km"),
        );
    }
    if section == "Lightning" && show_glm {
        header(ui, "Flash-extent density");
        ui.add(
            egui::Slider::new(&mut detectors.glm_fed_cell_deg, 0.02..=0.2)
                .text("Cell size")
                .suffix("°"),
        )
        .on_hover_text("Grid resolution: 0.05° is about 5 km");
        ui.add(
            egui::Slider::new(&mut detectors.glm_fed_window_min, 5..=30)
                .text("Window")
                .suffix(" min"),
        );
        ui.weak("Takes effect on the next flash-density refresh.");
    }
    if (section == "Detectors" || section == "Lightning")
        && (filters.show_tbss || filters.show_zdr_columns || show_glm)
        && ui.button("Reset detector thresholds").clicked()
    {
        *detectors = crate::settings::DetectorTuning::default();
    }

    if section == "Level 3 grids"
        && [FL::Vil, FL::EchoTops, FL::Hca]
            .iter()
            .any(|l| on.contains(l))
    {
        header(ui, "Level 3 grids");
        ui.weak(format!("Site: {}", l3grid_site.unwrap_or("—")));
    }

    actions.overlays_changed |= changed;
}
