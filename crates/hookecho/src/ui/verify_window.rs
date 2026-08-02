//! Warning verification lab: how well did this office's warnings do?
//!
//! The scoring is IEM's (see `wxdata::verify`); this window is the reading of it. It answers the
//! two questions a chaser or a forecaster actually asks about a past event — did the warning go
//! out in time, and what happened that nobody warned for — and it does that in a table rather than
//! a dashboard, because the individual rows are where the story is.

use wxdata::verify::Verification;

/// What the window wants the app to do this frame.
#[derive(Default)]
pub struct VerifyActions {
    /// Run (or re-run) the query for the shown WFO and window.
    pub refresh: bool,
    /// Fly the map to a warning or report the user clicked, and (for a warning) to its issue time.
    pub goto: Option<(f64, f64, Option<chrono::DateTime<chrono::Utc>>)>,
}

#[derive(Default)]
pub struct VerifyWindow {
    pub open: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub data: Option<Verification>,
    /// The office to score. Prefilled from whatever warnings are in view, editable.
    pub wfo: String,
    /// The window to score, as `YYYY-MM-DD` — the archive day the timeline is on.
    pub day: String,
    /// Show unwarned reports only; the misses are usually what you came for.
    pub misses_only: bool,
}

impl VerifyWindow {
    pub fn show(&mut self, ctx: &egui::Context, tz: Option<wxdata::tz::Tz>) -> VerifyActions {
        let mut act = VerifyActions::default();
        if !self.open {
            return act;
        }
        let mut open = self.open;
        crate::ui::phone_surface(ctx, egui::Window::new("Warning Verification"))
            .open(&mut open)
            .default_width(560.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Office:");
                    ui.add(egui::TextEdit::singleline(&mut self.wfo).desired_width(56.0));
                    ui.label("Day (UTC):");
                    ui.add(egui::TextEdit::singleline(&mut self.day).desired_width(96.0));
                    if ui.button("Verify").clicked() {
                        act.refresh = true;
                    }
                    if self.busy {
                        crate::ui::loading(ui, "Scoring warnings…");
                    }
                });
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 120), e);
                }
                let Some(v) = &self.data else {
                    ui.weak(
                        "Scored against storm reports by the Iowa Environmental Mesonet's Cow \
                         service — the same arbitration the published numbers use.",
                    );
                    return;
                };
                ui.separator();
                stats_row(ui, v);
                ui.separator();
                ui.checkbox(&mut self.misses_only, "Unwarned reports only");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if !self.misses_only {
                        ui.strong("Warnings");
                        for w in &v.warnings {
                            // ASCII marks, not check/cross glyphs: the monospace face the rows
                            // use has no coverage for those and draws tofu boxes instead.
                            let mark = if w.verified { "+" } else { "x" };
                            let color = if w.verified {
                                egui::Color32::from_rgb(120, 220, 140)
                            } else {
                                egui::Color32::from_rgb(230, 130, 130)
                            };
                            let lead = w
                                .lead_min
                                .map(|m| format!("{m} min lead"))
                                .unwrap_or_else(|| "no verifying report".into());
                            let label = format!(
                                "{mark} {}-{:04}  {}  — {lead}  ({})",
                                w.phenomena,
                                w.eventid,
                                crate::timefmt::fmt_clock(w.issue, tz, true),
                                w.counties.join(", ")
                            );
                            if ui
                                .add(egui::Label::new(
                                    egui::RichText::new(label).color(color).monospace(),
                                ))
                                .on_hover_text("Click to fly there at the moment it was issued")
                                .clicked()
                            {
                                act.goto = Some((w.lon, w.lat, Some(w.issue)));
                            }
                        }
                        ui.add_space(6.0);
                    }
                    ui.strong("Storm reports");
                    for r in v.reports.iter().filter(|r| !self.misses_only || !r.warned) {
                        let color = if r.warned {
                            egui::Color32::from_rgb(120, 220, 140)
                        } else {
                            egui::Color32::from_rgb(230, 130, 130)
                        };
                        let mag = r
                            .magnitude
                            .map(|m| format!(" {m}"))
                            .unwrap_or_else(String::new);
                        let lead = match (r.warned, r.lead_min) {
                            (true, Some(m)) => format!("warned {m} min ahead"),
                            (true, None) => "warned".to_string(),
                            (false, _) => "NOT WARNED".to_string(),
                        };
                        let label = format!(
                            "{}  {}{mag}  {} — {lead}",
                            crate::timefmt::fmt_clock(r.valid, tz, true),
                            r.kind,
                            r.city
                        );
                        if ui
                            .add(egui::Label::new(
                                egui::RichText::new(label).color(color).monospace(),
                            ))
                            .clicked()
                        {
                            act.goto = Some((r.lon, r.lat, Some(r.valid)));
                        }
                    }
                });
            });
        self.open = open;
        act
    }
}

fn stats_row(ui: &mut egui::Ui, v: &Verification) {
    let s = &v.stats;
    ui.horizontal(|ui| {
        let metric = |ui: &mut egui::Ui, label: &str, value: String, hover: &str| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(value).size(20.0).strong());
                ui.weak(label);
            })
            .response
            .on_hover_text(hover);
            ui.add_space(12.0);
        };
        metric(
            ui,
            "POD",
            format!("{:.0}%", s.pod * 100.0),
            "Probability of detection — the share of storm reports that had a warning out",
        );
        metric(
            ui,
            "FAR",
            format!("{:.0}%", s.far * 100.0),
            "False alarm ratio — the share of warnings nothing verified",
        );
        metric(
            ui,
            "CSI",
            format!("{:.2}", s.csi),
            "Critical success index — one number that punishes both misses and false alarms",
        );
        metric(
            ui,
            "avg lead",
            format!("{:.0} min", s.avg_lead_min),
            "Mean minutes between a warning going out and the first report that verified it",
        );
    });
    ui.weak(format!(
        "{}/{} warnings verified · {} reports ({} warned, {} missed) · mean polygon {:.0} km²",
        s.events_verified,
        s.events_total,
        s.reports_total,
        s.warned_reports,
        s.unwarned_reports,
        s.avg_size_km2
    ));
}
