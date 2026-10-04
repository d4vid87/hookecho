//! Nearby dock thresholds and storm-track display rules. Never gate map alerts or notifications.
use crate::settings::PriorityRules;
use chrono::{DateTime, Utc};
use wxdata::{
    level3::Cell,
    overlay::{FeatureKind, GeoFeature},
};
pub const LABELS: [&str; 4] = ["Advisory / statement", "Watch", "Warning", "High impact"];
pub const COLORS: [egui::Color32; 4] = [
    egui::Color32::from_rgb(141, 190, 223),
    egui::Color32::from_rgb(232, 199, 120),
    egui::Color32::from_rgb(253, 164, 127),
    egui::Color32::from_rgb(255, 140, 174),
];

pub fn category(f: &GeoFeature) -> usize {
    match f.kind {
        FeatureKind::Warning
            if f.alert
                .as_ref()
                .is_some_and(|a| wxdata::alerts::escalation(a) > 0) =>
        {
            3
        }
        FeatureKind::Warning => 2,
        FeatureKind::Watch | FeatureKind::WatchBox => 1,
        _ => 0,
    }
}
/// Map alerts are independent of the dock's proximity and severity thresholds.
pub fn map_visible(f: &GeoFeature, now: DateTime<Utc>) -> bool {
    !f.alert.as_ref().and_then(|a| a.expires).is_some_and(|e| e <= now)
}
pub fn visible(
    f: &GeoFeature,
    rules: &PriorityRules,
    point: (f64, f64),
    now: DateTime<Utc>,
) -> bool {
    if f.alert.is_none() { return true; }
    if !map_visible(f, now) {
        return false;
    }
    let level = category(f);
    if level >= 2 && f.contains(point.0, point.1) {
        return true;
    }
    let radius = rules.radii_mi[level];
    level >= usize::from(rules.minimum.min(3))
        && radius.is_finite()
        && f.distance_km(point.0, point.1) <= radius.clamp(0.0, 500.0) * crate::geo::KM_PER_MILE
}
pub fn rows<'a>(
    features: &'a [GeoFeature],
    rules: &PriorityRules,
    point: (f64, f64),
    now: DateTime<Utc>,
) -> Vec<&'a GeoFeature> {
    let mut rows: Vec<_> = features
        .iter()
        .filter(|f| f.alert.is_some() && visible(f, rules, point, now))
        .collect();
    rows.sort_by(|a, b| {
        let pinned = |f: &GeoFeature| category(f) >= 2 && f.contains(point.0, point.1);
        pinned(b)
            .cmp(&pinned(a))
            .then_with(|| category(b).cmp(&category(a)))
            .then_with(|| {
                super::alert_panel::severity_rank(&b.title)
                    .cmp(&super::alert_panel::severity_rank(&a.title))
            })
            .then_with(|| {
                a.distance_km(point.0, point.1)
                    .total_cmp(&b.distance_km(point.0, point.1))
            })
    });
    // Sort before dedup: the relevant part of a MultiPolygon owns its bulletin's row.
    let mut seen = std::collections::HashSet::new();
    rows.retain(|f| seen.insert(f.alert.as_ref().unwrap().id.as_str()));
    rows
}
pub fn cell_marker_visible(cell: &Cell) -> bool {
    cell.max_dbz.is_some_and(|dbz| dbz.is_finite() && dbz > 50.0)
}
pub fn track_strength_visible(cell: &Cell, rules: &PriorityRules) -> bool {
    rules.track_min_dbz.is_finite()
        && cell_marker_visible(cell)
        && cell.max_dbz.is_some_and(|dbz| dbz >= rules.track_min_dbz.clamp(50.0, 70.0))
}
pub fn track_visible(cell: &Cell, rules: &PriorityRules, now: DateTime<Utc>) -> bool {
    track_strength_visible(cell, rules)
        && super::cell_window::projection_valid(cell, now)
        && cell.time.is_some_and(|t| {
            (now - t).num_seconds().max(0) <= i64::from(rules.track_age_min.clamp(1, 15)) * 60
        })
        && super::cell_window::error_km(cell).is_some_and(|km| {
            rules.track_error_nm.is_finite()
                && km <= f64::from(rules.track_error_nm.clamp(0.5, 20.0)) * 1.852
        })
}
pub fn controls(ui: &mut egui::Ui, rules: &mut PriorityRules) {
    dock_controls(ui, rules);
    ui.separator();
    track_controls(ui, rules);
    if ui.button("Reset display defaults").clicked() {
        *rules = PriorityRules::default();
    }
}

pub fn dock_controls(ui: &mut egui::Ui, rules: &mut PriorityRules) {
    ui.weak("Alert thresholds apply only to the priority dock. All active map alerts remain visible.");
    ui.weak("Local warnings stay in the dock. Sounds and notification rules are unchanged.");
    egui::ComboBox::from_label("Minimum dock category")
        .selected_text(LABELS[usize::from(rules.minimum.min(3))])
        .show_ui(ui, |ui| {
            for (i, label) in LABELS.iter().enumerate() {
                ui.selectable_value(&mut rules.minimum, i as u8, *label);
            }
        });
    for (i, label) in LABELS.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.colored_label(COLORS[i], *label);
            ui.add(egui::Slider::new(&mut rules.radii_mi[i], 0.0..=250.0).suffix(" mi"));
        });
    }
}

pub fn track_controls(ui: &mut egui::Ui, rules: &mut PriorityRules) {
    ui.strong("Estimated storm tracks");
    ui.add(egui::Slider::new(&mut rules.track_min_dbz, 50.0..=70.0).text("Minimum cell reflectivity (dBZ)"));
    ui.add(egui::Slider::new(&mut rules.track_age_min, 1..=15).text("Maximum age (min)"));
    ui.add(
        egui::Slider::new(&mut rules.track_error_nm, 0.5..=20.0)
            .text("Maximum forecast error (nm)"),
    );
    egui::ComboBox::from_label("Projection horizon")
        .selected_text(format!("{} minutes", rules.track_horizon_min))
        .show_ui(ui, |ui| {
            for n in [15, 30, 45, 60] {
                ui.selectable_value(&mut rules.track_horizon_min, n, format!("{n} minutes"));
            }
        });
    ui.weak("Only cells meeting the reflectivity limit draw tracks. This is a display filter, not an official severe-weather threshold; missing age, motion, or error also keeps a projection off the map.");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feature(event: &str, kind: FeatureKind) -> GeoFeature {
        GeoFeature { rings: vec![vec![[-98.,32.],[-97.,32.],[-97.,33.],[-98.,33.],[-98.,32.]]],
            fill:[0;4],stroke:[0;4],kind,title:event.into(),detail:String::new(),
            alert:Some(serde_json::from_value(serde_json::json!({"id":event,"event":event,"headline":event,"area":"Test","description":"","instruction":""})).unwrap()) }
    }
    #[test]
    fn thresholds_keep_local_warnings_but_expire_and_dedupe_them() {
        let now = Utc::now();
        let mut r = PriorityRules {
            minimum: 3,
            radii_mi: [0.; 4],
            ..Default::default()
        };
        let mut warning = feature("Severe Thunderstorm Warning", FeatureKind::Warning);
        let watch = feature("Tornado Watch", FeatureKind::Watch);
        assert!(visible(&warning, &r, (-97.5, 32.5), now));
        assert!(!visible(&watch, &r, (-97.5, 32.5), now));
        assert!(!visible(&warning, &r, (-95., 32.5), now));
        warning.alert.as_mut().unwrap().damage_threat = Some("CONSIDERABLE".into());
        assert_eq!(category(&warning), 3);
        let features = [warning.clone(), warning.clone()];
        assert_eq!(rows(&features, &r, (-97.5, 32.5), now).len(), 1);
        warning.alert.as_mut().unwrap().expires = Some(now);
        assert!(!visible(&warning, &r, (-97.5, 32.5), now));
        r.minimum = 0;
        r.radii_mi[1] = 250.;
        assert!(visible(&watch, &r, (-96.5, 32.5), now));
        r.radii_mi[1] = 10.;
        assert!(!visible(&watch, &r, (-96.5, 32.5), now));
        assert_eq!(
            serde_json::from_str::<PriorityRules>("{}").unwrap(),
            PriorityRules::default()
        );
    }
    #[test]
    fn map_keeps_every_alert_category_outside_dock_thresholds() {
        let now = Utc::now();
        let rules = PriorityRules { minimum: 3, radii_mi: [0.; 4], ..Default::default() };
        for (event, kind) in [
            ("Tornado Warning", FeatureKind::Warning),
            ("Tornado Watch", FeatureKind::Watch),
            ("Flood Advisory", FeatureKind::Advisory),
            ("Special Weather Statement", FeatureKind::Statement),
        ] {
            let mut alert = feature(event, kind);
            alert.alert.as_mut().unwrap().expires = Some(now + chrono::Duration::hours(1));
            assert!(!visible(&alert, &rules, (-80., 40.), now));
            assert!(map_visible(&alert, now), "{event} must remain on the radar");
            alert.alert.as_mut().unwrap().expires = Some(now);
            assert!(!map_visible(&alert, now), "expired {event} must leave the radar");
        }
    }
    #[test]
    fn projections_require_fresh_valid_motion_and_measured_error() {
        let now = Utc::now();
        let r = PriorityRules::default();
        let mut c = Cell {
            time: Some(now),
            lon: -97.,
            lat: 32.,
            mvt_deg: Some(45.),
            mvt_kt: Some(25.),
            max_dbz: Some(55.),
            fcst_err_nm: Some(2.),
            ..Default::default()
        };
        assert!(track_visible(&c, &r, now));
        c.max_dbz = Some(50.0);
        assert!(!track_visible(&c, &r, now));
        c.max_dbz = Some(49.9);
        assert!(!track_visible(&c, &r, now));
        c.max_dbz = None;
        assert!(!track_visible(&c, &r, now));
        c.max_dbz = Some(55.);
        c.fcst_err_nm = Some(8.);
        assert!(!track_visible(&c, &r, now));
        c.fcst_err_nm = None;
        assert!(!track_visible(&c, &r, now));
        c.fcst_err_nm = Some(2.);
        c.time = Some(now - chrono::Duration::minutes(6));
        assert!(!track_visible(&c, &r, now));
        c.time = Some(now);
        c.mvt_deg = Some(f32::NAN);
        assert!(!track_visible(&c, &r, now));
    }
    #[test]
    fn map_cell_markers_require_more_than_fifty_dbz() {
        let mut cell = Cell::default();
        for (dbz, visible) in [(None, false), (Some(50.0), false), (Some(50.1), true), (Some(f32::NAN), false)] {
            cell.max_dbz = dbz;
            assert_eq!(cell_marker_visible(&cell), visible);
        }
    }
}
