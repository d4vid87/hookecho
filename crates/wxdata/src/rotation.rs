//! Client-side velocity-couplet (mesocyclone / TVS) detection from a single Level 2 sweep.
//!
//! A rotation couplet is adjacent inbound and outbound velocity maxima at the same range: the
//! radar sees one side of the vortex moving toward it and the other away. The legacy operational
//! criterion is gate-to-gate azimuthal shear — the velocity difference between neighboring
//! azimuths at one range — with ~25 m/s marking a weak signature and ~36 m/s a strong one.
//!
//! This runs on the dealiased velocity [`BinnedSweep`] the app already bins for display, so it
//! works per volume with no extra download, and it is complementary to the coarse MRMS AzShear
//! national grid (2-min cadence, ~1 km cells) rather than a replacement for it.

use crate::level2::{BinnedSweep, Moment};

/// A detected rotation couplet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoupletHit {
    pub lon: f64,
    pub lat: f64,
    /// Rotational velocity: half the inbound/outbound spread over the cluster (m/s).
    pub vrot_ms: f32,
    /// Strongest gate-to-gate velocity difference in the cluster (m/s).
    pub g2g_ms: f32,
    /// Range from the radar to the cluster centroid (km).
    pub range_km: f32,
    /// Candidate gate pairs in the cluster (bigger = more confident).
    pub gates: usize,
}

/// Decode a binned `u8` gate index back to its physical value, or `None` for below-threshold /
/// range-folded gates (indices 0/1). Same encoding as [`crate::tds`].
fn decode(sweep: &BinnedSweep, idx: u8) -> Option<f32> {
    if idx < 2 {
        return None;
    }
    let (lo, hi) = (sweep.value_min, sweep.value_max);
    Some(lo + (idx as f32 - 2.0) / 253.0 * (hi - lo))
}

/// Great-circle destination point (places a gate at its azimuth/range).
fn dest(lon: f64, lat: f64, bearing_deg: f64, dist_km: f64) -> (f64, f64) {
    let r = 6371.0;
    let ad = dist_km / r;
    let (br, la1, lo1) = (bearing_deg.to_radians(), lat.to_radians(), lon.to_radians());
    let la2 = (la1.sin() * ad.cos() + la1.cos() * ad.sin() * br.cos()).asin();
    let lo2 = lo1 + (br.sin() * ad.sin() * la1.cos()).atan2(ad.cos() - la1.sin() * la2.sin());
    (lo2.to_degrees(), la2.to_degrees())
}

/// Detect rotation couplets in a velocity sweep. A gate pair is a candidate when two azimuthally
/// adjacent gates at the same range differ by `>= g2g_min_ms` in opposite senses (one inbound,
/// one outbound — same-sign shear is convergence/divergence, not rotation), between
/// `min_range_km` and `max_range_km`. Candidates cluster on a ~4 km geographic grid; clusters
/// with `>= min_gates` pairs become hits, and `vrot` is half the cluster's inbound→outbound spread.
///
/// The near-range floor matters as much as the far one: inside ~15 km the azimuthal bins are only
/// a few hundred metres apart, so ground clutter and ordinary turbulence routinely clear a 25 m/s
/// gate-to-gate difference — a live KTLX clear-air sweep reported six "couplets", all within 8 km.
///
/// Feed this the *dealiased* sweep: folded velocity manufactures huge false gate-to-gate jumps.
pub fn detect(
    vel: &BinnedSweep,
    g2g_min_ms: f32,
    min_range_km: f32,
    max_range_km: f32,
    min_gates: usize,
) -> Vec<CoupletHit> {
    debug_assert_eq!(vel.moment, Moment::Velocity);
    if vel.az_bins == 0 || vel.gate_count == 0 {
        return Vec::new();
    }
    const CELL: f64 = 0.04; // ~4 km cluster cells, matching the TDS detector
    use std::collections::HashMap;
    /// Running per-cell accumulator: pairs, summed lon/lat/range, min/max velocity, max |dv|.
    type Cell = (usize, f64, f64, f64, f32, f32, f32);
    let mut cells: HashMap<(i64, i64), Cell> = HashMap::new();
    let (rlon, rlat) = (vel.radar_lon as f64, vel.radar_lat as f64);

    for az in 0..vel.az_bins {
        let next = (az + 1) % vel.az_bins; // wraps 719 → 0
        let az_deg = (az as f64 + 0.5) * 360.0 / vel.az_bins as f64;
        for gate in 0..vel.gate_count {
            let range = vel.first_gate_km + gate as f32 * vel.gate_interval_km;
            if range > max_range_km {
                break; // ranges increase with index; nothing further qualifies on this radial
            }
            if range < min_range_km {
                continue;
            }
            let Some(a) = decode(vel, vel.data[az * vel.gate_count + gate]) else {
                continue;
            };
            let Some(b) = decode(vel, vel.data[next * vel.gate_count + gate]) else {
                continue;
            };
            let dv = (a - b).abs();
            if dv < g2g_min_ms || a * b >= 0.0 {
                continue; // too weak, or both gates on the same side of zero (not a couplet)
            }
            let (lon, lat) = dest(rlon, rlat, az_deg, range as f64);
            let key = ((lon / CELL).round() as i64, (lat / CELL).round() as i64);
            let e = cells
                .entry(key)
                .or_insert((0, 0.0, 0.0, 0.0, f32::MAX, f32::MIN, 0.0));
            e.0 += 1;
            e.1 += lon;
            e.2 += lat;
            e.3 += range as f64;
            e.4 = e.4.min(a).min(b);
            e.5 = e.5.max(a).max(b);
            e.6 = e.6.max(dv);
        }
    }

    let mut hits: Vec<CoupletHit> = cells
        .into_values()
        .filter(|(n, ..)| *n >= min_gates)
        .map(|(n, slon, slat, srange, vmin, vmax, g2g)| CoupletHit {
            lon: slon / n as f64,
            lat: slat / n as f64,
            vrot_ms: (vmax - vmin) / 2.0,
            g2g_ms: g2g,
            range_km: (srange / n as f64) as f32,
            gates: n,
        })
        .collect();
    // Strongest rotation first — the alarm and the banner both want the worst one.
    hits.sort_by(|a, b| b.vrot_ms.total_cmp(&a.vrot_ms));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A velocity sweep that is uniformly `background`, with `inbound`/`outbound` values painted
    /// on two adjacent azimuth wedges over one gate span — a synthetic couplet.
    fn couplet_sweep(inbound: f32, outbound: f32, background: f32) -> BinnedSweep {
        let (az_bins, gate_count) = (720usize, 200usize);
        let (lo, hi) = Moment::Velocity.value_range();
        let idx = |v: f32| (2.0 + (v - lo) / (hi - lo) * 253.0).round() as u8;
        let mut data = vec![idx(background); az_bins * gate_count];
        for g in 40..60 {
            for az in 96..100 {
                data[az * gate_count + g] = idx(inbound);
            }
            for az in 100..104 {
                data[az * gate_count + g] = idx(outbound);
            }
        }
        BinnedSweep {
            moment: Moment::Velocity,
            az_bins,
            gate_count,
            data,
            first_gate_km: 2.0,
            gate_interval_km: 0.25,
            radar_lat: 35.0,
            radar_lon: -97.5,
            elevation_deg: 0.5,
            value_min: lo,
            value_max: hi,
        }
    }

    #[test]
    fn flags_adjacent_inbound_outbound() {
        let hits = detect(&couplet_sweep(-30.0, 30.0, 0.0), 25.0, 5.0, 150.0, 3);
        assert!(!hits.is_empty(), "a ±30 m/s couplet should be flagged");
        let h = hits[0];
        assert!(
            (h.vrot_ms - 30.0).abs() < 2.0,
            "vrot ≈ 30 m/s, got {}",
            h.vrot_ms
        );
        assert!(h.g2g_ms >= 55.0, "gate-to-gate ≈ 60 m/s, got {}", h.g2g_ms);
        assert!(
            h.range_km > 10.0 && h.range_km < 30.0,
            "range {} km",
            h.range_km
        );
    }

    #[test]
    fn ignores_weak_and_same_sign_shear() {
        // Weak couplet: ±8 m/s is well under the 25 m/s criterion.
        assert!(detect(&couplet_sweep(-8.0, 8.0, 0.0), 25.0, 5.0, 150.0, 3).is_empty());
        // Strong shear but both sides inbound: convergence, not rotation.
        assert!(detect(&couplet_sweep(-40.0, -5.0, -5.0), 25.0, 5.0, 150.0, 3).is_empty());
    }

    #[test]
    fn range_gates_exclude_far_and_near_couplets() {
        // The synthetic couplet sits ~12-17 km out.
        let s = couplet_sweep(-30.0, 30.0, 0.0);
        assert!(
            detect(&s, 25.0, 5.0, 8.0, 3).is_empty(),
            "beyond the far gate"
        );
        assert!(
            detect(&s, 25.0, 30.0, 150.0, 3).is_empty(),
            "inside the near gate"
        );
    }
}
