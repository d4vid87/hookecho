//! Signal rail radar selector: nearby, favorite and recent sites across every supported network.

use crate::render::mercator::{world_to_lonlat, Camera};
use crate::settings::Settings;
use crate::ui::a11y::Named;
use crate::view::MapView;

#[derive(Default, PartialEq, Clone, Copy)]
pub enum SortCol {
    Id,
    City,
    State,
    Recent,
    // Nearest first, not A-to-Z. The alphabetical default was harmless while the list was
    // WSR-88D only; adding the DWD table put ten German sites seven thousand kilometres away at
    // the top of the picker for every user in North America.
    #[default]
    Distance,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Collection {
    #[default]
    Nearby,
    Favorites,
    Recent,
}

#[derive(Default)]
pub struct SiteDialog {
    collection: Collection,
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

fn network_label(network: wxdata::sites::Network) -> &'static str {
    use wxdata::sites::Network;
    match network {
        Network::Nexrad => "WSR-88D",
        Network::Tdwr => "TDWR",
        Network::Dwd => "DWD",
        Network::Opera => "OPERA",
    }
}

fn matching_sites(dialog: &SiteDialog, center: (f64, f64), settings: &Settings) -> Vec<Row> {
    let needle = dialog.filter.trim().to_ascii_uppercase();
    let mut rows: Vec<Row> = wxdata::sites::all()
        .filter(|s| {
            dialog
                .network
                .is_none_or(|n| wxdata::sites::network(s.id) == n)
        })
        .filter(|s| match dialog.collection {
            Collection::Nearby => true,
            Collection::Favorites => settings.presets.iter().any(|id| id == s.id),
            Collection::Recent => settings.recent_sites.iter().any(|id| id == s.id),
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
            kind: network_label(wxdata::sites::network(s.id)),
            dist_km: haversine_km(center, (s.longitude as f64, s.latitude as f64)),
            starred: settings.presets.iter().any(|id| id == s.id),
        })
        .collect();
    match dialog.sort {
        SortCol::Id => rows.sort_by(|a, b| a.id.cmp(&b.id)),
        SortCol::City => rows.sort_by(|a, b| a.city.cmp(&b.city).then(a.id.cmp(&b.id))),
        SortCol::State => rows.sort_by(|a, b| a.state.cmp(&b.state).then(a.id.cmp(&b.id))),
        SortCol::Distance => rows.sort_by(|a, b| a.dist_km.total_cmp(&b.dist_km)),
        SortCol::Recent => {
            rows.sort_by_key(|r| settings.recent_sites.iter().position(|id| *id == r.id))
        }
    }
    if dialog.desc {
        rows.reverse();
    }
    rows
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
    let mut open = true;
    let mut apply = None;
    let mut clear = false;
    let mut toggle_star = None;
    let Some(window) = drawer.page_sized_fixed(
        ctx,
        "Select Radar Site",
        &mut open,
        520.0,
        egui::Window::new("Select Radar Site"),
    ) else {
        return open;
    };
    window.vscroll(false).show(ctx, |ui| {
        ui.style_mut().interaction.selectable_labels = false;
        // Match the selected SPC Signal rail while retaining readable light-theme colors.
        if ui.visuals().dark_mode {
            ui.visuals_mut().selection.bg_fill = egui::Color32::from_rgb(36, 62, 83);
            ui.visuals_mut().selection.stroke.color = egui::Color32::from_rgb(143, 207, 255);
        }
        let accent = ui.visuals().selection.stroke.color;
        let height = (ui.clip_rect().bottom() - ui.cursor().top() - 8.0).max(120.0);
        let compact = ui.available_width() < 400.0;
        let rail_width = if compact { 64.0 } else { 82.0 };
        let detail_width = (ui.available_width() - rail_width - 24.0).max(140.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(rail_width);
                egui::ScrollArea::vertical()
                    .id_salt("site_collections")
                    .max_height(height)
                    .show(ui, |ui| {
                        for (collection, label, glyph) in [
                            (
                                Collection::Nearby,
                                "Nearby",
                                egui_phosphor::regular::CROSSHAIR,
                            ),
                            (
                                Collection::Favorites,
                                "Favorites",
                                egui_phosphor::regular::STAR,
                            ),
                            (
                                Collection::Recent,
                                "Recent",
                                egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE,
                            ),
                        ] {
                            if ui
                                .add_sized(
                                    [rail_width, 62.0],
                                    egui::Button::new(
                                        egui::RichText::new(format!("{glyph}\n{label}")).size(if compact { 10.0 } else { 12.0 }),
                                    )
                                    .selected(dialog.collection == collection),
                                )
                                .clicked()
                            {
                                dialog.collection = collection;
                                dialog.sort = if collection == Collection::Recent {
                                    SortCol::Recent
                                } else {
                                    SortCol::Distance
                                };
                                dialog.desc = false;
                            }
                        }
                        ui.add_space(12.0);
                        ui.separator();
                        if ui
                            .add_sized([rail_width, 38.0], egui::Button::new("Home"))
                            .on_hover_text("Switch to your saved home radar")
                            .clicked()
                        {
                            apply = Some(settings.default_site.clone());
                        }
                        if ui
                            .add_sized([rail_width, 38.0], egui::Button::new("No radar"))
                            .on_hover_text("Clear the active radar selection")
                            .clicked()
                        {
                            clear = true;
                        }
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(detail_width);
                if height > 440.0 {
                    egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .stroke(ui.visuals().window_stroke)
                        .corner_radius(12.0)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width((detail_width - 24.0).max(100.0));
                            ui.label(
                                egui::RichText::new("CURRENT RADAR")
                                    .size(10.0)
                                    .color(accent),
                            );
                            if let Some(site) =
                                view.site.as_deref().and_then(wxdata::sites::site_by_id)
                            {
                                ui.label(
                                    egui::RichText::new(site.id)
                                        .monospace()
                                        .size(20.0)
                                        .strong()
                                        .color(accent),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{}, {}", site.city, site.state))
                                        .size(13.0),
                                );
                                ui.weak(
                                    egui::RichText::new(network_label(wxdata::sites::network(
                                        site.id,
                                    )))
                                    .size(11.0),
                                );
                            } else {
                                ui.label("No radar selected");
                            }
                        });
                    ui.add_space(10.0);
                }
                ui.add_sized(
                    [detail_width, 34.0],
                    egui::TextEdit::singleline(&mut dialog.filter)
                        .hint_text("Search city, state or radar ID")
                        .font(egui::TextStyle::Body),
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    use wxdata::sites::Network;
                    for (label, network) in [
                        ("All", None),
                        ("WSR-88D", Some(Network::Nexrad)),
                        ("TDWR", Some(Network::Tdwr)),
                        ("DWD", Some(Network::Dwd)),
                        ("OPERA", Some(Network::Opera)),
                    ] {
                        ui.selectable_value(
                            &mut dialog.network,
                            network,
                            egui::RichText::new(label).size(11.0),
                        );
                    }
                });
                ui.separator();
                let rows = matching_sites(dialog, center, settings);
                ui.horizontal_wrapped(|ui| {
                    ui.weak(egui::RichText::new(format!("{} sites", rows.len())).size(11.0));
                    egui::ComboBox::from_id_salt("site_sort")
                        .width(104.0)
                        .selected_text(match dialog.sort {
                            SortCol::Distance => "Nearest first",
                            SortCol::City => "City",
                            SortCol::Id => "Radar ID",
                            SortCol::State => "State",
                            SortCol::Recent => "Most recent",
                        })
                        .show_ui(ui, |ui| {
                            for (sort, label) in [
                                (SortCol::Distance, "Nearest first"),
                                (SortCol::City, "City"),
                                (SortCol::Id, "Radar ID"),
                                (SortCol::State, "State"),
                            ] {
                                ui.selectable_value(&mut dialog.sort, sort, label);
                            }
                            if dialog.collection == Collection::Recent {
                                ui.selectable_value(
                                    &mut dialog.sort,
                                    SortCol::Recent,
                                    "Most recent",
                                );
                            }
                        });
                    if ui
                        .small_button(if dialog.desc { "↓" } else { "↑" })
                        .named("Reverse site sort order")
                        .clicked()
                    {
                        dialog.desc = !dialog.desc;
                    }
                });
                let list_height = (ui.clip_rect().bottom() - ui.cursor().top() - 34.0).max(48.0);
                if rows.is_empty() {
                    ui.add_space(12.0);
                    ui.label("No matching radar sites");
                    ui.weak(match dialog.collection {
                        Collection::Favorites => {
                            "Star a radar in Nearby to save it here, or clear your filters."
                        }
                        Collection::Recent => {
                            "Recently selected radars appear here. Try clearing your filters."
                        }
                        Collection::Nearby => "Try another city, state, radar ID or network.",
                    });
                    if ui.button("Reset filters").clicked() {
                        dialog.filter.clear();
                        dialog.network = None;
                    }
                } else {
                    // Virtualized buttons keep long global lists responsive and accept touch reliably.
                    egui::ScrollArea::vertical()
                        .id_salt((
                            "signal_site_list",
                            dialog.collection as u8,
                            dialog.network.map(network_label),
                            &dialog.filter,
                            dialog.sort as u8,
                            dialog.desc,
                        ))
                        .max_height(list_height)
                        .auto_shrink([false, false])
                        .show_rows(ui, 56.0, rows.len(), |ui, range| {
                            for row in &rows[range] {
                                ui.push_id(&row.id, |ui| {
                                    ui.horizontal(|ui| {
                                        let width = (ui.available_width() - 38.0).max(70.0);
                                        let text = format!(
                                            "{}  {}\n{} · {} · {:.0} km",
                                            row.id, row.city, row.state, row.kind, row.dist_km
                                        );
                                        let response = ui
                                            .add_sized(
                                                [width, 54.0],
                                                egui::Button::new("")
                                                    .corner_radius(9.0)
                                                    .selected(
                                                        view.site.as_deref() == Some(&row.id),
                                                    ),
                                            )
                                            .named(&format!(
                                                "Select {} — {}, {}",
                                                row.id, row.city, row.state
                                            ))
                                            .on_hover_text(&text);
                                        // Truncate each line independently so metadata never disappears at a newline.
                                        for (line, y, size, color) in [
                                            (
                                                format!("{}  {}", row.id, row.city),
                                                9.0,
                                                if compact { 11.0 } else { 13.0 },
                                                ui.visuals().text_color(),
                                            ),
                                            (
                                                format!(
                                                    "{} · {} · {:.0} km",
                                                    row.state, row.kind, row.dist_km
                                                ),
                                                31.0,
                                                10.0,
                                                ui.visuals().weak_text_color(),
                                            ),
                                        ] {
                                            let mut job = egui::text::LayoutJob::simple_singleline(
                                                line,
                                                egui::FontId::proportional(size),
                                                color,
                                            );
                                            job.wrap.max_width =
                                                (response.rect.width() - 20.0).max(1.0);
                                            job.wrap.max_rows = 1;
                                            let galley =
                                                ui.fonts_mut(|fonts| fonts.layout_job(job));
                                            ui.painter()
                                                .with_clip_rect(
                                                    response.rect.intersect(ui.clip_rect()),
                                                )
                                                .galley(
                                                    response.rect.min + egui::vec2(10.0, y),
                                                    galley,
                                                    color,
                                                );
                                        }
                                        if response.clicked() {
                                            apply = Some(row.id.clone());
                                        }
                                        if ui
                                            .add_sized(
                                                [30.0, 44.0],
                                                egui::Button::new(
                                                    egui::RichText::new(if row.starred {
                                                        "★"
                                                    } else {
                                                        "☆"
                                                    })
                                                    .color(if row.starred {
                                                        accent
                                                    } else {
                                                        ui.visuals().weak_text_color()
                                                    }),
                                                )
                                                .frame(false),
                                            )
                                            .named_toggle(
                                                &format!("Favorite {}", row.id),
                                                row.starred,
                                            )
                                            .clicked()
                                        {
                                            toggle_star = Some(row.id.clone());
                                        }
                                    });
                                });
                            }
                        });
                }
                ui.add_space(6.0);
                ui.weak(
                    egui::RichText::new("Distances from map center · select to switch").size(10.0),
                );
            });
        });
    });
    if let Some(id) = toggle_star {
        if let Some(pos) = settings.presets.iter().position(|p| *p == id) {
            settings.presets.remove(pos);
        } else {
            settings.presets.push(id);
        }
        settings.save();
    }
    if clear {
        view.site = None;
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
        let globe = camera.globe;
        *camera = Camera::at_lonlat(s.longitude as f64, s.latitude as f64, camera.zoom);
        camera.globe = globe;
    }
}

#[cfg(test)]
mod selector_tests {
    use super::*;

    #[test]
    fn rail_filters_preserve_global_networks_favorites_and_recent_order() {
        let settings = Settings {
            presets: vec!["TDJT".into(), "KAMX".into()],
            recent_sites: vec!["KAMX".into(), "TDJT".into()],
            ..Settings::default()
        };
        let mut dialog = SiteDialog::default();
        let center = (-80.273, 26.688);
        let all = matching_sites(&dialog, center, &settings);
        assert_eq!(all[0].id, "TDJT");
        assert!(all.windows(2).all(|r| r[0].dist_km <= r[1].dist_km));
        dialog.collection = Collection::Favorites;
        assert_eq!(
            matching_sites(&dialog, center, &settings)
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["TDJT", "KAMX"]
        );
        dialog.filter = " miami ".into();
        assert_eq!(matching_sites(&dialog, center, &settings)[0].id, "KAMX");
        dialog.filter.clear();
        dialog.collection = Collection::Recent;
        dialog.sort = SortCol::Recent;
        assert_eq!(
            matching_sites(&dialog, center, &settings)
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["KAMX", "TDJT"]
        );
        dialog.collection = Collection::Nearby;
        for network in [
            wxdata::sites::Network::Nexrad,
            wxdata::sites::Network::Tdwr,
            wxdata::sites::Network::Dwd,
            wxdata::sites::Network::Opera,
        ] {
            dialog.network = Some(network);
            let rows = matching_sites(&dialog, center, &settings);
            assert!(!rows.is_empty());
            assert!(rows
                .iter()
                .all(|row| wxdata::sites::network(&row.id) == network));
        }
        dialog.network = Some(wxdata::sites::Network::Nexrad);
        dialog.filter = "TJUA".into();
        assert_eq!(
            matching_sites(&dialog, center, &settings)[0].kind,
            "WSR-88D"
        );
        dialog.filter = "not-a-radar".into();
        assert!(matching_sites(&dialog, center, &settings).is_empty());
    }

    #[test]
    fn recent_sites_are_bounded_canonical_and_backward_compatible() {
        let mut settings: Settings = serde_json::from_str(r#"{"presets":["KAMX"]}"#).unwrap();
        assert!(settings.recent_sites.is_empty());
        assert!(!settings.remember_radar_site("invalid"));
        assert!(settings.remember_radar_site("TPBI"));
        assert!(!settings.remember_radar_site("TDJT"));
        settings.remember_radar_site("KAMX");
        settings.remember_radar_site("tdjt");
        assert_eq!(settings.recent_sites, ["TDJT", "KAMX"]);
        for site in wxdata::sites::all().take(20) {
            settings.remember_radar_site(site.id);
        }
        assert_eq!(settings.recent_sites.len(), 12);
        assert_eq!(settings.presets, ["KAMX"]);
        let saved = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(settings.recent_sites, restored.recent_sites);
    }
}
#[cfg(test)]
mod projection_tests {
    use super::*;
    #[test]
    fn recentering_preserves_the_pane_projection_and_zoom() {
        let mut camera = Camera::at_lonlat(0.0, 0.0, 1.0);
        camera.globe = true;
        center_on_site(&mut camera, "KDMX");
        assert!(camera.globe);
        assert_eq!(camera.zoom, 1.0);
        let site = wxdata::sites::site_by_id("KDMX").unwrap();
        assert_eq!(
            camera.center,
            crate::render::mercator::lonlat_to_world(site.longitude as f64, site.latitude as f64)
        );
    }
}
