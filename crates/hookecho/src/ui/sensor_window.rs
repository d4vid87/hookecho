//! Sensor dashboard: current conditions + 24h trend sparklines from the nearest NWS/METAR
//! station. Sparklines are hand-rolled on the painter (no egui_plot dependency).

use crate::theme::stat_card;
use chrono::{DateTime, Utc};
use wxdata::obs::{Observation, StationObs};

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
    let Some(window) = drawer.page(
        ctx,
        "Sensors",
        &mut open,
        false,
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
    ui.horizontal(|ui| {
        ui.strong(&station.station_id);
        if !station.name.is_empty() {
            ui.weak(&station.name);
        }
    });
    let Some(cur) = station.obs.first() else {
        ui.weak("(no observations)");
        return;
    };
    if let Some(t) = cur.time {
        let age = (chrono::Utc::now() - t).num_minutes().max(0);
        ui.weak(format!(
            "Observed {} ({age} min ago)",
            crate::timefmt::fmt_clock(t, tz, false)
        ));
    }
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Big temperature.
        let temp_f = cur.temp_c.map(c_to_f);
        ui.label(
            egui::RichText::new(
                temp_f
                    .map(|f| format!("{f:.0}°F"))
                    .unwrap_or_else(|| "—".into()),
            )
            .size(34.0)
            .strong(),
        );

        ui.horizontal_wrapped(|ui| {
            stat_card(ui, "Humidity", &opt(cur.rh, "%", 0));
            stat_card(ui, "Dewpoint", &opt(cur.dewpoint_c.map(c_to_f), "°F", 0));
            let wind = match (cur.wind_kmh.map(|k| k * KMH_TO_MPH), cur.wind_dir_deg) {
                (Some(s), Some(d)) => format!("{s:.0} mph {}", compass(d)),
                (Some(s), None) => format!("{s:.0} mph"),
                _ => "—".into(),
            };
            stat_card(ui, "Wind", &wind);
            stat_card(
                ui,
                "Gust",
                &opt(cur.gust_kmh.map(|k| k * KMH_TO_MPH), " mph", 0),
            );
            let pres = match cur.pressure_pa {
                Some(pa) => format!("{:.0} mb / {:.2}\"", pa / 100.0, pa * 0.000_295_3),
                None => "—".into(),
            };
            stat_card(ui, "Pressure", &pres);
            stat_card(
                ui,
                "Sea-level",
                &cur.slp_pa
                    .map(|pa| format!("{:.0} mb", pa / 100.0))
                    .unwrap_or_else(|| "—".into()),
            );
        });

        ui.add_space(6.0);
        // Trend sparklines (oldest -> newest, left to right).
        let series = |f: fn(&Observation) -> Option<f32>| -> Vec<f32> {
            station.obs.iter().rev().filter_map(f).collect()
        };
        trend(
            ui,
            "Temperature °F",
            series(|o| o.temp_c.map(c_to_f)),
            egui::Color32::from_rgb(255, 140, 90),
        );
        trend(
            ui,
            "Dewpoint °F",
            series(|o| o.dewpoint_c.map(c_to_f)),
            egui::Color32::from_rgb(120, 200, 140),
        );
        trend(
            ui,
            "Humidity %",
            series(|o| o.rh),
            egui::Color32::from_rgb(90, 170, 255),
        );
        trend(
            ui,
            "Wind mph",
            series(|o| o.wind_kmh.map(|k| k * KMH_TO_MPH)),
            egui::Color32::from_rgb(200, 200, 200),
        );
        if let Some(history) = history.filter(|history| history.site == station.station_id) {
            ui.separator();
            ui.strong(format!("Loaded temperature history at {}", station.station_id));
            ui.weak("RTMA: recent 6 hours · HRRR/GFS: current run · Other global: viewed frames. Observations above cover 24 hours.");
            temperature_comparison(ui, station, history);
            if history.analysis.is_empty() && history.hrrr.is_empty() && history.forecast.is_empty() {
                ui.weak("Waiting for station analysis and forecast samples.");
            }
            for (label, series) in [("RTMA / URMA", &history.analysis), ("HRRR analysis + forecast", &history.hrrr), ("Global forecast", &history.forecast)] {
                if !series.is_empty() {
                    ui.label(format!("{label} · {} valid times", series.len()));
                    for reading in series.iter().rev().take(8) {
                        let bias = nearest_temperature(&station.obs, reading.valid)
                            .map(|(observed, time)| format!(" · vs observed {:+.1} °F ({} min apart)",
                                c_to_f(reading.temp_k - 273.15) - c_to_f(observed),
                                (time - reading.valid).num_minutes().abs()))
                            .unwrap_or_default();
                        let lead = reading.run.map(|run| if run == reading.valid { "analysis".to_string() }
                            else { format!("f+{} h", (reading.valid - run).num_hours()) })
                            .unwrap_or_default();
                        ui.weak(format!("{} {lead} · {} UTC · {:.1} °F{bias}",
                            source_label(&reading.source), reading.valid.format("%Y-%m-%d %H:%M"),
                            c_to_f(reading.temp_k - 273.15)));
                    }
                }
            }
        } else {
            ui.weak(if station.location.is_some() {
                "No temperature field frames sampled at this station yet."
            } else {
                "Station coordinates unavailable; gridded comparison cannot be sampled here."
            });
        }
    });
}

/// Same time and temperature axes for observations and every loaded source. Dots are exact samples;
/// no line is drawn across missing hours or between forecast steps.
fn temperature_comparison(ui: &mut egui::Ui, station: &StationObs, history: &PointHistory) {
    let now = Utc::now();
    let start = now - chrono::Duration::hours(6);
    let end = now + chrono::Duration::hours(6);
    let observed = station.obs.iter().filter_map(|ob| Some((ob.time?, ob.temp_c?)))
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
    let middle = plot.left() + plot.width() * 0.5;
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
    ui.columns(3, |cols| {
        cols[0].weak("−6 h");
        cols[1].weak("now");
        cols[2].weak("+6 h");
    });
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

/// A labelled sparkline row: the series drawn as a min-max normalized polyline.
fn trend(ui: &mut egui::Ui, label: &str, vals: Vec<f32>, color: egui::Color32) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(label).small().weak());
    crate::theme::sparkline(ui, &vals, color);
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
    }
}
