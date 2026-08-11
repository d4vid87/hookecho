//! Dual-polarization signatures that are not debris: three-body scatter spikes, ZDR columns, and
//! the melting-layer bright band.
//!
//! [`crate::tds`] flags the one signature that means a tornado may be on the ground. These are the
//! other three an operational eye looks for, and none of them is an alarm:
//!
//! * **TBSS** — a "hail spike": a hail core so reflective that energy bounces off the ground and
//!   back through the hail before returning, painting a weak, low-CC spike *behind* the core along
//!   the same radial. It cannot be caused by anything else, so it is near-proof of large hail.
//! * **ZDR column** — oblate raindrops carried above the freezing level by an updraft, seen as a
//!   plume of positive ZDR reaching over the melting layer. It is an updraft proxy, and it tends
//!   to deepen before a storm intensifies.
//! * **Bright band** — the melting layer itself: partly-melted snow looks huge and wet to the
//!   radar, giving a ring of depressed CC (roughly 0.7–0.97) at one height. Knowing where it is
//!   tells you which "heavy rain" is really the melting layer seen edge-on.
//!
//! All three are display-only context. They deliberately raise nothing.

use crate::level2::{BinnedSweep, Moment};
use crate::tds::{dest, decode};
use crate::xsection::column_samples;

/// Geographic cluster cell, ~4 km — the same lattice [`crate::tds`] clusters on, so a signature
/// reports as one hit rather than a few hundred gates.
const CELL: f64 = 0.04;

/// A three-body scatter spike: the hail core it points away from, and how long the spike runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TbssHit {
    /// The core the spike came off, not the spike itself — that is where the hail is.
    pub lon: f64,
    pub lat: f64,
    /// Peak reflectivity of the core (dBZ).
    pub core_dbz: f32,
    /// Length of the weak-echo, low-CC run downrange of the core (km).
    pub len_km: f32,
    /// Lowest CC seen inside the spike.
    pub min_cc: f32,
}

/// A ZDR column: an updraft carrying rain above the freezing level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZdrColumnHit {
    pub lon: f64,
    pub lat: f64,
    /// How far the ZDR > threshold layer reaches above the freezing level (km).
    pub depth_km: f64,
    /// Height the column tops out at, above radar level (km).
    pub top_km: f64,
    /// Peak ZDR inside the column (dB).
    pub max_zdr: f32,
}

/// The melting layer, as read off the CC field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrightBand {
    /// Height above radar level of the CC minimum (km).
    pub height_km: f64,
    /// How many gates voted for that height — small counts are noise, not a melting layer.
    pub samples: usize,
    /// Mean CC in the winning height bin.
    pub mean_cc: f32,
}

/// One cluster cell while accumulating: `lon`/`lat` are running sums, averaged at the end.
#[derive(Default)]
struct TbssCell {
    n: usize,
    lon: f64,
    lat: f64,
    core_dbz: f32,
    len_km: f32,
    min_cc: f32,
}

/// The same, for ZDR columns: the deepest column in the cell wins.
#[derive(Default)]
struct ZdrCell {
    n: usize,
    lon: f64,
    lat: f64,
    depth_km: f64,
    top_km: f64,
    max_zdr: f32,
}

/// Detect three-body scatter spikes.
///
/// Walks each radial outward: at the strongest gate at or above `core_dbz`, look downrange for a
/// run of gates that are both weak (`< weak_dbz`, or no echo at all) and depolarized
/// (`CC < cc_max`). A run of at least `min_len_km` is the spike. Ordinary weak echo behind a core
/// fails the CC half; ordinary low CC fails the "must sit behind a 60 dBZ core" half.
///
/// Hits are the *cores*, clustered on the ~4 km lattice and sorted by core reflectivity.
#[allow(clippy::too_many_arguments)]
pub fn tbss(
    z: &BinnedSweep,
    cc: &BinnedSweep,
    core_dbz: f32,
    weak_dbz: f32,
    cc_max: f32,
    min_len_km: f32,
    max_range_km: f32,
) -> Vec<TbssHit> {
    debug_assert_eq!(z.moment, Moment::Reflectivity);
    debug_assert_eq!(cc.moment, Moment::CorrelationCoefficient);
    if z.az_bins == 0 || z.gate_count == 0 || cc.gate_count == 0 {
        return Vec::new();
    }
    use std::collections::HashMap;
    let mut cells: HashMap<(i64, i64), TbssCell> = HashMap::new();
    let (rlon, rlat) = (z.radar_lon as f64, z.radar_lat as f64);

    for az in 0..z.az_bins {
        let az_deg = az as f64 * 360.0 / z.az_bins as f64;
        // Strongest gate on this radial, within range.
        let mut core: Option<(usize, f32)> = None;
        for gate in 0..z.gate_count {
            let range = z.first_gate_km + gate as f32 * z.gate_interval_km;
            if range > max_range_km {
                break;
            }
            let Some(v) = decode(z, z.data[az * z.gate_count + gate]) else {
                continue;
            };
            if v >= core_dbz && core.is_none_or(|(_, best)| v > best) {
                core = Some((gate, v));
            }
        }
        let Some((core_gate, core_val)) = core else {
            continue;
        };

        // The spike begins where the storm ends: walk off the back of the core through whatever
        // echo is still attached to it, and start counting at the first weak gate.
        let mut start = core_gate + 1;
        while start < z.gate_count
            && decode(z, z.data[az * z.gate_count + start]).is_some_and(|v| v >= weak_dbz)
        {
            start += 1;
        }

        // The spike must then be contiguous.
        let mut len_gates = 0usize;
        let mut min_cc = 1.05f32;
        for gate in start..z.gate_count {
            let range = z.first_gate_km + gate as f32 * z.gate_interval_km;
            let zv = decode(z, z.data[az * z.gate_count + gate]);
            // No echo at all still counts as weak — a spike often trails off into nothing.
            if zv.is_some_and(|v| v >= weak_dbz) {
                break;
            }
            let ci = ((range - cc.first_gate_km) / cc.gate_interval_km).round() as i64;
            if ci < 0 || ci as usize >= cc.gate_count {
                break;
            }
            let Some(cc_val) = decode(cc, cc.data[az * cc.gate_count + ci as usize]) else {
                break; // depolarization is the point; a gate with no CC is not evidence of it
            };
            if cc_val >= cc_max {
                break;
            }
            len_gates += 1;
            min_cc = min_cc.min(cc_val);
        }
        let len_km = len_gates as f32 * z.gate_interval_km;
        if len_km < min_len_km {
            continue;
        }

        let core_range = z.first_gate_km + core_gate as f32 * z.gate_interval_km;
        let (lon, lat) = dest(rlon, rlat, az_deg, core_range as f64);
        let key = ((lon / CELL).round() as i64, (lat / CELL).round() as i64);
        let e = cells.entry(key).or_insert(TbssCell {
            min_cc: 1.05,
            ..Default::default()
        });
        e.n += 1;
        e.lon += lon;
        e.lat += lat;
        e.core_dbz = e.core_dbz.max(core_val);
        e.len_km = e.len_km.max(len_km);
        e.min_cc = e.min_cc.min(min_cc);
    }

    let mut hits: Vec<TbssHit> = cells
        .into_values()
        .map(|c| TbssHit {
            lon: c.lon / c.n as f64,
            lat: c.lat / c.n as f64,
            core_dbz: c.core_dbz,
            len_km: c.len_km,
            min_cc: c.min_cc,
        })
        .collect();
    hits.sort_by(|a, b| b.core_dbz.total_cmp(&a.core_dbz));
    hits
}

/// Detect ZDR columns above `h0_km` (freezing level, above *radar* level).
///
/// Samples a coarse polar lattice — every 4th azimuth, 2 km steps — through
/// [`column_samples`], the same vertical-profile machinery the derived products use. A column
/// qualifies when ZDR stays above `min_zdr_db` from at or below the freezing level up to at least
/// `min_depth_km` above it, and there is real echo (`>= min_core_dbz`) there to carry it.
///
/// Hits are clustered on the ~4 km lattice, deepest first.
#[allow(clippy::too_many_arguments)]
pub fn zdr_columns(
    zdr_tilts: &[BinnedSweep],
    z_tilts: &[BinnedSweep],
    h0_km: f64,
    min_zdr_db: f32,
    min_depth_km: f64,
    min_core_dbz: f32,
    max_range_km: f64,
) -> Vec<ZdrColumnHit> {
    let Some(first) = zdr_tilts.first() else {
        return Vec::new();
    };
    if h0_km <= 0.0 {
        return Vec::new(); // freezing level at or below the radar: nothing to be "above"
    }
    use std::collections::HashMap;
    let mut cells: HashMap<(i64, i64), ZdrCell> = HashMap::new();
    let (rlon, rlat) = (first.radar_lon as f64, first.radar_lat as f64);
    let mut zdr_samples: Vec<(f64, f32)> = Vec::with_capacity(zdr_tilts.len());
    let mut z_samples: Vec<(f64, f32)> = Vec::with_capacity(z_tilts.len());

    // ponytail: coarse lattice on the calling thread — ~9k columns, tens of ms. If it ever shows
    // up in a frame time, it moves to spawn_blocking like `derived::derive` did.
    let az_step = 4;
    for az_bin in (0..first.az_bins).step_by(az_step) {
        let az = az_bin as f64 * 360.0 / first.az_bins as f64;
        let mut ground = 4.0f64;
        while ground <= max_range_km {
            let g = ground;
            ground += 2.0;
            column_samples(zdr_tilts, g, az, &mut zdr_samples);
            if zdr_samples.len() < 2 {
                continue;
            }
            // The column has to start at or below the freezing level: positive ZDR that only
            // exists aloft is not rain that was lifted there.
            if !zdr_samples
                .iter()
                .any(|(h, v)| *h <= h0_km && *v >= min_zdr_db)
            {
                continue;
            }
            // Contiguous run of ZDR >= threshold reaching above h0.
            let mut top = 0.0f64;
            let mut max_zdr = f32::MIN;
            let mut run_alive = true;
            for (h, v) in zdr_samples.iter().filter(|(h, _)| *h > h0_km) {
                if !run_alive {
                    break;
                }
                if *v >= min_zdr_db {
                    top = *h;
                    max_zdr = max_zdr.max(*v);
                } else {
                    run_alive = false;
                }
            }
            let depth = top - h0_km;
            if depth < min_depth_km {
                continue;
            }
            // Real echo up there, so a noisy ZDR gate in clear air can't invent a column.
            column_samples(z_tilts, g, az, &mut z_samples);
            if !z_samples
                .iter()
                .any(|(h, v)| *h >= h0_km && *v >= min_core_dbz)
            {
                continue;
            }

            let (lon, lat) = dest(rlon, rlat, az, g);
            let key = ((lon / CELL).round() as i64, (lat / CELL).round() as i64);
            let e = cells.entry(key).or_insert(ZdrCell {
                max_zdr: f32::MIN,
                ..Default::default()
            });
            e.n += 1;
            e.lon += lon;
            e.lat += lat;
            if depth > e.depth_km {
                e.depth_km = depth;
                e.top_km = top;
            }
            e.max_zdr = e.max_zdr.max(max_zdr);
        }
    }

    let mut hits: Vec<ZdrColumnHit> = cells
        .into_values()
        .map(|c| ZdrColumnHit {
            lon: c.lon / c.n as f64,
            lat: c.lat / c.n as f64,
            depth_km: c.depth_km,
            top_km: c.top_km,
            max_zdr: c.max_zdr,
        })
        .collect();
    hits.sort_by(|a, b| b.depth_km.total_cmp(&a.depth_km));
    hits
}

/// Estimate the melting-layer height from the CC bright band.
///
/// Melting snow depolarizes: CC drops into roughly 0.7–0.97 in a thin layer and is above 0.97
/// in both the rain below and the snow above. Histogram those gates by beam height in 250 m bins
/// and take the modal bin — with enough votes in it that a few noisy gates cannot pass for a
/// melting layer.
///
/// Tilts at or below ~1° see the band only at long range where the beam is enormous, so callers
/// should pass the mid tilts (roughly 2°–10°).
// ponytail: display-only. The hail algorithm still takes its freezing level from the model rather
// than from this — an estimator this new gets watched against real events before anything is
// computed from it.
pub fn bright_band(cc_tilts: &[BinnedSweep], max_height_km: f64) -> Option<BrightBand> {
    const BIN_KM: f64 = 0.25;
    const MIN_SAMPLES: usize = 200;
    let nbins = (max_height_km / BIN_KM).ceil() as usize;
    let mut counts = vec![0usize; nbins];
    let mut sums = vec![0f64; nbins];

    for s in cc_tilts {
        if s.moment != Moment::CorrelationCoefficient || s.gate_count == 0 {
            continue;
        }
        let e = s.elevation_deg as f64;
        for az in 0..s.az_bins {
            for gate in 0..s.gate_count {
                let Some(v) = decode(s, s.data[az * s.gate_count + gate]) else {
                    continue;
                };
                if !(0.7..0.97).contains(&v) {
                    continue;
                }
                let slant = (s.first_gate_km + gate as f32 * s.gate_interval_km) as f64;
                let h = crate::xsection::beam_height_km(slant, e);
                if h < 0.0 || h >= max_height_km {
                    continue;
                }
                let b = (h / BIN_KM) as usize;
                counts[b] += 1;
                sums[b] += v as f64;
            }
        }
    }

    let (b, n) = counts
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|(_, n)| *n)
        .filter(|(_, n)| *n >= MIN_SAMPLES)?;
    Some(BrightBand {
        height_km: (b as f64 + 0.5) * BIN_KM,
        samples: n,
        mean_cc: (sums[b] / n as f64) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sweep whose `hot` index fills a wedge of azimuths/gates; everything else is `cold`.
    fn sweep(
        moment: Moment,
        hot: u8,
        cold: u8,
        hot_az: std::ops::Range<usize>,
        hot_gate: std::ops::Range<usize>,
    ) -> BinnedSweep {
        let (az_bins, gate_count) = (720usize, 400usize);
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

    fn idx(moment: Moment, v: f32) -> u8 {
        let (lo, hi) = moment.value_range();
        (2.0 + (v - lo) / (hi - lo) * 253.0).round() as u8
    }

    /// Reflectivity: a 65 dBZ core at gates 100..108, weak echo behind it; CC low behind the core.
    fn spike_sweeps(cc_behind: f32) -> (BinnedSweep, BinnedSweep) {
        let mut z = sweep(
            Moment::Reflectivity,
            idx(Moment::Reflectivity, 65.0),
            idx(Moment::Reflectivity, 5.0),
            100..104,
            100..108,
        );
        // Behind the core: 8 dBZ, i.e. weak but present.
        for az in 100..104 {
            for g in 108..160 {
                z.data[az * z.gate_count + g] = idx(Moment::Reflectivity, 8.0);
            }
        }
        let mut cc = sweep(
            Moment::CorrelationCoefficient,
            idx(Moment::CorrelationCoefficient, 0.98),
            idx(Moment::CorrelationCoefficient, 0.98),
            0..0,
            0..0,
        );
        for az in 100..104 {
            for g in 108..160 {
                cc.data[az * cc.gate_count + g] = idx(Moment::CorrelationCoefficient, cc_behind);
            }
        }
        (z, cc)
    }

    #[test]
    fn flags_a_weak_depolarized_spike_behind_a_big_core() {
        let (z, cc) = spike_sweeps(0.5);
        let hits = tbss(&z, &cc, 60.0, 20.0, 0.8, 4.0, 150.0);
        assert!(!hits.is_empty(), "spike should be flagged");
        assert!(hits[0].core_dbz >= 60.0);
        assert!(hits[0].len_km >= 4.0);
        assert!(hits[0].min_cc < 0.8);
    }

    #[test]
    fn meteorological_cc_behind_the_core_is_not_a_spike() {
        // Same geometry, but the echo behind the core is ordinary rain: CC 0.98.
        let (z, cc) = spike_sweeps(0.98);
        assert!(tbss(&z, &cc, 60.0, 20.0, 0.8, 4.0, 150.0).is_empty());
    }

    #[test]
    fn no_big_core_means_no_spike() {
        let z = sweep(
            Moment::Reflectivity,
            idx(Moment::Reflectivity, 45.0),
            idx(Moment::Reflectivity, 5.0),
            100..104,
            100..108,
        );
        let cc = sweep(
            Moment::CorrelationCoefficient,
            idx(Moment::CorrelationCoefficient, 0.5),
            idx(Moment::CorrelationCoefficient, 0.5),
            0..0,
            0..0,
        );
        assert!(tbss(&z, &cc, 60.0, 20.0, 0.8, 4.0, 150.0).is_empty());
    }

    /// Uniform sweeps at a ladder of elevations, so a column exists everywhere.
    fn tilts(moment: Moment, value: f32, elevations: &[f32]) -> Vec<BinnedSweep> {
        elevations
            .iter()
            .map(|e| {
                let mut s = sweep(moment, idx(moment, value), idx(moment, value), 0..0, 0..0);
                s.elevation_deg = *e;
                s
            })
            .collect()
    }

    #[test]
    fn deep_positive_zdr_over_the_freezing_level_is_a_column() {
        let els = [0.5, 1.5, 2.4, 3.4, 4.3, 6.0, 9.9, 14.6, 19.5];
        let zdr = tilts(Moment::DifferentialReflectivity, 2.5, &els);
        let z = tilts(Moment::Reflectivity, 50.0, &els);
        let hits = zdr_columns(&zdr, &z, 3.0, 1.0, 1.0, 40.0, 60.0);
        assert!(!hits.is_empty(), "uniform 2.5 dB ZDR should qualify");
        assert!(hits[0].depth_km >= 1.0);
        assert!(hits[0].top_km > 3.0);
    }

    #[test]
    fn zdr_that_never_gets_above_the_freezing_level_is_not_a_column() {
        let els = [0.5, 1.5, 2.4, 3.4];
        let zdr = tilts(Moment::DifferentialReflectivity, 2.5, &els);
        let z = tilts(Moment::Reflectivity, 50.0, &els);
        // Freezing level well above every beam these tilts reach at this range.
        assert!(zdr_columns(&zdr, &z, 12.0, 1.0, 1.0, 40.0, 60.0).is_empty());
    }

    #[test]
    fn weak_zdr_is_not_a_column() {
        let els = [0.5, 1.5, 2.4, 3.4, 4.3, 6.0, 9.9];
        let zdr = tilts(Moment::DifferentialReflectivity, 0.2, &els);
        let z = tilts(Moment::Reflectivity, 50.0, &els);
        assert!(zdr_columns(&zdr, &z, 3.0, 1.0, 1.0, 40.0, 60.0).is_empty());
    }

    #[test]
    fn a_ring_of_depressed_cc_reads_as_the_melting_layer() {
        // One tilt of uniform CC 0.93: every gate lands in the band, at that tilt's own heights.
        let mut s = sweep(
            Moment::CorrelationCoefficient,
            idx(Moment::CorrelationCoefficient, 0.93),
            idx(Moment::CorrelationCoefficient, 0.93),
            0..0,
            0..0,
        );
        s.elevation_deg = 4.0;
        let bb = bright_band(&[s], 6.0).expect("melting layer");
        assert!(bb.height_km > 0.0 && bb.height_km < 6.0);
        assert!((bb.mean_cc - 0.93).abs() < 0.02);
        assert!(bb.samples >= 200);
    }

    #[test]
    fn clean_meteorological_cc_has_no_bright_band() {
        let mut s = sweep(
            Moment::CorrelationCoefficient,
            idx(Moment::CorrelationCoefficient, 0.995),
            idx(Moment::CorrelationCoefficient, 0.995),
            0..0,
            0..0,
        );
        s.elevation_deg = 4.0;
        assert!(bright_band(&[s], 6.0).is_none());
    }
}
