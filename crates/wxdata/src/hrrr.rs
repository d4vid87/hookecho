//! HRRR "future radar": composite reflectivity (REFC) forecast grids from the NOAA HRRR AWS PDS.
//!
//! Fetches only the REFC message of a `wrfsfcf{HH}` file via an `.idx` byte-range request
//! (~0.4 MB instead of the ~130 MB full file), decodes it with `gribberish` (Lambert-conformal
//! grid), then scatter-regrids the native grid onto a regular lat/lon grid so it can reuse the
//! MRMS field-layer render pipeline (a plate-carrée→mercator warp).

use crate::alerts::USER_AGENT;
use crate::mrms::MrmsField;
use chrono::{DateTime, Datelike, Timelike, Utc};

const BUCKET: &str = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com";
/// Regular-grid cell size (degrees) for the regrid. Slightly coarser than HRRR's ~3 km so the
/// scatter fills every target cell without holes.
const RES_DEG: f64 = 0.04;

/// A decoded HRRR forecast field plus its run/valid times.
pub struct HrrrForecast {
    pub field: MrmsField,
    /// Model cycle (run) initialization time (UTC).
    pub run: DateTime<Utc>,
    /// Forecast hour past the run.
    pub fcst_hour: u8,
}

impl HrrrForecast {
    /// Valid time = run + forecast hour.
    pub fn valid(&self) -> DateTime<Utc> {
        self.run + chrono::Duration::hours(self.fcst_hour as i64)
    }
}

/// Fetch the REFC forecast for `fcst_hour` (0..=18) from the most recent available HRRR run.
/// Tries recent cycles (allowing for the ~1–2 h data latency), newest first.
pub async fn fetch_forecast(http: &reqwest::Client, fcst_hour: u8) -> anyhow::Result<HrrrForecast> {
    fetch_field(http, "REFC", "entire atmosphere", fcst_hour, -30.0).await
}

/// Fetch any single HRRR surface field for `fcst_hour` by variable + level idx strings, regridding
/// with `min_valid` as the drop threshold (REFC uses −30 dBZ; CAPE 0; SRH −∞ so negatives survive).
/// Walks back up to 6 cycles until a run has this forecast hour posted.
pub async fn fetch_field(
    http: &reqwest::Client,
    var: &str,
    level: &str,
    fcst_hour: u8,
    min_valid: f64,
) -> anyhow::Result<HrrrForecast> {
    let fh = fcst_hour.min(18);
    let now = Utc::now();
    let mut last_err = None;
    for back in 1..=6 {
        let run = (now - chrono::Duration::hours(back)).with_minute(0).unwrap().with_second(0).unwrap().with_nanosecond(0).unwrap();
        match fetch_run_field(http, run, fh, var, level, min_valid).await {
            Ok(field) => return Ok(HrrrForecast { field, run, fcst_hour: fh }),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no HRRR run found")))
}

async fn fetch_run_field(
    http: &reqwest::Client,
    run: DateTime<Utc>,
    fh: u8,
    var: &str,
    level: &str,
    min_valid: f64,
) -> anyhow::Result<MrmsField> {
    let date = format!("{:04}{:02}{:02}", run.year(), run.month(), run.day());
    let base = format!("{BUCKET}/hrrr.{date}/conus/hrrr.t{:02}z.wrfsfcf{:02}.grib2", run.hour(), fh);

    // The .idx sidecar lists each message's start byte; find the one for this var+level.
    let idx = http
        .get(format!("{base}.idx"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let (start, end) = field_byte_range(&idx, var, level)
        .ok_or_else(|| anyhow::anyhow!("no {var}:{level} in idx"))?;

    let range = match end {
        Some(e) => format!("bytes={start}-{}", e - 1),
        None => format!("bytes={start}-"),
    };
    let bytes = http
        .get(&base)
        .header("User-Agent", USER_AGENT)
        .header("Range", range)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // gribberish can panic on some packings; contain it (see mrms::fetch_latest).
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_regrid(&bytes, min_valid)))
        .unwrap_or_else(|_| anyhow::bail!("HRRR grib decode panicked"))
}

/// Find the `[start, end)` byte range of the message matching `var` (field 3) and `level`
/// (field 4) in a GRIB2 `.idx`. `end` is `None` when it's the last message (read to EOF).
fn field_byte_range(idx: &str, var: &str, level: &str) -> Option<(u64, Option<u64>)> {
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

/// Decode a single-message HRRR GRIB2 (Lambert grid) and scatter-regrid onto a regular lat/lon
/// grid, keeping the max dBZ per target cell (reflectivity composites well under max).
fn decode_regrid(raw: &[u8], min_valid: f64) -> anyhow::Result<MrmsField> {
    use gribberish::data_message::DataMessage;
    use gribberish::message::read_message;
    let msg = read_message(raw, 0).ok_or_else(|| anyhow::anyhow!("no GRIB2 message"))?;
    let time = msg.forecast_date().unwrap_or_else(|_| Utc::now());
    let dm = DataMessage::try_from(&msg).map_err(|e| anyhow::anyhow!("hrrr decode: {e:?}"))?;
    let (lats, lons) = dm.metadata.latlng();
    let data = dm.data;
    anyhow::ensure!(
        lats.len() == data.len() && lons.len() == data.len(),
        "hrrr latlng/data length mismatch"
    );

    regrid(&lats, &lons, &data, time, min_valid)
}

/// Scatter native (lat, lon, value) triples onto a regular lat/lon grid (max per cell).
/// Pure + fixture-testable. Non-finite or below-`min_valid` samples are ignored.
/// `// ponytail: max-per-cell — for SRH this keeps the strongest (most positive) value per cell;`
/// `// negative (anticyclonic) SRH is retained only where no positive sample shares the cell.`
fn regrid(lats: &[f64], lons: &[f64], data: &[f64], time: DateTime<Utc>, min_valid: f64) -> anyhow::Result<MrmsField> {
    let mut lonmin = f64::MAX;
    let mut lonmax = f64::MIN;
    let mut latmin = f64::MAX;
    let mut latmax = f64::MIN;
    for k in 0..data.len() {
        if !lats[k].is_finite() || !lons[k].is_finite() {
            continue;
        }
        lonmin = lonmin.min(lons[k]);
        lonmax = lonmax.max(lons[k]);
        latmin = latmin.min(lats[k]);
        latmax = latmax.max(lats[k]);
    }
    anyhow::ensure!(lonmax > lonmin && latmax > latmin, "hrrr grid has no finite extent");

    let nx = (((lonmax - lonmin) / RES_DEG).ceil() as usize).max(1);
    let ny = (((latmax - latmin) / RES_DEG).ceil() as usize).max(1);
    let mut values = vec![f32::NAN; nx * ny];
    for k in 0..data.len() {
        let v = data[k];
        if !v.is_finite() || v < min_valid || !lats[k].is_finite() || !lons[k].is_finite() {
            continue;
        }
        let gx = (((lons[k] - lonmin) / RES_DEG) as usize).min(nx - 1);
        // Row 0 is the northernmost latitude (matches MrmsField convention).
        let gy = (((latmax - lats[k]) / RES_DEG) as usize).min(ny - 1);
        let cell = &mut values[gy * nx + gx];
        *cell = if cell.is_nan() { v as f32 } else { cell.max(v as f32) };
    }

    Ok(MrmsField {
        values,
        nx,
        ny,
        lon_west: lonmin,
        lon_east: lonmax,
        lat_north: latmax,
        lat_south: latmin,
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idx_range_finds_field() {
        let idx = "1:0:d=2026:REFC:entire atmosphere:1 hour fcst:\n\
                   2:396353:d=2026:RETOP:cloud top:1 hour fcst:\n";
        assert_eq!(field_byte_range(idx, "REFC", "entire atmosphere"), Some((0, Some(396353))));
        // Last message → open-ended range; var+level disambiguates same-var different levels.
        let idx2 = "1:100:d=2026:CAPE:surface:\n\
                    2:5000:d=2026:CAPE:90-0 mb above ground:\n\
                    3:9000:d=2026:HLCY:3000-0 m above ground:\n";
        assert_eq!(field_byte_range(idx2, "CAPE", "surface"), Some((100, Some(5000))));
        assert_eq!(field_byte_range(idx2, "CAPE", "90-0 mb above ground"), Some((5000, Some(9000))));
        assert_eq!(field_byte_range(idx2, "HLCY", "3000-0 m above ground"), Some((9000, None)));
    }

    #[test]
    fn regrid_scatters_into_regular_grid() {
        // Two points ~0.1° apart land in distinct cells; the higher dBZ wins its cell.
        let lats = vec![40.0, 40.0, 40.001];
        let lons = vec![-100.0, -99.9, -100.0];
        let data = vec![25.0, 50.0, 45.0]; // first and third share a cell → keep max (45)
        let f = regrid(&lats, &lons, &data, Utc::now(), -30.0).unwrap();
        assert!(f.nx >= 2 && f.ny >= 1);
        let north_west = f.values[0]; // row 0 = north, col 0 = west
        assert!((north_west - 45.0).abs() < 1e-3, "max-per-cell kept: {north_west}");
    }

    #[test]
    fn regrid_min_valid_keeps_negatives_for_srh() {
        // A −50 SRH sample survives with min_valid = −∞ but is dropped at the REFC −30 threshold.
        // Two spread points give the grid a finite extent; the NW cell (row 0, col 0) is the −50.
        let lats = vec![41.0, 40.0];
        let lons = vec![-100.0, -99.0];
        let data = vec![-50.0, 20.0];
        let kept = regrid(&lats, &lons, &data, Utc::now(), f64::NEG_INFINITY).unwrap();
        assert!((kept.values[0] - -50.0).abs() < 1e-3, "negative SRH kept in NW cell: {}", kept.values[0]);
        let dropped = regrid(&lats, &lons, &data, Utc::now(), -30.0).unwrap();
        assert!(dropped.values[0].is_nan(), "below-threshold dropped");
    }
}
