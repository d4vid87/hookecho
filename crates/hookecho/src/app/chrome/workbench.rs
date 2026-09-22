//! Optional analyst dock, sharing the same frames and probes as the map.

use super::*;

impl HookEchoApp {
    pub(crate) fn workbench_inspector(&mut self, _ctx: &egui::Context, root: &mut egui::Ui) {
        if !self.analyst_open || !self.analyst_inspector_open
            || self.chrome_rect.width() < 1200.0 || self.drawer.is_open()
        {
            return;
        }
        egui::Panel::right("analyst_inspector_dock")
            .resizable(true)
            .default_size(292.0)
            .size_range(240.0..=380.0)
            .show(root, |ui| {
                ui.add_space(48.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.workbench_inspector_body(ui);
                });
            });
    }

    pub(crate) fn workbench_inspector_body(&mut self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        ui.weak("Native values and actual source times");
        if let Some([lon, lat]) = self.linked_probe {
            ui.label(format!("Probe  {lat:.3}°, {lon:.3}°"));
        } else {
            ui.weak("Move over a linked pane to compare one location.");
        }
        ui.separator();
        for i in 0..self.views.len() {
            let view = &self.views[i];
            let site = view.site.as_deref().unwrap_or("No radar").to_string();
            let product = view.moment.short_name();
            let active = i == self.active;
            let layer = crate::render::FieldLayer::draw_order()
                .rev()
                .find(|layer| view.fields_on.contains(layer));
            let field = layer.and_then(|layer| self.fields.get(&layer)?.frame.as_ref());
            let health = layer.and_then(|layer| self.palette_health(PaletteAction::ToggleField(layer)))
                .map(|health| format!(" · {}", crate::ui::layers_panel::health_look(health.state()).0))
                .unwrap_or_default();
            let source = field.map(|frame| format!(
                "{} · {} · valid {} · {}",
                frame.descriptor.short_name,
                frame.stamp.source_identity,
                frame.stamp.valid_time.format("%H:%M UTC"),
                frame.stamp.quality.label(),
            )).or_else(|| view.volume.as_ref().map(|volume| format!(
                "{site} · {product} · valid {}", volume.time.format("%H:%M UTC")
            ))).unwrap_or_else(|| format!("{site} · {product} · waiting for data")) + &health;
            let probe = self.linked_probe.and_then(|[lon, lat]| {
                self.field_probe_at(i, lon, lat)
                    .or_else(|| self.radar_probe_at(i, lon, lat))
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(active, format!("Pane {}", i + 1)).clicked() {
                        self.active = i;
                    }
                    ui.weak(&site);
                });
                ui.label(source);
                if let Some(probe) = probe {
                    ui.separator();
                    ui.label(probe);
                }
            });
            ui.add_space(6.0);
        }
    }
}
