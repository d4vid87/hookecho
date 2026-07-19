//! "Select Radar Site" dialog: a sortable, filterable table over the NEXRAD registry.

use crate::render::mercator::{world_to_lonlat, Camera};
use crate::settings::Settings;
use crate::view::MapView;
use egui_extras::{Column, TableBuilder};

#[derive(Default, PartialEq, Clone, Copy)]
pub enum SortCol {
    #[default]
    Id,
    City,
    State,
    Distance,
}

#[derive(Default)]
pub struct SiteDialog {
    pub filter: String,
    pub sort: SortCol,
    pub desc: bool,
}

struct Row {
    id: String,
    city: String,
    state: String,
    kind: &'static str,
    dist_km: f32,
    starred: bool,
}

fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f32 {
    let r = 6371.0_f64;
    let (lon1, lat1) = (a.0.to_radians(), a.1.to_radians());
    let (lon2, lat2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon * 0.5).sin().powi(2);
    (2.0 * r * h.sqrt().asin()) as f32
}

/// Show the dialog. Returns `false` when it should close.
pub fn show(
    ctx: &egui::Context,
    dialog: &mut SiteDialog,
    view: &mut MapView,
    settings: &mut Settings,
) -> bool {
    let center = world_to_lonlat(view.camera.center.0, view.camera.center.1);

    // Build filtered rows (cloned so no borrow is held across the table).
    let needle = dialog.filter.to_ascii_uppercase();
    let mut rows: Vec<Row> = wxdata::sites::sites()
        .iter()
        .filter(|s| {
            needle.is_empty()
                || s.id.to_ascii_uppercase().contains(&needle)
                || s.city.to_ascii_uppercase().contains(&needle)
                || s.state.to_ascii_uppercase().contains(&needle)
        })
        .map(|s| Row {
            id: s.id.to_string(),
            city: s.city.to_string(),
            state: s.state.to_string(),
            kind: if s.id.starts_with('T') { "TDWR" } else { "WSR-88D" },
            dist_km: haversine_km(center, (s.longitude as f64, s.latitude as f64)),
            starred: settings.presets.iter().any(|p| p == s.id),
        })
        .collect();

    match dialog.sort {
        SortCol::Id => rows.sort_by(|a, b| a.id.cmp(&b.id)),
        SortCol::City => rows.sort_by(|a, b| a.city.cmp(&b.city)),
        SortCol::State => rows.sort_by(|a, b| a.state.cmp(&b.state)),
        SortCol::Distance => rows.sort_by(|a, b| a.dist_km.total_cmp(&b.dist_km)),
    }
    if dialog.desc {
        rows.reverse();
    }

    let mut open = true;
    let mut apply: Option<String> = None;
    let mut clear = false;
    let mut go_home = false;
    let mut toggle_star: Option<String> = None;

    egui::Window::new("Select Radar Site")
        .open(&mut open)
        .default_size([460.0, 520.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(&mut dialog.filter);
                if ui.button("None").clicked() {
                    clear = true;
                }
                if ui.button("Home").clicked() {
                    go_home = true;
                }
            });
            ui.separator();

            let mut header_button = |ui: &mut egui::Ui, label: &str, col: SortCol| {
                let active = dialog.sort == col;
                let text = if active {
                    format!("{label} {}", if dialog.desc { "▼" } else { "▲" })
                } else {
                    label.to_string()
                };
                if ui.button(text).clicked() {
                    if active {
                        dialog.desc = !dialog.desc;
                    } else {
                        dialog.sort = col;
                        dialog.desc = false;
                    }
                }
            };

            TableBuilder::new(ui)
                .striped(true)
                .sense(egui::Sense::click())
                .column(Column::exact(28.0)) // star
                .column(Column::exact(56.0)) // id
                .column(Column::remainder()) // city
                .column(Column::exact(40.0)) // state
                .column(Column::exact(70.0)) // kind
                .column(Column::exact(80.0)) // distance
                .min_scrolled_height(0.0)
                .header(22.0, |mut h| {
                    h.col(|ui| { ui.label("★"); });
                    h.col(|ui| header_button(ui, "ID", SortCol::Id));
                    h.col(|ui| header_button(ui, "City", SortCol::City));
                    h.col(|ui| header_button(ui, "St", SortCol::State));
                    h.col(|ui| { ui.label("Type"); });
                    h.col(|ui| header_button(ui, "Dist", SortCol::Distance));
                })
                .body(|mut body| {
                    for r in &rows {
                        body.row(20.0, |mut row| {
                            let mut star_hit = false;
                            row.col(|ui| {
                                let star = if r.starred { "★" } else { "☆" };
                                if ui.button(star).clicked() {
                                    toggle_star = Some(r.id.clone());
                                    star_hit = true;
                                }
                            });
                            row.col(|ui| { ui.strong(&r.id); });
                            row.col(|ui| { ui.label(&r.city); });
                            row.col(|ui| { ui.label(&r.state); });
                            row.col(|ui| { ui.label(r.kind); });
                            row.col(|ui| { ui.label(format!("{:.0} km", r.dist_km)); });
                            if row.response().clicked() && !star_hit {
                                apply = Some(r.id.clone());
                            }
                        });
                    }
                });
        });

    if let Some(id) = toggle_star {
        if let Some(pos) = settings.presets.iter().position(|p| *p == id) {
            settings.presets.remove(pos);
        } else {
            settings.presets.push(id);
        }
    }
    if clear {
        view.site = None;
        return false;
    }
    if go_home {
        view.site = Some(settings.default_site.clone());
        return false;
    }
    if let Some(id) = apply {
        view.site = Some(id);
        return false;
    }
    open
}

/// Recenter a view's camera on a site by id, keeping zoom.
pub fn center_on_site(camera: &mut Camera, site: &str) {
    if let Some(s) = wxdata::sites::site_by_id(site) {
        *camera = Camera::at_lonlat(s.longitude as f64, s.latitude as f64, camera.zoom);
    }
}
