//! Storm console with every available attribute visible on selection.
use crate::theme;
use wxdata::level3::Cell;
const KT_TO_MPH: f32 = 1.150_78;
#[derive(Debug, Clone, Copy, Default)]
pub struct CellSample {
    pub vil: Option<f32>,
    pub top: Option<f32>,
    pub dbz: Option<f32>,
}

pub fn show(
    ctx: &egui::Context,
    cell: &Cell,
    trend: &[CellSample],
    following: bool,
    tz: Option<wxdata::tz::Tz>,
    popovers: &mut crate::ui::popover::Popovers,
) -> (bool, bool, bool) {
    let (mut open, mut follow, mut view3d) = (true, false, false);
    popovers
        .card(
            ctx,
            "cell",
            egui::Window::new(format!("Cell {}", cell.id))
                .id(egui::Id::new(("cell_console", &cell.id))),
        )
        .open(&mut open)
        .default_width((ctx.content_rect().width() - 48.0).clamp(280.0, 720.0))
        .default_height(560.0)
        .max_height((ctx.content_rect().height() - 160.0).max(260.0))
        .vscroll(true)
        .resizable(false)
        .collapsible(false)
        .frame(
            egui::Frame::window(&ctx.style_of(ctx.theme()))
                .fill(egui::Color32::from_rgb(17, 23, 31))
                .corner_radius(16)
                .inner_margin(18),
        )
        .show(ctx, |ui| {
            ui.weak(track_time(cell.time, 0, tz));
            ui.add_space(10.0);
            attributes(ui, cell, trend);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                follow = ui
                    .add_sized(
                        [150.0, 38.0],
                        egui::Button::new(if following {
                            "Stop following"
                        } else {
                            "Follow cell"
                        })
                        .selected(true),
                    )
                    .clicked();
                view3d = ui
                    .add_sized([140.0, 38.0], egui::Button::new("View in 3D"))
                    .clicked();
            });
        });
    (open, follow, view3d)
}
fn opt(v: Option<f32>, unit: &str, decimals: usize) -> String {
    v.map(|x| format!("{x:.*}{unit}", decimals))
        .unwrap_or_else(|| "—".into())
}
fn value(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(egui::RichText::new(label).size(11.0).weak());
    ui.label(egui::RichText::new(value).size(15.0).strong());
    ui.add_space(5.0);
}
/// Shared with the selected detail pane; no disclosure hides missing or populated fields.
pub(crate) fn attributes(ui: &mut egui::Ui, c: &Cell, trend: &[CellSample]) {
    let movement = match (c.mvt_deg, c.mvt_kt) {
        (Some(d), Some(k)) => format!("{} · {k:.0} kt", crate::geo::compass(d)),
        _ => "—".into(),
    };
    let groups = [
        (
            "STORM",
            vec![
                ("Hail size", opt(c.hail_in, " in", 2)),
                ("Movement", movement),
                ("Peak reflectivity", opt(c.max_dbz, " dBZ", 0)),
                ("Peak height", opt(c.max_dbz_hgt_kft, " kft", 1)),
                ("Cell top", opt(c.top_kft, " kft", 1)),
                (
                    "Cell base",
                    c.base_kft
                        .map(|v| format!("{}{v:.1} kft", if c.base_below { "<" } else { "" }))
                        .unwrap_or_else(|| "—".into()),
                ),
                ("VIL", opt(c.vil, " kg/m²", 0)),
            ],
        ),
        (
            "POSITION",
            vec![
                ("Latitude", format!("{:.3}°", c.lat)),
                ("Longitude", format!("{:.3}°", c.lon)),
                ("Radar range", opt(c.range_nm, " NM", 0)),
                ("Bearing", opt(c.az_deg, "°", 0)),
                ("Forecast error", opt(c.fcst_err_nm, " NM", 1)),
                ("Mean error", opt(c.mean_err_nm, " NM", 1)),
            ],
        ),
        (
            "MOTION & FEATURES",
            vec![
                ("Speed", opt(c.mvt_kt.map(|k| k * KT_TO_MPH), " mph", 0)),
                ("Direction", opt(c.mvt_deg, "°", 0)),
                (
                    "Hail probability",
                    c.poh.map(|v| format!("{v}%")).unwrap_or_else(|| "—".into()),
                ),
                (
                    "Severe hail probability",
                    c.posh
                        .map(|v| format!("{v}%"))
                        .unwrap_or_else(|| "—".into()),
                ),
                ("TVS", c.tvs.clone().unwrap_or_else(|| "—".into())),
                ("Mesocyclone", c.meso.clone().unwrap_or_else(|| "—".into())),
            ],
        ),
    ];
    let columns = if ui.available_width() >= 540.0 { 3 } else { 2 };
    // Flow the last group beneath on smaller screens; all fields remain visible.
    for chunk in groups.chunks(columns) {
        ui.columns(columns, |cols| {
            for (col, (title, fields)) in cols.iter_mut().zip(chunk) {
                col.label(egui::RichText::new(*title).size(11.0).strong());
                col.separator();
                for (label, v) in fields {
                    value(col, label, v.clone());
                }
            }
        });
    }
    if trend.len() >= 2 {
        ui.separator();
        ui.columns(3, |cols| {
            for (col, (label, values)) in cols.iter_mut().zip([
                (
                    "Reflectivity trend",
                    trend.iter().filter_map(|s| s.dbz).collect::<Vec<_>>(),
                ),
                ("Top trend", trend.iter().filter_map(|s| s.top).collect()),
                ("VIL trend", trend.iter().filter_map(|s| s.vil).collect()),
            ]) {
                col.small(label);
                theme::sparkline(col, &values, egui::Color32::from_rgb(80, 165, 245));
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
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                assert_eq!(
                    show(ui.ctx(), &cell, &[], false, None, &mut popovers),
                    (true, false, false)
                );
            });
            labels = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) if s.clip_rect.contains(t.pos) => Some(t.galley.job.text.clone()),
                    _ => None,
                })
                .collect();
        }
        assert!(labels.iter().any(|s| s == "0.50 in"), "{labels:?}");
        assert!(labels.iter().any(|s| s == "ENE · 19 kt"), "{labels:?}");
        assert!(labels.iter().any(|s| s == "Latitude"));
        assert!(labels.iter().any(|s| s == "Forecast error"));
        assert!(labels.iter().any(|s| s == "View in 3D"), "actions must fit without scrolling: {labels:?}");
    }

    #[test]
    fn forecast_clock_uses_scan_time_and_handles_midnight_and_missing_time() {
        let time = "2026-09-09T23:56:00Z".parse().unwrap();
        assert_eq!(track_time(Some(time), 15, None), "00:11Z");
        assert_eq!(track_time(None, 15, None), "+15 min");
        assert_eq!(track_time(None, 0, None), "Time unavailable");
    }
}
