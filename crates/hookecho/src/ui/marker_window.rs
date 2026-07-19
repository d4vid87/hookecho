//! Location Markers manager: name/edit/remove user-placed markers.
//!
//! Markers can also be dropped directly on the map via Tools ▸ Drop marker.

use crate::settings::{Marker, Settings};

#[derive(Default)]
pub struct MarkerWindow {
    pub open: bool,
}

impl MarkerWindow {
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        let mut open = self.open;
        egui::Window::new("Location Markers")
            .open(&mut open)
            .default_size([420.0, 300.0])
            .show(ctx, |ui| {
                ui.weak("Tip: Tools ▸ Drop marker adds one by clicking the map.");
                ui.add_space(4.0);

                let mut remove: Option<usize> = None;
                egui::Grid::new("markers_grid").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Lat");
                    ui.strong("Lon");
                    ui.end_row();
                    for (i, m) in settings.markers.iter_mut().enumerate() {
                        ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(160.0));
                        ui.add(egui::DragValue::new(&mut m.lat).range(-90.0..=90.0).speed(0.01).max_decimals(4));
                        ui.add(egui::DragValue::new(&mut m.lon).range(-180.0..=180.0).speed(0.01).max_decimals(4));
                        if ui.button("✖").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    settings.markers.remove(i);
                }

                ui.add_space(6.0);
                if ui.button("➕ Add marker").clicked() {
                    let n = settings.markers.len() + 1;
                    settings.markers.push(Marker { name: format!("Marker {n}"), lat: 0.0, lon: 0.0 });
                }
            });
        self.open = open;
    }
}
