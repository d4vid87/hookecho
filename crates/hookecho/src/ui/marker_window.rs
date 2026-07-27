//! Location Markers manager: name/edit/remove/icon user-placed markers.
//!
//! Markers can also be dropped directly on the map with the "Drop marker" tool.

use crate::settings::{Marker, Settings};
use egui::TextureHandle;
use std::collections::HashMap;

/// Texture cache keyed by marker icon filename. `None` = load failed / missing (negative cache).
pub type IconTextures = HashMap<String, Option<TextureHandle>>;

#[derive(Default)]
pub struct MarkerWindow {
    pub open: bool,
    /// Address/place search box contents.
    pub query: String,
    /// A geocode request is in flight (the app clears this when the result arrives).
    pub searching: bool,
    /// Last search outcome, shown under the box ("Added …" or an error).
    pub status: Option<String>,
}

impl MarkerWindow {
    /// Returns the address/place to geocode when the user submits the search box; the app resolves
    /// it (see `wxdata::geocode`) and adds a marker at the result.
    #[must_use]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        icon_tex: &IconTextures,
    ) -> Option<String> {
        let mut open = self.open;
        let mut go: Option<String> = None;
        crate::ui::fit_phone(ctx, egui::Window::new("Location Markers"))
            .open(&mut open)
            .default_size([520.0, 360.0])
            .show(ctx, |ui| {
                ui.strong("Add by address");
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("City, address, or place")
                            .desired_width(ui.available_width() - 96.0),
                    );
                    let entered =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let label = if self.searching {
                        "Searching…"
                    } else {
                        "🔍 Search"
                    };
                    let clicked = ui
                        .add_enabled(!self.searching, egui::Button::new(label))
                        .clicked();
                    if (clicked || entered) && !self.searching && !self.query.trim().is_empty() {
                        go = Some(self.query.trim().to_string());
                    }
                });
                if let Some(s) = &self.status {
                    ui.weak(s);
                }
                ui.separator();
                ui.weak(
                    "Tip: search a place in the top bar and hit Save marker, or drop one with \
                     Ctrl+K ▸ \"Tool: Drop marker\". Tap a marker on the map to rename or remove it.",
                );
                ui.add_space(4.0);
                marker_grid(ui, &mut settings.markers, icon_tex);
                ui.add_space(6.0);
                if ui.button("➕ Add blank marker").clicked() {
                    let n = settings.markers.len() + 1;
                    settings.markers.push(Marker {
                        name: format!("Marker {n}"),
                        lat: 0.0,
                        lon: 0.0,
                        icon: None,
                    });
                }
            });
        self.open = open;
        go
    }
}

/// Editable marker table (name/lat/lon/icon). Shared by the manager window and the wizard.
pub fn marker_grid(ui: &mut egui::Ui, markers: &mut Vec<Marker>, icon_tex: &IconTextures) {
    let mut remove: Option<usize> = None;
    egui::Grid::new("markers_grid")
        .num_columns(5)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Lat");
            ui.strong("Lon");
            ui.strong("Icon");
            ui.end_row();
            for (i, m) in markers.iter_mut().enumerate() {
                ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(140.0));
                ui.add(
                    egui::DragValue::new(&mut m.lat)
                        .range(-90.0..=90.0)
                        .speed(0.01)
                        .max_decimals(4),
                );
                ui.add(
                    egui::DragValue::new(&mut m.lon)
                        .range(-180.0..=180.0)
                        .speed(0.01)
                        .max_decimals(4),
                );
                ui.horizontal(|ui| {
                    // Thumbnail of the current icon, if its texture is loaded.
                    if let Some(tex) = m
                        .icon
                        .as_ref()
                        .and_then(|n| icon_tex.get(n))
                        .and_then(|t| t.as_ref())
                    {
                        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(20.0, 20.0)));
                    }
                    if ui.button("Browse…").clicked() {
                        if let Some(name) = pick_and_store_icon() {
                            m.icon = Some(name);
                        }
                    }
                    if m.icon.is_some() && ui.button("✖icon").on_hover_text("Clear icon").clicked()
                    {
                        m.icon = None;
                    }
                });
                if ui.button("✖").on_hover_text("Remove marker").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        markers.remove(i);
    }
}

/// Prompt for a PNG, copy it into the marker-icons dir, and return the stored filename.
fn pick_and_store_icon() -> Option<String> {
    let src = crate::dialog::open_path("PNG", &["png"])?;
    let name = src.file_name()?.to_string_lossy().into_owned();
    let dir = Settings::marker_icons_dir()?;
    if let Err(e) = std::fs::copy(&src, dir.join(&name)) {
        log::warn!("marker icon copy failed ({name}): {e}");
        return None;
    }
    Some(name)
}
