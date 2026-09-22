//! Sensor dashboard: current conditions + 24h trend sparklines from the nearest NWS/METAR
//! station. Sparklines are hand-rolled on the painter (no egui_plot dependency).

use crate::theme::stat_card;
use wxdata::obs::{Observation, StationObs};

const KMH_TO_MPH: f32 = 0.621_371;

/// Show the sensor window. `data` is `Ok(station)`, `Err(message)`, or `None` (loading).
/// Returns `false` when it should close.
pub fn show(
    ctx: &egui::Context,
    data: Option<&Result<StationObs, String>>,
    analysis: Option<&wxdata::field::FieldFrame>,
    forecast: Option<&wxdata::field::FieldFrame>,
    point: Option<(f64, f64)>,
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
        Some(Ok(station)) => dashboard(ui, station, analysis, forecast, point, tz),
    });
    open
}

fn dashboard(
    ui: &mut egui::Ui,
    station: &StationObs,
    analysis: Option<&wxdata::field::FieldFrame>,
    forecast: Option<&wxdata::field::FieldFrame>,
    point: Option<(f64, f64)>,
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
        if let Some((lon, lat)) = point {
            ui.separator();
            ui.strong("Loaded temperature fields at radar site");
            ui.weak("Observations above cover 24 hours; each field below is one loaded valid time.");
            for (label, frame) in [("Surface analysis", analysis), ("Forecast", forecast)] {
                if let Some(frame) = frame {
                    let sample = frame.sample(lon, lat);
                    if let Some(kelvin) = sample.value {
                        ui.label(format!("{} · {:.1} °F · valid {} UTC",
                            label, c_to_f(kelvin - 273.15),
                            sample.valid_time.format("%Y-%m-%d %H:%M")));
                    }
                }
            }
        }
    });
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
