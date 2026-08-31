//! Drawing the NHC tropical picture: forecast cone outline, track, and per-point callouts.
//!
//! The cone arrives as a server polygon and is filled by the GPU overlay pipeline like any
//! other `GeoFeature`. Its *outline* is drawn here instead, because a dashed edge is what says
//! "this boundary is a probability, not a place" — and the overlay tessellator only does solid
//! strokes.
//!
//! Everything else is the storm itself: a dashed centerline through the forecast positions, a
//! ringed dot per position in its Saffir–Simpson color, a cyclone glyph at the current
//! position, and a small callout box per point carrying the valid time, the wind, and (at the
//! current position) the central pressure.

use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use wxdata::tropical::{saffir_simpson, TropicalData, TropicalStorm};

/// Below this zoom the map is a whole-basin view: dots and glyphs only, or the boxes cover the
/// ocean they are describing.
const CALLOUT_ZOOM: f32 = 3.5;

/// Callout background. Dark and near-opaque so it reads over radar, ocean, and light basemaps
/// alike — the same weight as the cell-ETA boxes.
const BOX_BG: Color32 = Color32::from_rgba_premultiplied(16, 19, 26, 190);

/// Draw the tropical suite. `to_screen` projects `(lon, lat)`; `clip` is the pane rect.
pub fn draw(
    painter: &Painter,
    data: &TropicalData,
    clip: Rect,
    zoom: f32,
    to_screen: impl Fn(f64, f64) -> Pos2,
) {
    for cone in &data.cones {
        draw_cone(painter, &cone.rings, clip, &to_screen);
    }
    // ponytail: one occupancy list for this layer only. Not `labelplace::Placer` — that
    // asserts a global non-decreasing priority order across layers, and tropical paints after
    // the place labels, so joining it would mean reordering unrelated layers for a handful of
    // boxes. Storms are few; greedy overlap rejection is enough.
    let mut taken: Vec<Rect> = Vec::new();
    for storm in &data.storms {
        draw_storm(painter, storm, clip, zoom, &to_screen, &mut taken);
    }
}

/// The cone edge: dashed, white, soft. Holes are drawn too — a ring is a ring.
fn draw_cone(
    painter: &Painter,
    rings: &[Vec<[f64; 2]>],
    clip: Rect,
    to_screen: &impl Fn(f64, f64) -> Pos2,
) {
    for ring in rings {
        if ring.len() < 3 {
            continue;
        }
        let mut pts: Vec<Pos2> = ring.iter().map(|p| to_screen(p[0], p[1])).collect();
        // A cone larger than the pane has no vertex on screen and still covers everything, so
        // the containment test is what keeps it visible when zoomed in.
        if !pts.iter().any(|p| clip.contains(*p)) && !contains(&pts, clip.center()) {
            continue;
        }
        pts.push(pts[0]);
        painter.add(Shape::dashed_line(
            &pts,
            Stroke::new(1.6, Color32::from_white_alpha(205)),
            7.0,
            5.0,
        ));
    }
}

/// Even-odd point-in-polygon on the projected ring.
fn contains(pts: &[Pos2], p: Pos2) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[j]);
        if (a.y > p.y) != (b.y > p.y)
            && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y + f32::EPSILON) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn draw_storm(
    painter: &Painter,
    storm: &TropicalStorm,
    clip: Rect,
    zoom: f32,
    to_screen: &impl Fn(f64, f64) -> Pos2,
    taken: &mut Vec<Rect>,
) {
    let pts: Vec<Pos2> = storm
        .points
        .iter()
        .map(|p| to_screen(p.lon, p.lat))
        .collect();

    // Centerline: a shadow copy under a dashed white line. There is no drop-shadow primitive
    // for map painting here; an offset dark copy is the codebase's idiom.
    if pts.len() >= 2 {
        let bb = pts.iter().fold(Rect::NOTHING, |r, p| {
            r.union(Rect::from_center_size(*p, Vec2::splat(1.0)))
        });
        if bb.expand(24.0).intersects(clip) {
            let shadow: Vec<Pos2> = pts.iter().map(|p| *p + Vec2::splat(1.5)).collect();
            painter.add(Shape::dashed_line(
                &shadow,
                Stroke::new(3.0, Color32::from_black_alpha(90)),
                9.0,
                6.0,
            ));
            painter.add(Shape::dashed_line(
                &pts,
                Stroke::new(2.5, Color32::WHITE),
                9.0,
                6.0,
            ));
        }
    }

    // Current position: the cyclone symbol, not another dot. Drawn (and its callout reserved)
    // before the forecast points, because point 0 sits on top of it and the box that says how
    // strong the storm is *now* is the one that must survive.
    let cp = to_screen(storm.lon, storm.lat);
    if clip.expand(20.0).contains(cp) {
        let (cat, rgb) = saffir_simpson(storm.intensity_kt);
        let col = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let head = format!("{} · {}", storm.name, cat);
        let mut line2 = format!("{:.0} kt", storm.intensity_kt);
        if let Some(mb) = storm.pressure_mb {
            line2 = format!("{line2} | {mb:.0} mb");
        }
        // Same outside-the-bend rule as the forecast points: the track leaves the current
        // position, so the box goes on the side the storm is not heading.
        let dir = pts
            .iter()
            .find(|p| p.distance(cp) > 6.0)
            .map(|p| *p - cp)
            .unwrap_or(Vec2::new(0.0, -1.0));
        callout(
            painter,
            clip,
            cp,
            dir.y >= 0.0,
            dir.x >= 0.0,
            &[&head, &line2],
            col,
            taken,
        );
        cyclone(painter, cp, (5.0 + zoom).clamp(9.0, 15.0), col);
    }

    for (i, p) in storm.points.iter().enumerate() {
        let sp = pts[i];
        if !clip.expand(8.0).contains(sp) {
            continue;
        }
        // Hour 0 is the current position; its dot would sit on top of the cyclone glyph.
        if sp.distance(cp) < 6.0 {
            continue;
        }
        let (_, rgb) = saffir_simpson(p.kt);
        let col = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        painter.circle_filled(sp, 5.5, Color32::from_black_alpha(120));
        painter.circle_filled(sp, 4.5, col);
        painter.circle_stroke(sp, 4.5, Stroke::new(1.5, Color32::WHITE));

        if zoom >= CALLOUT_ZOOM && !p.label.is_empty() {
            let wind = format!("{:.0} kt", p.kt);
            // Put the box on the outside of the bend: away from where the track goes next, so a
            // curving forecast does not have its own labels sitting on the line.
            let next = pts.get(i + 1).copied().unwrap_or(sp);
            let dir = next - sp;
            callout(
                painter,
                clip,
                sp,
                dir.y >= 0.0,
                dir.x >= 0.0,
                &[&p.label, &wind],
                col,
                taken,
            );
        }
    }
}

fn cyclone(painter: &Painter, c: Pos2, r: f32, col: Color32) {
    for (stroke, off) in [
        (Stroke::new(r * 0.26, Color32::from_black_alpha(160)), 1.0),
        (Stroke::new(r * 0.18, col), 0.0),
    ] {
        for phase in [0.0_f32, std::f32::consts::PI] {
            let arm: Vec<Pos2> = (0..16)
                .map(|k| {
                    let t = k as f32 / 15.0;
                    // Both arms turn the same way, half a turn apart: that is what makes the
                    // symbol an S around the eye rather than a bowl under it.
                    let ang = phase + t * 2.4;
                    let rad = r * (0.28 + 0.72 * t);
                    c + Vec2::new(ang.cos() * rad + off, ang.sin() * rad + off)
                })
                .collect();
            painter.add(Shape::line(arm, stroke));
        }
    }
    painter.circle_filled(c, r * 0.16, Color32::from_black_alpha(160));
    painter.circle_filled(c, r * 0.11, Color32::WHITE);
}

/// A rounded translucent box with a leader line back to `anchor`. Skipped (and `false`
/// returned) when it would overlap a box this layer already drew.
#[allow(clippy::too_many_arguments)]
fn callout(
    painter: &Painter,
    clip: Rect,
    anchor: Pos2,
    up: bool,
    left: bool,
    lines: &[&str],
    accent: Color32,
    taken: &mut Vec<Rect>,
) -> bool {
    let font = FontId::proportional(11.0);
    let galleys: Vec<_> = lines
        .iter()
        .map(|t| painter.layout_no_wrap((*t).to_string(), font.clone(), Color32::WHITE))
        .collect();
    let w = galleys.iter().fold(0.0_f32, |m, g| m.max(g.size().x));
    let h = galleys.iter().map(|g| g.size().y).sum::<f32>();
    let size = Vec2::new(w + 12.0, h + 8.0);

    let lead = 20.0;
    let dx = size.x * 0.5 + 8.0;
    let center = anchor + Vec2::new(if left { -dx } else { dx }, if up { -lead } else { lead });
    let rect = Rect::from_center_size(center, size);
    if !clip.contains_rect(rect) || taken.iter().any(|r| r.expand(2.0).intersects(rect)) {
        return false;
    }
    taken.push(rect);

    let tie = if left { rect.right() } else { rect.left() };
    painter.line_segment(
        [anchor, Pos2::new(tie, center.y)],
        Stroke::new(1.0, Color32::from_white_alpha(120)),
    );
    painter.rect_filled(rect, 4.0, BOX_BG);
    // One accent edge, on the side the anchor is, ties the box to its point's category color.
    let edge = if left {
        Rect::from_min_size(
            rect.right_top() - Vec2::new(2.0, 0.0),
            Vec2::new(2.0, rect.height()),
        )
    } else {
        Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height()))
    };
    painter.rect_filled(edge, 2.0, accent);
    let mut y = rect.top() + 4.0;
    for g in galleys {
        let gh = g.size().y;
        painter.galley(Pos2::new(rect.left() + 8.0, y), g, Color32::WHITE);
        y += gh;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_even_odd_on_a_square() {
        let sq = [
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
            Pos2::new(0.0, 10.0),
        ];
        assert!(contains(&sq, Pos2::new(5.0, 5.0)));
        assert!(!contains(&sq, Pos2::new(15.0, 5.0)));
        assert!(!contains(&sq, Pos2::new(5.0, -1.0)));
    }
}
