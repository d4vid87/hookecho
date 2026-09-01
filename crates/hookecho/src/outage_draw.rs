//! Cross-hatching for the county power-outage layer.
//!
//! A translucent fill is how this app draws *forecast* areas — outlooks, watches, warnings — so
//! shading a county solid made an outage read as a hazard polygon at a glance. Hatching is the
//! cartographic answer: the county is obviously marked, obviously not a warning, and the radar
//! underneath stays fully visible between the lines.
//!
//! The overlay pipeline tessellates solid fills only, so the lines are drawn here instead: a
//! scanline pass in screen space, clipping each diagonal to the polygon with the same even-odd
//! rule the fill would have used. Holes come out unhatched for free.

use egui::{Color32, Painter, Pos2, Rect, Stroke};
use wxdata::overlay::GeoFeature;

/// Screen-space gap between hatch lines. Wide enough that a small county is not a solid block,
/// tight enough to read as a texture rather than as three stray lines.
const SPACING: f32 = 11.0;

/// Draw the cross-hatch for every feature. `to_screen` projects `(lon, lat)`; `clip` is the pane.
pub fn draw(
    painter: &Painter,
    features: &[GeoFeature],
    clip: Rect,
    to_screen: impl Fn(f64, f64) -> Pos2,
) {
    let painter = painter.with_clip_rect(clip);
    for f in features {
        let rings: Vec<Vec<Pos2>> = f
            .rings
            .iter()
            .filter(|r| r.len() >= 3)
            .map(|r| r.iter().map(|p| to_screen(p[0], p[1])).collect())
            .collect();
        if rings.is_empty() {
            continue;
        }
        let bb = rings
            .iter()
            .flatten()
            .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p)));
        if !bb.intersects(clip) {
            continue;
        }
        // Hatch only what is on screen: a county the size of the pane would otherwise generate
        // scanlines across its whole extent, nearly all of them off-view.
        let area = bb.intersect(clip);
        let color = Color32::from_rgba_unmultiplied(f.stroke[0], f.stroke[1], f.stroke[2], 150);
        let stroke = Stroke::new(1.2, color);
        for dir in [1.0_f32, -1.0] {
            hatch(&painter, &rings, area, dir, stroke);
        }
    }
}

/// One family of parallel lines `x + dir*y = c`, clipped to the polygon by even-odd crossings.
fn hatch(painter: &Painter, rings: &[Vec<Pos2>], area: Rect, dir: f32, stroke: Stroke) {
    let key = |p: Pos2| p.x + dir * p.y;
    // The range of `c` that can touch `area`: its four corners bound it.
    let corners = [
        area.left_top(),
        area.right_top(),
        area.left_bottom(),
        area.right_bottom(),
    ];
    let (lo, hi) = corners.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
        (lo.min(key(*p)), hi.max(key(*p)))
    });
    // Anchor the lines to the world origin rather than to `lo`, so panning slides the map under
    // a stable hatch instead of making it shimmer.
    let mut c = (lo / SPACING).ceil() * SPACING;
    let mut xs: Vec<Pos2> = Vec::new();
    while c <= hi {
        xs.clear();
        for ring in rings {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                let (ka, kb) = (key(a), key(b));
                // Half-open crossing test: a vertex exactly on the line counts once, not twice.
                if (ka > c) == (kb > c) {
                    continue;
                }
                let t = (c - ka) / (kb - ka);
                xs.push(a + (b - a) * t);
            }
        }
        // Along the line, `x` increases monotonically for both directions, so it orders the
        // crossings without another projection.
        xs.sort_by(|p, q| p.x.total_cmp(&q.x));
        for pair in xs.as_chunks::<2>().0 {
            painter.line_segment([pair[0], pair[1]], stroke);
        }
        c += SPACING;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crossings of a square by one diagonal: two, and the segment between them is inside.
    #[test]
    fn a_diagonal_crosses_a_square_twice() {
        let sq = vec![vec![
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
            Pos2::new(0.0, 10.0),
        ]];
        let key = |p: Pos2| p.x + p.y;
        let c = 10.0_f32;
        let mut hits = 0;
        for ring in &sq {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                if (key(a) > c) != (key(b) > c) {
                    hits += 1;
                }
            }
        }
        assert_eq!(hits, 2);
    }
}
