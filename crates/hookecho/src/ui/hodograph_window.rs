//! VAD hodograph: the wind profile plotted as a u/v curve (kt), colored low→high by altitude,
//! over range rings. Hand-drawn on the painter — no plotting dependency.

use wxdata::level3::VwpLevel;

const MS_TO_KT: f32 = 1.943_844;

/// Show the hodograph window. Returns `false` when it should close.
pub fn show(ctx: &egui::Context, site: Option<&str>, levels: &[VwpLevel]) -> bool {
    let mut open = true;
    egui::Window::new("VAD Hodograph")
        .open(&mut open)
        .default_size([360.0, 420.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(site.unwrap_or("—"));
                ui.weak(format!("{} levels", levels.len()));
            });
            ui.separator();
            if levels.len() < 2 {
                ui.weak("Loading VAD wind profile…");
                return;
            }
            plot(ui, levels);
            ui.add_space(4.0);
            legend(ui, levels);
        });
    open
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
        painter.text(center + egui::vec2(2.0, -rr), egui::Align2::LEFT_BOTTOM,
            format!("{r:.0}"), egui::FontId::proportional(9.0), grid);
        r += 10.0;
    }
    // N/S/E/W spokes.
    painter.line_segment([center - egui::vec2(radius, 0.0), center + egui::vec2(radius, 0.0)], egui::Stroke::new(1.0, grid));
    painter.line_segment([center - egui::vec2(0.0, radius), center + egui::vec2(0.0, radius)], egui::Stroke::new(1.0, grid));
    for (label, off) in [("N", egui::vec2(0.0, -radius)), ("S", egui::vec2(0.0, radius)),
                         ("E", egui::vec2(radius, 0.0)), ("W", egui::vec2(-radius, 0.0))] {
        painter.text(center + off, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), grid);
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
