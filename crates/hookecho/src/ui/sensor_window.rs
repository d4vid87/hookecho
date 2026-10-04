//! Sensor dashboard: current conditions + 24h trend sparklines from the nearest NWS/METAR
//! station. Sparklines are hand-rolled on the painter (no egui_plot dependency).

use chrono::{DateTime, Utc};
use wxdata::obs::{Observation, StationObs};

const BG: egui::Color32 = egui::Color32::from_rgb(16, 40, 61);
const TILE: egui::Color32 = egui::Color32::from_rgb(27, 58, 82);
const LINE: egui::Color32 = egui::Color32::from_rgb(66, 98, 122);
const MUTED: egui::Color32 = egui::Color32::from_rgb(168, 193, 210);
const BLUE: egui::Color32 = egui::Color32::from_rgb(104, 186, 255);
const AMBER: egui::Color32 = egui::Color32::from_rgb(255, 209, 120);

const KMH_TO_MPH: f32 = 0.621_371;

/// Native-value samples accumulated as distinct frames are viewed at one observation station.
#[derive(Default)]
pub struct PointHistory {
    pub site: String,
    analysis: Vec<Reading>,
    hrrr: Vec<Reading>,
    forecast: Vec<Reading>,
}

#[derive(Clone)]
struct Reading {
    valid: DateTime<Utc>,
    run: Option<DateTime<Utc>>,
    source: String,
    temp_k: f32,
}

impl PointHistory {
    pub fn has_samples(&self) -> bool {
        !self.analysis.is_empty() || !self.hrrr.is_empty() || !self.forecast.is_empty()
    }

    pub fn record(
        &mut self,
        site: &str,
        lon: f64,
        lat: f64,
        analysis: Option<&wxdata::field::FieldFrame>,
        forecast: Option<&wxdata::field::FieldFrame>,
    ) {
        if self.site != site {
            self.site = site.to_string();
            self.analysis.clear();
            self.hrrr.clear();
            self.forecast.clear();
        }
        for (series, frame) in [(&mut self.analysis, analysis), (&mut self.forecast, forecast)] {
            let Some(frame) = frame else { continue };
            // GEFS spread is a temperature *difference* in K, not an absolute temperature.
            if frame.stamp.source_identity.contains("/gespr.") {
                continue;
            }
            let Some(temp_k) = frame.sample(lon, lat).value.filter(|v| v.is_finite()) else { continue };
            record_native(series, &frame.stamp, temp_k);
        }
    }

    pub fn record_hrrr(&mut self, station: &str, points: &[wxdata::hrrr::PointTemperature]) {
        if self.site != station { return; }
        for point in points {
            record_native(&mut self.hrrr, &point.stamp, point.kelvin);
        }
    }

    pub fn record_rtma(&mut self, station: &str, points: &[wxdata::rtma::PointTemperature]) {
        if self.site != station { return; }
        for point in points {
            record_native(&mut self.analysis, &point.stamp, point.kelvin);
        }
    }

    pub fn record_gfs(&mut self, station: &str, points: &[wxdata::global::PointTemperature]) {
        if self.site != station { return; }
        for point in points {
            record_native(&mut self.forecast, &point.stamp, point.kelvin);
        }
    }
}

fn record_native(series: &mut Vec<Reading>, stamp: &wxdata::field::DataStamp, temp_k: f32) {
    if !temp_k.is_finite() { return; }
    if let Some(old) = series.iter_mut().find(|old| old.valid == stamp.valid_time
        && old.run == stamp.run_time && old.source == stamp.source_identity) {
        old.temp_k = temp_k;
    } else {
        series.push(Reading {
            valid: stamp.valid_time, run: stamp.run_time,
            source: stamp.source_identity.clone(), temp_k,
        });
        series.sort_by_key(|reading| reading.valid);
        if series.len() > 72 { series.remove(0); }
    }
}

/// Show the sensor window. `data` is `Ok(station)`, `Err(message)`, or `None` (loading).
/// Returns `false` when it should close.
pub fn show(
    ctx: &egui::Context,
    data: Option<&Result<StationObs, String>>,
    history: Option<&PointHistory>,
    tz: Option<wxdata::tz::Tz>,
    drawer: &mut crate::ui::drawer::Drawer,
) -> bool {
    let mut open = true;
    let Some(window) = drawer.page_sized(
        ctx,
        "Sensors",
        &mut open,
        false,
        820.0,
        egui::Window::new("Sensors"),
    ) else {
        return open;
    };
    window.show(ctx, |ui| match data {
        None => {
            ui.weak("Loading nearest station…");
        }
        Some(Err(e)) => {
            ui.colored_label(egui::Color32::from_rgb(220, 120, 120), "No nearby station");
            ui.weak(e);
        }
        Some(Ok(station)) => dashboard(ui, station, history, tz),
    });
    open
}

fn dashboard(
    ui: &mut egui::Ui,
    station: &StationObs,
    history: Option<&PointHistory>,
    tz: Option<wxdata::tz::Tz>,
) {
    let Some(cur) = station.obs.first() else {
        ui.weak("No station observations available.");
        return;
    };
    let history = history.filter(|h| h.site == station.station_id);
    let age = cur.time.map(|t| (Utc::now() - t).num_minutes().max(0));
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Frame::new().fill(BG).corner_radius(12.0)
            .inner_margin(egui::Margin::same(18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(format!("{} / {}", station.station_id, station.name.to_uppercase()))
                .monospace().size(10.0).color(BLUE));
            ui.label(egui::RichText::new("Temperature & trend").size(24.0).strong().color(egui::Color32::WHITE));
            ui.label(egui::RichText::new("Station observation and loaded model samples").size(11.0).color(MUTED));
            ui.add_space(13.0);
            ui.separator();
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(opt(cur.temp_c.map(c_to_f), "°F", 0))
                    .size(62.0).strong().color(AMBER));
                ui.vertical(|ui| {
                    ui.add_space(7.0);
                    ui.label(egui::RichText::new(match (cur.time, age) {
                        (Some(time), Some(age)) => format!("Observed {} · {age} min ago", crate::timefmt::fmt_clock(time, tz, false)),
                        _ => "Observation time unavailable".into(),
                    }).size(11.0).color(MUTED));
                    ui.label(egui::RichText::new(format!("Humidity {} · {}",
                        opt(cur.rh, "%", 0), wind_label(cur))).size(11.0).color(MUTED));
                });
            });
            ui.add_space(9.0);
            ui.label(egui::RichText::new("COMPARE SOURCES ON THE SAME SCALE")
                .size(10.0).strong().color(BLUE));
            ui.add_space(6.0);
            atlas_chart(ui, &station.obs, history);
            ui.label(egui::RichText::new("Observed: station reading · RTMA/URMA: recent analysis · HRRR: current run · Global: loaded forecast frames. Model values are not station measurements.")
                .size(10.0).color(MUTED));
            ui.add_space(12.0);
            let compact = ui.available_width() < 540.0;
            let gap = 9.0;
            let width = if compact { ui.available_width() } else { (ui.available_width() - gap) / 2.0 };
            let cards = [
                ("Dewpoint", opt(cur.dewpoint_c.map(c_to_f), "°F", 0),
                    station.obs.iter().rev().filter_map(|o| o.dewpoint_c.map(c_to_f)).collect::<Vec<_>>(), egui::Color32::from_rgb(108, 224, 189)),
                ("Humidity", opt(cur.rh, "%", 0),
                    station.obs.iter().rev().filter_map(|o| o.rh).collect(), BLUE),
                ("Wind", wind_label(cur),
                    station.obs.iter().rev().filter_map(|o| o.wind_kmh.map(|v| v * KMH_TO_MPH)).collect(), egui::Color32::from_rgb(204, 214, 220)),
                ("Pressure", cur.pressure_pa.map(|v| format!("{:.0} mb", v / 100.0)).unwrap_or_else(|| "—".into()),
                    station.obs.iter().rev().filter_map(|o| o.pressure_pa.map(|v| v / 100.0)).collect(), AMBER),
            ];
            if compact {
                for (label, value, points, color) in cards { atlas_card(ui, width, label, &value, &points, color); ui.add_space(gap); }
            } else {
                for row in cards.chunks(2) {
                    ui.horizontal(|ui| {
                        for (label, value, points, color) in row {
                            atlas_card(ui, width, label, value, points, *color);
                            ui.add_space(gap);
                        }
                    });
                    ui.add_space(gap);
                }
            }
            ui.label(egui::RichText::new(format!("Gust {} · Sea-level pressure {}",
                opt(cur.gust_kmh.map(|v| v * KMH_TO_MPH), " mph", 0),
                cur.slp_pa.map(|v| format!("{:.0} mb", v / 100.0)).unwrap_or_else(|| "—".into())))
                .size(11.0).color(MUTED));
            if let Some(history) = history {
                ui.collapsing("Model sample details", |ui| {
                    for (label, series) in [("RTMA / URMA", &history.analysis), ("HRRR", &history.hrrr), ("Global", &history.forecast)] {
                        ui.label(format!("{label} · {} valid times", series.len()));
                        for reading in series.iter().rev().take(8) {
                            let bias = nearest_temperature(&station.obs, reading.valid)
                                .map(|(observed, time)| format!(" · vs observed {:+.1} °F ({} min apart)",
                                    c_to_f(reading.temp_k - 273.15) - c_to_f(observed),
                                    (time - reading.valid).num_minutes().abs()))
                                .unwrap_or_default();
                            ui.label(egui::RichText::new(format!("{} · {} UTC · {:.1} °F{bias}",
                                source_label(&reading.source), reading.valid.format("%Y-%m-%d %H:%M"),
                                c_to_f(reading.temp_k - 273.15))).size(11.0).color(MUTED));
                        }
                    }
                });
            }
        });
    });
}

fn wind_label(cur: &Observation) -> String {
    match (cur.wind_kmh.map(|v| v * KMH_TO_MPH), cur.wind_dir_deg) {
        (Some(speed), Some(direction)) => format!("{speed:.0} mph {}", compass(direction)),
        (Some(speed), None) => format!("{speed:.0} mph"),
        _ => "—".into(),
    }
}

fn atlas_card(ui: &mut egui::Ui, width: f32, label: &str, value: &str, points: &[f32], color: egui::Color32) {
    egui::Frame::new().fill(TILE).stroke(egui::Stroke::new(1.0, LINE))
        .corner_radius(9.0).inner_margin(egui::Margin::same(11)).show(ui, |ui| {
            ui.set_width(width - 22.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).size(11.0).color(egui::Color32::WHITE));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(value).size(17.0).strong().color(egui::Color32::WHITE));
                });
            });
            ui.add_space(6.0);
            let size = egui::vec2((width - 22.0).max(80.0), 45.0);
            crate::theme::sparkline_sized(ui, points, color, size);
            ui.label(egui::RichText::new("older  →  latest").size(9.0).color(MUTED));
        });
}

fn atlas_chart(ui: &mut egui::Ui, observations: &[Observation], history: Option<&PointHistory>) {
    let empty = PointHistory::default();
    let history = history.unwrap_or(&empty);
    let mut series = [
        ("Observed", egui::Color32::from_rgb(255, 152, 113), observations.iter()
            .filter_map(|o| Some((o.time?, c_to_f(o.temp_c?)))).collect::<Vec<_>>()),
        ("RTMA / URMA", egui::Color32::from_rgb(104, 201, 237), history.analysis.iter()
            .map(|p| (p.valid, c_to_f(p.temp_k - 273.15))).collect()),
        ("HRRR", egui::Color32::from_rgb(182, 163, 255), history.hrrr.iter()
            .map(|p| (p.valid, c_to_f(p.temp_k - 273.15))).collect()),
        ("Global", egui::Color32::from_rgb(131, 216, 157), history.forecast.iter()
            .map(|p| (p.valid, c_to_f(p.temp_k - 273.15))).collect()),
    ];
    for (_, _, points) in &mut series { points.sort_by_key(|(time, _)| *time); }
    let id = ui.id().with("atlas_source");
    let mut selected = ui.ctx().data_mut(|d| d.get_temp::<usize>(id).unwrap_or(0)).min(3);
    let now = Utc::now();
    let start = now - chrono::Duration::hours(24);
    let end = now + chrono::Duration::hours(12);
    let all: Vec<f32> = series.iter().flat_map(|(_, _, points)| points.iter())
        .filter(|(time, value)| *time >= start && *time <= end && value.is_finite())
        .map(|(_, value)| *value).collect();
    let lo = all.iter().copied().reduce(f32::min).unwrap_or(32.0) - 2.0;
    let hi = all.iter().copied().reduce(f32::max).unwrap_or(80.0) + 2.0;
    egui::Frame::new().fill(egui::Color32::from_rgb(12, 27, 43))
        .stroke(egui::Stroke::new(1.0, LINE)).corner_radius(9.0)
        .inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 150.0), egui::Sense::hover());
            let plot = rect.shrink2(egui::vec2(12.0, 14.0));
            let painter = ui.painter();
            painter.line_segment([egui::pos2(plot.left(), plot.bottom()), egui::pos2(plot.right(), plot.bottom())],
                egui::Stroke::new(1.0, LINE));
            let (_, color, points) = &series[selected];
            let mut previous: Option<(DateTime<Utc>, egui::Pos2)> = None;
            for &(time, value) in points {
                let Some(pos) = temperature_plot_point(plot, start, end, lo, hi, time, value) else { continue };
                if let Some((prior_time, prior_pos)) = previous {
                    if (time - prior_time).num_minutes().abs() <= 90 {
                        painter.line_segment([prior_pos, pos], egui::Stroke::new(2.5, *color));
                    }
                }
                painter.circle_filled(pos, 3.0, *color);
                previous = Some((time, pos));
            }
            let font = egui::FontId::proportional(10.0);
            painter.text(egui::pos2(plot.left(), rect.bottom()-2.0), egui::Align2::LEFT_BOTTOM, "−24 h", font.clone(), MUTED);
            painter.text(egui::pos2(plot.left()+plot.width()*2.0/3.0, rect.bottom()-2.0), egui::Align2::CENTER_BOTTOM, "now", font.clone(), MUTED);
            painter.text(egui::pos2(plot.right(), rect.bottom()-2.0), egui::Align2::RIGHT_BOTTOM, "+12 h", font, MUTED);
            ui.horizontal_wrapped(|ui| {
                for (index, (label, color, _)) in series.iter().enumerate() {
                    let button = egui::Button::new(egui::RichText::new(format!("● {label}")).size(10.0).color(*color))
                        .fill(if selected == index { egui::Color32::from_rgb(38, 87, 131) } else { TILE })
                        .stroke(egui::Stroke::new(1.0, LINE)).corner_radius(5.0);
                    if ui.add(button).clicked() { selected = index; }
                }
            });
            if series[selected].2.is_empty() {
                ui.label(egui::RichText::new("No loaded samples for this source yet.").size(11.0).color(MUTED));
            }
        });
    ui.ctx().data_mut(|d| d.insert_temp(id, selected));
}

/// Same time and temperature axes for observations and every loaded source. Dots are exact samples;
/// no line is drawn across missing hours or between forecast steps.
pub(crate) fn temperature_comparison(ui: &mut egui::Ui, observations: &[Observation], history: &PointHistory) {
    let now = Utc::now();
    let start = now - chrono::Duration::hours(24);
    let end = now + chrono::Duration::hours(12);
    let observed = observations.iter().filter_map(|ob| Some((ob.time?, ob.temp_c?)))
        .map(|(time, c)| (time, c_to_f(c)));
    let series = [
        ("Observed", egui::Color32::from_rgb(255, 145, 95), observed.collect::<Vec<_>>()),
        ("RTMA / URMA", egui::Color32::from_rgb(105, 205, 240), history.analysis.iter()
            .map(|p| (p.valid, c_to_f(p.temp_k - 273.15))).collect()),
        ("HRRR", egui::Color32::from_rgb(170, 145, 245), history.hrrr.iter()
            .map(|p| (p.valid, c_to_f(p.temp_k - 273.15))).collect()),
        ("Global", egui::Color32::from_rgb(145, 205, 145), history.forecast.iter()
            .map(|p| (p.valid, c_to_f(p.temp_k - 273.15))).collect()),
    ];
    let values: Vec<f32> = series.iter().flat_map(|(_, _, points)| points.iter())
        .filter(|(time, value)| *time >= start && *time <= end && value.is_finite())
        .map(|(_, value)| *value).collect();
    let Some(lo) = values.iter().copied().reduce(f32::min) else { return };
    let hi = values.iter().copied().reduce(f32::max).unwrap_or(lo);
    let (lo, hi) = (lo - 2.0, hi + 2.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 140.0), egui::Sense::hover());
    let plot = rect.shrink2(egui::vec2(12.0, 8.0));
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    let middle = plot.left() + plot.width() * (24.0 / 36.0);
    painter.line_segment([egui::pos2(middle, plot.top()), egui::pos2(middle, plot.bottom())],
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()));
    for (_, color, points) in &series {
        for &(time, value) in points {
            if let Some(pos) = temperature_plot_point(plot, start, end, lo, hi, time, value) {
                painter.circle_filled(pos, 3.0, *color);
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        for (label, color, _) in &series {
            ui.colored_label(*color, *label);
        }
    });
    let (axis, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
    let axis_text = ui.visuals().weak_text_color();
    let font = egui::FontId::proportional(11.0);
    for (x, align, label) in [
        (plot.left(), egui::Align2::LEFT_TOP, "−24 h"),
        (middle, egui::Align2::CENTER_TOP, "now"),
        (plot.right(), egui::Align2::RIGHT_TOP, "+12 h"),
    ] {
        ui.painter().text(egui::pos2(x, axis.top()), align, label, font.clone(), axis_text);
    }
    ui.weak(format!("{lo:.0}–{hi:.0} °F · exact points only"));
}

fn temperature_plot_point(
    rect: egui::Rect, start: DateTime<Utc>, end: DateTime<Utc>,
    lo: f32, hi: f32, time: DateTime<Utc>, value: f32,
) -> Option<egui::Pos2> {
    if time < start || time > end || !value.is_finite() || hi <= lo { return None; }
    let x = (time - start).num_seconds() as f32 / (end - start).num_seconds() as f32;
    let y = 1.0 - ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    Some(egui::pos2(rect.left() + rect.width() * x, rect.top() + rect.height() * y))
}

fn nearest_temperature(obs: &[Observation], valid: DateTime<Utc>) -> Option<(f32, DateTime<Utc>)> {
    obs.iter()
        .filter_map(|ob| Some((ob.temp_c.filter(|v| v.is_finite())?, ob.time?)))
        .filter(|(_, time)| (*time - valid).num_seconds().abs() <= 90 * 60)
        .min_by_key(|(_, time)| (*time - valid).num_seconds().abs())
}

/// Source identities are full object URLs; display only a stable public provider label.
fn source_label(identity: &str) -> &'static str {
    if identity.contains("/urma/") { "URMA" }
    else if identity.contains("/rtma/") { "RTMA" }
    else if identity.contains("noaa-gefs") { "GEFS" }
    else if identity.contains("noaa-gfs") { "GFS" }
    else if identity.contains("noaa-hrrr") { "HRRR" }
    else if identity.contains("ecmwf") { "ECMWF" }
    else { "Other source" }
}

fn c_to_f(c: f32) -> f32 {
    c * 9.0 / 5.0 + 32.0
}

fn opt(v: Option<f32>, unit: &str, decimals: usize) -> String {
    v.map(|x| format!("{x:.*}{unit}", decimals))
        .unwrap_or_else(|| "—".into())
}

/// 16-point compass label for a wind direction in degrees.
pub(crate) fn compass(deg: f32) -> &'static str {
    const D: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    D[((deg / 22.5).round() as usize) % 16]
}

#[cfg(test)]
mod tests {
    use super::*;
    use wxdata::field::{DataClass, DataStamp, FieldFrame, QualitySummary};

    #[test]
    fn loaded_history_deduplicates_frames_and_resets_on_site_change() {
        let valid = Utc::now();
        let frame = |hour: i64| {
            let time = valid + chrono::Duration::hours(hour);
            FieldFrame::new(
                &wxdata::rtma::TEMP_DESCRIPTOR,
                wxdata::mrms::MrmsField {
                    values: vec![300.0; 4], nx: 2, ny: 2,
                    lon_west: -100.0, lon_east: -99.0,
                    lat_north: 40.0, lat_south: 39.0, time,
                },
                DataStamp {
                    source_identity: "rtma".into(), issue_time: None, run_time: None,
                    valid_time: time, received_time: valid, class: DataClass::Analysis,
                    quality: QualitySummary::Good, available_members: None,
                },
            )
        };
        let mut history = PointHistory::default();
        history.record("KAAA", -99.5, 39.5, Some(&frame(0)), None);
        history.record("KAAA", -99.5, 39.5, Some(&frame(0)), None);
        history.record("KAAA", -99.5, 39.5, Some(&frame(1)), None);
        assert_eq!(history.analysis.len(), 2);
        assert_eq!(history.analysis[0].valid, valid);
        history.record("KBBB", -99.5, 39.5, Some(&frame(1)), None);
        assert_eq!(history.analysis.len(), 1);
        assert_eq!(history.site, "KBBB");
        let point = wxdata::hrrr::PointTemperature {
            stamp: frame(0).stamp,
            kelvin: 301.0,
        };
        history.record_hrrr("KAAA", std::slice::from_ref(&point));
        assert!(history.hrrr.is_empty(), "late result from old station is ignored");
        history.record_hrrr("KBBB", &[point.clone(), point]);
        assert_eq!(history.hrrr.len(), 1);
        let analysis = wxdata::rtma::PointTemperature { stamp: frame(0).stamp, kelvin: 299.0 };
        history.record_rtma("KAAA", std::slice::from_ref(&analysis));
        assert_eq!(history.analysis.len(), 1);
        history.record_rtma("KBBB", &[analysis.clone(), analysis]);
        assert_eq!(history.analysis.len(), 2);
        let global = wxdata::global::PointTemperature {
            stamp: frame(0).stamp, kelvin: 298.0,
        };
        history.record_gfs("KAAA", std::slice::from_ref(&global));
        assert!(history.forecast.is_empty());
        history.record_gfs("KBBB", &[global.clone(), global]);
        assert_eq!(history.forecast.len(), 1);
    }

    #[test]
    fn gefs_spread_is_not_recorded_as_absolute_temperature() {
        let time = Utc::now();
        let frame = FieldFrame::new(
            &wxdata::global::TEMP_2M_DESCRIPTOR,
            wxdata::mrms::MrmsField {
                values: vec![4.0; 4], nx: 2, ny: 2,
                lon_west: -100.0, lon_east: -99.0,
                lat_north: 40.0, lat_south: 39.0, time,
            },
            DataStamp {
                source_identity: "https://example.test/pgrb2sp25/gespr.t00z.pgrb2s.0p25.f003".into(),
                issue_time: None, run_time: Some(time), valid_time: time,
                received_time: time, class: DataClass::Forecast,
                quality: QualitySummary::Good, available_members: None,
            },
        );
        let mut history = PointHistory::default();
        history.record("KAAA", -99.5, 39.5, None, Some(&frame));
        assert!(history.forecast.is_empty());
    }

    #[test]
    fn station_comparison_uses_nearest_valid_observation_only() {
        let valid = Utc::now();
        let ob = |minutes, temp| Observation {
            time: Some(valid + chrono::Duration::minutes(minutes)), temp_c: temp,
            dewpoint_c: None, rh: None, wind_kmh: None, gust_kmh: None,
            wind_dir_deg: None, pressure_pa: None, slp_pa: None,
        };
        let observations = [ob(-20, Some(20.0)), ob(10, Some(21.0)), ob(2, None)];
        assert_eq!(nearest_temperature(&observations, valid), Some((21.0, valid + chrono::Duration::minutes(10))));
        assert_eq!(nearest_temperature(&observations, valid + chrono::Duration::hours(3)), None);
    }

    #[test]
    fn source_labels_do_not_expose_object_urls() {
        assert_eq!(source_label("https://noaa-gfs-bdp-pds.s3.amazonaws.com/key?secret=1"), "GFS");
        assert_eq!(source_label("https://nomads.ncep.noaa.gov/pub/data/nccf/com/urma/prod/file"), "URMA");
    }

    #[test]
    fn temperature_plot_uses_actual_time_and_rejects_outside_window() {
        let start = Utc::now();
        let end = start + chrono::Duration::hours(12);
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(120.0, 100.0));
        let middle = temperature_plot_point(rect, start, end, 50.0, 100.0,
            start + chrono::Duration::hours(6), 75.0).unwrap();
        assert_eq!(middle, egui::pos2(70.0, 70.0));
        assert!(temperature_plot_point(rect, start, end, 50.0, 100.0,
            start - chrono::Duration::seconds(1), 75.0).is_none());
        assert!(temperature_plot_point(rect, start, end, 50.0, 100.0,
            start, f32::NAN).is_none());
        let now = start + chrono::Duration::hours(24);
        let wide = temperature_plot_point(rect, start, start + chrono::Duration::hours(36),
            50.0, 100.0, now, 75.0).unwrap();
        assert_eq!(wide.x, 90.0);
    }
}
