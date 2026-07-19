//! Vertical cross-sections (RHI-style) sampled from a volume's stacked tilts.
//!
//! Given all elevation tilts of one moment as [`BinnedSweep`]s and a ground line A→B, this
//! reconstructs a distance×height panel: for each column (a point along the line) it samples every
//! tilt's beam that passes overhead — using the 4/3-earth beam-height model — then fills the
//! height axis by interpolating between the bracketing tilt beams.

use crate::level2::BinnedSweep;

const R_EARTH_KM: f64 = 6371.0;
/// Standard-atmosphere effective earth radius (4/3 earth) for beam propagation.
const R_EFF_KM: f64 = R_EARTH_KM * 4.0 / 3.0;

/// A reconstructed vertical cross-section. `dbz` is row-major `rows × cols`; row 0 is the top of
/// the panel (highest altitude), column 0 is endpoint A. `None` = no beam coverage.
pub struct CrossSection {
    pub cols: usize,
    pub rows: usize,
    pub max_height_km: f32,
    pub length_km: f64,
    pub dbz: Vec<Option<f32>>,
}

impl CrossSection {
    pub fn at(&self, col: usize, row: usize) -> Option<f32> {
        self.dbz.get(row * self.cols + col).copied().flatten()
    }
}

/// 4/3-earth beam height (km) at slant range `slant_km` and elevation `elev_deg`.
pub(crate) fn beam_height_km(slant_km: f64, elev_deg: f64) -> f64 {
    let e = elev_deg.to_radians();
    (slant_km * slant_km + R_EFF_KM * R_EFF_KM + 2.0 * slant_km * R_EFF_KM * e.sin()).sqrt() - R_EFF_KM
}

/// Great-circle distance (km) and initial bearing (deg from north) from `(lon0,lat0)` to `(lon,lat)`.
pub(crate) fn dist_bearing(lon0: f64, lat0: f64, lon: f64, lat: f64) -> (f64, f64) {
    let (p0, p1) = (lat0.to_radians(), lat.to_radians());
    let dl = (lon - lon0).to_radians();
    let a = (p1.sin() * p0.sin()) + (p1.cos() * p0.cos() * dl.cos());
    let dist = R_EARTH_KM * a.clamp(-1.0, 1.0).acos();
    let y = dl.sin() * p1.cos();
    let x = p0.cos() * p1.sin() - p0.sin() * p1.cos() * dl.cos();
    let brg = y.atan2(x).to_degrees().rem_euclid(360.0);
    (dist, brg)
}

/// Build a cross-section along the ground line A→B (both `(lon, lat)`), sampling all `sweeps`.
/// Returns `None` if there are no sweeps.
pub fn build(
    sweeps: &[BinnedSweep],
    a: (f64, f64),
    b: (f64, f64),
    cols: usize,
    rows: usize,
    max_height_km: f32,
) -> Option<CrossSection> {
    let s0 = sweeps.first()?;
    let (rlon, rlat) = (s0.radar_lon as f64, s0.radar_lat as f64);
    let cols = cols.max(2);
    let rows = rows.max(2);
    let length_km = dist_bearing(a.0, a.1, b.0, b.1).0;
    let mut dbz = vec![None; cols * rows];

    for i in 0..cols {
        let t = i as f64 / (cols - 1) as f64;
        let plon = a.0 + (b.0 - a.0) * t;
        let plat = a.1 + (b.1 - a.1) * t;
        let (ground_km, az) = dist_bearing(rlon, rlat, plon, plat);

        // One sample per tilt whose beam reaches this ground range and has data at (az, range).
        let mut samples: Vec<(f64, f32)> = Vec::with_capacity(sweeps.len());
        for s in sweeps {
            let e = s.elevation_deg as f64;
            // ponytail: flat-fan slant approximation (ground/cos elev); exact inversion of the
            // 4/3-earth ground-range formula only matters past ~200 km — fine for interrogation.
            let slant = ground_km / e.to_radians().cos();
            let gate = ((slant - s.first_gate_km as f64) / s.gate_interval_km.max(f32::EPSILON) as f64).round();
            if gate < 0.0 || gate as usize >= s.gate_count {
                continue;
            }
            let bin = ((az / 360.0 * s.az_bins as f64) as usize) % s.az_bins;
            let idx = s.data[bin * s.gate_count + gate as usize];
            if idx < 2 {
                continue; // 0/1 = no data / below threshold
            }
            let h = beam_height_km(slant, e);
            let v = s.value_min + (idx as f32 - 2.0) / 253.0 * (s.value_max - s.value_min);
            samples.push((h, v));
        }
        samples.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));

        for r in 0..rows {
            let hr = max_height_km as f64 * (1.0 - r as f64 / (rows - 1) as f64); // row 0 = top
            dbz[r * cols + i] = sample_profile(&samples, hr);
        }
    }

    Some(CrossSection { cols, rows, max_height_km, length_km, dbz })
}

/// Interpolate the vertical profile at height `hr` (km): linear between the two bracketing tilt
/// beams when the gap is reasonable (< 4 km), nearest within 1.5 km at the panel edges, else None.
pub(crate) fn sample_profile(samples: &[(f64, f32)], hr: f64) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    // Below the lowest / above the highest beam: use the nearest if it's close.
    if hr <= samples[0].0 {
        return (samples[0].0 - hr < 1.5).then_some(samples[0].1);
    }
    if hr >= samples[samples.len() - 1].0 {
        let last = samples[samples.len() - 1];
        return (hr - last.0 < 1.5).then_some(last.1);
    }
    // Between two beams: linear interpolate if the vertical gap isn't a huge void.
    for w in samples.windows(2) {
        let (h0, v0) = w[0];
        let (h1, v1) = w[1];
        if hr >= h0 && hr <= h1 {
            if h1 - h0 > 4.0 {
                return None;
            }
            let k = ((hr - h0) / (h1 - h0)) as f32;
            return Some(v0 + (v1 - v0) * k);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beam_rises_with_range_and_elevation() {
        // Higher elevation and longer range both lift the beam.
        assert!(beam_height_km(100.0, 0.5) > beam_height_km(50.0, 0.5));
        assert!(beam_height_km(100.0, 4.0) > beam_height_km(100.0, 0.5));
        // Near the radar at low tilt the beam is near the surface.
        assert!(beam_height_km(10.0, 0.5) < 0.5);
    }

    #[test]
    fn profile_interpolates_between_beams() {
        let s = vec![(1.0, 20.0f32), (3.0, 40.0)];
        assert_eq!(sample_profile(&s, 2.0), Some(30.0)); // midpoint
        assert_eq!(sample_profile(&s, 1.0), Some(20.0));
        assert_eq!(sample_profile(&s, 10.0), None); // far above top beam
    }
}
