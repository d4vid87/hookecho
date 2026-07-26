//! Drawing the surface analysis: frontal polylines with their pips, and H/L pressure centers.
//!
//! The pips are the point of this layer — a colored line is ambiguous in a screenshot and useless
//! to anyone colorblind, whereas triangles-versus-semicircles is the notation everyone learned
//! from a TV weather map. They're emitted into one `egui::Mesh` by walking the polyline in screen
//! space and dropping a glyph every fixed number of pixels, which keeps spacing even regardless of
//! zoom or how the projection stretches a segment.

use egui::{Align2, Color32, FontId, Mesh, Painter, Pos2, Shape, Stroke, Vec2};
use wxdata::fronts::{Front, FrontKind, SurfaceAnalysis};

/// Screen-space spacing between pips.
const PIP_EVERY_PX: f32 = 34.0;
const PIP_SIZE: f32 = 6.0;

/// Draw the analysis. `to_screen` projects `(lon, lat)`; `clip` culls off-screen work.
pub fn draw(
    painter: &Painter,
    analysis: &SurfaceAnalysis,
    clip: egui::Rect,
    to_screen: impl Fn(f64, f64) -> Pos2,
) {
    for f in &analysis.fronts {
        let pts: Vec<Pos2> = f.pts.iter().map(|&(lo, la)| to_screen(lo, la)).collect();
        if pts.len() < 2 {
            continue;
        }
        // Cheap reject: skip a boundary entirely off-pane.
        let bb = pts.iter().fold(egui::Rect::NOTHING, |r, p| {
            r.union(egui::Rect::from_center_size(*p, Vec2::splat(1.0)))
        });
        if !bb.expand(PIP_SIZE * 2.0).intersects(clip) {
            continue;
        }
        let c = f.kind.color();
        let col = Color32::from_rgb(c[0], c[1], c[2]);
        match f.kind {
            // Troughs are dashed and pipless, which is how they're distinguished from fronts.
            FrontKind::Trough => {
                for w in pts.windows(2) {
                    painter.add(Shape::dashed_line(
                        &[w[0], w[1]],
                        Stroke::new(1.6, col),
                        6.0,
                        4.0,
                    ));
                }
            }
            _ => {
                painter.add(Shape::line(pts.clone(), Stroke::new(2.0, col)));
                pips(painter, &pts, f, col);
            }
        }
    }

    // Pressure centers: the big blue H and red L everyone reads first.
    for c in &analysis.centers {
        let p = to_screen(c.lon, c.lat);
        if !clip.expand(20.0).contains(p) {
            continue;
        }
        let (letter, col) = if c.high {
            ("H", Color32::from_rgb(80, 140, 245))
        } else {
            ("L", Color32::from_rgb(235, 70, 70))
        };
        // Halo first so the letter survives a bright basemap.
        for d in [
            Vec2::new(-1.5, 0.0),
            Vec2::new(1.5, 0.0),
            Vec2::new(0.0, -1.5),
            Vec2::new(0.0, 1.5),
        ] {
            painter.text(
                p + d,
                Align2::CENTER_CENTER,
                letter,
                FontId::proportional(20.0),
                Color32::from_black_alpha(210),
            );
        }
        painter.text(
            p,
            Align2::CENTER_CENTER,
            letter,
            FontId::proportional(20.0),
            col,
        );
        painter.text(
            p + Vec2::new(0.0, 13.0),
            Align2::CENTER_CENTER,
            format!("{}", c.mb),
            FontId::proportional(10.0),
            Color32::from_gray(235),
        );
    }
}

/// Walk the polyline dropping glyphs at even screen-space intervals.
fn pips(painter: &Painter, pts: &[Pos2], f: &Front, col: Color32) {
    let mut mesh = Mesh::default();
    // Half a step in, so even a short front gets one glyph.
    let mut carry = PIP_EVERY_PX * 0.5;
    // Stationary fronts alternate warm and cold notation on opposite sides; occluded fronts
    // alternate the two shapes on the same side.
    let mut alternate = false;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = b - a;
        let len = seg.length();
        if len < 1e-3 {
            continue;
        }
        let dir = seg / len;
        // Left-hand normal in screen space (y grows downward).
        let nrm = Vec2::new(dir.y, -dir.x);
        let mut d = carry;
        while d < len {
            let at = a + dir * d;
            let (kind, side) = match f.kind {
                FrontKind::Cold => (FrontKind::Cold, 1.0),
                FrontKind::Warm => (FrontKind::Warm, 1.0),
                FrontKind::Occluded => (
                    if alternate {
                        FrontKind::Warm
                    } else {
                        FrontKind::Cold
                    },
                    1.0,
                ),
                FrontKind::Stationary => (
                    if alternate {
                        FrontKind::Warm
                    } else {
                        FrontKind::Cold
                    },
                    if alternate { -1.0 } else { 1.0 },
                ),
                FrontKind::Trough => break,
            };
            let n = nrm * side;
            match kind {
                FrontKind::Warm => semicircle(&mut mesh, at, dir, n, col),
                _ => triangle(&mut mesh, at, dir, n, col),
            }
            alternate = !alternate;
            d += PIP_EVERY_PX;
        }
        carry = d - len;
    }
    if !mesh.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}

/// A cold-front barb: a triangle standing on the line.
fn triangle(mesh: &mut Mesh, at: Pos2, dir: Vec2, nrm: Vec2, col: Color32) {
    let half = PIP_SIZE * 0.7;
    let i = mesh.vertices.len() as u32;
    for p in [at - dir * half, at + dir * half, at + nrm * PIP_SIZE * 1.3] {
        mesh.colored_vertex(p, col);
    }
    mesh.add_triangle(i, i + 1, i + 2);
}

/// A warm-front bump: a semicircle fanned out of triangles.
fn semicircle(mesh: &mut Mesh, at: Pos2, dir: Vec2, nrm: Vec2, col: Color32) {
    const STEPS: usize = 7;
    let r = PIP_SIZE * 0.95;
    let center = at;
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(center, col);
    for k in 0..=STEPS {
        let t = std::f32::consts::PI * (k as f32 / STEPS as f32);
        // Sweep from one side of the line to the other, bulging along the normal.
        let p = center + dir * (r * t.cos()) + nrm * (r * t.sin());
        mesh.colored_vertex(p, col);
    }
    for k in 0..STEPS as u32 {
        mesh.add_triangle(base, base + 1 + k, base + 2 + k);
    }
}
