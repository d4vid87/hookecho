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
            let Some(temp_k) = frame.sample(lon, lat).value.filter(|v| v.is_finite()) else { continue };
            if let Some(old) = series.iter_mut().find(|old| old.valid == frame.stamp.valid_time
                && old.run == frame.stamp.run_time && old.source == frame.stamp.source_identity) {
                old.temp_k = temp_k;
            } else {
                series.push(Reading {
                    valid: frame.stamp.valid_time,
                    run: frame.stamp.run_time,
                    source: frame.stamp.source_identity.clone(),
                    temp_k,
                });
                series.sort_by_key(|reading| reading.valid);
                if series.len() > 72 { series.remove(0); }
            }
        }
    }

    pub fn record_hrrr(&mut self, station: &str, points: &[wxdata::hrrr::PointTemperature]) {
        if self.site != station { return; }
        for point in points {
            if !point.kelvin.is_finite() { continue; }
            if let Some(old) = self.hrrr.iter_mut().find(|old|
                old.valid == point.stamp.valid_time && old.run == point.stamp.run_time) {
                old.temp_k = point.kelvin;
            } else {
                self.hrrr.push(Reading {
                    valid: point.stamp.valid_time, run: point.stamp.run_time,
                    source: point.stamp.source_identity.clone(), temp_k: point.kelvin,
                });
                self.hrrr.sort_by_key(|reading| reading.valid);
                if self.hrrr.len() > 72 { self.hrrr.remove(0); }
            }
        }
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
            ui.weak("Only frames viewed this session are included; observation history above covers 24 hours.");
            if history.analysis.is_empty() && history.hrrr.is_empty() && history.forecast.is_empty() {
                ui.weak("Enable a surface temperature analysis or global temperature forecast layer, or wait for HRRR samples.");
            }
            for (label, series) in [("Surface analysis", &history.analysis), ("HRRR analysis + forecast", &history.hrrr), ("Global forecast", &history.forecast)] {
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
}
