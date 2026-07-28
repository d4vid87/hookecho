//! Color legend bar, painted over the top-left of the map.
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
const PAD_TOP: f32 = 20.0;
const PAD_BOTTOM: f32 = 20.0;
/// Full height of the moment legend card — `app.rs` stacks field scales under it by this much.
pub const PANEL_H: f32 = BAR_H + PAD_TOP + PAD_BOTTOM;

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

/// Paint the legend into the top-left of `map_rect`. `threshold` dims the sub-threshold part.
///
/// Tick values are shown in display units (internal value × `disp_factor`, labeled
/// `disp_label`); the color domain and threshold stay in internal units.
pub fn draw(
    painter: &egui::Painter,
    map_rect: Rect,
    moment: Moment,
    table: &ColorTable,
    threshold: Option<f32>,
    disp_factor: f32,
    disp_label: &str,
) {
    let (vmin, vmax) = moment.value_range();
    let span = (vmax - vmin).max(f32::EPSILON);
    let panel = Rect::from_min_size(
        map_rect.left_top() + Vec2::new(INSET, INSET),
        Vec2::new(BAR_W + PAD_X * 2.0, PANEL_H),
    );
    let bar = Rect::from_min_size(
        panel.min + Vec2::new(PAD_X, PAD_TOP),
        Vec2::new(BAR_W, BAR_H),
    );
    card(painter, panel);

    let x_of = |value: f32| bar.left() + ((value - vmin) / span).clamp(0.0, 1.0) * bar.width();
    let col = |c: [u8; 4]| Color32::from_rgb(c[0], c[1], c[2]);

    // One gradient (or flat) quad per stop segment.
    let mut mesh = Mesh::default();
    let mut quad = |x0: f32, x1: f32, c0: Color32, c1: Color32| {
        if x1 <= x0 {
            return;
        }
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x0, bar.top()), c0);
        mesh.colored_vertex(egui::pos2(x1, bar.top()), c1);
        mesh.colored_vertex(egui::pos2(x1, bar.bottom()), c1);
        mesh.colored_vertex(egui::pos2(x0, bar.bottom()), c0);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    for (i, s) in table.stops.iter().enumerate() {
        let x0 = x_of(s.value);
        match table.stops.get(i + 1) {
            Some(n) => {
                let x1 = x_of(n.value);
                if s.solid {
                    quad(x0, x1, col(s.rgba), col(s.rgba));
                } else {
                    quad(x0, x1, col(s.rgba), col(s.end.unwrap_or(n.rgba)));
                }
            }
            None => quad(
                x0,
                bar.right(),
                col(s.end.unwrap_or(s.rgba)),
                col(s.end.unwrap_or(s.rgba)),
            ),
        }
    }
    painter.add(Shape::mesh(mesh));

    // Dim the sub-threshold span.
    if let Some(t) = threshold {
        let xr = x_of(t);
        if xr > bar.left() {
            painter.rect_filled(
                Rect::from_min_max(bar.left_top(), egui::pos2(xr, bar.bottom())),
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

    // Ticks: from Step multiples when the table declares one, else min/mid/max.
    let font = FontId::proportional(11.0);
    let label = |v: f32, align: Align2, x: f32| {
        let shown = v * disp_factor;
        painter.text(
            egui::pos2(x, bar.bottom() + 2.0),
            align,
            format!("{shown:.0}"),
            font.clone(),
            Color32::WHITE,
        );
    };
    match table.step.filter(|s| *s > 0.0) {
        Some(step) => {
            // Tick every `step`, but only *label* every Nth so numbers never collide: at ~24 px per
            // label, skip enough ticks to clear that gap (a 5-dBZ step over a 220-px bar packs 25
            // ticks — labelling all of them smears "-30-25-20" into an unreadable blur).
            let tick_px = (step / span) * bar.width();
            let label_stride = (24.0 / tick_px.max(0.1)).ceil().max(1.0) as i32;
            let first = (vmin / step).ceil() * step;
            let mut v = first;
            let mut n = 0;
            while v <= vmax && n < 128 {
                let x = x_of(v);
                painter.line_segment(
                    [
                        egui::pos2(x, bar.bottom()),
                        egui::pos2(x, bar.bottom() + 3.0),
                    ],
                    Stroke::new(1.0, Color32::from_gray(160)),
                );
                // Always label the ends; thin the interior to the stride.
                let is_end = v <= vmin + 0.01 || v >= vmax - 0.01;
                if is_end || n % label_stride == 0 {
                    let align = if v <= vmin + 0.01 {
                        Align2::LEFT_TOP
                    } else if v >= vmax - 0.01 {
                        Align2::RIGHT_TOP
                    } else {
                        Align2::CENTER_TOP
                    };
                    label(v, align, x);
                }
                v += step;
                n += 1;
            }
        }
        None => {
            label(vmin, Align2::LEFT_TOP, bar.left());
            label((vmin + vmax) * 0.5, Align2::CENTER_TOP, bar.center().x);
            label(vmax, Align2::RIGHT_TOP, bar.right());
        }
    }
    painter.text(
        panel.left_top() + Vec2::new(PAD_X, 4.0),
        Align2::LEFT_TOP,
        format!("{} ({})", moment.short_name(), disp_label),
        font,
        Color32::WHITE,
    );
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
    use crate::render::field_ramps::{ramp_for, FieldScale};
    let Some(r) = ramp_for(layer) else {
        return 0.0;
    };
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
