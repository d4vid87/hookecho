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

/// Show the window. Returns `false` when it should close.
pub fn show(
    ctx: &egui::Context,
    state: &State,
    at: (f64, f64),
    tz: Option<wxdata::tz::Tz>,
) -> bool {
    let mut open = true;
    crate::ui::fit_phone(ctx, egui::Window::new("Forecast"))
        .open(&mut open)
        .default_size([460.0, 460.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("{:.3}, {:.3}", at.1, at.0));
                if let State::Ready(f) = state {
                    if !f.office.is_empty() {
                        ui.weak(format!("· NWS {}", f.office));
                    }
                }
            });
            ui.separator();
            match state {
                State::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Fetching forecast…");
                    });
                }
                State::Failed(e) => {
                    ui.colored_label(Color32::from_rgb(230, 120, 120), e);
                    ui.small("The NWS grid occasionally 500s; tap the map again to retry.");
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

fn short_hour(t: DateTime<Utc>, tz: Option<wxdata::tz::Tz>) -> String {
    match tz {
        Some(tz) => t.with_timezone(&tz).format("%-I%p").to_string(),
        None => t.format("%HZ").to_string(),
    }
}
