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
        let enabled_overlays: Vec<_> = OverlayToggle::ALL.into_iter()
            .filter(|toggle| !toggle.session_only() && *self.overlay_flag(*toggle))
            .collect();
        let entries = self.palette_entries();
        let overlay_sources: Vec<_> = enabled_overlays.into_iter().map(|toggle| {
            let name = entries.iter().find(|entry| matches!(entry.action, PaletteAction::ToggleOverlay(other) if other == toggle))
                .map_or_else(|| format!("{toggle:?}"), |entry| entry.label.clone());
            (name, self.palette_health(PaletteAction::ToggleOverlay(toggle)))
        }).collect();
        if !overlay_sources.is_empty() {
            ui.collapsing("Weather overlay sources", |ui| {
                ui.weak("These overlays are shared by the visible panes.");
                for (name, health) in &overlay_sources {
                    ui.separator();
                    ui.strong(name);
                    if let Some(health) = health {
                        ui.label(format!("Feed: {} · {}", health.source, crate::ui::layers_panel::health_look(health.state()).0));
                        if let Some(age) = health.last_success { ui.label(format!("Last successful receipt {:.1} min ago", age.as_secs_f64() / 60.0)); }
                        if let Some(age) = health.data_age { ui.label(format!("Source valid age {:.1} min", age.as_secs_f64() / 60.0)); }
                        else { ui.weak("No shared valid time; receipt time does not establish observation time."); }
                    } else {
                        ui.weak("Local or feature-level data; no shared source clock.");
                    }
                }
            });
        }
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
                if compact(ui.ctx()) {
                    ui.spacing_mut().interact_size.y = 48.0;
                }
                for &(cut, carries) in &cuts {
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
                    if view.timeline.cut_playback {
                        if let Some(index) = cuts
                            .iter()
                            .filter(|(_, carries)| *carries)
                            .position(|(candidate, _)| *candidate == cut)
                        {
                            view.timeline.cut_index = index;
                        }
                    }
                    view.cut_selection = Some((cut.ended_at_ms, view.moment, view.tilt));
                } else {
                    view.timeline.enable_cut_playback(false);
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
            let field = layer.and_then(|layer| self.field_state(i, layer)?.frame.as_ref());
            let health = layer.and_then(|layer| if layer.is_native_abi() { Some(self.request_health(RequestLane::PaneField(i, layer))) } else { self.palette_health(PaletteAction::ToggleField(layer)) })
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
                ui.collapsing("Layer sources and times", |ui| {
                    let view = &self.views[i];
                    let radar = view.displayed_time();
                    if let Some(time) = radar {
                        ui.label(format!("Radar observed {} UTC", time.format("%Y-%m-%d %H:%M:%S")));
                    } else {
                        ui.weak("Radar displayed time unavailable");
                    }
                    for layer in crate::render::FieldLayer::draw_order().filter(|layer| view.fields_on.contains(layer)) {
                        ui.separator();
                        let state = self.field_state(i, layer);
                        let frame = state.and_then(|state| state.frame.as_ref());
                        let name = frame.map_or_else(|| format!("{layer:?}"), |frame| frame.descriptor.display_name.to_owned());
                        ui.strong(name);
                        let stamp = frame.map(|frame| &frame.stamp)
                            .or_else(|| state.and_then(|state| state.metadata.as_ref().map(|(_, stamp)| stamp)));
                        if let Some(stamp) = stamp {
                            for line in provenance_lines(stamp, radar, chrono::Utc::now()) { ui.label(line); }
                            if !self.field_aligned(i, layer) {
                                ui.colored_label(egui::Color32::from_rgb(255, 186, 86), "Withheld: outside this pane's time tolerance");
                            }
                        } else {
                            ui.weak("Source time unavailable · waiting for metadata");
                        }
                    }
                    if let Some(time) = view.satellite.time {
                        ui.separator();
                        ui.label(format!("Raster satellite selected {} UTC", time.format("%Y-%m-%d %H:%M:%S")));
                        if let Some(radar) = radar { ui.weak(format!("Selected satellite − radar {:+.1}s", (time - radar).num_milliseconds() as f64 / 1000.0)); }
                        ui.weak("Image residency and displayed time are shown on the map");
                    }
                });
                if let Some(probe) = probe {
                    ui.separator();
                    ui.label(probe);
                }
            });
            ui.add_space(6.0);
        }
    }
}

/// Source times retain their meaning: forecast lead is measured from run, never receipt.
fn provenance_lines(stamp: &wxdata::field::DataStamp, radar: Option<chrono::DateTime<chrono::Utc>>, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
    let mut lines = vec![
        format!("Source: {}", stamp.source_identity),
        format!("{} valid {} UTC · {}", stamp.class.label(), stamp.valid_time.format("%Y-%m-%d %H:%M:%S"), stamp.quality.label()),
        format!("Valid age {:+.1} min · received {:.1} min ago", (now - stamp.valid_time).num_milliseconds() as f64 / 60_000.0, (now - stamp.received_time).num_milliseconds().max(0) as f64 / 60_000.0),
    ];
    if let Some(run) = stamp.run_time {
        lines.push(format!("Run {} UTC · lead {:+.1} h", run.format("%Y-%m-%d %H:%M"), (stamp.valid_time - run).num_milliseconds() as f64 / 3_600_000.0));
    }
    if let Some(issue) = stamp.issue_time { lines.push(format!("Issued {} UTC", issue.format("%Y-%m-%d %H:%M:%S"))); }
    if let Some(radar) = radar { lines.push(format!("Layer − displayed radar {:+.1}s", (stamp.valid_time - radar).num_milliseconds() as f64 / 1000.0)); }
    lines
}

#[cfg(test)]
mod provenance_tests {
    #[test]
    fn forecast_lead_uses_run_and_offset_uses_displayed_radar() {
        use chrono::{TimeZone, Utc};
        let run = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
        let stamp = wxdata::field::DataStamp {
            source_identity: "fixture model".into(), issue_time: None, run_time: Some(run),
            valid_time: run + chrono::Duration::hours(3), received_time: run + chrono::Duration::hours(1),
            class: wxdata::field::DataClass::Forecast, quality: wxdata::field::QualitySummary::Good, available_members: None,
        };
        let lines = super::provenance_lines(&stamp, Some(stamp.valid_time - chrono::Duration::milliseconds(1500)), stamp.received_time);
        assert!(lines.iter().any(|line| line.contains("Forecast valid")));
        assert!(lines.iter().any(|line| line.contains("lead +3.0 h")));
        assert!(lines.iter().any(|line| line.contains("radar +1.5s")));
        assert!(lines.iter().any(|line| line.contains("age -120.0 min")));
    }
}
