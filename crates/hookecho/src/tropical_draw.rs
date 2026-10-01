//! Drawing the NHC tropical picture: forecast cone outline, track, and per-point callouts.
//!
//! The cone arrives as a server polygon and is filled by the GPU overlay pipeline like any
//! other `GeoFeature`. Field Atlas paints its dashed screen-space outline.
//!
//! Everything else is the storm itself: a solid centerline through the forecast positions, a
//! ringed dot per position in its Saffir–Simpson color, a cyclone glyph at the current
//! position, and a small callout box per point carrying the valid time, the wind, and (at the
//! current position) the central pressure.

use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use wxdata::tropical::{saffir_simpson, TropicalData, TropicalStorm};

/// Below this zoom the map is a whole-basin view: dots and glyphs only, or the boxes cover the
/// ocean they are describing.
const CALLOUT_ZOOM: f32 = 7.0;

/// Callout background. Dark and near-opaque so it reads over radar, ocean, and light basemaps
/// alike — the same weight as the cell-ETA boxes.
const BOX_BG: Color32 = crate::field_atlas::INK;

/// Draw the tropical suite. `to_screen` projects `(lon, lat)`; `clip` is the pane rect.
pub fn draw(
    painter: &Painter,
    data: &TropicalData,
    clip: Rect,
    zoom: f32,
    to_screen: impl Fn(f64, f64) -> Pos2,
) {
    // Field Atlas draws cone edges in the shared overlay pass. Preserve the wind/surge
    // intensity colors while giving those boundaries the same dark casing.
    for feature in data.wind_radii.iter().chain(data.surge.iter()) {
        let style = crate::field_atlas::Style {
            rgb: [feature.stroke[0], feature.stroke[1], feature.stroke[2]],
            fill: 0, dash: None, emergency: false, warning: false,
        };
        for ring in &feature.rings {
            if ring.len() < 3 { continue; }
            let mut pts: Vec<_> = ring.iter().map(|p| to_screen(p[0], p[1])).collect();
            pts.push(pts[0]);
            crate::field_atlas::boundary(painter, &pts, clip, style, 1.0);
        }
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

fn draw_storm(
    painter: &Painter,
    storm: &TropicalStorm,
    clip: Rect,
    zoom: f32,
    to_screen: &impl Fn(f64, f64) -> Pos2,
    taken: &mut Vec<Rect>,
) {
    let pts: Vec<Pos2> = storm.points.iter().map(|p| to_screen(p.lon, p.lat)).collect();

    // Solid center track; the forecast cone remains dashed.
    let track_style = crate::field_atlas::Style {
        rgb: [119, 221, 255], fill: 0, dash: None, emergency: false, warning: false,
    };
    // A forecast crossing the date line has a discontinuity in map coordinates. Never
    // connect the two sides with a line spanning the whole map.
    for (pair, geo) in pts.windows(2).zip(storm.points.windows(2)) {
        if (geo[0].lon - geo[1].lon).abs() <= 180.0 {
            crate::field_atlas::boundary(painter, pair, clip, track_style, 1.0);
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
    let font = FontId::monospace(11.0);
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
    painter.rect_filled(rect, 2.0, BOX_BG);
    painter.rect_stroke(rect, 2.0, Stroke::new(0.8, accent), egui::StrokeKind::Inside);
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
