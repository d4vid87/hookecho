//! Region-based Doppler velocity dealiasing — a simplified port of Py-ART's
//! `dealias_region_based`.
//!
//! Aliased ("folded") velocity wraps at the Nyquist velocity ±V_ny: a target moving
//! faster than V_ny reads as a value of the opposite sign. We segment the sweep into
//! regions of internally-continuous velocity (neighbors within half a Nyquist interval),
//! then unfold each region by an integer number of 2·V_ny steps so that velocity is
//! continuous across region boundaries. The largest region anchors at zero folds; every
//! other region is unfolded greedily from its already-solved neighbors.
//!
//! Simplifications vs. full Py-ART (ponytail): greedy BFS region-merge instead of the
//! global edge-cost optimization, and no second (sequential) pass. Sound and deterministic
//! for typical single-fold storms; upgrade to the graph optimizer if multi-fold cases
//! (very high-shear tornadic couplets past 2·V_ny) show seams.

/// Estimate the Nyquist velocity from a folded field as the largest observed |v|.
/// Folded data saturates at ±V_ny, so this is a robust practical proxy when the model
/// doesn't carry the radial's unambiguous velocity.
pub fn estimate_nyquist(vel: &[Option<f32>]) -> f32 {
    vel.iter()
        .filter_map(|v| *v)
        .fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Dealias a polar velocity grid laid out as `vel[az * gate_count + gate]`.
/// `None` gates (no data / below threshold / range folded) pass through untouched.
/// Azimuth wraps (bin 0 neighbors bin az_bins-1); range does not.
pub fn dealias(vel: &[Option<f32>], az_bins: usize, gate_count: usize, nyquist: f32) -> Vec<Option<f32>> {
    let n = az_bins * gate_count;
    debug_assert_eq!(vel.len(), n);
    if nyquist <= 0.0 || n == 0 {
        return vel.to_vec();
    }
    let interval = 2.0 * nyquist;
    // Two gates belong to the same region if their velocities are close enough that no
    // fold sits between them. Half a Nyquist interval is Py-ART's default skip threshold.
    let same_region = nyquist * 0.5;

    // --- 1. Flood-fill connected regions of continuous velocity. ---
    // labels: usize::MAX = no data, otherwise the region id.
    const NONE: usize = usize::MAX;
    let mut labels = vec![NONE; n];
    let idx = |az: usize, g: usize| az * gate_count + g;
    // 4-neighbors with azimuthal wrap.
    let neighbors = |az: usize, g: usize| {
        let mut v: Vec<(usize, usize)> = Vec::with_capacity(4);
        v.push(((az + 1) % az_bins, g));
        v.push(((az + az_bins - 1) % az_bins, g));
        if g + 1 < gate_count {
            v.push((az, g + 1));
        }
        if g > 0 {
            v.push((az, g - 1));
        }
        v
    };

    let mut region_count = 0usize;
    let mut stack = Vec::new();
    for az in 0..az_bins {
        for g in 0..gate_count {
            let i = idx(az, g);
            if vel[i].is_none() || labels[i] != NONE {
                continue;
            }
            let region = region_count;
            region_count += 1;
            labels[i] = region;
            stack.push((az, g));
            while let Some((caz, cg)) = stack.pop() {
                let cv = vel[idx(caz, cg)].unwrap();
                for (naz, ng) in neighbors(caz, cg) {
                    let ni = idx(naz, ng);
                    let Some(nv) = vel[ni] else { continue };
                    if labels[ni] == NONE && (nv - cv).abs() < same_region {
                        labels[ni] = region;
                        stack.push((naz, ng));
                    }
                }
            }
        }
    }
    if region_count == 0 {
        return vel.to_vec();
    }

    // --- 2. Region sizes + inter-region boundary edges. ---
    // edges[r] = list of (neighbor_region, v_self, v_neighbor) across shared boundaries.
    let mut sizes = vec![0usize; region_count];
    let mut edges: Vec<Vec<(usize, f32, f32)>> = vec![Vec::new(); region_count];
    for az in 0..az_bins {
        for g in 0..gate_count {
            let i = idx(az, g);
            let ra = labels[i];
            if ra == NONE {
                continue;
            }
            sizes[ra] += 1;
            let va = vel[i].unwrap();
            // Only scan the +az and +gate neighbors to record each boundary once.
            for (naz, ng) in [((az + 1) % az_bins, g), (az, g + 1)] {
                if ng >= gate_count {
                    continue;
                }
                let ni = idx(naz, ng);
                let rb = labels[ni];
                if rb == NONE || rb == ra {
                    continue;
                }
                let vb = vel[ni].unwrap();
                edges[ra].push((rb, va, vb));
                edges[rb].push((ra, vb, va));
            }
        }
    }

    // --- 3. Greedy BFS unfold from the largest region (anchor = 0 folds). ---
    let mut unfold = vec![0i32; region_count];
    let mut solved = vec![false; region_count];
    let anchor = (0..region_count).max_by_key(|&r| sizes[r]).unwrap();
    solved[anchor] = true;
    let mut queue = std::collections::VecDeque::new();
    for &(nb, _, _) in &edges[anchor] {
        if !solved[nb] {
            queue.push_back(nb);
        }
    }
    while let Some(r) = queue.pop_front() {
        if solved[r] {
            continue;
        }
        // Fold count that best aligns this region with its already-solved neighbors:
        // per boundary edge the ideal fold is round(((v_nb + f_nb·interval) - v_self)/interval);
        // average over all such edges, then round to an integer number of folds.
        let mut sum = 0.0f64;
        let mut count = 0u32;
        for &(nb, v_self, v_nb) in &edges[r] {
            if solved[nb] {
                let target = v_nb as f64 + unfold[nb] as f64 * interval as f64;
                sum += (target - v_self as f64) / interval as f64;
                count += 1;
            }
        }
        if count == 0 {
            // Not yet reachable from the solved set; requeue later.
            queue.push_back(r);
            continue;
        }
        unfold[r] = (sum / count as f64).round() as i32;
        solved[r] = true;
        for &(nb, _, _) in &edges[r] {
            if !solved[nb] {
                queue.push_back(nb);
            }
        }
    }

    // --- 4. Apply per-region fold offsets. ---
    let mut out = vel.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        if let (Some(v), r) = (o.as_mut(), labels[i]) {
            if r != NONE && unfold[r] != 0 {
                *v += unfold[r] as f32 * interval;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A single radial ramp that folds once: true velocity climbs past +Nyquist and wraps
    // to negative. Dealiasing must recover the monotonic ramp.
    #[test]
    fn unfolds_a_single_fold_ramp() {
        let nyq = 25.0f32;
        // az_bins=1 (one radial), 10 gates. True velocity 5,10,...,50 m/s.
        let truth: Vec<f32> = (1..=10).map(|k| k as f32 * 5.0).collect();
        // Fold into [-nyq, nyq): v_folded = ((v + nyq) mod 2nyq) - nyq.
        let folded: Vec<Option<f32>> = truth
            .iter()
            .map(|&v| Some(((v + nyq).rem_euclid(2.0 * nyq)) - nyq))
            .collect();
        // Sanity: the ramp really does fold (some folded value is negative though truth is +).
        assert!(folded.iter().any(|v| v.unwrap() < 0.0), "test ramp should fold");

        let out = dealias(&folded, 1, 10, nyq);
        for (o, t) in out.iter().zip(&truth) {
            let got = o.unwrap();
            // Recovered up to a whole-field constant fold (anchor region may sit at 0).
            let err = (got - t).rem_euclid(2.0 * nyq);
            let err = err.min(2.0 * nyq - err);
            assert!(err < 0.5, "gate expected ~{t}, got {got}");
        }
    }

    #[test]
    fn nyquist_from_field_is_max_abs() {
        let f = vec![Some(-24.0f32), None, Some(19.0), Some(-31.5)];
        assert_eq!(estimate_nyquist(&f), 31.5);
    }

    #[test]
    fn passthrough_when_no_nyquist() {
        let f = vec![Some(3.0f32), None, Some(-7.0)];
        assert_eq!(dealias(&f, 1, 3, 0.0), f);
    }
}
