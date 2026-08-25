//! Windows that never grew their own ui/ module.

use super::*;

impl HookEchoApp {
    /// The tornado-climatology results window: a magnitude histogram + strongest-first list of
    /// historical tornadoes near the clicked point.
    pub(crate) fn show_climatology_window(&mut self, ctx: &egui::Context) {
        if !self.climo_open {
            return;
        }
        let mut open = self.climo_open;
        crate::ui::phone_surface(ctx, egui::Window::new("Tornado climatology"))
            .open(&mut open)
            .default_width(360.0)
            .show(ctx, |ui| {
                if let Some((lon, lat)) = self.climo_center {
                    ui.label(format!("Within 25 mi of {lat:.3}, {lon:.3}"));
                }
                if self.climo_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading SPC tornado database (1950–2022)…");
                    });
                } else if let Some(e) = &self.climo_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 90, 90),
                        format!("Load failed: {e}"),
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.strong(format!("{} tornadoes on record", self.climo_hits.len()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let hits = &self.climo_hits;
                            crate::ui::csv_buttons(
                                ui,
                                "tornadoes.csv",
                                "Every tornado on record here, not just the first 50",
                                || {
                                    let mut s = String::from(
                                        "year,mag,start_lat,start_lon,end_lat,end_lon\n",
                                    );
                                    for t in hits {
                                        s.push_str(&format!(
                                            "{},{},{:.4},{:.4},{:.4},{:.4}\n",
                                            t.year, t.mag, t.slat, t.slon, t.elat, t.elon
                                        ));
                                    }
                                    s
                                },
                            );
                        });
                    });
                    let hist = wxdata::torclimo::mag_histogram(&self.climo_hits);
                    ui.horizontal_wrapped(|ui| {
                        for (i, label) in ["EF0", "EF1", "EF2", "EF3", "EF4", "EF5", "Unk"]
                            .iter()
                            .enumerate()
                        {
                            crate::theme::stat_card(ui, label, &hist[i].to_string());
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for t in self.climo_hits.iter().take(50) {
                                let mag = if t.mag < 0 {
                                    "EF?".to_string()
                                } else {
                                    format!("EF{}", t.mag)
                                };
                                ui.label(format!(
                                    "{}  {}  start {:.2},{:.2}",
                                    t.year, mag, t.slat, t.slon
                                ));
                            }
                            if self.climo_hits.len() > 50 {
                                ui.weak(format!("… and {} more", self.climo_hits.len() - 50));
                            }
                        });
                }
                ui.separator();
                ui.strong("Warning history");
                ui.weak("How often this spot's county has been warned (IEM, 1986–present).");
                match (&self.climo_warn, self.climo_warn_rx.is_some()) {
                    (Some(s), _) if s.total == 0 => {
                        ui.label("No warnings on record here.");
                    }
                    (Some(s), _) => {
                        ui.horizontal_wrapped(|ui| {
                            crate::theme::stat_card(ui, "Warnings", &s.total.to_string());
                            if let Some(y) = s.first_year {
                                crate::theme::stat_card(ui, "Since", &y.to_string());
                            }
                            if let Some((y, n)) = s.busiest_year {
                                crate::theme::stat_card(ui, "Busiest year", &format!("{y} ({n})"));
                            }
                            if let Some((d, n)) = s.worst_day {
                                crate::theme::stat_card(ui, "Worst day", &format!("{d} ({n})"));
                            }
                        });
                        for (name, n) in s.by_name.iter().take(8) {
                            ui.label(format!("{n} × {name}"));
                        }
                    }
                    (None, true) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading warning history…");
                        });
                    }
                    (None, false) => {
                        ui.weak("Warning history unavailable.");
                    }
                }
            });
        self.climo_open = open;
    }

    /// Offer back the report from a run that panicked. Once, with a Copy button — the point is
    /// that a crash the user can only describe as "it closed" becomes something they can paste
    /// into an issue.
    pub(crate) fn crash_report_window(&mut self, ctx: &egui::Context) {
        let Some(report) = self.crash_report.clone() else {
            return;
        };
        let mut open = true;
        let mut dismiss = false;
        egui::Window::new("Hook Echo-WX closed unexpectedly last time")
            .open(&mut open)
            .collapsible(false)
            .default_size([560.0, 320.0])
            .show(ctx, |ui| {
                ui.label("Here is what it left behind. Nothing in it identifies you.");
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut report.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy report").clicked() {
                        ui.ctx().copy_text(report.clone());
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                    ui.hyperlink_to(
                        "Open an issue",
                        "https://github.com/d4vid87/hookecho/issues/new",
                    );
                });
            });
        if dismiss || !open {
            #[cfg(not(target_arch = "wasm32"))]
            crate::crash::dismiss();
            self.crash_report = None;
        }
    }
    /// The card behind a click on one of your own watch zones: rename it, or get rid of it.
    pub(crate) fn zone_popup_card(&mut self, ctx: &egui::Context) {
        if let Some(i) = self.zone_popup {
            match self.settings.alert_polygons.get_mut(i) {
                None => self.zone_popup = None,
                Some(z) => {
                    let mut open = true;
                    let mut remove = false;
                    self.popovers
                        .card(ctx, "zone", egui::Window::new("Watch zone"))
                        .open(&mut open)
                        .resizable(false)
                        .vscroll(false)
                        .default_width(240.0)
                        .show(ctx, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut z.name)
                                    .hint_text("Name")
                                    .desired_width(ui.available_width()),
                            );
                            ui.weak(format!("{} corners", z.ring.len()));
                            // ponytail: no vertex editing — redrawing a four-click shape is
                            // faster than any handle-dragging UI would be to build.
                            remove = ui.button("✖ Remove").clicked();
                        });
                    if remove {
                        self.settings.alert_polygons.remove(i);
                        self.zone_popup = None;
                        self.settings.save();
                    } else if !open {
                        self.zone_popup = None;
                        self.settings.save();
                    }
                }
            }
        }
    }

    /// Naming step for a watch zone that was just drawn — modal in spirit, so it stays centered
    /// rather than anchored to the last corner clicked.
    pub(crate) fn zone_naming_dialog(&mut self, ctx: &egui::Context) {
        if self.zone_naming.is_some() {
            let mut save = false;
            let mut cancel = false;
            if let Some((ring, name)) = &mut self.zone_naming {
                egui::Window::new("Name this watch zone")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.weak(format!("{} corners", ring.len()));
                        let edit = ui.add(
                            egui::TextEdit::singleline(name)
                                .hint_text("Home area")
                                .desired_width(220.0),
                        );
                        edit.request_focus();
                        ui.weak("Alerts fire when a warning polygon touches this area.");
                        ui.horizontal(|ui| {
                            save = ui.button("Save").clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Enter));
                            cancel = ui.button("Cancel").clicked();
                        });
                    });
            }
            if save {
                if let Some((ring, name)) = self.zone_naming.take() {
                    self.settings
                        .alert_polygons
                        .push(crate::settings::AlertPolygon { name, ring });
                    self.settings.save();
                }
            } else if cancel {
                self.zone_naming = None;
            }
        }
    }
}
