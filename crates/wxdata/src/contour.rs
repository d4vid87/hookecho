//! Isoline (contour) extraction from a regular lat/lon [`MrmsField`], for drawing HRRR model
//! fields (MSLP, 2 m temp/dewpoint, CAPE, SRH) as labeled contour lines over the radar.
//!
//! Hand-rolled marching squares: for each level (a multiple of `interval` inside the data range)
//! we walk every grid cell, emit the line segment(s) where the level crosses the cell, then stitch
//! the loose segments into polylines. `// ponytail: the `contour` crate emits closed border-tracing
//! isorings that need post-filtering; ~120 lines of marching squares is less to reason about.`

use crate::mrms::MrmsField;
use std::collections::HashMap;

/// A single contour polyline at one level, in `(lon, lat)` degrees. `bbox` is the polyline's
/// `(min_lon, min_lat, max_lon, max_lat)` so the painter can cull off-view lines without
/// projecting every point (CAPE/SRH produce thousands of small rings).
pub struct ContourLine {
    pub level: f32,
    pub pts: Vec<(f64, f64)>,
    pub bbox: (f64, f64, f64, f64),
}

/// Extract contour polylines from `f` at every multiple of `interval` inside the data's value
/// range (capped at 40 levels). Polylines shorter than 3 points are dropped. `NaN` grid corners
/// skip their cell, so masked regions leave gaps rather than spurious lines.
pub fn contour_lines(f: &MrmsField, interval: f32) -> Vec<ContourLine> {
    if interval <= 0.0 || f.nx < 2 || f.ny < 2 {
        return Vec::new();
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in &f.values {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let first = (lo / interval).ceil() as i64;
    let last = (hi / interval).floor() as i64;
    let mut out = Vec::new();
    let mut levels = 0;
    let mut k = first;
    while k <= last && levels < 40 {
        let level = k as f32 * interval;
        for line in stitch(level_segments(f, level)) {
            if line.len() >= 3 {
                let bbox = line.iter().fold(
                    (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
                    |(x0, y0, x1, y1), p| (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1)),
                );
                out.push(ContourLine {
                    level,
                    pts: line,
                    bbox,
                });
            }
        }
        levels += 1;
        k += 1;
    }
    out
}

fn lon_at(f: &MrmsField, c: usize) -> f64 {
    f.lon_west + (f.lon_east - f.lon_west) * c as f64 / (f.nx - 1) as f64
}
fn lat_at(f: &MrmsField, r: usize) -> f64 {
    f.lat_north + (f.lat_south - f.lat_north) * r as f64 / (f.ny - 1) as f64
}

/// Linear crossing point on the edge from corner `pa` (value `a`) to `pb` (value `b`) at `level`.
fn cross(a: f32, b: f32, pa: (f64, f64), pb: (f64, f64), level: f32) -> (f64, f64) {
    let t = if (b - a).abs() < f32::EPSILON {
        0.5
    } else {
        ((level - a) / (b - a)) as f64
    };
    let t = t.clamp(0.0, 1.0);
    (pa.0 + (pb.0 - pa.0) * t, pa.1 + (pb.1 - pa.1) * t)
}

/// All loose line segments where `level` crosses the grid, via the 16-case marching-squares table.
fn level_segments(f: &MrmsField, level: f32) -> Vec<[(f64, f64); 2]> {
    let mut segs = Vec::new();
    for r in 0..f.ny - 1 {
        let (lat_r, lat_r1) = (lat_at(f, r), lat_at(f, r + 1));
        for c in 0..f.nx - 1 {
            let v00 = f.values[r * f.nx + c]; // TL (north-west)
            let v10 = f.values[r * f.nx + c + 1]; // TR
            let v11 = f.values[(r + 1) * f.nx + c + 1]; // BR
            let v01 = f.values[(r + 1) * f.nx + c]; // BL
            if !(v00.is_finite() && v10.is_finite() && v11.is_finite() && v01.is_finite()) {
                continue;
            }
            let (lon_c, lon_c1) = (lon_at(f, c), lon_at(f, c + 1));
            let (tl, tr) = ((lon_c, lat_r), (lon_c1, lat_r));
            let (bl, br) = ((lon_c, lat_r1), (lon_c1, lat_r1));
            let idx = ((v00 >= level) as u8) << 3
                | ((v10 >= level) as u8) << 2
                | ((v11 >= level) as u8) << 1
                | (v01 >= level) as u8;
            // Edge crossings (computed lazily-ish; cheap enough to always evaluate the ones used).
            let et = || cross(v00, v10, tl, tr, level); // top
            let er = || cross(v10, v11, tr, br, level); // right
            let eb = || cross(v01, v11, bl, br, level); // bottom
            let el = || cross(v00, v01, tl, bl, level); // left
            match idx {
                1 | 14 => segs.push([el(), eb()]),
                2 | 13 => segs.push([eb(), er()]),
                3 | 12 => segs.push([el(), er()]),
                4 | 11 => segs.push([et(), er()]),
                6 | 9 => segs.push([et(), eb()]),
                7 | 8 => segs.push([et(), el()]),
                5 => {
                    segs.push([et(), el()]);
                    segs.push([eb(), er()]);
                }
                10 => {
                    segs.push([et(), er()]);
                    segs.push([el(), eb()]);
                }
                _ => {} // 0 and 15: no crossing
            }
        }
    }
    segs
}

/// Stitch loose segments into polylines by matching shared endpoints (quantized to ~1e-4°).
fn stitch(segs: Vec<[(f64, f64); 2]>) -> Vec<Vec<(f64, f64)>> {
    let key = |p: (f64, f64)| ((p.0 * 1e4).round() as i64, (p.1 * 1e4).round() as i64);
    let mut adj: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        adj.entry(key(s[0])).or_default().push(i);
        adj.entry(key(s[1])).or_default().push(i);
    }
    // Given an endpoint key, find an unused segment touching it and return (seg index, other end).
    let next_at = |k: (i64, i64), used: &[bool], adj: &HashMap<(i64, i64), Vec<usize>>| {
        adj.get(&k).and_then(|cands| {
            cands.iter().find(|&&si| !used[si]).map(|&si| {
                let s = &segs[si];
                let other = if key(s[0]) == k { s[1] } else { s[0] };
                (si, other)
            })
        })
    };
    let mut used = vec![false; segs.len()];
    let mut lines = Vec::new();
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut line = vec![segs[start][0], segs[start][1]];
        // Grow the tail, then the head.
        while let Some((si, next)) = next_at(key(*line.last().unwrap()), &used, &adj) {
            used[si] = true;
            line.push(next);
        }
        while let Some((si, prev)) = next_at(key(*line.first().unwrap()), &used, &adj) {
            used[si] = true;
            line.insert(0, prev);
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(nx: usize, ny: usize, vals: Vec<f32>) -> MrmsField {
        MrmsField {
            values: vals,
            nx,
            ny,
            lon_west: -100.0,
            lon_east: -100.0 + (nx - 1) as f64 * 0.1,
            lat_north: 40.0,
            lat_south: 40.0 - (ny - 1) as f64 * 0.1,
            time: chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn cone_produces_rings() {
        // A radial cone peaking at the center: contours at each level are concentric rings.
        let (nx, ny) = (41usize, 41usize);
        let (cx, cy) = (20.0f32, 20.0f32);
        let mut vals = vec![0.0f32; nx * ny];
        for r in 0..ny {
            for c in 0..nx {
                let d = (((c as f32 - cx).powi(2)) + ((r as f32 - cy).powi(2))).sqrt();
                vals[r * nx + c] = 100.0 - d; // 100 at center, decreasing outward
            }
        }
        let f = field(nx, ny, vals);
        let lines = contour_lines(&f, 10.0); // levels 70, 80, 90 sit inside the cone
        assert!(!lines.is_empty(), "a cone should yield contour rings");
        // Each ring should be a closed-ish loop of many points.
        let longest = lines.iter().map(|l| l.pts.len()).max().unwrap();
        assert!(
            longest >= 8,
            "expected a substantial ring, got {longest} points"
        );
        // A higher level ring must sit closer to the center (smaller bounding box) than a lower one.
        let bbox = |lvl: f32| {
            lines
                .iter()
                .filter(|l| (l.level - lvl).abs() < 0.5)
                .flat_map(|l| l.pts.iter())
                .fold((f64::MAX, f64::MIN), |(mn, mx), p| {
                    (mn.min(p.0), mx.max(p.0))
                })
        };
        // Levels 80 and 90 both sit inside the cone (min value ≈ 72); the 90 ring is inner.
        let (lo90, hi90) = bbox(90.0);
        let (lo80, hi80) = bbox(80.0);
        assert!(
            (hi90 - lo90) < (hi80 - lo80),
            "inner ring should be tighter"
        );
    }
}
