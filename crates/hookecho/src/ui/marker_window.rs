//! Location Markers manager: name/edit/remove/icon user-placed markers.
//!
//! Markers can also be dropped directly on the map with the "Drop marker" tool.

use crate::settings::{Marker, Settings};
use egui::TextureHandle;
use std::collections::HashMap;

/// Texture cache keyed by marker icon filename. `None` = load failed / missing (negative cache).
pub type IconTextures = HashMap<String, Option<TextureHandle>>;

/// Diameter of a marker icon on the map, in points. Icons are drawn as discs.
pub const ICON_D: f32 = 24.0;

#[derive(Default)]
pub struct MarkerWindow {
    pub open: bool,
    /// Address/place search box contents.
    pub query: String,
    /// A geocode request is in flight (the app clears this when the result arrives).
    pub searching: bool,
    /// Last search outcome, shown under the box ("Added …" or an error).
    pub status: Option<String>,
    /// Marker index removed this frame. The app's map popup indexes into the same list, so a
    /// delete above it leaves it describing somebody else's marker.
    pub removed: Option<usize>,
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
        self.removed = None;
        crate::ui::phone_surface(ctx, egui::Window::new("Location Markers"))
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
                self.removed = marker_grid(ui, &mut settings.markers, icon_tex).or(self.removed);
                ui.add_space(6.0);
                if ui.button("➕ Add blank marker").clicked() {
                    let n = settings.markers.len() + 1;
                    settings.markers.push(Marker {
                        name: format!("Marker {n}"),
                        lat: 0.0,
                        lon: 0.0,
                        icon: None,
                        alert_radius_mi: crate::settings::default_alert_radius_mi(),
                            video_url: String::new(),
                        home: false,
                    });
                }
            });
        self.open = open;
        go
    }
}

/// Editable marker table (home/name/lat/lon/watch radius/icon). Shared by the manager window and
/// the wizard.
/// Returns the index that was removed this frame, if any: a popup open on a later marker is
/// pointing at the wrong one afterwards, and the caller is the only place that knows.
pub fn marker_grid(
    ui: &mut egui::Ui,
    markers: &mut Vec<Marker>,
    icon_tex: &IconTextures,
) -> Option<usize> {
    let mut remove: Option<usize> = None;
    let mut make_home: Option<usize> = None;
    egui::Grid::new("markers_grid")
        .num_columns(8)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.strong("Home")
                .on_hover_text("The one place alerts speak of first");
            ui.strong("Name");
            ui.strong("Lat");
            ui.strong("Lon");
            ui.strong("Watch")
                .on_hover_text("Alert when a warning comes within this many miles");
            ui.strong("Video")
                .on_hover_text("Live stream URL for this place (HLS/MJPEG plays in-app)");
            ui.strong("Icon");
            ui.end_row();
            for (i, m) in markers.iter_mut().enumerate() {
                // Radio, not a checkbox: exactly one marker is home.
                if ui.radio(m.home, "").clicked() {
                    make_home = Some(i);
                }
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
                ui.add(
                    egui::DragValue::new(&mut m.alert_radius_mi)
                        .range(0.0..=200.0)
                        .speed(1.0)
                        .max_decimals(0)
                        .suffix(" mi"),
                )
                .on_hover_text("0 = only alert when the warning polygon actually covers this spot");
                ui.add(
                    egui::TextEdit::singleline(&mut m.video_url)
                        .desired_width(160.0)
                        .hint_text("stream URL"),
                );
                ui.horizontal(|ui| {
                    // Thumbnail of the current icon, if its texture is loaded.
                    if let Some(tex) = m
                        .icon
                        .as_ref()
                        .and_then(|n| icon_tex.get(n))
                        .and_then(|t| t.as_ref())
                    {
                        // Rounded to match how the marker actually draws on the map.
                        ui.add(
                            egui::Image::new(tex)
                                .fit_to_exact_size(egui::vec2(20.0, 20.0))
                                .corner_radius(10.0),
                        );
                    }
                    if ui.button("Browse…").clicked() {
                        crate::dialog::request_open(
                            crate::dialog::ImportKind::MarkerIcon,
                            i.to_string(),
                        );
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
    if let Some(i) = make_home {
        for (j, m) in markers.iter_mut().enumerate() {
            m.home = j == i;
        }
    }
    remove
}

/// Copy a picked PNG into the marker-icons dir and return the stored filename.
pub(crate) fn store_icon(src: &std::path::Path) -> Option<String> {
    let name = src.file_name()?.to_string_lossy().into_owned();
    let dir = Settings::marker_icons_dir()?;
    if let Err(e) = std::fs::copy(src, dir.join(&name)) {
        log::warn!("marker icon copy failed ({name}): {e}");
        return None;
    }
    Some(name)
}
