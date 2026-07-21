//! Placefile Manager: add/remove GRLevelX placefile URLs and toggle each on/off.

use crate::settings::{PlacefileConfig, Settings};

/// Load status for a configured placefile (built by the app from its loaded set).
pub struct PlacefileStatus {
    pub url: String,
    pub loaded: bool,
    pub items: usize,
    pub title: String,
}

#[derive(Default)]
pub struct PlacefileWindow {
    pub open: bool,
    new_url: String,
}

impl PlacefileWindow {
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings, status: &[PlacefileStatus]) {
        let mut open = self.open;
        crate::ui::fit_phone(ctx, egui::Window::new("Placefile Manager"))
            .open(&mut open)
            .default_size([520.0, 320.0])
            .show(ctx, |ui| {
                ui.label("GRLevelX placefiles (lines, polygons, text, icons at lat/lon).");
                ui.add_space(4.0);

                let mut remove: Option<usize> = None;
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for (i, cfg) in settings.placefiles.iter_mut().enumerate() {
                        let st = status.iter().find(|s| s.url == cfg.url);
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut cfg.enabled, "");
                            if ui.button("✖").on_hover_text("Remove").clicked() {
                                remove = Some(i);
                            }
                            ui.vertical(|ui| {
                                let title = st
                                    .filter(|s| !s.title.is_empty())
                                    .map(|s| s.title.as_str())
                                    .unwrap_or("(untitled)");
                                ui.strong(title);
                                ui.weak(&cfg.url);
                                match st {
                                    Some(s) if s.loaded => {
                                        ui.small(format!("{} items", s.items));
                                    }
                                    Some(_) => {
                                        ui.small("loading…");
                                    }
                                    None => {}
                                }
                            });
                        });
                        ui.separator();
                    }
                });
                if let Some(i) = remove {
                    settings.placefiles.remove(i);
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("URL:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_url).desired_width(340.0).hint_text("http://…/placefile.txt"));
                    let valid = self.new_url.starts_with("http")
                        && !settings.placefiles.iter().any(|c| c.url == self.new_url.trim());
                    if ui.add_enabled(valid, egui::Button::new("Add")).clicked() {
                        settings.placefiles.push(PlacefileConfig {
                            url: self.new_url.trim().to_string(),
                            enabled: true,
                        });
                        self.new_url.clear();
                    }
                });
            });
        self.open = open;
    }
}
