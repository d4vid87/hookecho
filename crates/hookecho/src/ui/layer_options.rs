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
    /// Start an offline chase-pack download of the current view's basemap.
    pub download_chasepack: bool,
    /// Cancel the in-progress chase-pack download.
    pub cancel_chasepack: bool,
    /// A row in the embedded layers registry was clicked; the app applies it.
    pub(crate) palette: Option<crate::app::PaletteAction>,
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

#[allow(clippy::too_many_arguments)] // one flat call per frame; a params struct adds churn for no reader gain
pub(crate) fn show(
    ui: &mut egui::Ui,
    filters: &mut OverlayFilters,
    fields: &mut std::collections::HashMap<crate::render::FieldLayer, crate::app::FieldState>,
    rotation_minutes: &mut u16,
    hrrr_fcst_hour: &mut u8,
    hrrr_valid: Option<chrono::DateTime<chrono::Utc>>,
    tz: Option<wxdata::tz::Tz>,
    env_cape_ml: &mut bool,
    env_srh_km: &mut u8,
    env_model: &mut wxdata::hrrr::Model,
    contour_kind: &mut crate::app::ContourKind,
    l3grid_site: Option<&str>,
    // One line about the live composite: contributing sites and the age of its oldest scan, or
    // why there isn't one. Radars scan on their own schedules, so a composite is always a little
    // ragged in time and the honest thing is to show by how much.
    mosaic: Option<&str>,
    actions: &mut UiActions,
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

    // Where the environment fields and contours come from. RAP f00 is an analysis of what the
    // atmosphere is doing now (assimilated obs, 13 km) rather than an HRRR forecast at hour zero —
    // the thing people mean by "mesoanalysis". Labelled honestly, coarser grid and all.
    let env_before = *env_model;
    ui.horizontal(|ui| {
        ui.label("Environment:");
        ui.selectable_value(env_model, wxdata::hrrr::Model::Hrrr, "HRRR 3 km")
            .on_hover_text("HRRR forecast model, 3 km grid (analysis at F+0)");
        ui.selectable_value(env_model, wxdata::hrrr::Model::Rap, "RAP analysis")
            .on_hover_text("RAP f00 observed analysis, 13 km grid — coarser, but what is, not what's forecast");
    });
    if *env_model != env_before {
        // Both sources feed CAPE/SRH and the contours; drop their clocks so the next frame refetches.
        for l in [FL::Cape, FL::Srh] {
            if let Some(s) = fields.get_mut(&l) {
                s.last_fetch = None;
            }
        }
        // STP needs an LCL height the RAP file doesn't carry (see wxdata::severe::fetch_grid).
        if *env_model == wxdata::hrrr::Model::Rap && *contour_kind == crate::app::ContourKind::Stp {
            *contour_kind = crate::app::ContourKind::Off;
        }
        changed = true;
    }

    // Model contours (isolines) — MSLP / 2 m temp / dewpoint / SB-CAPE / 0-3 km SRH.
    egui::ComboBox::from_label("Contours")
        .selected_text(contour_kind.label())
        .show_ui(ui, |ui| {
            for k in crate::app::ContourKind::ALL {
                if k == crate::app::ContourKind::Stp && *env_model == wxdata::hrrr::Model::Rap {
                    continue; // no LCL height in the RAP analysis file
                }
                ui.selectable_value(contour_kind, k, k.label());
            }
        })
        .response
        .on_hover_text("Draw a surface field as labeled contour lines (f00)");

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

    if fields.get(&FL::Mosaic).is_some_and(|s| s.show) {
        header(ui, "Radar mosaic");
        if let Some(m) = mosaic {
            ui.weak(m);
        }
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
