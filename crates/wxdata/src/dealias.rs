//! Region-based Doppler velocity dealiasing — a simplified port of Py-ART's
//! `dealias_region_based`.
//!
//! Aliased ("folded") velocity wraps at the Nyquist velocity ±V_ny: a target moving
//! faster than V_ny reads as a value of the opposite sign. We segment the sweep into
//! regions of internally-continuous velocity (neighbors within half a Nyquist interval),
//! then unfold each region by an integer number of 2·V_ny steps so that velocity is
//! continuous across region boundaries. The largest region anchors at zero folds; every
//! other region is unfolded relative to the anchor.
//!
//! Region offsets are solved globally rather than by traversal order. Each pair of touching
//! regions is collapsed to a single voted fold difference plus a confidence — the number of
//! boundary gate pairs that backed it — and the sweep is solved along a maximum spanning tree
//! over those edges, strongest constraint first, followed by a bounded refinement pass that
//! re-checks every region against *all* its neighbours rather than just its tree parent. Folds
//! past a single interval fall out of the arithmetic, so multi-fold fields (very high-shear
//! tornadic couplets past 2·V_ny) no longer come apart at a thin early boundary the way the
//! previous greedy BFS could.
//!
//! ponytail: still one sweep at a time, no sequential second pass over the volume.
//!
//! Two things it does do beyond the simplest version of that idea. Region folds are chosen by a
//! *vote* over the gate pairs along a boundary rather than by averaging them: one stray pair on a
//! long boundary used to be able to drag the mean across the rounding line and fold a whole
//! region wrongly. And the anchor region can be tied to a reference field — the previous sweep's
//! already-dealiased velocity — instead of being assumed unfolded, which is what keeps a storm
//! that is genuinely moving faster than the Nyquist velocity from snapping back to zero folds the
//! moment it becomes the largest region in the sweep.
//!
//! ponytail: one previous sweep of continuity, not a 4D UNRAVEL-style solve.

/// The fold with the most votes. Ties break toward the smaller unfold: with no evidence either
/// way, the answer that moves the data least is the safer one.
fn winning_fold(votes: std::collections::HashMap<i32, u32>) -> i32 {
    votes
        .into_iter()
        .max_by_key(|&(fold, n)| (n, -fold.abs()))
        .map(|(fold, _)| fold)
        .unwrap_or(0)
}

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
pub fn dealias(
    vel: &[Option<f32>],
    az_bins: usize,
    gate_count: usize,
    nyquist: f32,
) -> Vec<Option<f32>> {
    dealias_with_reference(vel, az_bins, gate_count, nyquist, None)
}

/// As [`dealias`], with an optional continuity reference on the same grid — normally the previous
/// sweep's dealiased output.
///
/// The reference does one job: it decides how many folds the *anchor* region carries, instead of
/// the anchor being assumed unfolded. Everything else is unchanged, because every other region is
/// already solved relative to the anchor. A sweep whose fastest air is genuinely past the Nyquist
/// velocity is stable across volumes this way rather than snapping to zero whenever the fast
/// region happens to become the biggest one.
pub fn dealias_with_reference(
    vel: &[Option<f32>],
    az_bins: usize,
    gate_count: usize,
    nyquist: f32,
    reference: Option<&[Option<f32>]>,
) -> Vec<Option<f32>> {
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
                // A labelled cell always has a velocity (that is what labelling means); skip
                // rather than assert it, so a future labelling change degrades instead of panics.
                let Some(cv) = vel[idx(caz, cg)] else {
                    continue;
                };
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
            let Some(va) = vel[i] else { continue };
            sizes[ra] += 1;
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
                let Some(vb) = vel[ni] else { continue };
                edges[ra].push((rb, va, vb));
                edges[rb].push((ra, vb, va));
            }
        }
    }

    // --- 3. Global fold optimizer. ---
    // Every pair of touching regions votes, over all their shared gate pairs, for the integer
    // fold difference that would make the boundary continuous. That gives one weighted edge per
    // region pair, and the weight is evidence: a boundary of four hundred agreeing gates is a far
    // better constraint than one of three. Solving the sweep is then a matter of believing the
    // strongest constraints first — a maximum spanning tree over the region graph, rooted at the
    // anchor — and letting the weak ones fall out of the arithmetic instead of the traversal
    // order. The old greedy BFS took whatever edge it happened to reach first, which is why a
    // multi-fold field could come apart at a seam: one thin boundary early in the walk fixed a
    // region, and every region behind it inherited the mistake.
    let mut pair_votes: std::collections::HashMap<(usize, usize), std::collections::HashMap<i32, u32>> =
        std::collections::HashMap::new();
    for (ra, list) in edges.iter().enumerate() {
        for &(rb, v_self, v_nb) in list {
            if ra >= rb {
                continue; // each pair recorded once, oriented low -> high
            }
            // fold = unfold[rb] - unfold[ra] that makes this gate pair continuous.
            let fold = ((v_self as f64 - v_nb as f64) / interval as f64).round() as i32;
            *pair_votes
                .entry((ra, rb))
                .or_default()
                .entry(fold)
                .or_insert(0) += 1;
        }
    }
    // Collapse each pair to its winning fold plus a confidence = how many gate pairs backed it.
    // adjacency[r] = (neighbor, fold applied as unfold[neighbor] - unfold[r], confidence).
    let mut adjacency: Vec<Vec<(usize, i32, u32)>> = vec![Vec::new(); region_count];
    for ((ra, rb), votes) in pair_votes {
        let confidence = votes.values().copied().max().unwrap_or(0);
        let fold = winning_fold(votes);
        adjacency[ra].push((rb, fold, confidence));
        adjacency[rb].push((ra, -fold, confidence));
    }

    let mut unfold = vec![0i32; region_count];
    let mut solved = vec![false; region_count];
    let Some(anchor) = (0..region_count).max_by_key(|&r| sizes[r]) else {
        return vel.to_vec();
    };
    // Anchor fold: zero unless a reference field says otherwise. Same vote, against the previous
    // sweep's value at each of the anchor's own gates.
    unfold[anchor] = reference
        .filter(|r| r.len() == n)
        .map(|refv| {
            let mut votes: std::collections::HashMap<i32, u32> = std::collections::HashMap::new();
            for i in 0..n {
                if labels[i] != anchor {
                    continue;
                }
                let (Some(v), Some(rv)) = (vel[i], refv[i]) else {
                    continue;
                };
                let fold = ((rv as f64 - v as f64) / interval as f64).round() as i32;
                *votes.entry(fold).or_insert(0) += 1;
            }
            winning_fold(votes)
        })
        .unwrap_or(0);

    // Maximum spanning tree by confidence, grown Prim-style from the anchor. Region ids break
    // ties so the result does not depend on hash iteration order.
    let mut frontier: Vec<(u32, usize, usize, i32)> = Vec::new(); // (confidence, from, to, fold)
    let push_edges = |frontier: &mut Vec<(u32, usize, usize, i32)>, r: usize| {
        for &(nb, fold, confidence) in &adjacency[r] {
            frontier.push((confidence, r, nb, fold));
        }
    };
    solved[anchor] = true;
    push_edges(&mut frontier, anchor);
    while !frontier.is_empty() {
        // Best remaining edge into an unsolved region.
        let best = frontier
            .iter()
            .enumerate()
            .filter(|(_, &(_, _, to, _))| !solved[to])
            .max_by_key(|(_, &(confidence, from, to, _))| {
                (confidence, std::cmp::Reverse(from), std::cmp::Reverse(to))
            })
            .map(|(i, &e)| (i, e));
        let Some((_, (_, from, to, fold))) = best else {
            break; // nothing reachable left; disconnected regions keep their zero offset
        };
        unfold[to] = unfold[from] + fold;
        solved[to] = true;
        frontier.retain(|&(_, _, t, _)| !solved[t]);
        push_edges(&mut frontier, to);
    }

    // Bounded refinement. The tree used one edge per region; every other boundary is now evidence
    // that region's offset can be checked against. Re-vote each region against all its solved
    // neighbours, weighted by boundary confidence, and stop as soon as a pass changes nothing.
    // ponytail: ten passes, not to convergence — a field that is still moving after ten passes is
    // oscillating between two equally-supported answers, and picking one is as good as the other.
    for _ in 0..10 {
        let mut changed = false;
        for r in 0..region_count {
            if r == anchor || !solved[r] {
                continue;
            }
            let mut votes: std::collections::HashMap<i32, u32> = std::collections::HashMap::new();
            for &(nb, fold, confidence) in &adjacency[r] {
                if solved[nb] {
                    *votes.entry(unfold[nb] - fold).or_insert(0) += confidence;
                }
            }
            if votes.is_empty() {
                continue;
            }
            let best = winning_fold(votes);
            if best != unfold[r] {
                unfold[r] = best;
                changed = true;
            }
        }
        if !changed {
            break;
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
        assert!(
            folded.iter().any(|v| v.unwrap() < 0.0),
            "test ramp should fold"
        );

        let out = dealias(&folded, 1, 10, nyq);
        for (o, t) in out.iter().zip(&truth) {
            let got = o.unwrap();
            // Recovered up to a whole-field constant fold (anchor region may sit at 0).
            let err = (got - t).rem_euclid(2.0 * nyq);
            let err = err.min(2.0 * nyq - err);
            assert!(err < 0.5, "gate expected ~{t}, got {got}");
        }
    }

    // The region walk assumes a labelled gate has a velocity. These are the two fields where
    // that assumption is nearest the edge — no data at all, and exactly one gate of it.
    #[test]
    fn degenerate_fields_come_back_unchanged() {
        let empty: Vec<Option<f32>> = vec![None; 12];
        assert_eq!(dealias(&empty, 3, 4, 25.0), empty);

        let mut one = vec![None; 12];
        one[5] = Some(7.0);
        assert_eq!(dealias(&one, 3, 4, 25.0), one);
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

    /// Build a folded sweep from a truth field: `v_folded = ((v + nyq) mod 2nyq) - nyq`.
    fn fold(truth: &[f32], nyq: f32) -> Vec<Option<f32>> {
        truth
            .iter()
            .map(|&v| Some(((v + nyq).rem_euclid(2.0 * nyq)) - nyq))
            .collect()
    }

    /// Compare two fields up to one whole-field constant fold, which is all an unreferenced
    /// dealias can ever recover.
    fn matches_up_to_a_constant_fold(got: &[Option<f32>], truth: &[f32], nyq: f32) -> bool {
        let interval = 2.0 * nyq;
        let Some(offset) = got.first().and_then(|g| *g).map(|g| truth[0] - g) else {
            return false;
        };
        let offset = (offset / interval).round() * interval;
        got.iter()
            .zip(truth)
            .all(|(g, t)| g.is_none_or(|g| (g + offset - t).abs() < 0.5))
    }

    /// A tornadic couplet: inbound and outbound maxima either side of a shear line, both past the
    /// Nyquist velocity so both fold. The shear line itself is a real discontinuity — the point of
    /// the fixture is that the dealiaser must not treat it as a fold.
    #[test]
    fn unfolds_an_aliased_couplet_across_a_shear_line() {
        let nyq = 25.0f32;
        let (az_bins, gates) = (36, 20);
        let mut truth = vec![0.0f32; az_bins * gates];
        for az in 0..az_bins {
            for g in 0..gates {
                // Inbound on one side of the couplet, outbound on the other, peaking at 40 m/s —
                // well past the 25 m/s Nyquist, so the field folds on both sides.
                let across = if az < az_bins / 2 { -1.0 } else { 1.0 };
                let ramp = (g as f32 / (gates - 1) as f32) * 40.0;
                truth[az * gates + g] = across * ramp;
            }
        }
        let folded = fold(&truth, nyq);
        assert!(
            folded
                .iter()
                .enumerate()
                .any(|(i, v)| { (v.unwrap() - truth[i]).abs() > 1.0 }),
            "fixture should actually fold"
        );
        let out = dealias(&folded, az_bins, gates, nyq);
        // Every gate recovered, up to the one constant fold the anchor choice leaves free.
        assert!(
            matches_up_to_a_constant_fold(&out, &truth, nyq),
            "couplet not recovered"
        );
    }

    /// The failure the vote exists to prevent: a long boundary whose gate pairs mostly agree on
    /// one fold, plus a minority sitting on a shear line that pull the *mean* over the rounding
    /// line. Averaging folds the region the wrong way; voting does not.
    #[test]
    fn a_minority_of_shear_pairs_cannot_outweigh_the_boundary() {
        // Nine boundary gate pairs agree on fold 1; three sit on a shear line and ask for 3.
        // The mean of those is 1.5 and rounds away to 2, which is what the old code did. The vote
        // holds, because the outliers have to outnumber the majority, not merely outweigh it.
        let ideal = [1.0_f64; 9]
            .into_iter()
            .chain([3.0_f64; 3])
            .collect::<Vec<_>>();
        let mean = ideal.iter().sum::<f64>() / ideal.len() as f64;
        assert_eq!(mean.round() as i32, 2, "the mean really is dragged across");

        let mut votes = std::collections::HashMap::new();
        for v in &ideal {
            *votes.entry(v.round() as i32).or_insert(0) += 1;
        }
        assert_eq!(winning_fold(votes), 1);
    }

    #[test]
    fn an_empty_or_tied_vote_moves_the_data_least() {
        assert_eq!(winning_fold(std::collections::HashMap::new()), 0);
        let tied = std::collections::HashMap::from([(0, 4), (3, 4)]);
        assert_eq!(winning_fold(tied), 0);
        let tied_both_folded = std::collections::HashMap::from([(-1, 2), (4, 2)]);
        assert_eq!(winning_fold(tied_both_folded), -1);
    }

    /// Continuity: a field whose *whole* content is past the Nyquist velocity has no internal
    /// evidence of how many folds it carries, so an unreferenced dealias anchors it at zero. Given
    /// the previous sweep it should keep the folds instead of snapping back.
    #[test]
    fn a_reference_sweep_anchors_folds_the_field_cannot_infer() {
        let nyq = 25.0f32;
        let (az_bins, gates) = (4, 8);
        // Uniformly 60 m/s outbound: one region, folds once, and nothing inside the sweep says so.
        let truth = vec![60.0f32; az_bins * gates];
        let folded = fold(&truth, nyq);
        let bare = dealias(&folded, az_bins, gates, nyq);
        assert!(
            (bare[0].unwrap() - 60.0).abs() > 1.0,
            "without a reference there is nothing to anchor to"
        );
        // The previous sweep had the same air, correctly unfolded.
        let reference: Vec<Option<f32>> = truth.iter().map(|v| Some(*v)).collect();
        let out = dealias_with_reference(&folded, az_bins, gates, nyq, Some(&reference));
        for v in out.iter().flatten() {
            assert!((v - 60.0).abs() < 0.5, "expected 60 m/s, got {v}");
        }
    }

    /// Multi-fold recovery: velocity climbing past two whole Nyquist intervals, so neighbouring
    /// regions differ by two folds rather than one. This is what the greedy walk was documented
    /// as unable to hold.
    #[test]
    fn unfolds_a_field_that_folds_more_than_once() {
        let nyq = 20.0f32;
        let (az_bins, gates) = (16, 24);
        let mut truth = vec![0.0f32; az_bins * gates];
        for az in 0..az_bins {
            for g in 0..gates {
                // 0 to 95 m/s outbound: nearly three Nyquist intervals from end to end.
                truth[az * gates + g] = (g as f32 / (gates - 1) as f32) * 95.0;
            }
        }
        let folded = fold(&truth, nyq);
        assert!(
            folded.iter().any(|v| v.unwrap() < 0.0),
            "fixture should fold"
        );
        let out = dealias(&folded, az_bins, gates, nyq);
        assert!(
            matches_up_to_a_constant_fold(&out, &truth, nyq),
            "multi-fold ramp not recovered"
        );
    }

    /// The optimizer walks a hash map of region pairs; the answer must not depend on the order
    /// that map happens to iterate in.
    #[test]
    fn the_same_sweep_dealiases_the_same_way_every_time() {
        let nyq = 22.0f32;
        let (az_bins, gates) = (24, 16);
        let mut truth = vec![0.0f32; az_bins * gates];
        for az in 0..az_bins {
            for g in 0..gates {
                let a = az as f32 / az_bins as f32 * std::f32::consts::TAU;
                truth[az * gates + g] = a.sin() * 48.0 + g as f32 * 1.7;
            }
        }
        let folded = fold(&truth, nyq);
        let first = dealias(&folded, az_bins, gates, nyq);
        for _ in 0..8 {
            assert_eq!(dealias(&folded, az_bins, gates, nyq), first);
        }
    }

    /// A reference of the wrong shape, or one full of holes, must be ignored rather than trusted.
    #[test]
    fn a_useless_reference_changes_nothing() {
        let nyq = 25.0f32;
        let folded = fold(&[10.0, 20.0, 30.0, 40.0], nyq);
        let bare = dealias(&folded, 1, 4, nyq);
        let wrong_size = vec![Some(60.0f32); 99];
        assert_eq!(
            dealias_with_reference(&folded, 1, 4, nyq, Some(&wrong_size)),
            bare
        );
        let all_holes = vec![None; 4];
        assert_eq!(
            dealias_with_reference(&folded, 1, 4, nyq, Some(&all_holes)),
            bare
        );
    }
}
