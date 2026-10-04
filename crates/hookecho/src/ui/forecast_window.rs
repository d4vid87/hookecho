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
    comparison: (Option<&wxdata::obs::StationObs>, &crate::ui::sensor_window::PointHistory),
    popovers: &mut crate::ui::popover::Popovers,
) -> bool {
    let (now, history) = comparison;
    let mut close = false;
    let phone = cfg!(target_os = "android");
    popovers
        .card(ctx, "forecast", egui::Window::new("Forecast").title_bar(false))
        .frame(egui::Frame::new().fill(Color32::from_rgb(17, 34, 47))
            .stroke(Stroke::new(1.0, Color32::from_rgb(86, 115, 138)))
            .corner_radius(if phone { 0.0 } else { 16.0 })
            .inner_margin(if phone { 12.0 } else { 20.0 }))
        .default_size([440.0, 740.0])
        .show(ctx, |ui| {
            if !phone { ui.set_min_width(390.0); }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("POINT FORECAST · {:.3}, {:.3}", at.1, at.0))
                        .monospace().size(10.0).color(Color32::from_rgb(145, 185, 211)));
                    let title = match state {
                        State::Ready(f) if f.daily.first().is_some_and(|p| p.short.to_lowercase().contains("rain") || p.short.to_lowercase().contains("shower")) => "A wet stretch ahead.",
                        State::Ready(f) if f.daily.first().is_some_and(|p| p.short.to_lowercase().contains("thunder")) => "Storms are possible.",
                        _ => "Weather at this point.",
                    };
                    ui.label(RichText::new(title).size(25.0).strong().color(Color32::WHITE));
                    if let State::Ready(f) = state { ui.label(RichText::new(format!("{} · point forecast", f.office)).size(11.0).color(MUTED)); }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    if ui.button("×").on_hover_text("Close forecast").clicked() { close = true; }
                });
            });
            ui.add_space(15.0);
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 120.0).clamp(260.0, 740.0))
                .show(ui, |ui| match state {
                    State::Loading => { ui.spinner(); ui.weak("Fetching forecast…"); }
                    State::Failed(e) => { ui.colored_label(Color32::from_rgb(230, 120, 120), e); }
                    State::Ready(f) => atlas_body(ui, f, now, history, minute, tz, at),
                });
        });
    !close
}

const MUTED: Color32 = Color32::from_rgb(159, 178, 196);
const BLUE: Color32 = Color32::from_rgb(84, 185, 255);
const GOLD: Color32 = Color32::from_rgb(255, 208, 107);
const MINT: Color32 = Color32::from_rgb(100, 221, 182);

fn tile(ui: &mut egui::Ui, label: &str, value: String, color: Color32) {
    egui::Frame::new().fill(Color32::from_rgb(33, 56, 75))
        .stroke(Stroke::new(1.0, Color32::from_rgb(54, 84, 107)))
        .corner_radius(10.0).inner_margin(10.0).show(ui, |ui| {
            ui.set_min_width(145.0);
            ui.label(RichText::new(label).size(10.0).color(MUTED));
            ui.label(RichText::new(value).size(23.0).strong().color(color));
        });
}

fn atlas_body(
    ui: &mut egui::Ui, f: &PointForecast, now: Option<&wxdata::obs::StationObs>,
    history: &crate::ui::sensor_window::PointHistory, minute: Option<&[Option<f32>]>,
    tz: Option<wxdata::tz::Tz>, at: (f64, f64),
) {
    let observation = now.and_then(|s| s.obs.first());
    let current = observation.and_then(|o| o.temp_c.map(crate::ui::station_card::c_to_f))
        .or_else(|| f.hourly.first().map(|p| p.temp_f));
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("CURRENT").monospace().size(10.0).color(MUTED));
            ui.label(RichText::new(current.map_or("—".to_string(), |t| format!("{t:.0}°")))
                .size(58.0).strong().color(GOLD));
            if let Some((station, ob)) = now.and_then(|s| s.obs.first().map(|o| (s, o))) {
                ui.label(RichText::new(conditions_line(ob, &station.station_id)).size(11.0).color(MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(86.0), Sense::hover());
            let center = rect.center();
            ui.painter().circle_stroke(center, 34.0, Stroke::new(7.0, Color32::from_rgb(64, 91, 113)));
            let label = f.spc_risk.as_deref().unwrap_or("—");
            let fraction = match label { "HIGH" => 1.0, "MDT" => 0.85, "ENH" => 0.7,
                "SLGT" => 0.55, "MRGL" => 0.4, "TSTM" => 0.25, _ => 0.0 };
            if fraction > 0.0 {
                let points = (0..=24).map(|i| {
                    let angle = std::f32::consts::TAU * (i as f32 / 24.0 * fraction - 0.25);
                    center + Vec2::new(angle.cos(), angle.sin()) * 34.0
                }).collect();
                ui.painter().add(egui::Shape::line(points,
                    Stroke::new(7.0, Color32::from_rgb(250, 146, 114))));
            }
            ui.painter().text(center, Align2::CENTER_CENTER, label, FontId::proportional(14.0), Color32::from_rgb(255, 185, 151));
        });
    });
    ui.label(RichText::new("SPC Day 1 · categorical storm risk at this point").size(10.0).color(MUTED));
    ui.add_space(14.0);
    let risk = f.spc_risk.as_deref().unwrap_or("UNAVAILABLE");
    egui::Frame::new().fill(Color32::from_rgb(41, 44, 53))
        .stroke(Stroke::new(1.0, Color32::from_rgb(100, 78, 78)))
        .corner_radius(10.0).inner_margin(10.0).show(ui, |ui| {
            ui.label(RichText::new(match risk {
                "HIGH" | "MDT" | "ENH" | "SLGT" | "MRGL" => "⚡ SPC severe weather risk at this point",
                "TSTM" => "⚡ General thunderstorms possible",
                "NONE" => "No SPC storm-risk area at this point",
                _ => "SPC risk unavailable right now",
            }).strong().size(12.0).color(Color32::from_rgb(255, 181, 143)));
            ui.label(RichText::new("Forecast outlook · check active warning polygons on the map").size(10.0).color(MUTED));
        });
    ui.add_space(8.0);
    let rain_chance = f.hourly.iter().take(24).filter_map(|p| p.precip_pct).max();
    let totals: Vec<f32> = f.models.iter().filter_map(|m| m.inches).collect();
    let range = totals.iter().copied().reduce(f32::min).zip(totals.iter().copied().reduce(f32::max));
    ui.columns(2, |cols| {
        tile(&mut cols[0], "Rain probability · next 24h", rain_chance.map_or("—".into(), |p| format!("{p}%")), BLUE);
        tile(&mut cols[1], "Model rainfall range", range.map_or("—".into(), |(lo, hi)| format!("{lo:.1}–{hi:.1} in")), MINT);
    });
    ui.add_space(15.0);
    ui.horizontal(|ui| { ui.strong("Four-period outlook"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new(&f.office).small().color(MUTED)); }); });
    ui.add_space(7.0);
    ui.columns(4, |cols| for (col, period) in cols.iter_mut().zip(f.daily.iter().take(4)) {
        egui::Frame::new().fill(Color32::from_rgb(33, 56, 75)).corner_radius(9.0).inner_margin(7.0).show(col, |ui| {
            ui.label(RichText::new(&period.name).size(10.0).color(MUTED));
            ui.label(RichText::new(format!("{:.0}°", period.temp_f)).strong().size(18.0)
                .color(if period.is_day { GOLD } else { BLUE }));
            ui.label(RichText::new(period.precip_pct.map_or("—".into(), |p| format!("{p}% rain"))).size(10.0).color(BLUE));
        });
    });
    ui.add_space(15.0);
    ui.horizontal(|ui| { ui.strong("Guidance spread"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(RichText::new("24h rainfall · model").small().color(MUTED)); }); });
    ui.add_space(6.0);
    for model in &f.models {
        egui::Frame::new().fill(Color32::from_rgb(33, 56, 75)).corner_radius(8.0)
            .inner_margin(8.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized([62.0, 18.0], egui::Label::new(RichText::new(model.name).strong().size(11.0)));
                    let (rect, _) = ui.allocate_exact_size(Vec2::new((ui.available_width() - 73.0).max(50.0), 6.0), Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(20, 42, 59));
                    if let Some(inches) = model.inches {
                        let width = rect.width() * (inches / 1.0).clamp(0.0, 1.0);
                        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(width, 6.0)), 3.0, BLUE);
                    }
                    ui.label(RichText::new(model.inches.map_or("—".into(), |v| format!("{v:.2} in"))).size(11.0));
                });
            });
        ui.add_space(5.0);
    }
    ui.add_space(5.0);
    ui.label(RichText::new("NWS/Open-Meteo: official point forecast · SPC: Day-1 risk · Euro/GFS/HRRR: independent model guidance. Model rainfall is not an official warning.")
        .size(10.0).color(MUTED));
    egui::CollapsingHeader::new("All forecast details").show(ui, |ui| {
        ui.weak(almanac_line(at, tz, Utc::now()));
        if let Some(m) = minute { minute_strip(ui, m); }
        if !f.hourly.is_empty() { hourly_strip(ui, &f.hourly, tz); wind_strip(ui, &f.hourly); }
        crate::ui::sensor_window::temperature_comparison(ui,
            now.map(|station| station.obs.as_slice()).unwrap_or(&[]), history);
        if !history.has_samples() { ui.weak("Waiting for analysis and forecast samples."); }
        for p in &f.daily { ui.label(format!("{} · {:.0}° · {} · {}", p.name, p.temp_f,
            p.precip_pct.map_or("—".into(), |v| format!("{v}% rain")), p.short)); }
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

/// Wind speed over the same 24 hours the temperature curve covers, with an arrow every third
/// hour showing where the wind is coming from.
///
/// The window showed wind only as prose on the current-conditions line, which answers "what is
/// it doing now" but not "when does it get windy" — the question that decides whether you tie
/// something down. Gusts are missing on purpose: neither feed publishes an hourly gust, so
/// there is nothing honest to draw.
fn wind_strip(ui: &mut egui::Ui, hours: &[wxdata::forecast::Period]) {
    let hours: Vec<_> = hours.iter().take(24).collect();
    // Nothing to say if the feed gave no numbers — the NWS publishes wind as prose, and a
    // string it cannot parse must not become a flat line at zero.
    if hours.len() < 2 || hours.iter().all(|h| h.wind_mph.is_none()) {
        return;
    }
    let w = ui.available_width().max(220.0);
    let h = 46.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_black_alpha(90));
    let body = rect.shrink2(Vec2::new(6.0, 5.0));

    let peak = hours
        .iter()
        .filter_map(|x| x.wind_mph)
        .fold(0.0f32, f32::max)
        .max(10.0); // a calm day still gets a sensible scale rather than a magnified wobble
    let x_of = |i: usize| body.left() + (i as f32 + 0.5) / hours.len() as f32 * body.width();
    let y_of = |v: f32| body.bottom() - (v / peak) * body.height() * 0.62;

    let pts: Vec<egui::Pos2> = hours
        .iter()
        .enumerate()
        .filter_map(|(i, x)| Some(egui::pos2(x_of(i), y_of(x.wind_mph?))))
        .collect();
    p.add(egui::Shape::line(
        pts,
        Stroke::new(1.6, Color32::from_rgb(120, 210, 190)),
    ));

    // Direction arrows: every third hour, pointing the way the wind is going (the compass
    // reading is where it comes *from*, so the arrow is the reverse).
    for (i, x) in hours.iter().enumerate() {
        if i % 3 != 0 {
            continue;
        }
        let (Some(v), Some(from_deg)) = (x.wind_mph, x.wind_deg) else {
            continue;
        };
        let to = (from_deg + 180.0).to_radians();
        // Screen y grows downward, so north is -y.
        let dir = Vec2::new(to.sin(), -to.cos());
        let c = egui::pos2(x_of(i), y_of(v) - 9.0);
        let tip = c + dir * 4.5;
        let tail = c - dir * 4.5;
        let wing = Vec2::new(-dir.y, dir.x) * 2.4;
        let grey = Color32::from_gray(180);
        p.line_segment([tail, tip], Stroke::new(1.0, grey));
        p.line_segment([tip, tip - dir * 3.0 + wing], Stroke::new(1.0, grey));
        p.line_segment([tip, tip - dir * 3.0 - wing], Stroke::new(1.0, grey));
    }

    p.text(
        rect.left_top() + Vec2::new(6.0, 2.0),
        Align2::LEFT_TOP,
        format!("wind · peak {peak:.0} mph"),
        FontId::proportional(9.0),
        Color32::from_gray(170),
    );
    resp.on_hover_text("Sustained wind over the next 24 hours; arrows show which way it blows");
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
fn almanac_line(at: (f64, f64), tz: Option<wxdata::tz::Tz>, now: DateTime<Utc>) -> String {
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
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-21T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let s = almanac_line((-97.5, 35.5), None, now);
        assert!(s.contains("Sunrise") && s.contains("Sunset"), "got {s}");
        // Polar latitudes lose the sun but keep the moon.
        let polar = almanac_line((15.0, 89.0), None, now);
        assert!(!polar.contains("Sunrise"), "got {polar}");
    }
}
