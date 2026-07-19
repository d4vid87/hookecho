//! 3D reflectivity volume: resample the stacked radar tilts onto a regular Cartesian grid
//! (radar-relative km) for GPU raymarching. Reuses the same 4/3-earth beam geometry and
//! per-column vertical interpolation as the cross-section reconstruction.

use crate::level2::BinnedSweep;
use crate::xsection::{beam_height_km, sample_profile};

/// A regular Cartesian reflectivity grid centered on the radar. `data` is the normalized
/// reflectivity index (`0` = empty, `2..=255` = dBZ over the REF range), laid out
/// `x + n*y + n*n*z` so it uploads directly as a `width=n, height=n, depth=nz` 3D texture.
/// x = east, y = north, z = up.
pub struct Volume3d {
    pub data: Vec<u8>,
    pub n: usize,
    pub nz: usize,
    /// Half-width of the horizontal box (km); x,y span `[-half_km, +half_km]`.
    pub half_km: f32,
    /// Top of the box (km); z spans `[0, top_km]`.
    pub top_km: f32,
    pub value_min: f32,
    pub value_max: f32,
}

/// Build an `n × n × nz` reflectivity volume out to `half_km` horizontally and `top_km` up.
/// Returns `None` if there are no sweeps.
pub fn build(sweeps: &[BinnedSweep], n: usize, nz: usize, half_km: f32, top_km: f32) -> Option<Volume3d> {
    let s0 = sweeps.first()?;
    let (value_min, value_max) = (s0.value_min, s0.value_max);
    let span = (value_max - value_min).max(f32::EPSILON);
    let n = n.max(2);
    let nz = nz.max(2);
    let mut data = vec![0u8; n * n * nz];

    for j in 0..n {
        let y = -half_km as f64 + 2.0 * half_km as f64 * j as f64 / (n - 1) as f64;
        for i in 0..n {
            let x = -half_km as f64 + 2.0 * half_km as f64 * i as f64 / (n - 1) as f64;
            let ground = (x * x + y * y).sqrt();
            if ground < 0.5 {
                continue; // cone of silence at the radar
            }
            let az = x.atan2(y).to_degrees().rem_euclid(360.0);

            // Vertical profile of (beam_height, dBZ) from every tilt at this ground range/azimuth.
            let mut samples: Vec<(f64, f32)> = Vec::with_capacity(sweeps.len());
            for s in sweeps {
                let e = s.elevation_deg as f64;
                let slant = ground / e.to_radians().cos();
                let gate = ((slant - s.first_gate_km as f64) / s.gate_interval_km.max(f32::EPSILON) as f64).round();
                if gate < 0.0 || gate as usize >= s.gate_count {
                    continue;
                }
                let bin = ((az / 360.0 * s.az_bins as f64) as usize) % s.az_bins;
                let idx = s.data[bin * s.gate_count + gate as usize];
                if idx < 2 {
                    continue;
                }
                let h = beam_height_km(slant, e);
                let v = value_min + (idx as f32 - 2.0) / 253.0 * span;
                samples.push((h, v));
            }
            if samples.is_empty() {
                continue;
            }
            samples.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap_or(std::cmp::Ordering::Equal));

            for k in 0..nz {
                let z = top_km as f64 * k as f64 / (nz - 1) as f64;
                if let Some(v) = sample_profile(&samples, z) {
                    let t = ((v - value_min) / span).clamp(0.0, 1.0);
                    data[i + n * j + n * n * k] = 2 + (t * 253.0) as u8;
                }
            }
        }
    }

    Some(Volume3d { data, n, nz, half_km, top_km, value_min, value_max })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level2::{BinnedSweep, Moment};

    /// A synthetic single-tilt sweep with one hot gate at a known azimuth/range.
    fn sweep(elev: f32) -> BinnedSweep {
        let (az_bins, gate_count) = (720usize, 200usize);
        let mut data = vec![0u8; az_bins * gate_count];
        // Strong echo in an east-facing wedge (az ~75°..105° → bins 150..210) at ~40..60 km.
        for bin in 150..210 {
            for g in 40..60 {
                data[bin * gate_count + g] = 200;
            }
        }
        let (value_min, value_max) = Moment::Reflectivity.value_range();
        BinnedSweep {
            moment: Moment::Reflectivity,
            az_bins,
            gate_count,
            data,
            first_gate_km: 0.0,
            gate_interval_km: 1.0,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: elev,
            value_min,
            value_max,
        }
    }

    #[test]
    fn builds_nonempty_volume_with_echo() {
        let sweeps = vec![sweep(0.5), sweep(1.5), sweep(2.4)];
        let v = build(&sweeps, 64, 24, 120.0, 18.0).unwrap();
        assert_eq!(v.data.len(), 64 * 64 * 24);
        let filled = v.data.iter().filter(|&&b| b >= 2).count();
        assert!(filled > 0, "east-side echo should populate some voxels");
    }
}
