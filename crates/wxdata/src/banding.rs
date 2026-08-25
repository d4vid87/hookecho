//! Banded precipitation: which echoes are organised into a line, and which are just area.
//!
//! A snow squall is a narrow, fast, intense band — the thing that whites out an interstate in two
//! minutes — and on a national mosaic it looks much like any other patch of moderate reflectivity
//! until you notice its shape. Shape is the whole signal, so this is a geometry pass: threshold
//! the grid, find connected echo, and keep only the components that are long and thin.
//!
//! Output is an [`MrmsField`] of the same reflectivity values with everything unbanded set to
//! `NaN`, so it draws through the existing warp/upload path with no new pipeline and reads as
//! emphasis rather than as a separate scale.

use crate::mrms::MrmsField;

/// A component has to be at least this many grid cells before its shape means anything. Below it,
/// two cells in a row look infinitely elongated.
const MIN_CELLS: usize = 40;

/// Long-axis over short-axis, from the component's second moments. Three is about where a human
/// stops calling it a blob.
const MIN_ELONGATION: f64 = 3.0;

/// Keep the parts of `field` that belong to an elongated echo, and blank the rest.
///
/// `min_dbz` is what counts as echo at all. `mask` is an optional MRMS categorical grid on the
/// same lattice (`PrecipFlag`); when given, only cells whose flag is in `keep_flags` survive,
/// which is how "bands" narrows to "snow bands". `None` when the grid is empty.
pub fn bands(
    field: &MrmsField,
    min_dbz: f32,
    mask: Option<(&MrmsField, &[i32])>,
) -> Option<MrmsField> {
    if field.nx == 0 || field.ny == 0 {
        return None;
    }
    let (nx, ny) = (field.nx, field.ny);
    let masked = |i: usize| -> bool {
        let Some((m, keep)) = mask else { return true };
        match sample(m, field, i) {
            Some(v) if v.is_finite() => keep.contains(&(v.round() as i32)),
            // No flag under the cell: the categorical grid is coarser or older than the echo, and
            // dropping the echo for that would blank the band the layer exists to show.
            _ => true,
        }
    };
    let hot: Vec<bool> = (0..nx * ny)
        .map(|i| {
            let v = field.values[i];
            v.is_finite() && v >= min_dbz && masked(i)
        })
        .collect();

    let mut values = vec![f32::NAN; nx * ny];
    let mut seen = vec![false; nx * ny];
    let mut stack: Vec<usize> = Vec::new();
    let mut comp: Vec<usize> = Vec::new();
    for start in 0..nx * ny {
        if seen[start] || !hot[start] {
            continue;
        }
        // Flood fill, iteratively: a CONUS mosaic component can be tens of thousands of cells
        // deep, which is a stack overflow if this recurses.
        comp.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = (i % nx, i / nx);
            let push = |x: usize, y: usize, stack: &mut Vec<usize>, seen: &mut Vec<bool>| {
                let j = y * nx + x;
                if hot[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(x - 1, y, &mut stack, &mut seen);
            }
            if x + 1 < nx {
                push(x + 1, y, &mut stack, &mut seen);
            }
            if y > 0 {
                push(x, y - 1, &mut stack, &mut seen);
            }
            if y + 1 < ny {
                push(x, y + 1, &mut stack, &mut seen);
            }
        }
        if comp.len() < MIN_CELLS || elongation(&comp, nx) < MIN_ELONGATION {
            continue;
        }
        for &i in &comp {
            values[i] = field.values[i];
        }
    }

    Some(MrmsField {
        values,
        nx,
        ny,
        lon_west: field.lon_west,
        lon_east: field.lon_east,
        lat_north: field.lat_north,
        lat_south: field.lat_south,
        time: field.time,
    })
}

/// The value of `mask` under cell `i` of `field`, by lattice offset. `None` off the edge.
fn sample(mask: &MrmsField, field: &MrmsField, i: usize) -> Option<f32> {
    if mask.nx == 0 || mask.ny == 0 {
        return None;
    }
    let (mdx, mdy) = (
        (mask.lon_east - mask.lon_west) / mask.nx as f64,
        (mask.lat_north - mask.lat_south) / mask.ny as f64,
    );
    let (dx, dy) = (
        (field.lon_east - field.lon_west) / field.nx as f64,
        (field.lat_north - field.lat_south) / field.ny as f64,
    );
    let (x, y) = (i % field.nx, i / field.nx);
    let lon = field.lon_west + (x as f64 + 0.5) * dx;
    let lat = field.lat_north - (y as f64 + 0.5) * dy;
    let mx = ((lon - mask.lon_west) / mdx).floor();
    let my = ((mask.lat_north - lat) / mdy).floor();
    if mx < 0.0 || my < 0.0 || mx >= mask.nx as f64 || my >= mask.ny as f64 {
        return None;
    }
    Some(mask.values[my as usize * mask.nx + mx as usize])
}

/// Long-axis over short-axis of a component, from its second moments. A perfect line is infinite;
/// a disc is 1.
fn elongation(comp: &[usize], nx: usize) -> f64 {
    let n = comp.len() as f64;
    let (mut sx, mut sy) = (0.0, 0.0);
    for &i in comp {
        sx += (i % nx) as f64;
        sy += (i / nx) as f64;
    }
    let (cx, cy) = (sx / n, sy / n);
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for &i in comp {
        let (dx, dy) = ((i % nx) as f64 - cx, (i / nx) as f64 - cy);
        xx += dx * dx;
        yy += dy * dy;
        xy += dx * dy;
    }
    let (xx, yy, xy) = (xx / n, yy / n, xy / n);
    // Eigenvalues of the 2×2 covariance matrix.
    let tr = xx + yy;
    let det = xx * yy - xy * xy;
    let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
    let (l1, l2) = (tr / 2.0 + disc, tr / 2.0 - disc);
    if l2 <= 1e-9 {
        return f64::INFINITY;
    }
    (l1 / l2).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(nx: usize, ny: usize, f: impl Fn(usize, usize) -> f32) -> MrmsField {
        let mut values = Vec::with_capacity(nx * ny);
        for y in 0..ny {
            for x in 0..nx {
                values.push(f(x, y));
            }
        }
        MrmsField {
            values,
            nx,
            ny,
            lon_west: 0.0,
            lon_east: nx as f64 * 0.01,
            lat_north: 40.0,
            lat_south: 40.0 - ny as f64 * 0.01,
            time: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_line_survives_and_a_blob_of_the_same_area_does_not() {
        // A 2×40 band and, well clear of it, a 9×9 blob.
        let f = field(60, 20, |x, y| {
            let band = y < 2 && x < 40;
            let blob = (5..14).contains(&y) && (48..57).contains(&x);
            if band || blob {
                30.0
            } else {
                f32::NAN
            }
        });
        let b = bands(&f, 20.0, None).unwrap();
        assert!(b.values[5].is_finite(), "the band is kept");
        assert!(b.values[9 * 60 + 52].is_nan(), "the blob is not a band");
    }

    #[test]
    fn echo_below_the_threshold_and_specks_are_dropped() {
        let weak = field(60, 20, |x, y| if y < 2 && x < 40 { 10.0 } else { f32::NAN });
        let b = bands(&weak, 20.0, None).unwrap();
        assert!(b.values.iter().all(|v| v.is_nan()), "10 dBZ is not echo");

        // Three cells in a row are infinitely elongated and still not a band.
        let speck = field(60, 20, |x, y| if y == 0 && x < 3 { 30.0 } else { f32::NAN });
        let b = bands(&speck, 20.0, None).unwrap();
        assert!(b.values.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn the_categorical_mask_narrows_a_band_to_the_snow_half() {
        let f = field(60, 20, |x, y| if y < 2 && x < 40 { 30.0 } else { f32::NAN });
        // Flag 3 (snow) over the western half, flag 1 (rain) over the east.
        let mask = field(60, 20, |x, _| if x < 20 { 3.0 } else { 1.0 });
        let b = bands(&f, 20.0, Some((&mask, &[3, 4]))).unwrap();
        assert!(b.values[5].is_finite(), "snow half kept");
        assert!(b.values[30].is_nan(), "rain half masked out");
    }
}
