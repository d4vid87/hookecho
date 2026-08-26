//! Time-machine event library: curated famous storms plus user-saved bookmarks. Clicking an
//! entry deep-links the active pane (site + camera + archive seek); the app resolves the
//! returned action.

use crate::settings::Settings;
use chrono::{DateTime, Utc};

/// What the app should do after the window is shown.
pub enum EventAction {
    /// Jump the active pane to a site + camera, seeking the timeline to `time` (None = live).
    Goto {
        site: String,
        lon: f64,
        lat: f64,
        zoom: f64,
        time: Option<DateTime<Utc>>,
        /// Minutes to replay around `time`; `0` jumps there and stops.
        span_min: u16,
    },
    /// Save the active pane's current view as a bookmark. The span is the replay window to save
    /// with it, `0` for a still.
    AddBookmark(u16),
}

#[derive(Default)]
pub struct EventWindow {
    pub open: bool,
}

impl EventWindow {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        drawer: &mut crate::ui::drawer::Drawer,
    ) -> Option<EventAction> {
        let mut open = self.open;
        let mut action = None;
        let mut remove: Option<usize> = None;
        let Some(window) = drawer.page(
            ctx,
            "Event Library",
            &mut open,
            false,
            egui::Window::new("Event Library"),
        ) else {
            self.open = open;
            return None;
        };
        window.show(ctx, |ui| {
            crate::theme::section(ui, "Famous events", |ui| {
                for e in crate::events::EVENTS {
                    ui.horizontal(|ui| {
                        if ui
                            .button(egui_phosphor::regular::CARET_RIGHT)
                            .on_hover_text("Jump the active pane here")
                            .clicked()
                        {
                            action = Some(EventAction::Goto {
                                site: e.site.to_string(),
                                lon: e.lon,
                                lat: e.lat,
                                zoom: e.zoom,
                                time: Some(e.datetime()),
                                span_min: e.span_min,
                            });
                        }
                        ui.strong(e.name);
                        ui.weak(e.site);
                        ui.weak(format!("· {} min replay", e.span_min));
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
                        if ui.button(egui_phosphor::regular::CARET_RIGHT).clicked() {
                            let (lon, lat) = crate::render::mercator::world_to_lonlat(b.x, b.y);
                            action = Some(EventAction::Goto {
                                site: b.site.clone(),
                                lon,
                                lat,
                                zoom: b.zoom,
                                time: b.time_secs.and_then(|s| DateTime::from_timestamp(s, 0)),
                                span_min: b.span_min,
                            });
                        }
                        ui.strong(&b.name);
                        ui.weak(&b.site);
                        if b.time_secs.is_some() {
                            ui.weak(if b.span_min > 0 {
                                format!("· {} min replay", b.span_min)
                            } else {
                                "· archive".to_string()
                            });
                        }
                        if ui.button("✖").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("🔖 Bookmark current view")
                        .on_hover_text("The frame you are on, as a still")
                        .clicked()
                    {
                        action = Some(EventAction::AddBookmark(0));
                    }
                    if ui
                        .button("▶ Save as replay")
                        .on_hover_text(
                            "The hour around this frame, looped \u{2014} the storm becoming the \
                             thing worth saving, not one volume of it",
                        )
                        .clicked()
                    {
                        action = Some(EventAction::AddBookmark(60));
                    }
                });
            });
        });
        if let Some(i) = remove {
            settings.bookmarks.remove(i);
        }
        self.open = open;
        action
    }
}
