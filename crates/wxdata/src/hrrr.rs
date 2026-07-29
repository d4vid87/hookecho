//! HRRR "future radar": composite reflectivity (REFC) forecast grids from the NOAA HRRR AWS PDS.
//!
//! Fetches only the REFC message of a `wrfsfcf{HH}` file via an `.idx` byte-range request
//! (~0.4 MB instead of the ~130 MB full file), decodes it with `gribberish` (Lambert-conformal
//! grid), then scatter-regrids the native grid onto a regular lat/lon grid so it can reuse the
//! MRMS field-layer render pipeline (a plate-carrée→mercator warp).

use crate::alerts::USER_AGENT;
use crate::mrms::MrmsField;
use chrono::{DateTime, Datelike, Timelike, Utc};
use futures_util::StreamExt;

const BUCKET: &str = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com";
const RAP_BUCKET: &str = "https://noaa-rap-pds.s3.amazonaws.com";

/// Which model to pull a field from.
///
/// HRRR is the forecast model this module was written for. RAP is here for its **f00 analysis**:
/// the same fields, assimilated from observations rather than projected forward, which is what
/// people mean when they ask for "mesoanalysis" (SPC's own surface objective analysis is RAP plus
/// surface obs). It costs one URL and one grid spacing — everything downstream is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Model {
    #[default]
    Hrrr,
    Rap,
}

impl Model {
    /// The GRIB2 file for a cycle + forecast hour.
    fn url(self, date: &str, cycle_hour: u32, fh: u8) -> String {
        match self {
            Model::Hrrr => {
                format!("{BUCKET}/hrrr.{date}/conus/hrrr.t{cycle_hour:02}z.wrfsfcf{fh:02}.grib2")
            }
            // awp130 is the 13 km CONUS pressure/surface product — the one with CAPE and helicity.
            Model::Rap => {
                format!("{RAP_BUCKET}/rap.{date}/rap.t{cycle_hour:02}z.awp130pgrbf{fh:02}.grib2")
            }
        }
    }

    /// Regular-grid cell size (degrees) for the regrid: a shade coarser than the model's native
    /// spacing, so the scatter fills every target cell instead of leaving a grid of holes.
    /// HRRR is ~3 km, RAP ~13 km.
    fn res_deg(self) -> f64 {
        match self {
            Model::Hrrr => 0.04,
            Model::Rap => 0.15,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Hrrr => "HRRR",
            Model::Rap => "RAP",
        }
    }
}

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
    fetch_field(http, Model::Hrrr, "REFC", "entire atmosphere", fcst_hour, -30.0).await
}

/// Fetch any single HRRR surface field for `fcst_hour` by variable + level idx strings, regridding
/// with `min_valid` as the drop threshold (REFC uses −30 dBZ; CAPE 0; SRH −∞ so negatives survive).
/// Walks back up to 6 cycles until a run has this forecast hour posted.
pub async fn fetch_field(
    http: &reqwest::Client,
    model: Model,
    var: &str,
    level: &str,
    fcst_hour: u8,
    min_valid: f64,
) -> anyhow::Result<HrrrForecast> {
    let fh = fcst_hour.min(18);
    let now = Utc::now();
    let mut last_err = None;
    for back in 1..=6 {
        let run = (now - chrono::Duration::hours(back))
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        match fetch_run_field(http, model, run, fh, var, level, min_valid).await {
            Ok(field) => {
                return Ok(HrrrForecast {
                    field,
                    run,
                    fcst_hour: fh,
                })
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no HRRR run found")))
}

/// How many HRRR field fetches run at once. NOMADS is a shared public service; six is brisk
/// without being rude, and past that the regrid work dominates anyway.
const HRRR_CONCURRENCY: usize = 6;

/// Fetch several surface fields from ONE model cycle: walks back up to 6 runs and only returns
/// when every `(var, level, min_valid)` spec resolves against the same run. Composite parameters
/// (STP/SCP/EHI) must not mix ingredients from different cycles, and per-field [`fetch_field`]
/// calls can land on different ones when a newer run is mid-upload.
pub async fn fetch_fields_one_run(
    http: &reqwest::Client,
    model: Model,
    fcst_hour: u8,
    specs: &[(&str, &str, f64)],
) -> anyhow::Result<(DateTime<Utc>, Vec<MrmsField>)> {
    let fh = fcst_hour.min(18);
    // Owned up front: the concurrent stream below must not borrow `specs` across an await, or
    // the whole future stops being `Send` and the app can't spawn it.
    let owned_specs: Vec<(String, String, f64)> = specs
        .iter()
        .map(|(v, l, m)| (v.to_string(), l.to_string(), *m))
        .collect();
    let now = Utc::now();
    let mut last_err = None;
    for back in 1..=6 {
        let run = (now - chrono::Duration::hours(back))
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        let results: Vec<_> = futures_util::stream::iter(owned_specs.clone().into_iter().map(
            |(var, level, mv): (String, String, f64)| {
                let http = http.clone();
                async move { fetch_run_field(&http, model, run, fh, &var, &level, mv).await }
            },
        ))
        .buffered(HRRR_CONCURRENCY)
        .collect()
        .await;
        let mut fields = Vec::with_capacity(specs.len());
        let mut failed = None;
        for r in results {
            match r {
                Ok(f) => fields.push(f),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        match failed {
            None => return Ok((run, fields)),
            Some(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no HRRR run found")))
}

/// Fetch one field across forecast hours `1..=through_hour` from a SINGLE model cycle and fold
/// them into one grid by elementwise max — the "swath" a max-per-hour field is meant to be read as.
///
/// HRRR's max fields (MXUPHL and friends) each cover one hour's window, so a single hour answers
/// "where was rotation strongest between F+2 and F+3" — useful, but the map chasers actually want
/// is the union: everywhere a rotating storm is forecast to pass between now and then. Hours come
/// from one run for the same reason [`fetch_fields_one_run`] exists.
pub async fn fetch_field_swath(
    http: &reqwest::Client,
    var: &str,
    level: &str,
    through_hour: u8,
    min_valid: f64,
) -> anyhow::Result<HrrrForecast> {
    let through = through_hour.clamp(1, 18);
    let now = Utc::now();
    let mut last_err = None;
    for back in 1..=6 {
        let run = (now - chrono::Duration::hours(back))
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        // Up to 18 forecast hours, each its own ranged GRIB fetch. Sequentially that is 18
        // round trips stacked end to end; six at a time cuts the wall clock to roughly a third.
        // The fold is still ordered, and still all-or-nothing.
        // Each future owns its inputs (a `reqwest::Client` clone is a refcount bump): borrowed
        // ones make the combined future non-`Send`, which the app's tokio spawn requires.
        let results: Vec<_> = futures_util::stream::iter((1..=through).map(|fh| {
            let (http, var, level) = (http.clone(), var.to_string(), level.to_string());
            async move { fetch_run_field(&http, Model::Hrrr, run, fh, &var, &level, min_valid).await }
        }))
        .buffered(HRRR_CONCURRENCY)
        .collect()
        .await;
        let mut acc: Option<MrmsField> = None;
        let mut failed = None;
        for r in results {
            match r {
                Ok(f) => match acc.as_mut() {
                    None => acc = Some(f),
                    Some(a) => merge_max(a, &f),
                },
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        match (failed, acc) {
            (None, Some(field)) => {
                return Ok(HrrrForecast {
                    field,
                    run,
                    fcst_hour: through,
                })
            }
            (e, _) => last_err = e.or(last_err),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no HRRR run found")))
}

/// Fold `src` into `dst` by keeping the larger value per cell. Both come from the same run and
/// the same regrid parameters, so the grids are aligned; mismatched shapes are left alone rather
/// than producing a scrambled field.
fn merge_max(dst: &mut MrmsField, src: &MrmsField) {
    if dst.nx != src.nx || dst.ny != src.ny || dst.values.len() != src.values.len() {
        return;
    }
    for (d, s) in dst.values.iter_mut().zip(&src.values) {
        if s.is_nan() {
            continue;
        }
        if d.is_nan() || s > d {
            *d = *s;
        }
    }
}

async fn fetch_run_field(
    http: &reqwest::Client,
    model: Model,
    run: DateTime<Utc>,
    fh: u8,
    var: &str,
    level: &str,
    min_valid: f64,
) -> anyhow::Result<MrmsField> {
    let date = format!("{:04}{:02}{:02}", run.year(), run.month(), run.day());
    let base = model.url(&date, run.hour(), fh);

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
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_regrid(&bytes, model, min_valid)
    }))
    .unwrap_or_else(|_| anyhow::bail!("{} grib decode panicked", model.label()))
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
            // The end is the next *distinct* offset. RAP's idx lists a message that packs several
            // fields (VUCSH/VVCSH) as sibling lines sharing one offset; taking the very next line
            // would build the empty byte range `start..start` and fetch nothing.
            let end = lines[i + 1..]
                .iter()
                .filter_map(|n| n.split(':').nth(1))
                .filter_map(|s| s.parse::<u64>().ok())
                .find(|&o| o > start);
            return Some((start, end));
        }
    }
    None
}

/// Decode a single-message HRRR GRIB2 (Lambert grid) and scatter-regrid onto a regular lat/lon
/// grid, keeping the max dBZ per target cell (reflectivity composites well under max).
fn decode_regrid(raw: &[u8], model: Model, min_valid: f64) -> anyhow::Result<MrmsField> {
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

    regrid(&lats, &lons, &data, time, model.res_deg(), min_valid)
}

/// Scatter native (lat, lon, value) triples onto a regular lat/lon grid (max per cell).
/// Pure + fixture-testable. Non-finite or below-`min_valid` samples are ignored.
/// `// ponytail: max-per-cell — for SRH this keeps the strongest (most positive) value per cell;`
/// `// negative (anticyclonic) SRH is retained only where no positive sample shares the cell.`
fn regrid(
    lats: &[f64],
    lons: &[f64],
    data: &[f64],
    time: DateTime<Utc>,
    res_deg: f64,
    min_valid: f64,
) -> anyhow::Result<MrmsField> {
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
    anyhow::ensure!(
        lonmax > lonmin && latmax > latmin,
        "hrrr grid has no finite extent"
    );

    let nx = (((lonmax - lonmin) / res_deg).ceil() as usize).max(1);
    let ny = (((latmax - latmin) / res_deg).ceil() as usize).max(1);
    let mut values = vec![f32::NAN; nx * ny];
    for k in 0..data.len() {
        let v = data[k];
        if !v.is_finite() || v < min_valid || !lats[k].is_finite() || !lons[k].is_finite() {
            continue;
        }
        let gx = (((lons[k] - lonmin) / res_deg) as usize).min(nx - 1);
        // Row 0 is the northernmost latitude (matches MrmsField convention).
        let gy = (((latmax - lats[k]) / res_deg) as usize).min(ny - 1);
        let cell = &mut values[gy * nx + gx];
        *cell = if cell.is_nan() {
            v as f32
        } else {
            cell.max(v as f32)
        };
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
        assert_eq!(
            field_byte_range(idx, "REFC", "entire atmosphere"),
            Some((0, Some(396353)))
        );
        // Last message → open-ended range; var+level disambiguates same-var different levels.
        let idx2 = "1:100:d=2026:CAPE:surface:\n\
                    2:5000:d=2026:CAPE:90-0 mb above ground:\n\
                    3:9000:d=2026:HLCY:3000-0 m above ground:\n";
        assert_eq!(
            field_byte_range(idx2, "CAPE", "surface"),
            Some((100, Some(5000)))
        );
        assert_eq!(
            field_byte_range(idx2, "CAPE", "90-0 mb above ground"),
            Some((5000, Some(9000)))
        );
        assert_eq!(
            field_byte_range(idx2, "HLCY", "3000-0 m above ground"),
            Some((9000, None))
        );
        // RAP packs some pairs into one message, listed as sibling lines at the same offset; the
        // range must run to the next distinct offset, not to the sibling.
        let rap = "241.1:100:d=2026:USTM:0-6000 m above ground:anl:\n\
                   241.2:100:d=2026:VSTM:0-6000 m above ground:anl:\n\
                   242.1:200:d=2026:VUCSH:0-6000 m above ground:anl:\n";
        assert_eq!(
            field_byte_range(rap, "USTM", "0-6000 m above ground"),
            Some((100, Some(200)))
        );
    }

    #[test]
    fn rap_and_hrrr_urls() {
        assert!(Model::Hrrr
            .url("20260728", 21, 0)
            .ends_with("hrrr.20260728/conus/hrrr.t21z.wrfsfcf00.grib2"));
        assert!(Model::Rap
            .url("20260728", 21, 0)
            .ends_with("rap.20260728/rap.t21z.awp130pgrbf00.grib2"));
        assert!(Model::Rap.res_deg() > Model::Hrrr.res_deg(), "13 km vs 3 km");
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn rap_analysis_cape_decodes() {
        let http = reqwest::Client::new();
        let fc = fetch_field(&http, Model::Rap, "CAPE", "surface", 0, 0.0)
            .await
            .expect("RAP f00 CAPE");
        let finite: Vec<f32> = fc.field.values.iter().copied().filter(|v| v.is_finite()).collect();
        let max = finite.iter().copied().fold(f32::MIN, f32::max);
        eprintln!(
            "RAP {}x{} run {} — {} finite cells, max {max:.0} J/kg",
            fc.field.nx,
            fc.field.ny,
            fc.run,
            finite.len()
        );
        // A 13 km CONUS grid regridded at 0.14° should cover most of its own box, and surface CAPE
        // anywhere in the country tops out well under 10000 J/kg.
        assert!(finite.len() > fc.field.values.len() / 2, "coverage holes");
        assert!((0.0..10_000.0).contains(&max), "implausible CAPE {max}");

        // Cross-check against the HRRR analysis for the same hour: different models on different
        // grids won't match cell for cell, but their CONUS-wide peak CAPE should be the same order.
        let hrrr = fetch_field(&http, Model::Hrrr, "CAPE", "surface", 0, 0.0)
            .await
            .expect("HRRR f00 CAPE");
        let hmax = hrrr
            .field
            .values
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f32::MIN, f32::max);
        eprintln!("peak CAPE — RAP {max:.0} vs HRRR {hmax:.0} J/kg");
        let ratio = (max as f64 / hmax.max(1.0) as f64).max(hmax as f64 / max.max(1.0) as f64);
        assert!(ratio < 3.0, "RAP {max} and HRRR {hmax} disagree wildly");
    }

    #[test]
    fn regrid_scatters_into_regular_grid() {
        // Two points ~0.1° apart land in distinct cells; the higher dBZ wins its cell.
        let lats = vec![40.0, 40.0, 40.001];
        let lons = vec![-100.0, -99.9, -100.0];
        let data = vec![25.0, 50.0, 45.0]; // first and third share a cell → keep max (45)
        let f = regrid(&lats, &lons, &data, Utc::now(), 0.04, -30.0).unwrap();
        assert!(f.nx >= 2 && f.ny >= 1);
        let north_west = f.values[0]; // row 0 = north, col 0 = west
        assert!(
            (north_west - 45.0).abs() < 1e-3,
            "max-per-cell kept: {north_west}"
        );
    }

    #[test]
    fn regrid_min_valid_keeps_negatives_for_srh() {
        // A −50 SRH sample survives with min_valid = −∞ but is dropped at the REFC −30 threshold.
        // Two spread points give the grid a finite extent; the NW cell (row 0, col 0) is the −50.
        let lats = vec![41.0, 40.0];
        let lons = vec![-100.0, -99.0];
        let data = vec![-50.0, 20.0];
        let kept = regrid(&lats, &lons, &data, Utc::now(), 0.04, f64::NEG_INFINITY).unwrap();
        assert!(
            (kept.values[0] - -50.0).abs() < 1e-3,
            "negative SRH kept in NW cell: {}",
            kept.values[0]
        );
        let dropped = regrid(&lats, &lons, &data, Utc::now(), 0.04, -30.0).unwrap();
        assert!(dropped.values[0].is_nan(), "below-threshold dropped");
    }
}
