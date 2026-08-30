//! "Select Radar Site" dialog: a sortable, filterable table over every radar the app can fetch.

use crate::render::mercator::{world_to_lonlat, Camera};
use crate::settings::Settings;
use crate::view::MapView;
use egui_extras::{Column, TableBuilder};

#[derive(Default, PartialEq, Clone, Copy)]
pub enum SortCol {
    Id,
    City,
    State,
    // Nearest first, not A-to-Z. The alphabetical default was harmless while the list was
    // WSR-88D only; adding the DWD table put ten German sites seven thousand kilometres away at
    // the top of the picker for every user in North America.
    #[default]
    Distance,
}

#[derive(Default)]
pub struct SiteDialog {
    pub filter: String,
    pub sort: SortCol,
    pub desc: bool,
    /// Show only this network; `None` is everything. Distance sort is the right default, but it
    /// also means a user in North America never scrolls far enough to learn that the DWD and
    /// OPERA radars exist.
    pub network: Option<wxdata::sites::Network>,
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
    drawer: &mut crate::ui::drawer::Drawer,
) -> bool {
    let center = world_to_lonlat(view.camera.center.0, view.camera.center.1);

    // Build filtered rows (cloned so no borrow is held across the table).
    let needle = dialog.filter.to_ascii_uppercase();
    // The WSR-88D registry, the TDWR table and the DWD table, in one list. A site's kind comes
    // from which table it is in — the old `starts_with('T')` guess labelled TJUA (a WSR-88D) a
    // terminal radar.
    let mut rows: Vec<Row> = wxdata::sites::all()
        .filter(|s| {
            dialog
                .network
                .is_none_or(|n| wxdata::sites::network(s.id) == n)
        })
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
            // Which table it came from, not which letter it starts with: the old
            // `starts_with('T')` guess labelled TJUA (a WSR-88D) a terminal radar.
            kind: match wxdata::sites::network(s.id) {
                wxdata::sites::Network::Tdwr => "TDWR",
                wxdata::sites::Network::Dwd => "DWD",
                wxdata::sites::Network::Opera => "OPERA",
                wxdata::sites::Network::Nexrad => "WSR-88D",
            },
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

    let Some(window) = drawer.page_sized(
        ctx,
        "Select Radar Site",
        &mut open,
        false,
        460.0,
        egui::Window::new("Select Radar Site"),
    ) else {
        return open;
    };
    window.show(ctx, |ui| {
        // Selectable label text senses clicks and drags of its own, and a row is made of
        // nothing but labels — so every click on a row went into selecting the city name and
        // `row.response().clicked()` never fired. Clicking a site did nothing at all.
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            ui.label("Filter:");
            // The field's default width plus two buttons overflows a phone, pushing None/Home
            // off the screen edge — size it from what's actually left in the row.
            let w = (ui.available_width() - 130.0).max(80.0);
            ui.add(egui::TextEdit::singleline(&mut dialog.filter).desired_width(w));
            if ui.button("None").clicked() {
                clear = true;
            }
            if ui.button("Home").clicked() {
                go_home = true;
            }
        });
        // Network chips: the picker's only route to the non-US radars, since distance sort buries
        // them behind every site on the user's own continent.
        ui.horizontal_wrapped(|ui| {
            use wxdata::sites::Network;
            let mut chip = |ui: &mut egui::Ui, label: &str, net: Option<Network>| {
                if ui.selectable_label(dialog.network == net, label).clicked() {
                    dialog.network = net;
                }
            };
            chip(ui, "All", None);
            chip(ui, "WSR-88D", Some(Network::Nexrad));
            chip(ui, "TDWR", Some(Network::Tdwr));
            chip(ui, "DWD", Some(Network::Dwd));
            chip(ui, "OPERA", Some(Network::Opera));
        });
        ui.separator();

        // Android: `TableBuilder` row clicks don't register under touch (nested scroll eats
        // them), so the user could never switch sites. Use a plain scrollable button list —
        // buttons reliably take taps. Desktop keeps the sortable table.
        if cfg!(target_os = "android") {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for r in &rows {
                    ui.horizontal(|ui| {
                        let star = if r.starred { "★" } else { "☆" };
                        if ui.button(star).clicked() {
                            toggle_star = Some(r.id.clone());
                        }
                        let text = format!(
                            "{}   {}, {}   ·   {:.0} km",
                            r.id, r.city, r.state, r.dist_km
                        );
                        let btn = egui::Button::new(egui::RichText::new(text).size(15.0))
                            .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                            .corner_radius(8.0)
                            .min_size(egui::vec2(ui.available_width(), 40.0));
                        if ui.add(btn).clicked() {
                            apply = Some(r.id.clone());
                        }
                    });
                    ui.add_space(3.0);
                }
            });
        } else {
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
                    h.col(|ui| {
                        ui.label("★");
                    });
                    h.col(|ui| header_button(ui, "ID", SortCol::Id));
                    h.col(|ui| header_button(ui, "City", SortCol::City));
                    h.col(|ui| header_button(ui, "St", SortCol::State));
                    h.col(|ui| {
                        ui.label("Type");
                    });
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
                            row.col(|ui| {
                                ui.strong(&r.id);
                            });
                            row.col(|ui| {
                                ui.label(&r.city);
                            });
                            row.col(|ui| {
                                ui.label(&r.state);
                            });
                            row.col(|ui| {
                                ui.label(r.kind);
                            });
                            row.col(|ui| {
                                ui.label(format!("{:.0} km", r.dist_km));
                            });
                            if row.response().clicked() && !star_hit {
                                apply = Some(r.id.clone());
                            }
                        });
                    }
                });
        }
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
