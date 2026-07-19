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
}
