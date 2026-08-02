//! Point forecast window: tap the map, get the plain forecast for that spot.
//!
//! The hourly strip is hand-painted rather than pulled in as a plotting dependency — it's one
//! polyline and a row of bars, and the app already draws its own hodographs and cross-sections.

use chrono::{DateTime, Utc};
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};
use wxdata::forecast::PointForecast;

/// What the app is currently holding for the tapped point.
pub enum State {
    Loading,
    Ready(Box<PointForecast>),
    Failed(String),
}

/// Show the window. `minute` is the per-minute radar-advection profile over the point (dBZ per
/// minute from now); `None` in archive or without a volume, which hides that section. `now` is the
/// nearest station's latest observation, if one arrived.
pub fn show(
    ctx: &egui::Context,
    state: &State,
    at: (f64, f64),
    tz: Option<wxdata::tz::Tz>,
    minute: Option<&[Option<f32>]>,
    now: Option<(&str, &wxdata::obs::Observation)>,
) -> bool {
    let mut open = true;
    crate::ui::phone_surface(ctx, egui::Window::new("Forecast"))
        .open(&mut open)
        .default_size([460.0, 460.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("{:.3}, {:.3}", at.1, at.0));
                if let State::Ready(f) = state {
                    if !f.office.is_empty() {
                        ui.weak(format!("· {}", f.office));
                    }
                }
            });
            if let Some((station, o)) = now {
                ui.label(conditions_line(o, station));
            }
            ui.weak(almanac_line(at, tz));
            ui.separator();
            if let Some(m) = minute {
                minute_strip(ui, m);
                ui.add_space(6.0);
            }
            match state {
                State::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Fetching forecast…");
                    });
                }
                State::Failed(e) => {
                    ui.colored_label(Color32::from_rgb(230, 120, 120), e);
                    ui.small("Forecast services go down; tap the map again to retry.");
                }
                State::Ready(f) => body(ui, f, tz),
            }
        });
    open
}

fn body(ui: &mut egui::Ui, f: &PointForecast, tz: Option<wxdata::tz::Tz>) {
    if !f.hourly.is_empty() {
        ui.label(RichText::new("Next 24 hours").strong());
        hourly_strip(ui, &f.hourly, tz);
        ui.add_space(6.0);
    }
    ui.label(RichText::new("This week").strong());
    ui.add_space(2.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        for p in &f.daily {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [104.0, 18.0],
                    egui::Label::new(RichText::new(&p.name).strong()).selectable(false),
                );
                let temp = RichText::new(format!("{:.0}°", p.temp_f))
                    .strong()
                    .color(if p.is_day {
                        Color32::from_rgb(245, 190, 90)
                    } else {
                        Color32::from_rgb(150, 180, 240)
                    });
                ui.add_sized([44.0, 18.0], egui::Label::new(temp).selectable(false));
                if let Some(pc) = p.precip_pct {
                    ui.add_sized(
                        [42.0, 18.0],
                        egui::Label::new(
                            RichText::new(format!("{pc}%")).color(Color32::from_rgb(110, 180, 240)),
                        )
                        .selectable(false),
                    );
                } else {
                    ui.add_sized([42.0, 18.0], egui::Label::new("").selectable(false));
                }
                ui.label(&p.short);
            })
            .response
            .on_hover_text(if p.wind.is_empty() {
                p.short.clone()
            } else {
                format!("{}\nWind {}", p.short, p.wind)
            });
        }
    });
}

/// Minute-by-minute rain over the point for the next hour, advected from the current radar scan.
/// This is a nowcast off one volume, not a forecast product — hence the "~" in the caption.
fn minute_strip(ui: &mut egui::Ui, minute: &[Option<f32>]) {
    use crate::rain_arrival::intensity;
    if minute.len() < 2 {
        return;
    }
    ui.label(RichText::new("Next hour (radar)").strong());
    let w = ui.available_width().max(220.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_black_alpha(90));
    let bw = rect.width() / minute.len() as f32;
    for (i, v) in minute.iter().enumerate() {
        let lvl = v.map(intensity).unwrap_or(0);
        if lvl == 0 {
            continue;
        }
        let (color, frac) = match lvl {
            3 => (Color32::from_rgb(230, 90, 90), 1.0),
            2 => (Color32::from_rgb(80, 170, 230), 0.75),
            _ => (Color32::from_rgb(90, 130, 190), 0.45),
        };
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + i as f32 * bw, rect.bottom() - 22.0 * frac),
            egui::pos2(rect.left() + (i + 1) as f32 * bw, rect.bottom() - 2.0),
        );
        p.rect_filled(bar, 0.0, color);
    }
    let wet: Vec<usize> = minute
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_some_and(|d| intensity(d) > 0))
        .map(|(i, _)| i)
        .collect();
    ui.small(match (wet.first(), wet.last()) {
        (Some(0), Some(e)) => format!("raining now · ends ~{e} min"),
        (Some(s), Some(e)) => format!("rain starts ~{s} min · ends ~{e} min"),
        _ => "no rain in the next hour".to_string(),
    });
}

/// Temperature curve over precipitation-probability bars, 24 hours wide.
fn hourly_strip(ui: &mut egui::Ui, hours: &[wxdata::forecast::Period], tz: Option<wxdata::tz::Tz>) {
    let hours: Vec<_> = hours.iter().take(24).collect();
    if hours.len() < 2 {
        return;
    }
    let w = ui.available_width().max(220.0);
    let h = 96.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_black_alpha(90));

    let plot = rect.shrink2(Vec2::new(6.0, 4.0));
    let axis_h = 12.0;
    let body = egui::Rect::from_min_max(
        plot.left_top(),
        egui::pos2(plot.right(), plot.bottom() - axis_h),
    );

    let temps: Vec<f32> = hours.iter().map(|x| x.temp_f).collect();
    let (lo, hi) = temps
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &t| (a.min(t), b.max(t)));
    // Always give the curve some vertical room, even on a flat day.
    let (lo, hi) = if (hi - lo).abs() < 5.0 {
        (lo - 3.0, hi + 3.0)
    } else {
        (lo, hi)
    };
    let x_of = |i: usize| body.left() + (i as f32 + 0.5) / hours.len() as f32 * body.width();
    let y_of = |t: f32| body.bottom() - ((t - lo) / (hi - lo).max(1.0)) * body.height() * 0.78;

    // Precip bars first — they're the backdrop the temperature reads against.
    let bw = (body.width() / hours.len() as f32) * 0.7;
    for (i, x) in hours.iter().enumerate() {
        let Some(pc) = x.precip_pct.filter(|v| *v > 0) else {
            continue;
        };
        let frac = pc as f32 / 100.0;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x_of(i) - bw / 2.0, body.bottom() - frac * body.height()),
            egui::pos2(x_of(i) + bw / 2.0, body.bottom()),
        );
        p.rect_filled(bar, 1.0, Color32::from_rgba_unmultiplied(70, 140, 230, 120));
    }

    let pts: Vec<egui::Pos2> = temps
        .iter()
        .enumerate()
        .map(|(i, &t)| egui::pos2(x_of(i), y_of(t)))
        .collect();
    p.add(egui::Shape::line(
        pts.clone(),
        Stroke::new(1.6, Color32::from_rgb(245, 190, 90)),
    ));

    // Label the ends and the extremes only — 24 numbers is noise.
    let font = FontId::proportional(9.0);
    let hottest = temps
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i);
    for i in [Some(0), hottest, Some(hours.len() - 1)]
        .into_iter()
        .flatten()
    {
        p.text(
            pts[i] - Vec2::new(0.0, 4.0),
            Align2::CENTER_BOTTOM,
            format!("{:.0}°", temps[i]),
            font.clone(),
            Color32::from_gray(235),
        );
    }
    // Time axis every 6 hours.
    for (i, x) in hours.iter().enumerate() {
        if i % 6 != 0 {
            continue;
        }
        p.text(
            egui::pos2(x_of(i), plot.bottom() - axis_h + 1.0),
            Align2::CENTER_TOP,
            short_hour(x.start, tz),
            font.clone(),
            Color32::from_gray(170),
        );
    }
}

/// One-line current conditions, skipping whatever the station didn't report:
/// `74°F · dew 62°F · 62% rh · SW 12 kt G18 · 29.92 inHg · KOKC`.
fn conditions_line(o: &wxdata::obs::Observation, station: &str) -> String {
    use crate::ui::station_card::c_to_f;
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = o.temp_c {
        parts.push(format!("{:.0}°F", c_to_f(t)));
    }
    if let Some(d) = o.dewpoint_c {
        parts.push(format!("dew {:.0}°F", c_to_f(d)));
    }
    if let Some(rh) = o.rh {
        parts.push(format!("{rh:.0}% rh"));
    }
    if let Some(kmh) = o.wind_kmh {
        let kt = kmh / 1.852;
        let dir = o
            .wind_dir_deg
            .map(|d| format!("{} ", crate::ui::sensor_window::compass(d)))
            .unwrap_or_default();
        let gust = match o.gust_kmh {
            Some(g) => format!(" G{:.0}", g / 1.852),
            None => String::new(),
        };
        parts.push(if kt < 1.0 {
            "calm".to_string()
        } else {
            format!("{dir}{kt:.0} kt{gust}")
        });
    }
    // Sea-level pressure is what people read off a barometer; fall back to the station value.
    if let Some(pa) = o.slp_pa.or(o.pressure_pa) {
        parts.push(format!("{:.2} inHg", pa / 3386.389));
    }
    if !station.is_empty() {
        parts.push(station.to_string());
    }
    parts.join(" · ")
}

/// `Sunrise 6:14 AM · Sunset 8:42 PM · Waxing gibbous`, or just the moon during polar day/night.
// ponytail: words, not ↑/↓ arrows — those render as tofu boxes in the Android font stack.
fn almanac_line(at: (f64, f64), tz: Option<wxdata::tz::Tz>) -> String {
    let now = Utc::now();
    let date = match tz {
        Some(tz) => now.with_timezone(&tz).date_naive(),
        None => now.date_naive(),
    };
    let (moon, _) = crate::astro::moon_label(crate::astro::moon_phase(now));
    match crate::astro::sun_times(at.1, at.0, date) {
        Some((rise, set)) => {
            format!(
                "Sunrise {} · Sunset {} · {moon}",
                clock(rise, tz),
                clock(set, tz)
            )
        }
        None => moon.to_string(),
    }
}

fn clock(t: DateTime<Utc>, tz: Option<wxdata::tz::Tz>) -> String {
    match tz {
        Some(tz) => t.with_timezone(&tz).format("%-I:%M %p").to_string(),
        None => t.format("%H:%MZ").to_string(),
    }
}

fn short_hour(t: DateTime<Utc>, tz: Option<wxdata::tz::Tz>) -> String {
    match tz {
        Some(tz) => t.with_timezone(&tz).format("%-I%p").to_string(),
        None => t.format("%HZ").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wxdata::obs::Observation;

    fn blank() -> Observation {
        Observation {
            time: None,
            temp_c: None,
            dewpoint_c: None,
            rh: None,
            wind_kmh: None,
            gust_kmh: None,
            wind_dir_deg: None,
            pressure_pa: None,
            slp_pa: None,
        }
    }

    #[test]
    fn full_observation_reads_like_a_metar() {
        let o = Observation {
            temp_c: Some(23.3),
            dewpoint_c: Some(16.7),
            rh: Some(66.0),
            wind_kmh: Some(22.2),
            gust_kmh: Some(33.3),
            wind_dir_deg: Some(225.0),
            slp_pa: Some(101_320.0),
            ..blank()
        };
        assert_eq!(
            conditions_line(&o, "KOKC"),
            "74°F · dew 62°F · 66% rh · SW 12 kt G18 · 29.92 inHg · KOKC"
        );
    }

    #[test]
    fn missing_fields_are_dropped_not_blanked() {
        let o = Observation {
            temp_c: Some(10.0),
            ..blank()
        };
        assert_eq!(conditions_line(&o, "KXYZ"), "50°F · KXYZ");
        assert_eq!(conditions_line(&blank(), ""), "");
    }

    #[test]
    fn wind_without_gust_or_direction() {
        let o = Observation {
            wind_kmh: Some(18.5),
            ..blank()
        };
        assert_eq!(conditions_line(&o, ""), "10 kt");
        let calm = Observation {
            wind_kmh: Some(0.0),
            wind_dir_deg: Some(0.0),
            ..blank()
        };
        assert_eq!(conditions_line(&calm, ""), "calm");
    }

    #[test]
    fn station_pressure_backfills_sea_level() {
        let o = Observation {
            pressure_pa: Some(96_000.0),
            ..blank()
        };
        assert_eq!(conditions_line(&o, ""), "28.35 inHg");
    }

    #[test]
    fn almanac_line_has_both_events_in_the_tropics() {
        let s = almanac_line((-97.5, 35.5), None);
        assert!(s.contains("Sunrise") && s.contains("Sunset"), "got {s}");
        // Polar latitudes lose the sun but keep the moon.
        let polar = almanac_line((15.0, 89.0), None);
        assert!(!polar.contains("Sunrise"), "got {polar}");
    }
}
