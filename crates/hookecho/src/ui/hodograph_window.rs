//! VAD wind profile, two ways: the classic hodograph (u/v curve over range rings) and a
//! time-height panel of wind barbs. Hand-drawn on the painter — no plotting dependency.
//!
//! The radar only ever serves the *latest* profile, so the time-height view is built from
//! whatever the app has collected while running: a shear profile changing over the last hour is
//! the thing you actually want to see before a storm goes up, and no single product carries it.

use chrono::{DateTime, Utc};
use wxdata::level3::VwpLevel;

const MS_TO_KT: f32 = 1.943_844;

/// Which panel the window is showing.
#[derive(Default, PartialEq, Clone, Copy)]
pub enum Tab {
    #[default]
    Hodograph,
    TimeHeight,
}

/// Show the window. `history` is oldest→newest; `tz` renders the time axis in the site's clock.
/// Returns `false` when it should close.
pub fn show(
    ctx: &egui::Context,
    site: Option<&str>,
    levels: &[VwpLevel],
    history: &[(DateTime<Utc>, Vec<VwpLevel>)],
    tab: &mut Tab,
    tz: Option<wxdata::tz::Tz>,
    drawer: &mut crate::ui::drawer::Drawer,
) -> bool {
    let mut open = true;
    let Some(window) = drawer.page_sized(
        ctx,
        "VAD wind profile",
        &mut open,
        false,
        420.0,
        egui::Window::new("VAD wind profile"),
    ) else {
        return open;
    };
    window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(site.unwrap_or("—"));
                ui.weak(format!("{} levels", levels.len()));
                ui.separator();
                ui.selectable_value(tab, Tab::Hodograph, "Hodograph");
                ui.selectable_value(tab, Tab::TimeHeight, "Time-height");
            });
            ui.separator();
            match *tab {
                Tab::Hodograph => {
                    if levels.len() < 2 {
                        ui.weak("Loading VAD wind profile…");
                        return;
                    }
                    plot(ui, levels);
                    ui.add_space(4.0);
                    legend(ui, levels);
                }
                Tab::TimeHeight => time_height(ui, history, tz),
            }
        });
    open
}

/// Wind barbs on a time (x) by altitude (y) grid — one column per collected profile.
fn time_height(
    ui: &mut egui::Ui,
    history: &[(DateTime<Utc>, Vec<VwpLevel>)],
    tz: Option<wxdata::tz::Tz>,
) {
    if history.len() < 2 {
        ui.weak("Collecting profiles…");
        ui.small(
            "The radar only publishes the newest wind profile, so this panel fills in as the app \
             runs \u{2014} about one column every 5 minutes.",
        );
        return;
    }
    let w = ui.available_width().max(240.0);
    let h = 260.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let pad_l = 34.0;
    let pad_b = 18.0;
    let plot = egui::Rect::from_min_max(
        rect.left_top() + egui::vec2(pad_l, 6.0),
        rect.right_bottom() - egui::vec2(6.0, pad_b),
    );
    painter.rect_filled(plot, 3.0, egui::Color32::from_black_alpha(90));

    let max_kft = history
        .iter()
        .flat_map(|(_, ls)| ls.iter())
        .map(|l| l.alt_kft)
        .fold(10.0f32, f32::max)
        .ceil();
    let y_of = |kft: f32| plot.bottom() - (kft / max_kft).clamp(0.0, 1.0) * plot.height();
    let font = egui::FontId::proportional(9.0);
    let grid = egui::Color32::from_gray(70);

    // Altitude gridlines every 5 kft.
    let mut kft = 0.0;
    while kft <= max_kft {
        let y = y_of(kft);
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(0.5, grid),
        );
        painter.text(
            egui::pos2(plot.left() - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{kft:.0}"),
            font.clone(),
            egui::Color32::from_gray(180),
        );
        kft += 5.0;
    }
    painter.text(
        egui::pos2(rect.left() + 2.0, plot.top()),
        egui::Align2::LEFT_TOP,
        "kft",
        font.clone(),
        egui::Color32::from_gray(150),
    );

    let n = history.len();
    let col_w = plot.width() / n as f32;
    for (i, (t, levels)) in history.iter().enumerate() {
        let x = plot.left() + (i as f32 + 0.5) * col_w;
        // Time labels thin out so they never collide.
        let stride = ((44.0 / col_w).ceil() as usize).max(1);
        if i % stride == 0 || i + 1 == n {
            painter.text(
                egui::pos2(x, plot.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                crate::timefmt::fmt_clock(*t, tz, false),
                font.clone(),
                egui::Color32::from_gray(180),
            );
        }
        for l in levels {
            let y = y_of(l.alt_kft);
            let col = speed_color(l.speed_kt);
            // Same barb geometry as the METAR station plots, rotated to the FROM bearing.
            let th = l.dir_deg.to_radians();
            let (up, right) = ([th.sin(), -th.cos()], [th.cos(), th.sin()]);
            let scale = (col_w * 0.42).clamp(7.0, 16.0);
            let map = |u: [f32; 2]| {
                egui::pos2(x, y)
                    + egui::vec2(
                        (u[0] * right[0] + u[1] * up[0]) * scale,
                        (u[0] * right[1] + u[1] * up[1]) * scale,
                    )
            };
            let segs = wxdata::metar::barb_segments(l.speed_kt);
            if segs.is_empty() {
                painter.circle_stroke(egui::pos2(x, y), 2.0, egui::Stroke::new(1.0, col));
            }
            for (a, b) in segs {
                painter.line_segment([map(a), map(b)], egui::Stroke::new(1.2, col));
            }
        }
    }
    ui.add_space(2.0);
    ui.small("Barbs point the way the wind is coming from; color is speed.");
}

/// Blue (light) through green and yellow to red (damaging) — the same intuition as radar color.
fn speed_color(kt: f32) -> egui::Color32 {
    let stops = [
        (0.0, [90, 150, 240]),
        (25.0, [60, 200, 120]),
        (45.0, [240, 220, 70]),
        (70.0, [235, 100, 50]),
        (95.0, [235, 60, 200]),
    ];
    let mut rgb = stops[0].1;
    for w in stops.windows(2) {
        let (a, ca) = w[0];
        let (b, cb) = w[1];
        if kt >= a && kt <= b {
            let u = (kt - a) / (b - a);
            rgb = [
                (ca[0] as f32 + (cb[0] as f32 - ca[0] as f32) * u) as u8,
                (ca[1] as f32 + (cb[1] as f32 - ca[1] as f32) * u) as u8,
                (ca[2] as f32 + (cb[2] as f32 - ca[2] as f32) * u) as u8,
            ];
        } else if kt > b {
            rgb = cb;
        }
    }
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

fn plot(ui: &mut egui::Ui, levels: &[VwpLevel]) {
    let side = ui.available_width().clamp(160.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = side / 2.0 - 8.0;

    // Peak wind sets the ring scale (rounded up to 10 kt).
    let peak = levels
        .iter()
        .map(|l| (l.u_ms * l.u_ms + l.v_ms * l.v_ms).sqrt() * MS_TO_KT)
        .fold(10.0_f32, f32::max);
    let max_kt = (peak / 10.0).ceil() * 10.0;
    let grid = ui.visuals().weak_text_color().gamma_multiply(0.5);

    // Range rings + labels every 10 kt.
    let mut r = 10.0;
    while r <= max_kt {
        let rr = radius * r / max_kt;
        painter.circle_stroke(center, rr, egui::Stroke::new(1.0, grid));
        painter.text(
            center + egui::vec2(2.0, -rr),
            egui::Align2::LEFT_BOTTOM,
            format!("{r:.0}"),
            egui::FontId::proportional(9.0),
            grid,
        );
        r += 10.0;
    }
    // N/S/E/W spokes.
    painter.line_segment(
        [
            center - egui::vec2(radius, 0.0),
            center + egui::vec2(radius, 0.0),
        ],
        egui::Stroke::new(1.0, grid),
    );
    painter.line_segment(
        [
            center - egui::vec2(0.0, radius),
            center + egui::vec2(0.0, radius),
        ],
        egui::Stroke::new(1.0, grid),
    );
    for (label, off) in [
        ("N", egui::vec2(0.0, -radius)),
        ("S", egui::vec2(0.0, radius)),
        ("E", egui::vec2(radius, 0.0)),
        ("W", egui::vec2(-radius, 0.0)),
    ] {
        painter.text(
            center + off,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.0),
            grid,
        );
    }

    // Wind curve: (u, v) kt -> screen (east right, north up). Color low->high by altitude.
    let alt_lo = levels.first().map(|l| l.alt_kft).unwrap_or(0.0);
    let alt_hi = levels.last().map(|l| l.alt_kft).unwrap_or(1.0);
    let span = (alt_hi - alt_lo).max(0.1);
    let pt = |l: &VwpLevel| {
        let uk = l.u_ms * MS_TO_KT;
        let vk = l.v_ms * MS_TO_KT;
        center + egui::vec2(radius * uk / max_kt, -radius * vk / max_kt)
    };
    let mut prev: Option<egui::Pos2> = None;
    for l in levels {
        let p = pt(l);
        let f = ((l.alt_kft - alt_lo) / span).clamp(0.0, 1.0);
        // Blue (low) -> red (high).
        let col = egui::Color32::from_rgb((60.0 + 195.0 * f) as u8, 90, (255.0 - 195.0 * f) as u8);
        if let Some(pp) = prev {
            painter.line_segment([pp, p], egui::Stroke::new(2.0, col));
        }
        painter.circle_filled(p, 3.0, col);
        prev = Some(p);
    }
}

fn legend(ui: &mut egui::Ui, levels: &[VwpLevel]) {
    let sfc = levels.first();
    let top = levels.last();
    if let (Some(s), Some(t)) = (sfc, top) {
        ui.weak(format!(
            "Surface {:.1} kft: {:.0}° {:.0} kt   ·   Top {:.1} kft: {:.0}° {:.0} kt",
            s.alt_kft, s.dir_deg, s.speed_kt, t.alt_kft, t.dir_deg, t.speed_kt
        ));
    }
    ui.weak("Blue = low altitude, red = high.");
}
