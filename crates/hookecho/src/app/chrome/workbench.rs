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
        let view = &mut self.views[self.active];
        if ui.checkbox(&mut view.follow_low_cut, "Follow newest 0.5° cut")
            .on_hover_text("Keep this pane on the newest low-level radar revisit while live")
            .changed()
        {
            view.pin_low_cut();
        }
        if view.follow_low_cut && !view.timeline.following {
            ui.weak("Paused while viewing archive radar");
        }
        ui.checkbox(&mut self.show_scan_progress, "Scan progress")
            .on_hover_text("Show which azimuths of the selected radar cut belong to the current live volume");
        if self.show_scan_progress {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(80, 220, 255), "New scan");
                ui.colored_label(egui::Color32::from_rgb(255, 186, 86), "Older scan");
                ui.colored_label(egui::Color32::from_gray(110), "Not received");
            });
            if !view
                .volume
                .as_ref()
                .and_then(|volume| volume.live_status.as_ref())
                .is_some_and(|status| status.stream_active && status.volume_start_ms.is_some())
            {
                ui.weak("Waiting for live scan updates");
            }
        }
        let selectable = view.custom_product.is_none()
            && view.volume.as_ref().is_some_and(|v| v.live_status.is_none());
        let cuts = view
            .volume
            .as_ref()
            .map(|volume| {
                volume
                    .cuts
                    .iter()
                    .copied()
                    .map(|cut| {
                        (
                            cut,
                            selectable
                                && wxdata::level2::cut_has_moment(
                                    &volume.scan,
                                    cut,
                                    view.moment,
                                ),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !cuts.is_empty() {
            let mut chosen = None;
            ui.collapsing("Cut chronology", |ui| {
                for (cut, carries) in cuts {
                    if let Some(time) = chrono::DateTime::from_timestamp_millis(cut.ended_at_ms) {
                        let label = format!(
                            "Cut {} · {:.1}° · {} UTC",
                            cut.elevation_number,
                            cut.elevation_deg,
                            time.format("%H:%M:%S")
                        );
                        if carries {
                            if ui
                                .selectable_label(
                                    view.selected_cut_ms() == Some(cut.ended_at_ms),
                                    label,
                                )
                                .clicked()
                            {
                                chosen = Some(
                                    (view.selected_cut_ms() != Some(cut.ended_at_ms))
                                        .then_some(cut),
                                );
                            }
                        } else {
                            ui.weak(label);
                        }
                    }
                }
            });
            if let Some(cut) = chosen {
                if let Some(cut) = cut {
                    if let Some(volume) = &view.volume {
                        if let Some(tilt) = volume
                            .elevations
                            .iter()
                            .position(|angle| (*angle - cut.elevation_deg).abs() < 0.15)
                        {
                            view.tilt = tilt;
                        }
                    }
                    view.timeline.playing = false;
                    view.timeline.following = false;
                    view.cut_selection = Some((cut.ended_at_ms, view.moment, view.tilt));
                } else {
                    view.cut_selection = None;
                }
            }
        }
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
            )).or_else(|| view.volume.as_ref().map(|volume| {
                let time = view.selected_cut_ms()
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .unwrap_or(volume.time);
                format!("{site} · {product} · valid {}", time.format("%H:%M UTC"))
            })).unwrap_or_else(|| format!("{site} · {product} · waiting for data")) + &health;
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
