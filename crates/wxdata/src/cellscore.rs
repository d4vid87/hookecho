//! One number for "how bad is this storm", so a busy day sorts itself.
//!
//! The attributes table already lists everything the radar knows about a cell, but on a day with
//! thirty cells on screen the question is which row to click first, and no single column answers
//! it: the biggest hail is not always the rotating one, and the tallest top is often neither.
//!
//! The score blends the three things a warning decision actually rests on — the machine-learning
//! probability of severe (ProbSevere), how hard the storm is rotating (Vrot from the client-side
//! couplet detector), and how big the hail estimate is (MESH, which the Level 3 cell table
//! publishes as `hail_in`) — then nudges for the radar's own TVS/MESO flags.
//!
//! Components that are missing are dropped and the rest renormalized, rather than counted as
//! zero: a cell outside the ProbSevere feed's coverage is unknown there, not benign.

use crate::level3::Cell;
use crate::overlay::GeoFeature;
use crate::rotation::CoupletHit;

/// A couplet or a storm object belongs to the cell it is nearest, within this radius. Same 15 km
/// the ZDR-column badge uses — a storm's circulation is not 15 km from its centroid.
const CLAIM_KM: f64 = 15.0;

/// Linear ramp from `lo` (0) to `hi` (100), clamped.
fn ramp(v: f32, lo: f32, hi: f32) -> f32 {
    (((v - lo) / (hi - lo)) * 100.0).clamp(0.0, 100.0)
}

/// Flat-earth kilometres. Distances here are tens of km, where the error is well under a percent.
fn km(a_lon: f64, a_lat: f64, b_lon: f64, b_lat: f64) -> f64 {
    let dy = (a_lat - b_lat) * 111.0;
    let dx = (a_lon - b_lon) * 111.0 * b_lat.to_radians().cos();
    (dx * dx + dy * dy).sqrt()
}

/// Composite severity 0–100 for one cell.
///
/// `prob_pct` is ProbSevere's dominant probability and `vrot_ms` the rotational velocity of the
/// couplet claimed by this cell; both `None` when nothing covers the storm.
pub fn severity(cell: &Cell, prob_pct: Option<u8>, vrot_ms: Option<f32>) -> u8 {
    // Weights: the probability is the most informative single input, rotation and hail split the
    // rest. Thresholds are the operational ones — 10 m/s of Vrot is noise, 40 is a strong
    // mesocyclone; half-inch hail is the severe criterion, three inches is a destructive day.
    // POSH — the radar's own probability of severe hail — stands in for ProbSevere where the feed
    // has no storm object over the cell, which is most of the time: the layer is off by default.
    // It is weaker evidence than a machine-learning severe probability, so it carries less weight.
    let parts = [
        (0.4, prob_pct.map(|p| p as f32)),
        (0.3, vrot_ms.map(|v| ramp(v, 10.0, 40.0))),
        (0.3, cell.hail_in.map(|h| ramp(h, 0.5, 3.0))),
        (
            0.2,
            prob_pct
                .is_none()
                .then(|| cell.posh.map(|p| p as f32))
                .flatten(),
        ),
    ];
    let (sum, weight) = parts
        .iter()
        .filter_map(|&(w, v)| v.map(|v| (w * v, w)))
        .fold((0.0, 0.0), |(s, w), (a, b)| (s + a, w + b));
    // Reflectivity is the one field every cell has, and it is the weakest evidence of all: a
    // bright echo is a storm, not a threat. So it rides alongside the hazard blend at a fixed
    // small share rather than being renormalized with it — otherwise a cell with nothing but
    // 57 dBZ scored its full ramp and outranked storms carrying a measured hail core, which is
    // what a live KAMA table did. With no hazard signal at all it is halved: an unknown cell
    // belongs in the middle of the table, not the top.
    let dbz = cell.max_dbz.map(|d| ramp(d, 40.0, 70.0)).unwrap_or(0.0);
    let base = if weight > 0.0 {
        0.85 * (sum / weight) + 0.15 * dbz
    } else {
        0.5 * dbz
    };
    // The radar's own algorithm flags are weak evidence on their own and strong confirmation on
    // top of a score, so they add rather than blend.
    let bump = if cell.tvs.is_some() {
        15.0
    } else if cell.meso.is_some() {
        8.0
    } else {
        0.0
    };
    (base + bump).clamp(0.0, 100.0).round() as u8
}

/// Score every cell, claiming the ProbSevere polygon it sits inside and the nearest couplet.
///
/// Kept here rather than at the call site so the join is testable and the app wiring is one line.
pub fn score_all(cells: &[Cell], probsevere: &[GeoFeature], couplets: &[CoupletHit]) -> Vec<u8> {
    cells
        .iter()
        .map(|c| {
            let prob = crate::overlay::hit(probsevere, c.lon, c.lat).and_then(dominant_pct);
            let vrot = couplets
                .iter()
                .filter(|h| km(h.lon, h.lat, c.lon, c.lat) <= CLAIM_KM)
                .map(|h| h.vrot_ms)
                .fold(None::<f32>, |m, v| Some(m.map_or(v, |m| m.max(v))));
            severity(c, prob, vrot)
        })
        .collect()
}

/// The percentage out of a ProbSevere badge title ("Svr 78%" → 78).
pub fn dominant_pct(f: &GeoFeature) -> Option<u8> {
    f.title
        .rsplit(' ')
        .next()?
        .strip_suffix('%')?
        .parse::<u8>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level3::CellKind;

    fn cell(lon: f64, lat: f64) -> Cell {
        let mut c = Cell::new(CellKind::Storm, lon, lat, "A1".into(), "A1".into());
        c.max_dbz = Some(55.0);
        c
    }

    #[test]
    fn missing_inputs_are_unknown_rather_than_zero() {
        let mut c = cell(-97.5, 35.0);
        c.hail_in = Some(3.0);
        // Three inches of hail alongside 55 dBZ: the hail dominates its weight, and the weak
        // reflectivity component pulls the blend down only a little.
        assert!(
            severity(&c, None, None) >= 85,
            "got {}",
            severity(&c, None, None)
        );
        // A low probability alongside it does drag the blend down.
        assert!(severity(&c, Some(10), None) < 60);
    }

    #[test]
    fn a_bare_cell_still_ranks_on_reflectivity_and_flags() {
        let c = cell(-97.5, 35.0);
        let plain = severity(&c, None, None);
        // Nothing but a 55 dBZ echo: halfway up the 40–70 ramp, halved because that is all we
        // know about it.
        assert_eq!(plain, 25);
        let mut tvs = cell(-97.5, 35.0);
        tvs.tvs = Some("TVS".into());
        assert_eq!(severity(&tvs, None, None), plain + 15);
    }

    #[test]
    fn a_measured_hail_core_outranks_a_merely_bright_echo() {
        // The case a live KAMA table produced: a 57 dBZ cell with nothing else known was sorting
        // above a storm carrying a 1.2 in hail estimate and POSH 40.
        let mut bright = cell(-101.9, 35.5);
        bright.max_dbz = Some(57.0);
        let mut hail = cell(-101.8, 35.4);
        hail.max_dbz = Some(59.0);
        hail.hail_in = Some(1.2);
        hail.posh = Some(40);
        assert!(severity(&hail, None, None) > severity(&bright, None, None));
    }

    #[test]
    fn the_join_claims_the_polygon_it_sits_in_and_the_couplet_beside_it() {
        let cells = [cell(-97.45, 35.05), cell(-90.0, 40.0)];
        let feats = crate::probsevere::parse_probsevere(
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[-97.5,35.0],[-97.4,35.0],[-97.4,35.1],[-97.5,35.1],[-97.5,35.0]]]},
                 "properties":{"ID":"1","ProbSevere":"90","ProbTor":"5","ProbHail":"10","ProbWind":"20"}}]}"#,
        )
        .unwrap();
        let couplets = [CoupletHit {
            lon: -97.44,
            lat: 35.06,
            vrot_ms: 40.0,
            g2g_ms: 80.0,
            range_km: 30.0,
            gates: 6,
        }];
        let s = score_all(&cells, &feats, &couplets);
        // Inside the polygon and next to the couplet: 90% at weight 0.4 and a maxed Vrot ramp at
        // 0.3, renormalized over the two present components.
        assert_eq!(s[0], 88);
        // Six hundred km away, neither claims it — the halved reflectivity term only.
        assert_eq!(s[1], 25);
    }
}
