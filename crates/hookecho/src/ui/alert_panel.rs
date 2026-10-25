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
    // MeteoAlarm synthesizes its events as "{colour} {hazard} Warning" (see wxdata::meteoalarm),
    // so the awareness level is right there in the string — without this a Red (level 4) event
    // ranked exactly with a Yellow one, and Europe's worst day sorted like a drizzle advisory.
    } else if e.contains("red") && e.contains("warning") {
        5
    } else if e.contains("orange") && e.contains("warning") {
        4
    } else if e.contains("warning") {
        3
    } else if e.contains("watch") {
        2
    } else {
        1 // advisories, statements, etc.
    }
}

#[cfg(test)]
mod severity_tests {
    use super::severity_rank;

    #[test]
    fn european_alerts_rank_by_colour() {
        // MeteoAlarm's synthesized events, in the shape wxdata::meteoalarm produces them.
        assert_eq!(severity_rank("Red wind Warning"), 5);
        assert_eq!(severity_rank("Orange rain Warning"), 4);
        assert_eq!(severity_rank("Yellow snow-ice Warning"), 3);
        // US events keep their own order, and tornado still outranks everything.
        assert_eq!(severity_rank("Tornado Warning"), 6);
        assert_eq!(severity_rank("Flash Flood Warning"), 5);
        assert_eq!(severity_rank("Severe Thunderstorm Warning"), 4);
        assert_eq!(severity_rank("Tornado Watch"), 2);
        assert_eq!(severity_rank("Special Weather Statement"), 1);
    }
}

/// One row's data: the alert, its stroke color, and its polygon-center (lon, lat) for fly-to.
pub struct Row<'a> {
    pub info: &'a AlertInfo,
    pub color: [u8; 4],
    pub center: (f64, f64),
    /// Escalation tier, computed once per row — `escalation()` uppercases the (multi-KB)
    /// bulletin text, too heavy to re-run inside a per-frame sort comparator.
    pub esc: u8,
}

/// Collect alert features whose bbox overlaps the view bounds `(min_lon, min_lat, max_lon, max_lat)`,
/// deduped by alert id, sorted by severity then soonest expiry.
pub fn rows_in_view(feats: &[GeoFeature], bounds: (f64, f64, f64, f64)) -> Vec<Row<'_>> {
    let (vx0, vy0, vx1, vy1) = bounds;
    rows_matching(feats, |_, (x0, y0, x1, y1)| {
        !(x1 < vx0 || x0 > vx1 || y1 < vy0 || y0 > vy1)
    })
}

fn rows_matching(
    feats: &[GeoFeature],
    matches: impl Fn(&GeoFeature, (f64, f64, f64, f64)) -> bool,
) -> Vec<Row<'_>> {
    // Borrowed ids: this runs every frame, and cloning each id was an allocation per alert.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut rows: Vec<Row> = Vec::new();
    for f in feats {
        let Some(a) = &f.alert else { continue };
        let Some((x0, y0, x1, y1)) = f.bbox() else {
            continue;
        };
        if !matches(f, (x0, y0, x1, y1)) {
            continue;
        }
        if !seen.insert(a.id.as_str()) {
            continue;
        }
        rows.push(Row {
            info: a,
            color: f.stroke,
            center: ((x0 + x1) * 0.5, (y0 + y1) * 0.5),
            esc: wxdata::alerts::escalation(a),
        });
    }
    rows.sort_by(|a, b| {
        // Escalation leads: a PDS/destructive SVR outranks a plain TOR (matches broadcast practice).
        b.esc
            .cmp(&a.esc)
            .then_with(|| severity_rank(&b.info.event).cmp(&severity_rank(&a.info.event)))
            .then_with(|| expiry_key(a.info).cmp(&expiry_key(b.info)))
    });
    rows
}

/// Sort key for expiry: soonest first; missing expiry sinks to the bottom.
fn expiry_key(a: &AlertInfo) -> i64 {
    a.expires.map(|e| e.timestamp()).unwrap_or(i64::MAX)
}

#[derive(Clone, Copy, Default, PartialEq)]
enum AlertScope {
    #[default]
    View,
    SavedPlaces,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AlertCategory {
    Tornado,
    Thunderstorm,
    Flood,
    Winter,
    Heat,
    Wind,
    Other,
}

impl AlertCategory {
    const ALL: [Self; 7] = [
        Self::Tornado,
        Self::Thunderstorm,
        Self::Flood,
        Self::Winter,
        Self::Heat,
        Self::Wind,
        Self::Other,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Tornado => "Tornado",
            Self::Thunderstorm => "Thunderstorm",
            Self::Flood => "Flood",
            Self::Winter => "Winter",
            Self::Heat => "Heat",
            Self::Wind => "Wind",
            Self::Other => "Other",
        }
    }

    fn of(event: &str) -> Self {
        let event = event.to_ascii_lowercase();
        if event.contains("tornado") {
            Self::Tornado
        } else if event.contains("thunderstorm") {
            Self::Thunderstorm
        } else if event.contains("flood") {
            Self::Flood
        } else if [
            "winter", "snow", "ice", "freez", "frost", "blizzard", "cold",
        ]
        .iter()
        .any(|term| event.contains(term))
        {
            Self::Winter
        } else if event.contains("heat") {
            Self::Heat
        } else if event.contains("wind")
            || event.contains("hurricane")
            || event.contains("tropical storm")
        {
            Self::Wind
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Default)]
struct AlertFilters {
    query: String,
    scope: AlertScope,
    category: Option<AlertCategory>,
}

fn matches_search(alert: &AlertInfo, query: &str) -> bool {
    query.is_empty()
        || [&alert.event, &alert.headline, &alert.area]
            .iter()
            .any(|text| text.to_lowercase().contains(query))
}

fn near_saved_places(feature: &GeoFeature, places: &[crate::settings::Marker]) -> bool {
    places.iter().any(|place| {
        place.lon.is_finite()
            && place.lat.is_finite()
            && place.alert_radius_mi.is_finite()
            && place.alert_radius_mi >= 0.0
            && feature.distance_km(place.lon, place.lat)
                <= place.alert_radius_mi * crate::geo::KM_PER_MILE
    })
}

/// Render the shared alerts list. Search and scope affect this list only; overlay visibility and
/// notifications remain unchanged. Returns the clicked alert's `(id, center_lon, center_lat)`.
pub fn body(
    ui: &mut egui::Ui,
    feats: &[GeoFeature],
    bounds: (f64, f64, f64, f64),
    saved_places: &[crate::settings::Marker],
    muted: &mut bool,
    export: &mut bool,
) -> Option<(String, f64, f64)> {
    let state_id = egui::Id::new("alert-list-filters");
    let mut filters = ui
        .ctx()
        .data_mut(|data| data.get_temp::<AlertFilters>(state_id).unwrap_or_default());
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut filters.scope, AlertScope::View, "In this view");
        ui.selectable_value(
            &mut filters.scope,
            AlertScope::SavedPlaces,
            "Near saved places",
        );
    });
    if filters.scope == AlertScope::SavedPlaces {
        ui.weak("Within each saved place’s alert radius, including alerts covering it.");
    }
    ui.add(
        egui::TextEdit::singleline(&mut filters.query)
            .hint_text("Search event, headline or area")
            .desired_width(f32::INFINITY),
    );
    let view_rows = rows_in_view(feats, bounds);
    let can_export_view = !view_rows.is_empty();
    let mut rows = match filters.scope {
        AlertScope::View => view_rows,
        AlertScope::SavedPlaces => rows_matching(feats, |f, _| near_saved_places(f, saved_places)),
    };
    let query = filters.query.trim().to_lowercase();
    rows.retain(|row| matches_search(row.info, &query));
    let mut counts = [0usize; AlertCategory::ALL.len()];
    for row in &rows {
        counts[AlertCategory::of(&row.info.event) as usize] += 1;
    }
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut filters.category, None, format!("All · {}", rows.len()));
        for category in AlertCategory::ALL {
            let count = counts[category as usize];
            if count > 0 || filters.category == Some(category) {
                ui.selectable_value(
                    &mut filters.category,
                    Some(category),
                    format!("{} · {count}", category.label()),
                );
            }
        }
    });
    if let Some(category) = filters.category {
        rows.retain(|row| AlertCategory::of(&row.info.event) == category);
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(state_id, filters.clone()));
    let mut clicked = None;
    {
        {
            ui.horizontal(|ui| {
                ui.weak(format!("{} matching alerts", rows.len()));
                // Silence switch, where you look when something is making noise at you.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let glyph = if *muted {
                        egui_phosphor::regular::BELL_SLASH
                    } else {
                        egui_phosphor::regular::BELL
                    };
                    let hover = if *muted {
                        "Alert sounds are muted — click to unmute"
                    } else {
                        "Mute all alert sounds"
                    };
                    if ui
                        .selectable_label(*muted, egui::RichText::new(glyph).size(16.0))
                        .on_hover_text(hover)
                        .clicked()
                    {
                        *muted = !*muted;
                    }
                    if ui
                        .add_enabled(can_export_view, egui::Button::new("Export view GeoJSON…"))
                        .on_hover_text("Exports all alerts in the map view, independent of these list filters.")
                        .clicked()
                    {
                        *export = true;
                    }
                });
            });
            ui.separator();
            if rows.is_empty() {
                ui.weak(
                    if filters.scope == AlertScope::SavedPlaces && saved_places.is_empty() {
                        "No saved places yet. Add one in Custom locations."
                    } else if !query.is_empty() || filters.category.is_some() {
                        "No alerts match these filters. Clear the search or choose All."
                    } else if filters.scope == AlertScope::SavedPlaces {
                        "No alerts within your saved places’ alert radii."
                    } else {
                        "No alerts in view."
                    },
                );
                return None;
            }
            // The host owns scrolling, so warnings never trap the menu wheel.
            {
                for row in &rows {
                    let a = row.info;
                    let resp = egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .stroke(egui::Stroke::new(1.0, color32(row.color)))
                        .corner_radius(egui::CornerRadius::same(5))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(5.0, 15.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(rect, 1.0, color32(row.color));
                                ui.strong(&a.event);
                                // Emergency/PDS/destructive chip on escalated rows.
                                if row.esc >= 2 {
                                    let label = a.headline.to_ascii_uppercase();
                                    let chip = if label.contains("TORNADO EMERGENCY") {
                                        "TOR EMERGENCY"
                                    } else if a.damage_threat.as_deref().is_some_and(|d| {
                                        d.to_ascii_uppercase().contains("CATASTROPHIC")
                                    }) {
                                        "CATASTROPHIC"
                                    } else {
                                        "DESTRUCTIVE"
                                    };
                                    ui.label(
                                        egui::RichText::new(chip)
                                            .small()
                                            .strong()
                                            .color(egui::Color32::WHITE)
                                            .background_color(egui::Color32::from_rgb(200, 20, 20)),
                                    );
                                }
                            });
                            ui.add(
                                egui::Label::new(egui::RichText::new(&a.area).weak().small())
                                    .truncate(),
                            );
                            ui.label(
                                egui::RichText::new(crate::ui::warning_window::countdown(a))
                                    .small(),
                            );
                        })
                        .response;
                    if resp.interact(egui::Sense::click()).clicked() {
                        clicked = Some((a.id.clone(), row.center.0, row.center.1));
                    }
                    ui.add_space(4.0);
                }
            }
        }
    }
    clicked
}

fn color32(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(id: &str, event: &str, lon: f64) -> GeoFeature {
        serde_json::from_value(serde_json::json!({
            "rings": [[[lon,0.0],[lon+1.0,0.0],[lon+1.0,1.0],[lon,1.0],[lon,0.0]]],
            "fill": [0,0,0,0], "stroke": [255,0,0,255], "kind": "Warning",
            "title": event, "detail": "",
            "alert": { "id": id, "event": event, "headline": "County bulletin",
                "area": "Dallas County", "description": "", "instruction": "" }
        }))
        .unwrap()
    }

    fn place(lon: f64, lat: f64, radius_mi: f64) -> crate::settings::Marker {
        serde_json::from_value(serde_json::json!({
            "name": "Home", "lon": lon, "lat": lat, "alert_radius_mi": radius_mi
        }))
        .unwrap()
    }

    #[test]
    fn search_and_categories_match_events_and_areas() {
        let feature = feature("1", "Freeze Watch", 0.0);
        let alert = feature.alert.as_ref().unwrap();
        assert!(matches_search(alert, "freeze"));
        assert!(matches_search(alert, "dallas"));
        assert!(matches_search(alert, "bulletin"));
        assert!(matches_search(alert, ""));
        assert!(!matches_search(alert, "houston"));
        for (event, expected) in [
            ("Freeze Watch", AlertCategory::Winter),
            ("Frost Advisory", AlertCategory::Winter),
            ("Excessive Heat Warning", AlertCategory::Heat),
            ("Tornado Watch", AlertCategory::Tornado),
            ("Severe Thunderstorm Warning", AlertCategory::Thunderstorm),
            ("Flash Flood Warning", AlertCategory::Flood),
            ("Lake Wind Advisory", AlertCategory::Wind),
            ("Special Weather Statement", AlertCategory::Other),
        ] {
            assert!(AlertCategory::of(event) == expected, "{event}");
        }
    }

    #[test]
    fn saved_places_use_polygon_edges_and_each_saved_radius() {
        let feature = feature("1", "Flood Warning", 0.0);
        assert!(near_saved_places(&feature, &[place(0.5, 0.5, 0.0)]));
        // 0.1 degree from the edge is about 7 miles, while the center is much farther away.
        assert!(near_saved_places(&feature, &[place(1.1, 0.5, 10.0)]));
        assert!(!near_saved_places(&feature, &[place(1.1, 0.5, 5.0)]));
        assert!(!near_saved_places(&feature, &[]));
        assert!(!near_saved_places(&feature, &[place(0.5, 0.5, -1.0)]));
    }

    #[test]
    fn scoped_rows_keep_sort_order_and_dedupe_matching_polygon_parts() {
        let feats = vec![
            feature("same", "Tornado Warning", 10.0),
            feature("flood", "Flood Warning", 0.0),
            feature("same", "Tornado Warning", 0.0),
            feature("same", "Tornado Warning", 0.0),
        ];
        let places = [place(0.5, 0.5, 0.0)];
        let rows = rows_matching(&feats, |feature, _| near_saved_places(feature, &places));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].info.id, "same");
        assert_eq!(rows[0].center, (0.5, 0.5));
        assert_eq!(rows[1].info.id, "flood");
        let in_view = rows_in_view(&feats, (9.0, -1.0, 12.0, 2.0));
        assert_eq!(in_view.len(), 1);
        assert_eq!(in_view[0].center, (10.5, 0.5));
    }

    #[test]
    fn warnings_scroll_with_the_menu_in_both_directions() {
        let feats: Vec<GeoFeature> = (0..20)
            .map(|i| {
                serde_json::from_value(serde_json::json!({
            "rings": [[[0.0,0.0],[1.0,1.0]]], "fill": [0,0,0,0],
            "stroke": [255,0,0,255], "kind": "Warning", "title": "Warning", "detail": "",
            "alert": { "id": i.to_string(), "event": "Tornado Warning", "headline": "Warning",
                "area": "Test county", "description": "", "instruction": "" }
        })).unwrap()
            })
            .collect();
        let ctx = egui::Context::default();
        let mut offset = 0.0;
        let mut down = 0.0;
        for frame in 0..45 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))];
            if frame >= 3 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, if frame < 24 { -80.0 } else { 80.0 }),
                    modifiers: egui::Modifiers::default(),
                });
            }
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    time: Some(frame as f64 / 10.0),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(300.0);
                    let out = egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.label("Severe weather alerts");
                            assert!(body(
                                ui,
                                &feats,
                                (-2.0, -2.0, 2.0, 2.0),
                                &[],
                                &mut false,
                                &mut false
                            )
                            .is_none());
                            ui.label("Optional settings and Tools");
                        });
                    offset = out.state.offset.y;
                },
            );
            if frame == 23 {
                down = offset;
            }
        }
        assert!(
            down > 400.0,
            "wheel over warnings must scroll the host menu: {down}"
        );
        assert!(
            offset < down - 400.0,
            "scrolling back must not stick: {offset} / {down}"
        );
    }

    #[test]
    fn severity_orders_tornado_first() {
        assert!(severity_rank("Tornado Warning") > severity_rank("Severe Thunderstorm Warning"));
        assert!(severity_rank("Severe Thunderstorm Warning") > severity_rank("Flood Watch"));
        assert!(severity_rank("Flood Watch") > severity_rank("Special Weather Statement"));
    }
}
