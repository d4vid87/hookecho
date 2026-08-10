//! Color scales for what's on the map.
//!
//! The active moment's scale is a thin bar floating over the map's right edge ([`draw_vertical`]);
//! gridded field layers and the wind ramp keep their small cards over the map's top-left.
//!
//! Draws the active moment's `ColorTable` across its data range as a gradient bar (one
//! `egui::Mesh` quad per stop segment, per-vertex colors so `Color:` gradients show), with
//! ticks from the table's `Step`. It samples the SAME table as the radar LUT, so legend and
//! map never diverge.

use crate::colormap::ColorTable;
use egui::{Align2, Color32, FontId, Mesh, Rect, Shape, Stroke, Vec2};
use wxdata::level2::Moment;

const BAR_W: f32 = 220.0;
const BAR_H: f32 = 16.0;
/// Gap between the map edge and the legend card. The card is laid out from this inset outwards,
/// never from the bar inwards — deriving the panel by expanding the bar used to push its top edge
/// off the map rect entirely, so the window's rounded corner sliced the title off every screenshot.
const INSET: f32 = 12.0;
/// Card padding around the bar: title above, tick labels below.
const PAD_X: f32 = 6.0;

/// Card backing. Opaque: basemap place labels are baked into the raster tiles underneath, so the
/// only way to stop them reading through the legend is to not be translucent.
fn card(painter: &egui::Painter, panel: Rect) {
    let (r, g, b) = crate::ui::style::CARD_FILL;
    painter.rect_filled(panel, 8.0, Color32::from_rgb(r, g, b));
    painter.rect_stroke(
        panel,
        8.0,
        Stroke::new(1.0, Color32::from_white_alpha(20)),
        egui::StrokeKind::Inside,
    );
}

/// Paint the moment scale as a thin vertical bar floating over the right edge of `rect` (the map).
///
/// No card: the bar is the legend, so the map reads through everywhere else. It spans the color
/// table's own stops rather than the moment's full encodable range — reflectivity's -32..10 dBZ
/// is uncolored, and a third of the bar spent on nothing is a third of the map's edge wasted.
/// Labels ride on the bar (halo'd, so they read over any color) instead of a gutter beside it.
pub fn draw_vertical(
    painter: &egui::Painter,
    rect: Rect,
    moment: Moment,
    table: &ColorTable,
    threshold: Option<f32>,
    disp_factor: f32,
    disp_label: &str,
) {
    const W: f32 = 16.0;
    let (vmin, vmax) = match (table.stops.first(), table.stops.last()) {
        (Some(a), Some(b)) if b.value > a.value => (a.value, b.value),
        _ => moment.value_range(),
    };
    let span = (vmax - vmin).max(f32::EPSILON);
    let bar = Rect::from_min_max(
        egui::pos2(rect.right() - W - INSET, rect.top() + INSET + 32.0),
        egui::pos2(rect.right() - INSET, rect.bottom() - INSET),
    );
    let y_of = |value: f32| bar.bottom() - ((value - vmin) / span).clamp(0.0, 1.0) * bar.height();
    let col = |c: [u8; 4]| Color32::from_rgb(c[0], c[1], c[2]);

    let mut mesh = Mesh::default();
    let mut quad = |y0: f32, y1: f32, c0: Color32, c1: Color32| {
        if y0 <= y1 {
            return; // y grows downward: y0 is the lower value, so it must sit *below* y1
        }
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(bar.left(), y0), c0);
        mesh.colored_vertex(egui::pos2(bar.right(), y0), c0);
        mesh.colored_vertex(egui::pos2(bar.right(), y1), c1);
        mesh.colored_vertex(egui::pos2(bar.left(), y1), c1);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    for (i, s) in table.stops.iter().enumerate() {
        let y0 = y_of(s.value);
        match table.stops.get(i + 1) {
            Some(n) => {
                let y1 = y_of(n.value);
                if s.solid {
                    quad(y0, y1, col(s.rgba), col(s.rgba));
                } else {
                    quad(y0, y1, col(s.rgba), col(s.end.unwrap_or(n.rgba)));
                }
            }
            None => quad(
                y0,
                bar.top(),
                col(s.end.unwrap_or(s.rgba)),
                col(s.end.unwrap_or(s.rgba)),
            ),
        }
    }
    painter.add(Shape::mesh(mesh));

    if let Some(t) = threshold {
        let yr = y_of(t);
        if yr < bar.bottom() {
            painter.rect_filled(
                Rect::from_min_max(egui::pos2(bar.left(), yr), bar.right_bottom()),
                0.0,
                Color32::from_black_alpha(150),
            );
        }
    }
    painter.rect_stroke(
        bar,
        0.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );

    let font = FontId::proportional(10.0);
    // On-bar text needs its own contrast: an 8-way black halo, the same trick the map's city
    // labels use over imagery.
    // Correlation coefficient spans 0..1.05: rounded to whole numbers its whole scale reads
    // "1 1 1 … 0 0". Anything that narrow gets decimals.
    let decimals = if span * disp_factor >= 10.0 { 0 } else { 2 };
    let label = |v: f32, y: f32| {
        let text = format!("{:.*}", decimals, v * disp_factor);
        let at = egui::pos2(bar.center().x, y);
        for dx in [-1.0, 0.0, 1.0] {
            for dy in [-1.0, 0.0, 1.0] {
                if dx == 0.0 && dy == 0.0 {
                    continue;
                }
                painter.text(
                    at + Vec2::new(dx, dy),
                    Align2::CENTER_CENTER,
                    &text,
                    font.clone(),
                    Color32::BLACK,
                );
            }
        }
        painter.text(
            at,
            Align2::CENTER_CENTER,
            text,
            font.clone(),
            Color32::WHITE,
        );
    };
    match table.step.filter(|s| *s > 0.0) {
        Some(step) => {
            let tick_px = (step / span) * bar.height();
            let label_stride = (18.0 / tick_px.max(0.1)).ceil().max(1.0) as i32;
            let mut v = (vmin / step).ceil() * step;
            let mut n = 0;
            while v <= vmax && n < 128 {
                let y = y_of(v);
                painter.line_segment(
                    [egui::pos2(bar.left(), y), egui::pos2(bar.left() + 3.0, y)],
                    Stroke::new(1.0, Color32::from_black_alpha(120)),
                );
                if v <= vmin + 0.01 || v >= vmax - 0.01 || n % label_stride == 0 {
                    label(v, y);
                }
                v += step;
                n += 1;
            }
        }
        None => {
            label(vmin, bar.bottom());
            label((vmin + vmax) * 0.5, bar.center().y);
            label(vmax, bar.top());
        }
    }
    // Caption above the bar: product then units, small — the bar itself is the legend. Above,
    // because the bottom end carries the busiest tick labels.
    for (i, (text, size, color)) in [
        (moment.short_name(), 10.0, Color32::WHITE),
        (disp_label, 9.0, Color32::from_gray(200)),
    ]
    .into_iter()
    .enumerate()
    {
        let at = egui::pos2(bar.center().x, bar.top() - 32.0 + i as f32 * 11.0);
        for d in [Vec2::new(1.0, 1.0), Vec2::ZERO] {
            painter.text(
                at + d,
                Align2::CENTER_TOP,
                text,
                FontId::proportional(size),
                if d == Vec2::ZERO {
                    color
                } else {
                    Color32::BLACK
                },
            );
        }
    }
}

/// Paint a gridded field layer's scale under the moment legend (or at the top-left when the radar
/// legend is hidden). Reads [`crate::render::field_ramps`] — the same table that bakes the layer's
/// GPU LUT — so the key can't drift from the pixels.
///
/// `y_offset` is where to start vertically inside `map_rect`; returns the height consumed.
pub fn draw_field(
    painter: &egui::Painter,
    map_rect: Rect,
    layer: crate::render::FieldLayer,
    y_offset: f32,
) -> f32 {
    match crate::render::field_ramps::ramp_for(layer) {
        Some(r) => draw_ramp(painter, map_rect, r, y_offset),
        None => 0.0,
    }
}

/// The difference layer's key: the same diverging LUT the grid was colored with, drawn across
/// ±range so the reader can tell a cool cell from a warm one and read the size of the split. The
/// transparent middle is the deadband — where the two models agree and the layer draws nothing —
/// so the card shows through there exactly as the map does.
pub fn draw_diff(
    painter: &egui::Painter,
    map_rect: Rect,
    field: crate::fielddiff::DiffField,
    y_offset: f32,
) -> f32 {
    let font = FontId::proportional(10.0);
    let origin = map_rect.left_top() + Vec2::new(INSET, INSET + y_offset);
    let panel = Rect::from_min_size(origin, Vec2::new(BAR_W + PAD_X * 2.0, BAR_H + 16.0 + 14.0));
    let bar = Rect::from_min_size(panel.min + Vec2::new(PAD_X, 16.0), Vec2::new(BAR_W, BAR_H));
    card(painter, panel);

    let (range, deadband) = field.range();
    let lut = crate::fielddiff::diverging_lut(range, deadband);
    // One column per pixel of bar, sampled through the same value→index→color path as the grid.
    let cols = bar.width().round().max(1.0) as usize;
    for i in 0..cols {
        let v = ((i as f32 / (cols - 1).max(1) as f32) * 2.0 - 1.0) * range;
        let k = crate::fielddiff::diff_index(v, range) as usize * 4;
        let c = Color32::from_rgba_unmultiplied(lut[k], lut[k + 1], lut[k + 2], lut[k + 3]);
        let x = bar.left() + i as f32;
        painter.rect_filled(
            Rect::from_min_max(egui::pos2(x, bar.top()), egui::pos2(x + 1.0, bar.bottom())),
            0.0,
            c,
        );
    }
    painter.rect_stroke(
        bar,
        0.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Inside,
    );

    let (a, b) = field.pair();
    for (text, align, x) in [
        (format!("-{range:.0}"), Align2::LEFT_TOP, bar.left()),
        ("0 (\u{2248})".to_string(), Align2::CENTER_TOP, bar.center().x),
        (format!("+{range:.0}"), Align2::RIGHT_TOP, bar.right()),
    ] {
        painter.text(
            egui::pos2(x, bar.bottom() + 2.0),
            align,
            text,
            font.clone(),
            Color32::from_gray(225),
        );
    }
    painter.text(
        panel.left_top() + Vec2::new(PAD_X, 3.0),
        Align2::LEFT_TOP,
        format!("{} {a}\u{2212}{b} ({})", field.label(), field.units()),
        font,
        Color32::WHITE,
    );
    panel.height() + 6.0
}

/// The same key, for a ramp that isn't a [`crate::render::FieldLayer`] — the wind particles carry
/// their scale rather than uploading a grid, but they still owe the reader a legend.
pub fn draw_ramp(
    painter: &egui::Painter,
    map_rect: Rect,
    r: &crate::render::field_ramps::FieldRamp,
    y_offset: f32,
) -> f32 {
    use crate::render::field_ramps::FieldScale;
    let font = FontId::proportional(10.0);
    let origin = map_rect.left_top() + Vec2::new(INSET, INSET + y_offset);

    match r.scale {
        FieldScale::Ramp { lo, hi, stops, .. } => {
            let panel =
                Rect::from_min_size(origin, Vec2::new(BAR_W + PAD_X * 2.0, BAR_H + 16.0 + 14.0));
            let bar =
                Rect::from_min_size(panel.min + Vec2::new(PAD_X, 16.0), Vec2::new(BAR_W, BAR_H));
            card(painter, panel);

            // One quad per stop segment, per-vertex colors — same idiom as the moment legend.
            let mut mesh = Mesh::default();
            let col = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
            for w in stops.windows(2) {
                let (t0, c0) = w[0];
                let (t1, c1) = w[1];
                let x0 = bar.left() + t0 * bar.width();
                let x1 = bar.left() + t1 * bar.width();
                let q = Rect::from_min_max(egui::pos2(x0, bar.top()), egui::pos2(x1, bar.bottom()));
                let i = mesh.vertices.len() as u32;
                for (p, c) in [
                    (q.left_top(), col(c0)),
                    (q.right_top(), col(c1)),
                    (q.right_bottom(), col(c1)),
                    (q.left_bottom(), col(c0)),
                ] {
                    mesh.colored_vertex(p, c);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
            painter.add(Shape::mesh(mesh));
            painter.rect_stroke(
                bar,
                0.0,
                Stroke::new(1.0, Color32::from_gray(90)),
                egui::StrokeKind::Inside,
            );

            // Sub-unit thresholds (VIL 0.1, QPE 0.25) need decimals; everything else reads
            // better whole.
            let num = |v: f32| {
                if v >= 10.0 {
                    format!("{v:.0}")
                } else {
                    format!("{v:.2}")
                }
            };
            for (v, align, x) in [
                (lo, Align2::LEFT_TOP, bar.left()),
                (hi, Align2::RIGHT_TOP, bar.right()),
            ] {
                painter.text(
                    egui::pos2(x, bar.bottom() + 2.0),
                    align,
                    num(v),
                    font.clone(),
                    Color32::from_gray(225),
                );
            }
            let head = if r.units.is_empty() {
                r.label.to_string()
            } else {
                format!("{} ({})", r.label, r.units)
            };
            painter.text(
                panel.left_top() + Vec2::new(PAD_X, 3.0),
                Align2::LEFT_TOP,
                head,
                font,
                Color32::WHITE,
            );
            panel.height() + 6.0
        }
        FieldScale::Categorical(cats) => {
            // Swatch column: these codes ("HHC class 110") mean nothing without their names.
            let row_h = 13.0;
            let w = 132.0;
            let h = 16.0 + cats.len() as f32 * row_h;
            let panel = Rect::from_min_size(origin, Vec2::new(w, h));
            card(painter, panel);
            painter.text(
                panel.left_top() + Vec2::new(6.0, 2.0),
                Align2::LEFT_TOP,
                r.label,
                font.clone(),
                Color32::WHITE,
            );
            for (i, (_, rgb, name)) in cats.iter().enumerate() {
                let y = panel.top() + 16.0 + i as f32 * row_h;
                let sw = Rect::from_min_size(
                    egui::pos2(panel.left() + 6.0, y + 2.0),
                    Vec2::new(12.0, 8.0),
                );
                painter.rect_filled(sw, 2.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                painter.text(
                    egui::pos2(sw.right() + 5.0, y),
                    Align2::LEFT_TOP,
                    *name,
                    font.clone(),
                    Color32::from_gray(225),
                );
            }
            h + 6.0
        }
    }
}
