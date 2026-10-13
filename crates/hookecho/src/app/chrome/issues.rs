//! Session-local, bounded radar issues. An unresolved error remains actionable.
use super::*;
use crate::ui::a11y::Named as _;

#[derive(Default)]
pub(crate) struct RadarIssueLog {
    previous: Vec<(usize, String, String)>,
    history: std::collections::VecDeque<(String, String, f64)>,
}
impl RadarIssueLog {
    fn observe(&mut self, errors: &[(usize, String, String, String)], now: f64) {
        let current: Vec<_> = errors
            .iter()
            .map(|(pane, site, _, error)| (*pane, site.clone(), error.clone()))
            .collect();
        for ((_, _, context, message), key) in errors.iter().zip(&current) {
            if !self.previous.contains(key) {
                self.history
                    .push_front((context.clone(), message.clone(), now));
                self.history.truncate(8);
            }
        }
        self.previous = current;
    }
}

impl HookEchoApp {
    pub(crate) fn error_chip(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let errors: Vec<_> = self
            .views
            .iter()
            .enumerate()
            .filter_map(|(idx, v)| {
                v.error.as_ref().map(|error| {
                    (
                        idx,
                        v.site.clone().unwrap_or_else(|| "No station".into()),
                        format!("Pane {} · {}", idx + 1, self.pane_identity(idx)),
                        error.clone(),
                    )
                })
            })
            .collect();
        self.radar_issues.observe(&errors, now);
        if self.radar_issues.history.is_empty() {
            return;
        }
        let mut retry = None;
        let mut choose_site = None;
        let mut clear_history = false;
        let response = egui::Area::new("radar_issues".into())
            .order(egui::Order::Foreground)
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, if compact(ctx) { -210.0 } else { -110.0 }),
            )
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 246).show(ui, |ui| {
                    ui.spacing_mut().interact_size.y = 44.0;
                    ui.menu_button(
                        if errors.is_empty() {
                            "Recent radar issues".into()
                        } else {
                            format!("Radar issues · {}", errors.len())
                        },
                        |ui| {
                            ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 360.0));
                            if errors.is_empty() {
                                ui.label("No unresolved radar errors.");
                            }
                            for (idx, _, context, error) in &errors {
                                ui.strong(context);
                                ui.label(error);
                                ui.horizontal(|ui| {
                                    if ui
                                        .button("Retry")
                                        .named(&format!("Retry {context}"))
                                        .clicked()
                                    {
                                        retry = Some(*idx);
                                        ui.close();
                                    }
                                    if ui
                                        .button("Change station")
                                        .named(&format!("Change station for {context}"))
                                        .clicked()
                                    {
                                        choose_site = Some(*idx);
                                        ui.close();
                                    }
                                });
                                ui.separator();
                            }
                            egui::CollapsingHeader::new("Recent issues (this session)")
                                .default_open(errors.is_empty())
                                .show(ui, |ui| {
                                    for (context, message, at) in &self.radar_issues.history {
                                        ui.strong(context);
                                        ui.weak(format!(
                                            "Reported {}s ago",
                                            (now - at).max(0.0) as u64
                                        ));
                                        ui.label(message);
                                        ui.separator();
                                    }
                                });
                            if errors.is_empty() && ui.button("Clear resolved history").clicked() {
                                clear_history = true;
                                ui.close();
                            }
                        },
                    );
                });
            });
        if clear_history && errors.is_empty() {
            self.radar_issues.history.clear();
        }
        self.mobile_occlusion.push(response.response.rect);
        if let Some(idx) = retry {
            self.active = idx;
            let view = &mut self.views[idx];
            view.head_request.cancel();
            view.frame_request.cancel();
            view.loading = false;
            view.last_poll = None;
            self.trigger_reload(ctx);
            ctx.request_repaint();
        }
        if let Some(idx) = choose_site {
            self.active = idx;
            self.apply_palette(PaletteAction::OpenWindow(AppWindow::Site), ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_errors_are_not_duplicated_and_recurrences_are_recorded_with_a_bound() {
        let mut log = RadarIssueLog::default();
        let error = vec![(
            0,
            "KDMX".into(),
            "Reflectivity · KDMX".into(),
            "Offline".into(),
        )];
        log.observe(&error, 0.0);
        log.observe(&error, 60.0);
        assert_eq!(log.history.len(), 1);
        assert_eq!(log.previous.len(), 1);
        log.observe(&[], 61.0);
        log.observe(&error, 62.0);
        assert_eq!(log.history.len(), 2);
        for n in 0..12 {
            log.observe(
                &[(
                    1,
                    "KFWS".into(),
                    "Velocity · KFWS".into(),
                    format!("Error {n}"),
                )],
                63.0 + f64::from(n),
            );
        }
        assert_eq!(log.history.len(), 8);
        assert_eq!(log.history.front().unwrap().1, "Error 11");
    }
}
