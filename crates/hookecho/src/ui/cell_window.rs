//! "Storm {id} Attributes" window: a card-grid interrogation of a clicked storm cell,
//! mirroring the RadarOmega attributes layout (position / movement / structure / hail / features).

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

/// Show the storm-attributes window. `trend` is the cell's per-volume history (oldest→newest).
/// Returns `false` when it should close.
pub fn show(ctx: &egui::Context, cell: &Cell, trend: &[CellSample]) -> bool {
    let mut open = true;
    egui::Window::new(format!("Storm {} Attributes", cell.id))
        .open(&mut open)
        .default_size([380.0, 460.0])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                theme::section(ui, "Current Position", |ui| {
                    grid(ui, &[
                        ("Latitude", format!("{:.3}°", cell.lat)),
                        ("Longitude", format!("{:.3}°", cell.lon)),
                        ("Range", opt(cell.range_nm, " NM", 0)),
                        ("Bearing", opt(cell.az_deg, "°", 0)),
                    ]);
                });
                theme::section(ui, "Movement", |ui| {
                    let mph = cell.mvt_kt.map(|k| k * KT_TO_MPH);
                    grid(ui, &[
                        ("Speed", opt(mph, " mph", 0)),
                        ("Direction", opt(cell.mvt_deg, "°", 0)),
                    ]);
                });
                theme::section(ui, "Intensity & Structure", |ui| {
                    let base = cell.base_kft.map(|b| {
                        format!("{}{:.1} kft", if cell.base_below { "<" } else { "" }, b)
                    });
                    grid(ui, &[
                        ("Max dBZ", opt(cell.max_dbz, " dBZ", 0)),
                        ("Max ref hgt", opt(cell.max_dbz_hgt_kft, " kft", 1)),
                        ("Cell top", opt(cell.top_kft, " kft", 1)),
                        ("Cell base", base.unwrap_or_else(|| "—".into())),
                        ("Cell-based VIL", opt(cell.vil, "", 0)),
                    ]);
                });
                theme::section(ui, "Hail Potential", |ui| {
                    grid(ui, &[
                        ("POH", cell.poh.map(|v| format!("{v}%")).unwrap_or_else(|| "—".into())),
                        ("POSH", cell.posh.map(|v| format!("{v}%")).unwrap_or_else(|| "—".into())),
                        ("Max size", opt(cell.hail_in, " in", 2)),
                    ]);
                });
                theme::section(ui, "Features", |ui| {
                    grid(ui, &[
                        ("TVS", cell.tvs.clone().unwrap_or_else(|| "None".into())),
                        ("Mesocyclone", cell.meso.clone().unwrap_or_else(|| "None".into())),
                    ]);
                });
                theme::section(ui, "Error Metrics", |ui| {
                    grid(ui, &[
                        ("Forecast error", opt(cell.fcst_err_nm, " NM", 1)),
                        ("Mean error", opt(cell.mean_err_nm, " NM", 1)),
                    ]);
                });
                if trend.len() >= 2 {
                    theme::section(ui, "Trends (per volume)", |ui| {
                        trend_row(ui, "Max dBZ", trend, |s| s.dbz, egui::Color32::from_rgb(255, 140, 90));
                        trend_row(ui, "Cell top kft", trend, |s| s.top, egui::Color32::from_rgb(120, 200, 140));
                        trend_row(ui, "Cell-based VIL", trend, |s| s.vil, egui::Color32::from_rgb(90, 170, 255));
                    });
                }
            });
        });
    open
}

/// A labelled trend sparkline over the samples that carry the selected field.
fn trend_row(ui: &mut egui::Ui, label: &str, trend: &[CellSample], f: fn(&CellSample) -> Option<f32>, color: egui::Color32) {
    let vals: Vec<f32> = trend.iter().filter_map(f).collect();
    ui.label(egui::RichText::new(label).small().weak());
    theme::sparkline(ui, &vals, color);
}

/// Format an optional value with a unit suffix and `decimals` precision (`—` when absent).
fn opt(v: Option<f32>, unit: &str, decimals: usize) -> String {
    v.map(|x| format!("{x:.*}{unit}", decimals)).unwrap_or_else(|| "—".into())
}

/// Lay out label/value pairs as wrapped stat cards.
fn grid(ui: &mut egui::Ui, cards: &[(&str, String)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, value) in cards {
            stat_card(ui, label, value);
        }
    });
}
