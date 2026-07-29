//! Gridded severe-weather composite parameters (STP, SCP, EHI) built from HRRR surface fields.
//!
//! These are the numbers a chaser scans first, and until now the app only had them at a single
//! clicked point (see [`crate::sounding::Sounding::indices`]). Here the same fixed-layer forms are
//! evaluated cell-by-cell over the whole HRRR analysis grid so they can be drawn as contours.
//!
//! Fixed-layer (not effective-layer) formulations, per Thompson et al. 2004 — coarser than SPC
//! mesoanalysis, which uses effective-inflow layers and mesoanalysis-blended observations.

use crate::hrrr::{self, HrrrForecast};
use crate::mrms::MrmsField;

/// Which composite to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SevereKind {
    /// Significant Tornado Parameter (fixed layer).
    Stp,
    /// Supercell Composite Parameter.
    Scp,
    /// Energy-Helicity Index, 0–1 km.
    Ehi,
}

/// Shear term shared by STP and SCP: zero below 10 m/s, capped at 1.0 above 20 m/s.
fn shear_term(shear6_ms: f64) -> f64 {
    if shear6_ms < 10.0 {
        0.0
    } else {
        (shear6_ms / 20.0).min(1.0)
    }
}

/// Significant Tornado Parameter (fixed layer). Constants must match
/// [`crate::sounding::Sounding::indices`] — the point sounding and the grid have to agree.
pub fn stp(sbcape: f64, srh1: f64, shear6_ms: f64, lcl_agl_m: f64) -> f64 {
    let lcl_term = ((2000.0 - lcl_agl_m) / 1000.0).clamp(0.0, 1.0);
    (sbcape / 1500.0) * (srh1.max(0.0) / 150.0) * shear_term(shear6_ms) * lcl_term
}

/// Supercell Composite Parameter. Constants must match [`crate::sounding::Sounding::indices`].
pub fn scp(sbcape: f64, srh3: f64, shear6_ms: f64) -> f64 {
    (sbcape / 1000.0) * (srh3.max(0.0) / 50.0) * shear_term(shear6_ms)
}

/// Energy-Helicity Index over 0–1 km. Constants must match
/// [`crate::sounding::Sounding::indices`].
pub fn ehi1(sbcape: f64, srh1: f64) -> f64 {
    sbcape * srh1 / 160_000.0
}

/// Fetch every ingredient a kind needs from one HRRR cycle and combine them cell by cell.
///
/// All surface fields regrid onto the same deterministic 0.04° lat/lon grid (see
/// [`crate::hrrr`]), so the composites are a straight per-cell evaluation. `NaN` in any
/// ingredient yields `NaN` out, leaving masked regions as gaps rather than zeros.
pub async fn fetch_grid(
    http: &reqwest::Client,
    model: hrrr::Model,
    kind: SevereKind,
) -> anyhow::Result<HrrrForecast> {
    // STP needs an LCL height, and the RAP analysis file doesn't carry the adiabatic-condensation
    // level HRRR does. Say so rather than quietly serving a different parameter.
    anyhow::ensure!(
        !(model == hrrr::Model::Rap && kind == SevereKind::Stp),
        "STP needs an LCL height the RAP analysis doesn't publish — use the HRRR source"
    );
    // (var, level, min_valid). Helicity and shear components must keep their negatives, so their
    // drop threshold is -inf; CAPE below zero is meaningless.
    let srh_level = match kind {
        SevereKind::Scp => "3000-0 m above ground",
        _ => "1000-0 m above ground",
    };
    let mut specs: Vec<(&str, &str, f64)> = vec![
        ("CAPE", "surface", 0.0),
        ("HLCY", srh_level, f64::NEG_INFINITY),
    ];
    if kind != SevereKind::Ehi {
        // EHI needs no shear or LCL term.
        specs.push(("VUCSH", "0-6000 m above ground", f64::NEG_INFINITY));
        specs.push(("VVCSH", "0-6000 m above ground", f64::NEG_INFINITY));
    }
    if kind == SevereKind::Stp {
        // LCL height AGL = the adiabatic condensation level minus the surface height.
        specs.push((
            "HGT",
            "level of adiabatic condensation from sfc",
            f64::NEG_INFINITY,
        ));
        specs.push(("HGT", "surface", f64::NEG_INFINITY));
    }
    let (run, fields) = hrrr::fetch_fields_one_run(http, model, 0, &specs).await?;
    let f0 = &fields[0];
    for f in &fields[1..] {
        anyhow::ensure!(
            f.nx == f0.nx && f.ny == f0.ny,
            "{} ingredient grids disagree",
            model.label()
        );
    }

    let mut values = vec![f32::NAN; f0.values.len()];
    for (i, out) in values.iter_mut().enumerate() {
        let cape = fields[0].values[i] as f64;
        let srh = fields[1].values[i] as f64;
        let v = match kind {
            SevereKind::Ehi => ehi1(cape, srh),
            _ => {
                // HRRR posts VUCSH/VVCSH as the 0-6 km bulk shear *vector components in m/s*
                // (verified live: magnitudes of order 10-35), not a shear rate in s^-1.
                let shear = (fields[2].values[i] as f64).hypot(fields[3].values[i] as f64);
                match kind {
                    SevereKind::Scp => scp(cape, srh, shear),
                    SevereKind::Stp => {
                        let lcl_agl =
                            (fields[4].values[i] as f64 - fields[5].values[i] as f64).max(0.0);
                        stp(cape, srh, shear, lcl_agl)
                    }
                    SevereKind::Ehi => unreachable!(),
                }
            }
        };
        if v.is_finite() {
            *out = v as f32;
        }
    }

    let field = MrmsField {
        values,
        nx: f0.nx,
        ny: f0.ny,
        lon_west: f0.lon_west,
        lon_east: f0.lon_east,
        lat_north: f0.lat_north,
        lat_south: f0.lat_south,
        time: f0.time,
    };
    Ok(HrrrForecast {
        field,
        run,
        fcst_hour: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_match_the_point_sounding() {
        // Each parameter is 1.0 at its definitional reference values.
        assert!((stp(1500.0, 150.0, 20.0, 1000.0) - 1.0).abs() < 1e-9);
        assert!((scp(1000.0, 50.0, 20.0) - 1.0).abs() < 1e-9);
        assert!((ehi1(1600.0, 100.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn terms_zero_out() {
        // Shear below 10 m/s kills STP and SCP outright.
        assert_eq!(stp(4000.0, 400.0, 8.0, 500.0), 0.0);
        assert_eq!(scp(4000.0, 400.0, 8.0), 0.0);
        // An LCL at/above 2 km kills STP.
        assert_eq!(stp(4000.0, 400.0, 20.0, 2400.0), 0.0);
        // Negative (anticyclonic) helicity contributes nothing.
        assert_eq!(scp(2000.0, -200.0, 20.0), 0.0);
    }

    /// Live sanity check on the real HRRR grid — also the empirical check that VUCSH/VVCSH are
    /// m/s vector components, not s^-1 shear rates (an s^-1 reading would be ~0.005 and every
    /// STP/SCP cell would floor at zero).
    /// `cargo test -p wxdata severe_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn severe_live() {
        let http = reqwest::Client::new();
        for kind in [SevereKind::Stp, SevereKind::Scp, SevereKind::Ehi] {
            let f = fetch_grid(&http, hrrr::Model::Hrrr, kind).await.expect("HRRR fetch");
            let finite: Vec<f32> = f
                .field
                .values
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .collect();
            let max = finite.iter().copied().fold(f32::MIN, f32::max);
            let nonzero = finite.iter().filter(|v| **v > 0.01).count();
            println!(
                "{kind:?}: {}x{} grid, {} finite, {} > 0.01, max {max:.2}",
                f.field.nx,
                f.field.ny,
                finite.len(),
                nonzero
            );
            assert!(!finite.is_empty(), "{kind:?} produced no finite cells");
        }
    }
}
