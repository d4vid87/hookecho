//! Compact storm summary with expandable full attributes.

use crate::theme::{self, stat_card};
use wxdata::level3::Cell;

const KT_TO_MPH: f32 = 1.150_78;

/// One per-volume trend sample for a storm cell.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellSample {
    pub vil: Option<f32>,
    pub top: Option<f32>,
    pub dbz: Option<f32>,
}

/// Show the storm-attributes window. `trend` is the cell's per-volume history (oldest→newest);
/// `following` reflects whether the camera is currently tracking this cell. Returns
/// `(still_open, follow_toggled, to_3d)` — the two flags are `true` for the one frame their
/// button is hit.
pub fn show(
    ctx: &egui::Context,
    cell: &Cell,
    trend: &[CellSample],
    following: bool,
    tz: Option<wxdata::tz::Tz>,
    popovers: &mut crate::ui::popover::Popovers,
) -> (bool, bool, bool) {
    let mut open = true;
    let mut follow_toggled = false;
    let mut to_3d = false;
    popovers
        .card(
            ctx,
            "cell",
            egui::Window::new(format!(
                "Cell {}, {}",
                cell.id,
                track_time(cell.time, 0, tz)
            ))
            .id(egui::Id::new(("cell_summary", &cell.id))),
        )
        .open(&mut open)
        .default_width(290.0)
        .resizable(false)
        .collapsible(false)
        .frame(
            egui::Frame::window(&ctx.style_of(ctx.theme()))
                .fill(egui::Color32::from_gray(80))
                .corner_radius(12)
                .inner_margin(12),
        )
        .show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
            ui.label(
                egui::RichText::new(format!("Hail Size: {}", opt(cell.hail_in, "\"", 2)))
                    .size(19.0),
            );
            let movement = match (cell.mvt_deg, cell.mvt_kt) {
                (Some(dir), Some(kt)) => format!("{} at {kt:.0} kts", crate::geo::compass(dir)),
                _ => "Movement: —".into(),
            };
            ui.label(egui::RichText::new(movement).size(19.0));
            egui::CollapsingHeader::new(
                egui::RichText::new("ⓘ Full attributes")
                    .color(egui::Color32::from_rgb(80, 190, 235)),
            )
            .id_salt(("cell_attributes", &cell.id))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(460.0)
                    .show(ui, |ui| {
                        let label = if following {
                            "Following ✓ (tap to stop)"
                        } else {
                            "⌖ Follow"
                        };
                        if ui
                    .add(egui::Button::new(label).min_size(egui::vec2(ui.available_width(), 0.0)))
                    .on_hover_text(
                        "Keep the camera centered on this cell as it moves through each new volume",
                    )
                    .clicked()
                {
                    follow_toggled = true;
                }
                        if ui
                    .add(
                        egui::Button::new("\u{25A6} See in 3D")
                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                    )
                    .on_hover_text(
                        "Open the raymarched volume cropped to this storm \u{2014} the whole box \
                         at once is a wall of echo you then have to hunt through",
                    )
                    .clicked()
                {
                    to_3d = true;
                }
                        theme::section(ui, "Current Position", |ui| {
                            grid(
                                ui,
                                &[
                                    ("Latitude", format!("{:.3}°", cell.lat)),
                                    ("Longitude", format!("{:.3}°", cell.lon)),
                                    ("Range", opt(cell.range_nm, " NM", 0)),
                                    ("Bearing", opt(cell.az_deg, "°", 0)),
                                ],
                            );
                        });
                        theme::section(ui, "Movement", |ui| {
                            let mph = cell.mvt_kt.map(|k| k * KT_TO_MPH);
                            grid(
                                ui,
                                &[
                                    ("Speed", opt(mph, " mph", 0)),
                                    ("Direction", opt(cell.mvt_deg, "°", 0)),
                                ],
                            );
                        });
                        theme::section(ui, "Intensity & Structure", |ui| {
                            let base = cell.base_kft.map(|b| {
                                format!("{}{:.1} kft", if cell.base_below { "<" } else { "" }, b)
                            });
                            grid(
                                ui,
                                &[
                                    ("Max dBZ", opt(cell.max_dbz, " dBZ", 0)),
                                    ("Max ref hgt", opt(cell.max_dbz_hgt_kft, " kft", 1)),
                                    ("Cell top", opt(cell.top_kft, " kft", 1)),
                                    ("Cell base", base.unwrap_or_else(|| "—".into())),
                                    ("Cell-based VIL", opt(cell.vil, "", 0)),
                                ],
                            );
                        });
                        theme::section(ui, "Hail Potential", |ui| {
                            grid(
                                ui,
                                &[
                                    (
                                        "POH",
                                        cell.poh
                                            .map(|v| format!("{v}%"))
                                            .unwrap_or_else(|| "—".into()),
                                    ),
                                    (
                                        "POSH",
                                        cell.posh
                                            .map(|v| format!("{v}%"))
                                            .unwrap_or_else(|| "—".into()),
                                    ),
                                    ("Max size", opt(cell.hail_in, " in", 2)),
                                ],
                            );
                        });
                        theme::section(ui, "Features", |ui| {
                            grid(
                                ui,
                                &[
                                    ("TVS", cell.tvs.clone().unwrap_or_else(|| "None".into())),
                                    (
                                        "Mesocyclone",
                                        cell.meso.clone().unwrap_or_else(|| "None".into()),
                                    ),
                                ],
                            );
                        });
                        theme::section(ui, "Error Metrics", |ui| {
                            grid(
                                ui,
                                &[
                                    ("Forecast error", opt(cell.fcst_err_nm, " NM", 1)),
                                    ("Mean error", opt(cell.mean_err_nm, " NM", 1)),
                                ],
                            );
                        });
                        if trend.len() >= 2 {
                            theme::section(ui, "Trends (per volume)", |ui| {
                                trend_row(
                                    ui,
                                    "Max dBZ",
                                    trend,
                                    |s| s.dbz,
                                    egui::Color32::from_rgb(255, 140, 90),
                                );
                                trend_row(
                                    ui,
                                    "Cell top kft",
                                    trend,
                                    |s| s.top,
                                    egui::Color32::from_rgb(120, 200, 140),
                                );
                                trend_row(
                                    ui,
                                    "Cell-based VIL",
                                    trend,
                                    |s| s.vil,
                                    egui::Color32::from_rgb(90, 170, 255),
                                );
                            });
                        }
                    });
            });
        });
    (open, follow_toggled, to_3d)
}

/// A labelled trend sparkline over the samples that carry the selected field.
fn trend_row(
    ui: &mut egui::Ui,
    label: &str,
    trend: &[CellSample],
    f: fn(&CellSample) -> Option<f32>,
    color: egui::Color32,
) {
    let vals: Vec<f32> = trend.iter().filter_map(f).collect();
    ui.label(egui::RichText::new(label).small().weak());
    theme::sparkline(ui, &vals, color);
}

/// Format an optional value with a unit suffix and `decimals` precision (`—` when absent).
fn opt(v: Option<f32>, unit: &str, decimals: usize) -> String {
    v.map(|x| format!("{x:.*}{unit}", decimals))
        .unwrap_or_else(|| "—".into())
}

/// Lay out label/value pairs. Desktop: wrapped stat cards. Android: a compact vertical
/// `LABEL: value` list — the fixed-width cards force the window wider than the phone screen and
/// clip both edges, so a shrinkable list is the only reliable fit.
fn grid(ui: &mut egui::Ui, cards: &[(&str, String)]) {
    if cfg!(target_os = "android") {
        for (label, value) in cards {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{}:", label.to_uppercase()))
                        .size(11.0)
                        .weak(),
                );
                ui.label(egui::RichText::new(value).size(14.5).strong());
            });
        }
    } else {
        ui.horizontal_wrapped(|ui| {
            for (label, value) in cards {
                stat_card(ui, label, value);
            }
        });
    }
}

/// Anchor forecast clocks to the storm product, never the wall clock or radar playhead.
pub fn track_time(
    time: Option<chrono::DateTime<chrono::Utc>>,
    minutes: u16,
    tz: Option<wxdata::tz::Tz>,
) -> String {
    time.map(|t| {
        crate::timefmt::fmt_clock(t + chrono::Duration::minutes(minutes.into()), tz, false)
    })
    .unwrap_or_else(|| {
        if minutes == 0 {
            "Time unavailable".into()
        } else {
            format!("+{minutes} min")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_renders_without_expanding_full_attributes() {
        let ctx = egui::Context::default();
        let cell = Cell {
            id: "I4".into(),
            hail_in: Some(0.5),
            mvt_deg: Some(67.5),
            mvt_kt: Some(19.0),
            ..Cell::default()
        };
        let mut popovers = crate::ui::popover::Popovers::default();
        let mut labels = Vec::new();
        for _ in 0..3 {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(
                    show(ui.ctx(), &cell, &[], false, None, &mut popovers),
                    (true, false, false)
                );
            });
            labels = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                    _ => None,
                })
                .collect();
        }
        assert!(
            labels.iter().any(|s| s == "Hail Size: 0.50\""),
            "{labels:?}"
        );
        assert!(labels.iter().any(|s| s == "ENE at 19 kts"), "{labels:?}");
        assert!(!labels.iter().any(|s| s == "Current Position"));
    }

    #[test]
    fn forecast_clock_uses_scan_time_and_handles_midnight_and_missing_time() {
        let time = "2026-09-09T23:56:00Z".parse().unwrap();
        assert_eq!(track_time(Some(time), 15, None), "00:11Z");
        assert_eq!(track_time(None, 15, None), "+15 min");
        assert_eq!(track_time(None, 0, None), "Time unavailable");
    }
}
