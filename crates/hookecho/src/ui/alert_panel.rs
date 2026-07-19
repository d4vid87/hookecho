//! Right-dock active-alerts panel: every NWS alert whose polygon overlaps the current view,
//! sorted by severity then soonest expiry. Clicking a row flies the camera to that alert.

use wxdata::overlay::{AlertInfo, GeoFeature};

/// Severity ordering for the panel: higher = more urgent, listed first.
pub fn severity_rank(event: &str) -> u8 {
    let e = event.to_ascii_lowercase();
    if e.contains("tornado") && e.contains("warning") {
        6
    } else if e.contains("flash flood") && e.contains("warning") {
        5
    } else if e.contains("severe thunderstorm") && e.contains("warning") {
        4
    } else if e.contains("warning") {
        3
    } else if e.contains("watch") {
        2
    } else {
        1 // advisories, statements, etc.
    }
}

/// One row's data: the alert, its stroke color, and its polygon-center (lon, lat) for fly-to.
pub struct Row<'a> {
    pub info: &'a AlertInfo,
    pub color: [u8; 4],
    pub center: (f64, f64),
}

/// Collect alert features whose bbox overlaps the view bounds `(min_lon, min_lat, max_lon, max_lat)`,
/// deduped by alert id, sorted by severity then soonest expiry.
pub fn rows_in_view(feats: &[GeoFeature], bounds: (f64, f64, f64, f64)) -> Vec<Row<'_>> {
    let (vx0, vy0, vx1, vy1) = bounds;
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<Row> = Vec::new();
    for f in feats {
        let Some(a) = &f.alert else { continue };
        let Some((x0, y0, x1, y1)) = f.bbox() else { continue };
        if x1 < vx0 || x0 > vx1 || y1 < vy0 || y0 > vy1 {
            continue; // bbox disjoint from the view
        }
        if !seen.insert(a.id.clone()) {
            continue;
        }
        rows.push(Row { info: a, color: f.stroke, center: ((x0 + x1) * 0.5, (y0 + y1) * 0.5) });
    }
    rows.sort_by(|a, b| {
        severity_rank(&b.info.event)
            .cmp(&severity_rank(&a.info.event))
            .then_with(|| expiry_key(a.info).cmp(&expiry_key(b.info)))
    });
    rows
}

/// Sort key for expiry: soonest first; missing expiry sinks to the bottom.
fn expiry_key(a: &AlertInfo) -> i64 {
    a.expires.map(|e| e.timestamp()).unwrap_or(i64::MAX)
}

/// Render the panel. Returns the clicked alert's `(id, center_lon, center_lat)` when a row is picked.
pub fn show(root: &mut egui::Ui, feats: &[GeoFeature], bounds: (f64, f64, f64, f64)) -> Option<(String, f64, f64)> {
    let rows = rows_in_view(feats, bounds);
    let mut clicked = None;
    egui::Panel::right("alerts_panel")
        .resizable(true)
        .default_size(280.0)
        .show(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Active Alerts");
                ui.weak(format!("({})", rows.len()));
            });
            ui.separator();
            if rows.is_empty() {
                ui.weak("No alerts in view.");
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for row in &rows {
                    let a = row.info;
                    let resp = egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .stroke(egui::Stroke::new(1.0, color32(row.color)))
                        .corner_radius(egui::CornerRadius::same(5))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(5.0, 15.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 1.0, color32(row.color));
                                ui.strong(&a.event);
                            });
                            ui.add(egui::Label::new(egui::RichText::new(&a.area).weak().small()).truncate());
                            ui.label(egui::RichText::new(crate::ui::warning_window::countdown(a)).small());
                        })
                        .response;
                    if resp.interact(egui::Sense::click()).clicked() {
                        clicked = Some((a.id.clone(), row.center.0, row.center.1));
                    }
                    ui.add_space(4.0);
                }
            });
        });
    clicked
}

fn color32(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_tornado_first() {
        assert!(severity_rank("Tornado Warning") > severity_rank("Severe Thunderstorm Warning"));
        assert!(severity_rank("Severe Thunderstorm Warning") > severity_rank("Flood Watch"));
        assert!(severity_rank("Flood Watch") > severity_rank("Special Weather Statement"));
    }
}
