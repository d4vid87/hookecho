//! Specific differential phase (KDP) derived from differential phase (ΦDP).
//!
//! KDP is not transmitted in an Archive II volume — it is the range derivative of ΦDP,
//! halved because ΦDP accumulates on both the outbound and return legs:
//! `KDP = ½ · dΦDP/dr`, in degrees per kilometre.
//!
//! Two things make that derivative awkward on real data. ΦDP is reported modulo 360°, so a
//! radial that accumulates past a full turn wraps and would read as a huge negative slope;
//! and it is noisy gate to gate, so a two-point difference is mostly noise. This unwraps the
//! phase along each radial, then fits a least-squares line over a short window of gates.

/// Gates in the fitting window. Nine gates is 2.25 km at the 250 m gate spacing of a
/// super-res split cut — long enough to average down ΦDP noise, short enough to keep a
/// hail core's signature from smearing across the storm.
const WINDOW: usize = 9;

/// A jump larger than this between adjacent gates is read as a modulo-360 wrap rather than
/// real phase accumulation. ΦDP never climbs this fast over one gate in nature.
const WRAP_THRESHOLD: f32 = 180.0;

/// Derive KDP (deg/km) from a ΦDP field (degrees), row-major `[az_bin][gate]`.
///
/// `None` in means no measurement; `None` out means the window around that gate had too few
/// measurements to fit a line through.
pub fn from_differential_phase(
    phi: &[Option<f32>],
    az_bins: usize,
    gate_count: usize,
    gate_interval_km: f32,
) -> Vec<Option<f32>> {
    debug_assert_eq!(phi.len(), az_bins * gate_count);
    let mut out = vec![None; phi.len()];
    if gate_count < WINDOW || gate_interval_km <= 0.0 {
        return out;
    }
    let mut unwrapped = vec![None; gate_count];
    for (row, dst) in phi.chunks(gate_count).zip(out.chunks_mut(gate_count)) {
        unwrap_into(row, &mut unwrapped);
        slopes_into(&unwrapped, gate_interval_km, dst);
    }
    out
}

/// Undo the modulo-360 wrapping along one radial, writing into `out` (same length as `row`).
fn unwrap_into(row: &[Option<f32>], out: &mut [Option<f32>]) {
    let mut turns = 0.0f32;
    let mut previous: Option<f32> = None;
    for (slot, raw) in out.iter_mut().zip(row) {
        *slot = match raw {
            None => None,
            Some(v) => {
                if let Some(p) = previous {
                    // Compare against the *raw* previous value, so a run of gaps does not
                    // invent a wrap out of two unrelated readings.
                    if v - p < -WRAP_THRESHOLD {
                        turns += 360.0;
                    } else if v - p > WRAP_THRESHOLD {
                        turns -= 360.0;
                    }
                }
                previous = Some(*v);
                Some(v + turns)
            }
        };
    }
}

/// Least-squares slope of `phi` over a sliding [`WINDOW`], halved into KDP.
///
/// ponytail: a plain sliding fit, recomputed per gate — O(gates · WINDOW), which at 9 gates is
/// nothing next to the volume decode that produced the field. Running sums would make it O(gates)
/// if the window ever needs to be long.
fn slopes_into(phi: &[Option<f32>], gate_interval_km: f32, out: &mut [Option<f32>]) {
    /// Below this many real samples in a window the fit is noise, not a trend.
    const MIN_SAMPLES: usize = 5;

    let half = WINDOW / 2;
    for (g, slot) in out.iter_mut().enumerate() {
        *slot = None;
        if g < half || g + half >= phi.len() {
            continue;
        }
        let window = &phi[g - half..=g + half];
        let n = window.iter().filter(|v| v.is_some()).count();
        if n < MIN_SAMPLES {
            continue;
        }
        // x is the gate offset within the window, in kilometres.
        let mean_x: f32 = window
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_some())
            .map(|(i, _)| i as f32)
            .sum::<f32>()
            / n as f32;
        let mean_y: f32 = window.iter().flatten().sum::<f32>() / n as f32;
        let mut num = 0.0f32;
        let mut den = 0.0f32;
        for (i, v) in window.iter().enumerate() {
            let Some(y) = v else { continue };
            let dx = i as f32 - mean_x;
            num += dx * (y - mean_y);
            den += dx * dx;
        }
        if den <= f32::EPSILON {
            continue;
        }
        // num/den is deg per gate; divide by the gate length for deg/km, halve for the two-way path.
        *slot = Some(0.5 * (num / den) / gate_interval_km);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A radial whose phase climbs at a known rate must read back that rate, halved.
    #[test]
    fn a_linear_phase_ramp_has_a_constant_kdp() {
        let gate_km = 0.25;
        // 2 deg per gate = 8 deg/km of ΦDP = 4 deg/km of KDP.
        let phi: Vec<Option<f32>> = (0..40).map(|g| Some(g as f32 * 2.0)).collect();
        let kdp = from_differential_phase(&phi, 1, 40, gate_km);
        let middle = kdp[20].expect("mid-radial gate is fittable");
        assert!((middle - 4.0).abs() < 1e-3, "got {middle}");
    }

    /// The same ramp, but wrapping past 360 — the wrap must not read as a cliff.
    #[test]
    fn a_wrapped_phase_ramp_does_not_read_as_a_cliff() {
        let phi: Vec<Option<f32>> = (0..80)
            .map(|g| Some((g as f32 * 6.0).rem_euclid(360.0)))
            .collect();
        let kdp = from_differential_phase(&phi, 1, 80, 0.25);
        // 6 deg/gate at 0.25 km = 24 deg/km of ΦDP = 12 deg/km of KDP, everywhere it is defined.
        for (g, v) in kdp.iter().enumerate() {
            if let Some(v) = v {
                assert!((v - 12.0).abs() < 1e-2, "gate {g} read {v}");
            }
        }
        assert!(kdp.iter().filter(|v| v.is_some()).count() > 60);
    }

    /// Gates with no measurement nearby produce no KDP rather than a fabricated one.
    #[test]
    fn an_empty_radial_produces_nothing() {
        let kdp = from_differential_phase(&vec![None; 40], 1, 40, 0.25);
        assert!(kdp.iter().all(|v| v.is_none()));
    }

    /// Rows are independent: one radial's phase must not leak into the next one's fit.
    #[test]
    fn radials_do_not_bleed_into_each_other() {
        let gate_count = 20;
        let mut phi = vec![Some(0.0f32); gate_count]; // flat: KDP 0
        phi.extend((0..gate_count).map(|g| Some(g as f32 * 4.0))); // ramp: KDP 8 at 0.25 km
        let kdp = from_differential_phase(&phi, 2, gate_count, 0.25);
        assert!((kdp[10].unwrap() - 0.0).abs() < 1e-3);
        assert!((kdp[gate_count + 10].unwrap() - 8.0).abs() < 1e-3);
    }
}
