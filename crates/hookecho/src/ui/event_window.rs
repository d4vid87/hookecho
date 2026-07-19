//! Time-machine event library: curated famous storms plus user-saved bookmarks. Clicking an
//! entry deep-links the active pane (site + camera + archive seek); the app resolves the
//! returned action.

use crate::settings::Settings;
use chrono::{DateTime, Utc};

/// What the app should do after the window is shown.
pub enum EventAction {
    /// Jump the active pane to a site + camera, seeking the timeline to `time` (None = live).
    Goto { site: String, lon: f64, lat: f64, zoom: f64, time: Option<DateTime<Utc>> },
    /// Save the active pane's current view as a bookmark.
    AddBookmark,
}

#[derive(Default)]
pub struct EventWindow {
    pub open: bool,
}

impl EventWindow {
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Option<EventAction> {
        let mut open = self.open;
        let mut action = None;
        let mut remove: Option<usize> = None;
        egui::Window::new("Event Library")
            .open(&mut open)
            .default_size([460.0, 460.0])
            .show(ctx, |ui| {
                crate::theme::section(ui, "Famous events", |ui| {
                    for e in crate::events::EVENTS {
                        ui.horizontal(|ui| {
                            if ui.button("▶").on_hover_text("Jump the active pane here").clicked() {
                                action = Some(EventAction::Goto {
                                    site: e.site.to_string(),
                                    lon: e.lon,
                                    lat: e.lat,
                                    zoom: e.zoom,
                                    time: Some(e.datetime()),
                                });
                            }
                            ui.strong(e.name);
                            ui.weak(e.site);
                        });
                        ui.label(egui::RichText::new(e.blurb).size(11.0).weak());
                        ui.add_space(2.0);
                    }
                });

                ui.add_space(6.0);
                crate::theme::section(ui, "My bookmarks", |ui| {
                    if settings.bookmarks.is_empty() {
                        ui.weak("None yet — click “Bookmark current view”.");
                    }
                    for (i, b) in settings.bookmarks.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button("▶").clicked() {
                                let (lon, lat) = crate::render::mercator::world_to_lonlat(b.x, b.y);
                                action = Some(EventAction::Goto {
                                    site: b.site.clone(),
                                    lon,
                                    lat,
                                    zoom: b.zoom,
                                    time: b.time_secs.and_then(|s| DateTime::from_timestamp(s, 0)),
                                });
                            }
                            ui.strong(&b.name);
                            ui.weak(&b.site);
                            if b.time_secs.is_some() {
                                ui.weak("· archive");
                            }
                            if ui.button("✖").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    ui.add_space(4.0);
                    if ui.button("🔖 Bookmark current view").clicked() {
                        action = Some(EventAction::AddBookmark);
                    }
                });
            });
        if let Some(i) = remove {
            settings.bookmarks.remove(i);
        }
        self.open = open;
        action
    }
}
