//! Point soundings from HRRR pressure-level analysis: fetch TMP/DPT/UGRD/VGRD at a curated set
//! of mandatory levels via `.idx` byte-range requests, sample the nearest grid point, and return
//! a vertical profile for a Skew-T / hodograph. Reuses the HRRR bucket + gribberish decode.

use crate::alerts::USER_AGENT;
use chrono::{DateTime, Datelike, Timelike, Utc};

const BUCKET: &str = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com";

/// Mandatory pressure levels (hPa), surface-up. Kept small so a click is ~40 range fetches.
const LEVELS_HPA: &[u32] = &[1000, 925, 850, 700, 600, 500, 400, 300, 250, 200];

/// One level of the sounding.
#[derive(Debug, Clone, Copy)]
pub struct SoundingLevel {
    pub pressure_hpa: f64,
    pub temp_c: f64,
    pub dewpt_c: f64,
    pub u_ms: f64,
    pub v_ms: f64,
}

/// A vertical profile at a point.
pub struct Sounding {
    pub lon: f64,
    pub lat: f64,
    pub run: DateTime<Utc>,
    /// Levels ordered surface (highest pressure) first.
    pub levels: Vec<SoundingLevel>,
}

impl Sounding {
    /// Bulk wind shear magnitude (knots) between the lowest and ~500 hPa (≈ 0–6 km) levels.
    pub fn bulk_shear_kt(&self) -> Option<f64> {
        let sfc = self.levels.first()?;
        let top = self.levels.iter().find(|l| l.pressure_hpa <= 500.0)?;
        let (du, dv) = (top.u_ms - sfc.u_ms, top.v_ms - sfc.v_ms);
        Some((du * du + dv * dv).sqrt() * 1.943_844)
    }
}

/// Severe-weather composite indices derived from the profile (feature FF). All are the
/// *fixed-layer* published forms, computed from the 10 mandatory levels — coarse but honestly
/// labeled; a mesoanalysis-grade effective-layer version needs a denser profile.
#[derive(Debug, Clone, Copy)]
pub struct Indices {
    /// Surface-based CAPE (J/kg).
    pub sbcape: f64,
    /// LCL height (m AGL) of the surface parcel.
    pub lcl_m: f64,
    /// 0–1 km storm-relative helicity (m²/s², Bunkers right-mover motion).
    pub srh1: f64,
    /// 0–3 km storm-relative helicity (m²/s²).
    pub srh3: f64,
    /// 0–6 km bulk shear (kt).
    pub shear6_kt: f64,
    /// Supercell composite parameter (Thompson 2004 fixed-layer form).
    pub scp: f64,
    /// Significant tornado parameter (fixed-layer form).
    pub stp: f64,
    /// 0–1 km energy-helicity index.
    pub ehi1: f64,
}

const RD: f64 = 287.04; // J/(kg·K)
const G: f64 = 9.80665;
const KAPPA: f64 = 0.2854;

/// Saturation vapor pressure (hPa) over water — Bolton (1980).
fn e_sat_hpa(t_c: f64) -> f64 {
    6.112 * ((17.67 * t_c) / (t_c + 243.5)).exp()
}

/// Parcel temperature (K) after pseudoadiabatic ascent from `(p0, t0k)` to `p1` (hPa, p1 < p0),
/// stepped in small pressure increments.
fn moist_ascent_k(p0: f64, t0k: f64, p1: f64) -> f64 {
    let mut t = t0k;
    let mut p = p0;
    const LV: f64 = 2.501e6;
    const CPD: f64 = 1005.7;
    const EPS: f64 = 0.622;
    while p > p1 {
        let dp = (p - p1).min(5.0);
        let tc = t - 273.15;
        let rs = EPS * e_sat_hpa(tc) / (p - e_sat_hpa(tc)).max(1.0);
        // Pseudoadiabatic lapse dT/dp (K/hPa).
        let dtdp = (1.0 / p) * (RD * t + LV * rs) / (CPD + LV * LV * rs * EPS / (RD * t * t));
        t -= dtdp * dp;
        p -= dp;
    }
    t
}

impl Sounding {
    /// Geometric height (m AGL) of each level via hypsometric integration.
    /// `// ponytail: dry temperature stands in for virtual temperature (~1% height error).`
    pub fn heights_m(&self) -> Vec<f64> {
        let mut h = Vec::with_capacity(self.levels.len());
        let mut z = 0.0;
        for i in 0..self.levels.len() {
            if i > 0 {
                let (a, b) = (&self.levels[i - 1], &self.levels[i]);
                let t_mean = (a.temp_c + b.temp_c) / 2.0 + 273.15;
                z += RD * t_mean / G * (a.pressure_hpa / b.pressure_hpa).ln();
            }
            h.push(z);
        }
        h
    }

    /// Surface-based CAPE (J/kg) and LCL height (m AGL) via a stepped pseudoadiabatic parcel.
    pub fn sb_parcel(&self) -> Option<(f64, f64)> {
        let sfc = self.levels.first()?;
        if self.levels.len() < 3 {
            return None;
        }
        let tk = sfc.temp_c + 273.15;
        // Bolton (1980) LCL temperature from T and the vapor pressure at Td.
        let e = e_sat_hpa(sfc.dewpt_c).max(1e-3);
        let t_lcl = 2840.0 / (3.5 * tk.ln() - e.ln() - 4.805) + 55.0;
        let p_lcl = sfc.pressure_hpa * (t_lcl / tk).powf(1.0 / KAPPA);
        let lcl_m = RD * (tk + t_lcl) / 2.0 / G * (sfc.pressure_hpa / p_lcl).ln();

        // Parcel temperature at each level: dry below the LCL, pseudoadiabatic above.
        let parcel_k = |p: f64| -> f64 {
            if p >= p_lcl {
                tk * (p / sfc.pressure_hpa).powf(KAPPA)
            } else {
                moist_ascent_k(p_lcl, t_lcl, p)
            }
        };
        // Trapezoidal CAPE over positive-buoyancy layers: Rd Σ (Tp−Te) Δln p.
        let mut cape = 0.0;
        for w in self.levels.windows(2) {
            let (lo, hi) = (&w[0], &w[1]);
            let b_lo = parcel_k(lo.pressure_hpa) - (lo.temp_c + 273.15);
            let b_hi = parcel_k(hi.pressure_hpa) - (hi.temp_c + 273.15);
            let dlnp = (lo.pressure_hpa / hi.pressure_hpa).ln();
            let seg = RD * (b_lo.max(0.0) + b_hi.max(0.0)) / 2.0 * dlnp;
            cape += seg;
        }
        Some((cape, lcl_m))
    }

    /// Wind (u, v) linearly interpolated to height `z_m` AGL.
    fn wind_at(&self, heights: &[f64], z_m: f64) -> Option<(f64, f64)> {
        let i = heights.iter().position(|&h| h >= z_m)?;
        if i == 0 {
            let l = self.levels.first()?;
            return Some((l.u_ms, l.v_ms));
        }
        let (h0, h1) = (heights[i - 1], heights[i]);
        let (a, b) = (&self.levels[i - 1], &self.levels[i]);
        let k = ((z_m - h0) / (h1 - h0).max(1e-6)).clamp(0.0, 1.0);
        Some((a.u_ms + (b.u_ms - a.u_ms) * k, a.v_ms + (b.v_ms - a.v_ms) * k))
    }

    /// Bunkers right-mover storm motion: 0–6 km mean wind plus 7.5 m/s at right angles to the
    /// 0–6 km shear vector.
    pub fn bunkers_rm(&self) -> Option<(f64, f64)> {
        let h = self.heights_m();
        let (mut mu, mut mv, mut n) = (0.0, 0.0, 0);
        for z in (0..=6000).step_by(500) {
            if let Some((u, v)) = self.wind_at(&h, z as f64) {
                mu += u;
                mv += v;
                n += 1;
            }
        }
        if n == 0 {
            return None;
        }
        let (mu, mv) = (mu / n as f64, mv / n as f64);
        let (u0, v0) = self.wind_at(&h, 0.0)?;
        let (u6, v6) = self.wind_at(&h, 6000.0)?;
        let (su, sv) = (u6 - u0, v6 - v0);
        let mag = (su * su + sv * sv).sqrt().max(1e-6);
        // Right of the shear vector: rotate −90° → (sv, −su).
        Some((mu + 7.5 * sv / mag, mv - 7.5 * su / mag))
    }

    /// Storm-relative helicity (m²/s²) over 0..`depth_m`, relative to the Bunkers right mover.
    pub fn srh(&self, depth_m: f64) -> Option<f64> {
        let h = self.heights_m();
        let (cu, cv) = self.bunkers_rm()?;
        let mut total = 0.0;
        let step = 250.0;
        let mut z = 0.0;
        while z + step <= depth_m + 1e-6 {
            let (u0, v0) = self.wind_at(&h, z)?;
            let (u1, v1) = self.wind_at(&h, z + step)?;
            total += (u1 - cu) * (v0 - cv) - (u0 - cu) * (v1 - cv);
            z += step;
        }
        Some(total)
    }

    /// All fixed-layer composite indices, or `None` when the profile is too short.
    pub fn indices(&self) -> Option<Indices> {
        let (sbcape, lcl_m) = self.sb_parcel()?;
        let srh1 = self.srh(1000.0)?;
        let srh3 = self.srh(3000.0)?;
        let shear6_kt = self.bulk_shear_kt()?;
        let shear6_ms = shear6_kt / 1.943_844;
        // Shear term: zero below 10 m/s, capped at 1.0 above 20 m/s (Thompson et al. 2004).
        let shear_term = if shear6_ms < 10.0 { 0.0 } else { (shear6_ms / 20.0).min(1.0) };
        let scp = (sbcape / 1000.0) * (srh3.max(0.0) / 50.0) * shear_term;
        let lcl_term = ((2000.0 - lcl_m) / 1000.0).clamp(0.0, 1.0);
        let stp = (sbcape / 1500.0) * (srh1.max(0.0) / 150.0) * shear_term * lcl_term;
        let ehi1 = sbcape * srh1 / 160_000.0;
        Some(Indices { sbcape, lcl_m, srh1, srh3, shear6_kt, scp, stp, ehi1 })
    }
}

/// Fetch a sounding at `(lon, lat)` from the most recent HRRR pressure-level analysis (f00).
pub async fn fetch(http: &reqwest::Client, lon: f64, lat: f64) -> anyhow::Result<Sounding> {
    let now = Utc::now();
    let mut last_err = None;
    for back in 1..=6 {
        let run = (now - chrono::Duration::hours(back))
            .with_minute(0).unwrap().with_second(0).unwrap().with_nanosecond(0).unwrap();
        match fetch_run(http, run, lon, lat).await {
            Ok(s) => return Ok(s),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no HRRR run found")))
}

async fn fetch_run(http: &reqwest::Client, run: DateTime<Utc>, lon: f64, lat: f64) -> anyhow::Result<Sounding> {
    let date = format!("{:04}{:02}{:02}", run.year(), run.month(), run.day());
    let base = format!("{BUCKET}/hrrr.{date}/conus/hrrr.t{:02}z.wrfprsf00.grib2", run.hour());
    let idx = http
        .get(format!("{base}.idx"))
        .header("User-Agent", USER_AGENT)
        .send().await?.error_for_status()?.text().await?;

    // Fetch all (var, level) messages concurrently, then sample each at the point.
    let mut jobs = Vec::new();
    for &hpa in LEVELS_HPA {
        let level = format!("{hpa} mb");
        for var in ["TMP", "DPT", "UGRD", "VGRD"] {
            if let Some((start, end)) = message_range(&idx, var, &level) {
                jobs.push((hpa, var, sample_message(http, &base, start, end, lon, lat)));
            }
        }
    }
    // Resolve.
    let mut by_level: std::collections::BTreeMap<u32, [Option<f64>; 4]> = std::collections::BTreeMap::new();
    for (hpa, var, fut) in jobs {
        let val = fut.await.ok();
        let slot = by_level.entry(hpa).or_insert([None; 4]);
        let i = match var { "TMP" => 0, "DPT" => 1, "UGRD" => 2, _ => 3 };
        slot[i] = val;
    }

    // Assemble complete levels (surface-first = highest pressure first).
    let mut levels: Vec<SoundingLevel> = Vec::new();
    for (&hpa, vals) in by_level.iter().rev() {
        if let [Some(t), Some(d), Some(u), Some(v)] = *vals {
            levels.push(SoundingLevel {
                pressure_hpa: hpa as f64,
                temp_c: t - 273.15, // grib TMP/DPT are Kelvin
                dewpt_c: d - 273.15,
                u_ms: u,
                v_ms: v,
            });
        }
    }
    anyhow::ensure!(levels.len() >= 3, "sounding has too few complete levels");
    Ok(Sounding { lon, lat, run, levels })
}

/// Range-GET one GRIB2 message, decode it, and return the value at the grid point nearest
/// `(lon, lat)`.
async fn sample_message(
    http: &reqwest::Client,
    base: &str,
    start: u64,
    end: Option<u64>,
    lon: f64,
    lat: f64,
) -> anyhow::Result<f64> {
    let range = match end {
        Some(e) => format!("bytes={start}-{}", e - 1),
        None => format!("bytes={start}-"),
    };
    let bytes = http
        .get(base)
        .header("User-Agent", USER_AGENT)
        .header("Range", range)
        .send().await?.error_for_status()?.bytes().await?;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sample_nearest(&bytes, lon, lat)))
        .unwrap_or_else(|_| anyhow::bail!("grib decode panicked"))
}

fn sample_nearest(raw: &[u8], lon: f64, lat: f64) -> anyhow::Result<f64> {
    use gribberish::data_message::DataMessage;
    use gribberish::message::read_message;
    let msg = read_message(raw, 0).ok_or_else(|| anyhow::anyhow!("no GRIB2 message"))?;
    let dm = DataMessage::try_from(&msg).map_err(|e| anyhow::anyhow!("decode: {e:?}"))?;
    let (lats, lons) = dm.metadata.latlng();
    let data = dm.data;
    anyhow::ensure!(lats.len() == data.len() && lons.len() == data.len(), "latlng/data mismatch");
    let mut best = None;
    let mut best_d = f64::MAX;
    for k in 0..data.len() {
        if !data[k].is_finite() || !lats[k].is_finite() || !lons[k].is_finite() {
            continue;
        }
        let dlon = (lons[k] - lon) * (lat.to_radians().cos());
        let d = dlon * dlon + (lats[k] - lat).powi(2);
        if d < best_d {
            best_d = d;
            best = Some(data[k]);
        }
    }
    best.ok_or_else(|| anyhow::anyhow!("no finite grid point"))
}

/// Find the `[start, end)` byte range of the message for `var` at `level` (e.g. "500 mb") in a
/// GRIB2 `.idx`. `end` is `None` when it is the last message.
fn message_range(idx: &str, var: &str, level: &str) -> Option<(u64, Option<u64>)> {
    let lines: Vec<&str> = idx.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 5 {
            continue;
        }
        if f[3] == var && f[4] == level {
            let start: u64 = f[1].parse().ok()?;
            let end = lines.get(i + 1).and_then(|n| n.split(':').nth(1)).and_then(|s| s.parse().ok());
            return Some((start, end));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idx_finds_var_at_level() {
        let idx = "1:0:d=2026:TMP:500 mb:anl:\n\
                   2:1000:d=2026:DPT:500 mb:anl:\n\
                   3:2500:d=2026:UGRD:500 mb:anl:\n";
        assert_eq!(message_range(idx, "TMP", "500 mb"), Some((0, Some(1000))));
        assert_eq!(message_range(idx, "DPT", "500 mb"), Some((1000, Some(2500))));
        assert_eq!(message_range(idx, "UGRD", "500 mb"), Some((2500, None)));
        assert_eq!(message_range(idx, "TMP", "850 mb"), None);
    }

    #[test]
    fn bulk_shear_computes() {
        let s = Sounding {
            lon: -97.0,
            lat: 35.0,
            run: Utc::now(),
            levels: vec![
                SoundingLevel { pressure_hpa: 1000.0, temp_c: 20.0, dewpt_c: 18.0, u_ms: 0.0, v_ms: 0.0 },
                SoundingLevel { pressure_hpa: 500.0, temp_c: -10.0, dewpt_c: -20.0, u_ms: 20.0, v_ms: 0.0 },
            ],
        };
        // 20 m/s shear ≈ 38.9 kt.
        let sh = s.bulk_shear_kt().unwrap();
        assert!((sh - 38.9).abs() < 0.5, "shear ~38.9 kt, got {sh}");
    }

    /// A classic unstable, veering Great-Plains profile.
    fn supercell_profile() -> Sounding {
        let mk = |p, t, td, u, v| SoundingLevel { pressure_hpa: p, temp_c: t, dewpt_c: td, u_ms: u, v_ms: v };
        Sounding {
            lon: -97.0,
            lat: 35.0,
            run: Utc::now(),
            levels: vec![
                mk(1000.0, 30.0, 22.0, 0.0, 8.0),
                mk(925.0, 24.0, 19.0, 6.0, 12.0),
                mk(850.0, 20.0, 16.0, 10.0, 14.0),
                mk(700.0, 10.0, 2.0, 14.0, 16.0),
                mk(500.0, -8.0, -20.0, 20.0, 18.0),
                mk(400.0, -18.0, -32.0, 24.0, 18.0),
                mk(300.0, -32.0, -45.0, 27.0, 17.0),
                mk(250.0, -42.0, -55.0, 28.0, 16.0),
                mk(200.0, -52.0, -60.0, 29.0, 15.0),
            ],
        }
    }

    #[test]
    fn heights_increase_monotonically() {
        let h = supercell_profile().heights_m();
        assert_eq!(h[0], 0.0);
        assert!(h.windows(2).all(|w| w[1] > w[0]));
        // 500 hPa sits near 5.5–6 km in a warm airmass.
        assert!((4800.0..6500.0).contains(&h[4]), "500 hPa height {:.0}", h[4]);
    }

    #[test]
    fn supercell_profile_yields_severe_indices() {
        let s = supercell_profile();
        let ix = s.indices().expect("indices");
        assert!((300.0..6000.0).contains(&ix.sbcape), "CAPE plausible: {:.0}", ix.sbcape);
        assert!((200.0..2500.0).contains(&ix.lcl_m), "LCL plausible: {:.0}", ix.lcl_m);
        assert!(ix.srh1 > 0.0, "veering profile → positive 0-1 km SRH: {:.0}", ix.srh1);
        assert!(ix.srh3 >= ix.srh1, "deeper layer accumulates at least as much: {:.0} vs {:.0}", ix.srh3, ix.srh1);
        assert!((20.0..70.0).contains(&ix.shear6_kt), "0-6 shear: {:.0} kt", ix.shear6_kt);
        assert!(ix.scp > 0.0 && ix.stp > 0.0 && ix.ehi1 > 0.0);
    }

    #[test]
    fn stable_profile_has_no_cape() {
        // Cold, dry surface under warmer air aloft: no positive buoyancy anywhere.
        let mk = |p, t, td| SoundingLevel { pressure_hpa: p, temp_c: t, dewpt_c: td, u_ms: 0.0, v_ms: 0.0 };
        let s = Sounding {
            lon: 0.0,
            lat: 0.0,
            run: Utc::now(),
            levels: vec![mk(1000.0, -5.0, -20.0), mk(850.0, 5.0, -15.0), mk(500.0, -10.0, -30.0)],
        };
        let (cape, _) = s.sb_parcel().unwrap();
        assert!(cape < 10.0, "inversion profile CAPE ~0, got {cape:.1}");
    }
}
