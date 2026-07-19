//! Tornado Debris Signature (TDS) detection.
//!
//! A TDS is a "debris ball": lofted tornado debris scatters radar energy incoherently, dropping the
//! correlation coefficient (CC/ρhv) well below meteorological values while reflectivity stays high.
//! The classic operational heuristic — low CC collocated with high reflectivity in the storm core —
//! is what this module flags. (Collocation with a velocity couplet / rotation strengthens a real
//! diagnosis; that's left to the human reading the flagged location.)
//!
//! Input is two same-tilt [`BinnedSweep`]s (reflectivity + CC) sharing the 720-azimuth grid.
//! Candidate gates are clustered on a coarse geographic grid so a debris ball reports as one hit.

use crate::level2::{BinnedSweep, Moment};

/// A detected debris-signature cluster.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TdsHit {
    pub lon: f64,
    pub lat: f64,
    /// Number of candidate gates in the cluster (bigger = more confident).
    pub gates: usize,
    /// Lowest CC seen in the cluster.
    pub min_cc: f32,
}

/// Decode a binned `u8` gate index back to its physical value, or `None` for below-threshold /
/// range-folded gates (indices 0/1).
fn decode(sweep: &BinnedSweep, idx: u8) -> Option<f32> {
    if idx < 2 {
        return None;
    }
    let (lo, hi) = (sweep.value_min, sweep.value_max);
    Some(lo + (idx as f32 - 2.0) / 253.0 * (hi - lo))
}

/// Great-circle destination point (used to place a gate at its azimuth/range).
fn dest(lon: f64, lat: f64, bearing_deg: f64, dist_km: f64) -> (f64, f64) {
    let r = 6371.0;
    let ad = dist_km / r;
    let (br, la1, lo1) = (bearing_deg.to_radians(), lat.to_radians(), lon.to_radians());
    let la2 = (la1.sin() * ad.cos() + la1.cos() * ad.sin() * br.cos()).asin();
    let lo2 = lo1 + (br.sin() * ad.sin() * la1.cos()).atan2(ad.cos() - la1.sin() * la2.sin());
    (lo2.to_degrees(), la2.to_degrees())
}

/// Detect debris-signature clusters. A gate is a candidate when CC `< cc_max`, reflectivity
/// `>= z_min` dBZ, and its range `<= max_range_km` (resolution/low-tilt gating). Candidates are
/// clustered on a ~4 km grid; clusters with `>= min_gates` gates become hits.
pub fn detect(
    z: &BinnedSweep,
    cc: &BinnedSweep,
    cc_max: f32,
    z_min: f32,
    max_range_km: f32,
    min_gates: usize,
) -> Vec<TdsHit> {
    debug_assert_eq!(cc.moment, Moment::CorrelationCoefficient);
    if cc.az_bins == 0 || cc.gate_count == 0 || z.gate_count == 0 {
        return Vec::new();
    }
    // Accumulate candidates into ~0.04° (~4 km) geographic cells.
    const CELL: f64 = 0.04;
    use std::collections::HashMap;
    let mut cells: HashMap<(i64, i64), (usize, f64, f64, f32)> = HashMap::new();
    let (rlon, rlat) = (cc.radar_lon as f64, cc.radar_lat as f64);

    for az in 0..cc.az_bins {
        let az_deg = az as f64 * 360.0 / cc.az_bins as f64;
        for gate in 0..cc.gate_count {
            let range = cc.first_gate_km + gate as f32 * cc.gate_interval_km;
            if range > max_range_km {
                break; // gates increase with index; nothing further qualifies on this radial
            }
            let Some(cc_val) = decode(cc, cc.data[az * cc.gate_count + gate]) else { continue };
            if cc_val >= cc_max {
                continue;
            }
            // Reflectivity at the same range (map through the Z sweep's own gate spacing).
            let zi = ((range - z.first_gate_km) / z.gate_interval_km).round() as i64;
            if zi < 0 || zi as usize >= z.gate_count {
                continue;
            }
            let Some(z_val) = decode(z, z.data[az * z.gate_count + zi as usize]) else { continue };
            if z_val < z_min {
                continue; // low CC in weak echo = biological/clutter, not debris
            }
            let (lon, lat) = dest(rlon, rlat, az_deg, range as f64);
            let key = ((lon / CELL).round() as i64, (lat / CELL).round() as i64);
            let e = cells.entry(key).or_insert((0, 0.0, 0.0, 1.05));
            e.0 += 1;
            e.1 += lon;
            e.2 += lat;
            e.3 = e.3.min(cc_val);
        }
    }

    let mut hits: Vec<TdsHit> = cells
        .into_values()
        .filter(|(n, ..)| *n >= min_gates)
        .map(|(n, slon, slat, min_cc)| TdsHit {
            lon: slon / n as f64,
            lat: slat / n as f64,
            gates: n,
            min_cc,
        })
        .collect();
    hits.sort_by(|a, b| b.gates.cmp(&a.gates));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a sweep where a wedge of azimuths/gates carries `hot` (index) and the rest `cold`.
    fn sweep(moment: Moment, hot: u8, cold: u8, hot_az: std::ops::Range<usize>, hot_gate: std::ops::Range<usize>) -> BinnedSweep {
        let (az_bins, gate_count) = (720usize, 200usize);
        let mut data = vec![cold; az_bins * gate_count];
        for az in hot_az.clone() {
            for g in hot_gate.clone() {
                data[az * gate_count + g] = hot;
            }
        }
        let (lo, hi) = moment.value_range();
        BinnedSweep {
            moment,
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

    /// Map a physical value to its `u8` index for a moment's range.
    fn idx(moment: Moment, v: f32) -> u8 {
        let (lo, hi) = moment.value_range();
        (2.0 + (v - lo) / (hi - lo) * 253.0).round() as u8
    }

    #[test]
    fn flags_low_cc_in_high_z() {
        // Debris ball: CC 0.55 + Z 52 dBZ over a small wedge of gates.
        let cc_hot = idx(Moment::CorrelationCoefficient, 0.55);
        let cc_cold = idx(Moment::CorrelationCoefficient, 0.98);
        let z_hot = idx(Moment::Reflectivity, 52.0);
        let z_cold = idx(Moment::Reflectivity, 20.0);
        let cc = sweep(Moment::CorrelationCoefficient, cc_hot, cc_cold, 100..112, 40..60);
        let z = sweep(Moment::Reflectivity, z_hot, z_cold, 100..112, 40..60);

        let hits = detect(&z, &cc, 0.80, 40.0, 150.0, 4);
        assert!(!hits.is_empty(), "debris ball should be flagged");
        assert!(hits[0].min_cc < 0.80);
        // Centroid sits away from the radar (positive range along the hot azimuths).
        assert!(hits[0].lat > 35.0);
    }

    #[test]
    fn ignores_low_cc_in_weak_echo() {
        // Low CC but only 15 dBZ (biological / clutter) → no debris flag.
        let cc = sweep(Moment::CorrelationCoefficient, idx(Moment::CorrelationCoefficient, 0.5), idx(Moment::CorrelationCoefficient, 0.98), 100..112, 40..60);
        let z = sweep(Moment::Reflectivity, idx(Moment::Reflectivity, 15.0), idx(Moment::Reflectivity, 10.0), 100..112, 40..60);
        assert!(detect(&z, &cc, 0.80, 40.0, 150.0, 4).is_empty());
    }
}
