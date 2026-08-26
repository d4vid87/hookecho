//! Gridded severe-weather composite parameters (STP, SCP, EHI) built from HRRR surface fields.
//!
//! These are the numbers a chaser scans first, and until now the app only had them at a single
//! clicked point (see [`crate::sounding::Sounding::indices`]). Here the same fixed-layer forms are
//! evaluated cell-by-cell over the whole HRRR analysis grid so they can be drawn as contours.
//!
//! Two formulations live here. The fixed-layer forms (Thompson et al. 2004) are the cheap ones,
//! evaluated from surface fields alone. The effective-layer forms (Thompson et al. 2007, 2012)
//! solve a column per cell to find the effective inflow layer first, and are what SPC
//! mesoanalysis plots — still coarser than SPC, which blends observations into its analysis.

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
    /// Mid-level lapse rate, 700–500 hPa (°C/km).
    Lapse700500,
    /// Steep-lapse-rate proxy over a deeper layer, 850–500 hPa (°C/km).
    Lapse850500,
    /// Effective bulk wind difference (kt): effective inflow base to 50% of the MU parcel's EL.
    EffShear,
    /// Effective storm-relative helicity (m²/s²) over the effective inflow layer.
    EffSrh,
    /// Significant Tornado Parameter in its effective-layer form.
    StpEff,
}

impl SevereKind {
    /// Kinds built from pressure-level columns rather than surface fields.
    fn is_effective(self) -> bool {
        matches!(
            self,
            SevereKind::EffShear | SevereKind::EffSrh | SevereKind::StpEff
        )
    }
}

/// Lapse rate (°C/km) between two levels given their temperatures (°C) and heights (m).
pub fn lapse_rate(t_lo: f64, z_lo: f64, t_hi: f64, z_hi: f64) -> f64 {
    let dz_km = (z_hi - z_lo) / 1000.0;
    if dz_km.abs() < 1e-6 {
        return f64::NAN;
    }
    (t_lo - t_hi) / dz_km
}

/// Shear term shared by STP and SCP: zero below 10 m/s, capped at 1.0 above 20 m/s.
fn shear_term(shear6_ms: f64) -> f64 {
    if shear6_ms < 10.0 {
        0.0
    } else {
        (shear6_ms / 20.0).min(1.0)
    }
}

/// Significant Tornado Parameter (fixed layer). Shared with the point sounding, which calls this
/// rather than keeping a second copy of the constants.
pub fn stp(sbcape: f64, srh1: f64, shear6_ms: f64, lcl_agl_m: f64) -> f64 {
    let lcl_term = ((2000.0 - lcl_agl_m) / 1000.0).clamp(0.0, 1.0);
    (sbcape / 1500.0) * (srh1.max(0.0) / 150.0) * shear_term(shear6_ms) * lcl_term
}

/// Supercell Composite Parameter, shared with the point sounding.
pub fn scp(sbcape: f64, srh3: f64, shear6_ms: f64) -> f64 {
    (sbcape / 1000.0) * (srh3.max(0.0) / 50.0) * shear_term(shear6_ms)
}

/// Energy-Helicity Index over 0–1 km, shared with the point sounding.
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
        !(model != hrrr::Model::Hrrr && kind == SevereKind::Stp),
        "STP needs an LCL height only the HRRR surface file publishes — use the HRRR source"
    );
    if let SevereKind::Lapse700500 | SevereKind::Lapse850500 = kind {
        return fetch_lapse_grid(http, model, kind).await;
    }
    if kind.is_effective() {
        return fetch_effective_grid(http, model, kind).await;
    }
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
                    // Handled above, before any surface field was fetched.
                    _ => unreachable!("{kind:?} takes its own path"),
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

/// Mid-level lapse rates: two temperatures and two heights from one cycle, differenced per cell.
async fn fetch_lapse_grid(
    http: &reqwest::Client,
    model: hrrr::Model,
    kind: SevereKind,
) -> anyhow::Result<HrrrForecast> {
    let lo_mb = if kind == SevereKind::Lapse850500 {
        "850 mb"
    } else {
        "700 mb"
    };
    let specs: Vec<(&str, &str, f64)> = vec![
        ("TMP", lo_mb, f64::NEG_INFINITY),
        ("HGT", lo_mb, f64::NEG_INFINITY),
        ("TMP", "500 mb", f64::NEG_INFINITY),
        ("HGT", "500 mb", f64::NEG_INFINITY),
    ];
    let (run, fields) = hrrr::fetch_fields_one_run(http, model, 0, &specs).await?;
    let f0 = &fields[0];
    for f in &fields[1..] {
        anyhow::ensure!(
            f.nx == f0.nx && f.ny == f0.ny,
            "{} lapse-rate grids disagree",
            model.label()
        );
    }
    let mut values = vec![f32::NAN; f0.values.len()];
    for (i, out) in values.iter_mut().enumerate() {
        // GRIB TMP is Kelvin; a difference of two temperatures needs no conversion.
        let v = lapse_rate(
            fields[0].values[i] as f64,
            fields[1].values[i] as f64,
            fields[2].values[i] as f64,
            fields[3].values[i] as f64,
        );
        if v.is_finite() {
            *out = v as f32;
        }
    }
    Ok(as_forecast(f0, values, run))
}

/// Pressure levels the effective-layer parameters are built from: the HRRR pressure file's own
/// 25 hPa ladder through the inflow layer and the mid-levels, thinning to 50 hPa above 500 hPa
/// where the profile is smooth, and running to 100 hPa so a buoyant parcel has an equilibrium
/// level to be measured against.
///
/// Seven mandatory levels used to be the whole set, which put the top of the profile at 400 hPa —
/// under the EL of any parcel worth computing EBWD for, so the depth-dependent parameters came
/// back empty exactly when they mattered. This is what SPC's recipe assumes it has.
const EFF_LEVELS_HPA: [u32; 29] = [
    1000, 975, 950, 925, 900, 875, 850, 825, 800, 775, 750, 725, 700, 675, 650, 625, 600, 575, 550,
    525, 500, 450, 400, 350, 300, 250, 200, 150, 100,
];

/// Longest side of the grid the effective-layer solve runs on. 29 levels × 5 variables at full
/// HRRR resolution is most of a gigabyte of transient f32; at this cap it is tens of megabytes,
/// and the effective-layer parameters are a mesoanalysis product — SPC's own is drawn on a 40 km
/// grid, coarser than this.
///
/// ponytail: one cap for all three parameters. Raise it if the layer ever looks blocky against a
/// storm-scale feature, and watch the memory when you do.
const EFF_GRID_CAP: usize = 600;

/// Effective-inflow-layer parameters: EBWD, ESRH and effective-layer STP.
///
/// A hundred and forty-five range fetches from one cycle (five variables at 29 levels), each
/// subsampled to [`EFF_GRID_CAP`] as it lands, then a per-cell column solve. SPC does this on the
/// native model levels with mesoanalysis-blended observations; this is the same recipe on the
/// pressure-level ladder the HRRR publishes, which is the part that was missing.
async fn fetch_effective_grid(
    http: &reqwest::Client,
    _model: hrrr::Model,
    kind: SevereKind,
) -> anyhow::Result<HrrrForecast> {
    // HRRR's pressure file regardless of the picked source. The surface file carries only a
    // handful of pressure levels, and RAP's awp130 packs U and V as two submessages of one GRIB
    // record — a byte range for either decodes to U, so RAP winds here would be silently wrong.
    let model = hrrr::Model::HrrrPressure;

    let mut specs: Vec<(String, String, f64)> = Vec::new();
    for hpa in EFF_LEVELS_HPA {
        for var in ["TMP", "DPT", "HGT", "UGRD", "VGRD"] {
            specs.push((var.to_string(), format!("{hpa} mb"), f64::NEG_INFINITY));
        }
    }
    let borrowed: Vec<(&str, &str, f64)> = specs
        .iter()
        .map(|(v, l, m)| (v.as_str(), l.as_str(), *m))
        .collect();
    let (run, fields) =
        hrrr::fetch_fields_one_run_capped(http, model, 0, &borrowed, Some(EFF_GRID_CAP)).await?;
    let f0 = &fields[0];
    for f in &fields[1..] {
        anyhow::ensure!(
            f.nx == f0.nx && f.ny == f0.ny,
            "{} effective-layer grids disagree",
            model.label()
        );
    }

    let n = f0.values.len();
    let column_value = |i: usize| -> f32 {
        {
            // One column: mandatory levels, surface-first, dropping any level with a gap in it.
            let mut levels = Vec::with_capacity(EFF_LEVELS_HPA.len());
            let mut heights = Vec::with_capacity(EFF_LEVELS_HPA.len());
            for (k, hpa) in EFF_LEVELS_HPA.iter().enumerate() {
                let b = k * 5;
                let (t, d_k, z, u, v) = (
                    fields[b].values[i] as f64,
                    fields[b + 1].values[i] as f64,
                    fields[b + 2].values[i] as f64,
                    fields[b + 3].values[i] as f64,
                    fields[b + 4].values[i] as f64,
                );
                if !(t.is_finite()
                    && d_k.is_finite()
                    && z.is_finite()
                    && u.is_finite()
                    && v.is_finite())
                {
                    continue;
                }
                levels.push(crate::sounding::SoundingLevel {
                    pressure_hpa: *hpa as f64,
                    temp_c: t - 273.15,
                    dewpt_c: d_k - 273.15,
                    u_ms: u,
                    v_ms: v,
                });
                heights.push(z);
            }
            if levels.len() < 4 {
                return f32::NAN;
            }
            let column = crate::sounding::Sounding {
                lon: 0.0,
                lat: 0.0,
                run,
                fh: 0,
                levels,
            };
            match effective_value(&column, &heights, kind) {
                Some(v) if v.is_finite() => v as f32,
                _ => f32::NAN,
            }
        }
    };
    // A column solve per cell, over 1.4M cells: worth every thread on native. wasm has none.
    #[cfg(not(target_arch = "wasm32"))]
    let values: Vec<f32> = {
        use rayon::prelude::*;
        (0..n).into_par_iter().map(column_value).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let values: Vec<f32> = (0..n).map(column_value).collect();

    Ok(as_forecast(f0, values, run))
}

/// Effective-layer parameters for one clicked sounding — the same solve the gridded layers run
/// per cell, so the panel and the map now answer with one method instead of two.
#[derive(Debug, Clone, Copy)]
pub struct EffectiveIndices {
    /// Effective storm-relative helicity (m²/s²).
    pub esrh: f64,
    /// Effective bulk wind difference (kt), inflow base to half the MU parcel's EL. `None` when
    /// the parcel is still buoyant at the top of the profile, so there is no EL to halve.
    pub ebwd_kt: Option<f64>,
    /// Effective-layer significant tornado parameter. `None` for the same reason as `ebwd_kt`,
    /// which it depends on.
    pub stp_eff: Option<f64>,
}

/// Effective-layer parameters for a profile, or `None` when it has no effective inflow layer —
/// the honest answer for a capped or dry column, and the same one the grid gives.
///
/// The depth-limited fields go `None` rather than guessing an EL above the profile's top level.
/// Since the clicked sounding reaches 100 hPa, a parcel that runs off the end of the data is a
/// genuinely extraordinary one rather than an ordinary uncapped plains parcel.
pub fn effective_indices(s: &crate::sounding::Sounding) -> Option<EffectiveIndices> {
    let heights = s.heights_m();
    Some(EffectiveIndices {
        esrh: effective_value(s, &heights, SevereKind::EffSrh)?,
        ebwd_kt: effective_value(s, &heights, SevereKind::EffShear),
        stp_eff: effective_value(s, &heights, SevereKind::StpEff),
    })
}

/// One column's effective-layer answer. Returns `None` when there is no effective inflow layer,
/// which is the honest result for a capped or dry column.
fn effective_value(
    column: &crate::sounding::Sounding,
    heights_m: &[f64],
    kind: SevereKind,
) -> Option<f64> {
    // Effective inflow layer (Thompson et al. 2007): the contiguous run of levels whose parcels
    // have at least 100 J/kg of CAPE and no worse than −250 J/kg of CIN.
    let mut base = None;
    let mut top = None;
    for i in 0..column.levels.len() {
        let ok = column
            .parcel_from(i)
            .is_some_and(|p| p.cape >= 100.0 && p.cin >= -250.0);
        if ok {
            base.get_or_insert(i);
            top = Some(i);
        } else if base.is_some() {
            break;
        }
    }
    let (base, top) = (base?, top?);
    let base_z = heights_m[base];

    // Most-unstable parcel over the inflow layer, for the EL that caps the shear layer.
    let mu = (base..=top)
        .filter_map(|i| column.parcel_from(i).map(|p| (i, p)))
        .max_by(|a, b| a.1.cape.total_cmp(&b.1.cape))?;
    let el_z = mu.1.el_m.map(|el| heights_m[mu.0] + el);

    // Wind at a height, interpolated between the mandatory levels we have.
    let wind_at = |z: f64| -> Option<(f64, f64)> {
        let i = heights_m.iter().position(|&h| h >= z)?;
        if i == 0 {
            let l = column.levels.first()?;
            return Some((l.u_ms, l.v_ms));
        }
        let k =
            ((z - heights_m[i - 1]) / (heights_m[i] - heights_m[i - 1]).max(1e-6)).clamp(0.0, 1.0);
        let (a, b) = (&column.levels[i - 1], &column.levels[i]);
        Some((
            a.u_ms + (b.u_ms - a.u_ms) * k,
            a.v_ms + (b.v_ms - a.v_ms) * k,
        ))
    };

    match kind {
        SevereKind::EffShear => {
            // Base of the inflow layer to half the MU parcel's equilibrium level.
            let depth_top = el_z.map(|el| base_z + (el - base_z) * 0.5)?;
            let (u0, v0) = wind_at(base_z)?;
            let (u1, v1) = wind_at(depth_top)?;
            Some((u1 - u0).hypot(v1 - v0) * 1.943_844) // m/s → kt
        }
        SevereKind::EffSrh => effective_srh(column, heights_m, base_z, heights_m[top]),
        SevereKind::StpEff => {
            let esrh = effective_srh(column, heights_m, base_z, heights_m[top])?;
            let depth_top = el_z.map(|el| base_z + (el - base_z) * 0.5)?;
            let (u0, v0) = wind_at(base_z)?;
            let (u1, v1) = wind_at(depth_top)?;
            let ebwd = (u1 - u0).hypot(v1 - v0);
            let mu_cape = mu.1.cape;
            let mu_cin = mu.1.cin;
            // Effective-layer STP (Thompson et al. 2012): MLCAPE, ESRH, EBWD, MLLCL and MLCIN
            // terms, each clipped the way SPC clips them.
            let cape_term = mu_cape / 1500.0;
            let lcl = mu.1.lcl_m;
            let lcl_term = if lcl < 1000.0 {
                1.0
            } else if lcl > 2000.0 {
                0.0
            } else {
                (2000.0 - lcl) / 1000.0
            };
            let esrh_term = esrh / 150.0;
            let shear_term = (ebwd / 20.0).clamp(0.0, 1.5);
            let shear_term = if ebwd < 12.5 { 0.0 } else { shear_term };
            let cin_term = if mu_cin > -50.0 {
                1.0
            } else {
                (200.0 + mu_cin) / 150.0
            }
            .clamp(0.0, 1.0);
            Some((cape_term * lcl_term * esrh_term * shear_term * cin_term).max(0.0))
        }
        _ => None,
    }
}

/// Storm-relative helicity over `[base_z, top_z]` against the column's Bunkers right mover.
fn effective_srh(
    column: &crate::sounding::Sounding,
    heights_m: &[f64],
    base_z: f64,
    top_z: f64,
) -> Option<f64> {
    let (cu, cv) = column.bunkers_rm()?;
    let wind_at = |z: f64| -> Option<(f64, f64)> {
        let i = heights_m.iter().position(|&h| h >= z)?;
        if i == 0 {
            let l = column.levels.first()?;
            return Some((l.u_ms, l.v_ms));
        }
        let k =
            ((z - heights_m[i - 1]) / (heights_m[i] - heights_m[i - 1]).max(1e-6)).clamp(0.0, 1.0);
        let (a, b) = (&column.levels[i - 1], &column.levels[i]);
        Some((
            a.u_ms + (b.u_ms - a.u_ms) * k,
            a.v_ms + (b.v_ms - a.v_ms) * k,
        ))
    };
    let step = 250.0;
    let mut total = 0.0;
    let mut z = base_z;
    while z + step <= top_z + 1e-6 {
        let (u0, v0) = wind_at(z)?;
        let (u1, v1) = wind_at(z + step)?;
        total += (u1 - cu) * (v0 - cv) - (u0 - cu) * (v1 - cv);
        z += step;
    }
    Some(total)
}

/// Wrap a computed value grid in the geometry of the field it came from.
fn as_forecast(
    like: &MrmsField,
    values: Vec<f32>,
    run: chrono::DateTime<chrono::Utc>,
) -> HrrrForecast {
    HrrrForecast {
        field: MrmsField {
            values,
            nx: like.nx,
            ny: like.ny,
            lon_west: like.lon_west,
            lon_east: like.lon_east,
            lat_north: like.lat_north,
            lat_south: like.lat_south,
            time: like.time,
        },
        run,
        fcst_hour: 0,
    }
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
    fn lapse_rate_is_degrees_per_km() {
        // 15 °C over 3 km = 5 °C/km, and the sign follows "cools with height".
        assert!((lapse_rate(288.0, 1000.0, 273.0, 4000.0) - 5.0).abs() < 1e-9);
        // An inversion reads negative.
        assert!(lapse_rate(273.0, 1000.0, 283.0, 2000.0) < 0.0);
        // A zero-depth layer has no lapse rate to report.
        assert!(lapse_rate(288.0, 1000.0, 273.0, 1000.0).is_nan());
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
            let f = fetch_grid(&http, hrrr::Model::Hrrr, kind)
                .await
                .expect("HRRR fetch");
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
