//! Global model fields: NOAA's GFS and ECMWF's open IFS.
//!
//! Both publish quarter-degree GRIB2 on a plain lat/lon grid with a sidecar index, so a field is
//! two requests: read the index, range-GET the one message. That is the same shape as the HRRR
//! path, and the decode and regrid are literally the same code — only the URL and the index
//! format differ.
//!
//! * **GFS** — `noaa-gfs-bdp-pds`, NOAA's `.idx` text format, simple packing.
//! * **ECMWF** — `data.ecmwf.int`, a JSON-lines `.index`, CCSDS packing (which the vendored
//!   gribberish decodes in pure Rust, so this works on wasm too).
//!
//! Longitudes arrive on 0..360 and are wrapped to −180..180 inside the shared regrid, so a field
//! that spans the dateline lands continuous instead of drawing one quad across the whole map.
//!
//! ponytail: one quad covering −180..180, so the field does not repeat past the antimeridian —
//! pan east of +180 and it simply ends. Drawing a second copy is easy if anyone chases Fiji.

use crate::alerts::USER_AGENT;
use crate::mrms::MrmsField;
use chrono::{DateTime, Datelike, Timelike, Utc};

const GFS_BUCKET: &str = "https://noaa-gfs-bdp-pds.s3.amazonaws.com";
const ECMWF_BASE: &str = "https://data.ecmwf.int/forecasts";

/// Quarter-degree source grids resample onto this. Coarser than the grid itself, so the scatter
/// fills every cell; 1440×721 at 0.25° well under the 4096 texture cap either way.
const RES_DEG: f64 = 0.3;

/// Which global model to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlobalModel {
    #[default]
    Gfs,
    Ecmwf,
}

impl GlobalModel {
    pub fn label(self) -> &'static str {
        match self {
            GlobalModel::Gfs => "GFS",
            GlobalModel::Ecmwf => "ECMWF",
        }
    }

    /// Hours between cycles. Both run four times a day.
    fn cycle_step(self) -> u32 {
        6
    }
}

/// A field a global model can draw. Kept to what both publish, so switching source keeps the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalField {
    Mslp,
    Height500,
    Temp2m,
    Wind10m,
    Precip,
}

impl GlobalField {
    pub const ALL: [GlobalField; 5] = [
        GlobalField::Mslp,
        GlobalField::Height500,
        GlobalField::Temp2m,
        GlobalField::Wind10m,
        GlobalField::Precip,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GlobalField::Mslp => "MSLP",
            GlobalField::Height500 => "500 hPa height",
            GlobalField::Temp2m => "2 m temp",
            GlobalField::Wind10m => "10 m wind",
            GlobalField::Precip => "Total precip",
        }
    }

    /// Stable slug for settings and the headless CLI.
    pub fn slug(self) -> &'static str {
        match self {
            GlobalField::Mslp => "mslp",
            GlobalField::Height500 => "gh500",
            GlobalField::Temp2m => "t2m",
            GlobalField::Wind10m => "wind10m",
            GlobalField::Precip => "precip",
        }
    }

    pub fn from_slug(s: &str) -> Option<GlobalField> {
        GlobalField::ALL.into_iter().find(|f| f.slug() == s)
    }

    /// GFS `.idx` `(var, level)`.
    fn gfs_key(self) -> (&'static str, &'static str) {
        match self {
            GlobalField::Mslp => ("PRMSL", "mean sea level"),
            GlobalField::Height500 => ("HGT", "500 mb"),
            GlobalField::Temp2m => ("TMP", "2 m above ground"),
            GlobalField::Wind10m => ("UGRD", "10 m above ground"),
            GlobalField::Precip => ("PWAT", "entire atmosphere (considered as a single layer)"),
        }
    }

    /// ECMWF index `(param, levtype, level)`.
    fn ecmwf_key(self) -> (&'static str, &'static str, Option<&'static str>) {
        match self {
            GlobalField::Mslp => ("msl", "sfc", None),
            GlobalField::Height500 => ("gh", "pl", Some("500")),
            GlobalField::Temp2m => ("2t", "sfc", None),
            GlobalField::Wind10m => ("10u", "sfc", None),
            GlobalField::Precip => ("tp", "sfc", None),
        }
    }
}

/// One decoded global field plus the cycle it came from.
pub struct GlobalForecast {
    pub field: MrmsField,
    pub run: DateTime<Utc>,
    pub fcst_hour: u16,
}

impl GlobalForecast {
    pub fn valid(&self) -> DateTime<Utc> {
        self.run + chrono::Duration::hours(self.fcst_hour as i64)
    }
}

/// Fetch `field` at forecast hour `fh`, walking back through recent cycles until one has it.
///
/// Global models post slowly — GFS takes a few hours to finish a cycle — so the newest cycle
/// directory usually exists before the file does. Walking back is what makes the layer reliable.
pub async fn fetch(
    http: &reqwest::Client,
    model: GlobalModel,
    field: GlobalField,
    fh: u16,
) -> anyhow::Result<GlobalForecast> {
    let now = Utc::now();
    let mut last_err = None;
    for back in 0..5 {
        let step = model.cycle_step() as i64;
        let hours = (now.hour() as i64 / step) * step - back * step;
        let run = (now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc())
            + chrono::Duration::hours(hours);
        match fetch_run(http, model, field, run, fh).await {
            Ok(f) => return Ok(f),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no {} cycle found", model.label())))
}

async fn fetch_run(
    http: &reqwest::Client,
    model: GlobalModel,
    field: GlobalField,
    run: DateTime<Utc>,
    fh: u16,
) -> anyhow::Result<GlobalForecast> {
    let date = format!("{:04}{:02}{:02}", run.year(), run.month(), run.day());
    let (base, range) = match model {
        GlobalModel::Gfs => {
            let base = format!(
                "{GFS_BUCKET}/gfs.{date}/{:02}/atmos/gfs.t{:02}z.pgrb2.0p25.f{fh:03}",
                run.hour(),
                run.hour()
            );
            let idx = get_text(http, &format!("{base}.idx")).await?;
            let (var, level) = field.gfs_key();
            let r = crate::hrrr::field_byte_range(&idx, var, level)
                .ok_or_else(|| anyhow::anyhow!("no {var}:{level} in GFS idx"))?;
            (base, r)
        }
        GlobalModel::Ecmwf => {
            let base = format!(
                "{ECMWF_BASE}/{date}/{:02}z/ifs/0p25/oper/{date}{:02}0000-{fh}h-oper-fc.grib2",
                run.hour(),
                run.hour()
            );
            let idx = get_text(http, &format!("{}.index", strip_ext(&base))).await?;
            let r = ecmwf_byte_range(&idx, field)
                .ok_or_else(|| anyhow::anyhow!("no {:?} in ECMWF index", field))?;
            (base, r)
        }
    };

    let (start, end) = range;
    let http_range = match end {
        Some(e) => format!("bytes={start}-{}", e - 1),
        None => format!("bytes={start}-"),
    };
    let bytes = http
        .get(crate::net::fetch_url(&base))
        .header("User-Agent", USER_AGENT)
        .header("Range", http_range)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let raw = bytes.to_vec();
    let field_out = crate::task::blocking(move || decode(&raw)).await??;
    Ok(GlobalForecast {
        field: field_out,
        run,
        fcst_hour: fh,
    })
}

/// `foo.grib2` → `foo`, for the sidecar whose extension replaces rather than appends.
fn strip_ext(url: &str) -> &str {
    url.strip_suffix(".grib2").unwrap_or(url)
}

async fn get_text(http: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    Ok(http
        .get(crate::net::fetch_url(url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Byte range for a field in an ECMWF JSON-lines `.index`.
///
/// Each line is one message: `{"param":"2t","levtype":"sfc","_offset":N,"_length":M,…}`. Offsets
/// and lengths are given outright, so unlike NOAA's `.idx` there is no next-line arithmetic.
///
/// ponytail: substring matching rather than a JSON parse — these lines are machine-generated and
/// flat. A real parse is one `serde_json::from_str` away if the format ever grows nesting.
fn ecmwf_byte_range(index: &str, field: GlobalField) -> Option<(u64, Option<u64>)> {
    let (param, levtype, level) = field.ecmwf_key();
    for line in index.lines() {
        if !line.contains(&format!("\"param\": \"{param}\""))
            && !line.contains(&format!("\"param\":\"{param}\""))
        {
            continue;
        }
        if !line.contains(&format!("\"{levtype}\"")) {
            continue;
        }
        if let Some(lv) = level {
            if !line.contains(&format!("\"levelist\": \"{lv}\""))
                && !line.contains(&format!("\"levelist\":\"{lv}\""))
            {
                continue;
            }
        }
        let offset = json_number(line, "_offset")?;
        let length = json_number(line, "_length")?;
        return Some((offset, Some(offset + length)));
    }
    None
}

/// Pull an unquoted numeric value out of one flat JSON line.
fn json_number(line: &str, key: &str) -> Option<u64> {
    let at = line.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = line[at..].trim_start_matches([':', ' ']);
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Decode one GRIB2 message onto the shared regular lat/lon grid.
fn decode(raw: &[u8]) -> anyhow::Result<MrmsField> {
    use gribberish::data_message::DataMessage;
    use gribberish::message::read_message;
    let msg = read_message(raw, 0).ok_or_else(|| anyhow::anyhow!("no GRIB2 message"))?;
    let time = msg.forecast_date().unwrap_or_else(|_| Utc::now());
    let dm = DataMessage::try_from(&msg).map_err(|e| anyhow::anyhow!("global decode: {e:?}"))?;
    let (lats, lons) = dm.metadata.latlng();
    let data = dm.data;
    // A regular lat/lon grid hands back its two axes, not a coordinate per point (which is what
    // a Lambert projection like HRRR's produces). Expand the axes to the full grid so the shared
    // regrid sees the same shape either way.
    let (lats, lons) = if lats.len() * lons.len() == data.len() {
        let mut la = Vec::with_capacity(data.len());
        let mut lo = Vec::with_capacity(data.len());
        for lat in &lats {
            for lon in &lons {
                la.push(*lat);
                lo.push(*lon);
            }
        }
        (la, lo)
    } else {
        (lats, lons)
    };
    anyhow::ensure!(
        lats.len() == data.len() && lons.len() == data.len(),
        "global latlng/data length mismatch"
    );
    crate::hrrr::regrid(&lats, &lons, &data, time, RES_DEG, f64::NEG_INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecmwf_index_lines_resolve_to_ranges() {
        let idx = "{\"domain\": \"g\", \"param\": \"msl\", \"levtype\": \"sfc\", \"_offset\": 100, \"_length\": 250}\n\
                   {\"domain\": \"g\", \"param\": \"gh\", \"levtype\": \"pl\", \"levelist\": \"850\", \"_offset\": 400, \"_length\": 60}\n\
                   {\"domain\": \"g\", \"param\": \"gh\", \"levtype\": \"pl\", \"levelist\": \"500\", \"_offset\": 500, \"_length\": 75}\n";
        assert_eq!(
            ecmwf_byte_range(idx, GlobalField::Mslp),
            Some((100, Some(350)))
        );
        // The right pressure level, not just the right parameter.
        assert_eq!(
            ecmwf_byte_range(idx, GlobalField::Height500),
            Some((500, Some(575)))
        );
        assert_eq!(ecmwf_byte_range(idx, GlobalField::Temp2m), None);
    }

    #[test]
    fn field_slugs_round_trip() {
        for f in GlobalField::ALL {
            assert_eq!(GlobalField::from_slug(f.slug()), Some(f));
        }
        assert_eq!(GlobalField::from_slug("nope"), None);
    }

    /// Both sources, live, at the newest usable cycle.
    /// `cargo test -p wxdata global_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn global_live() {
        let http = reqwest::Client::new();
        for model in [GlobalModel::Gfs, GlobalModel::Ecmwf] {
            let f = fetch(&http, model, GlobalField::Mslp, 0)
                .await
                .unwrap_or_else(|e| panic!("{} fetch: {e}", model.label()));
            let finite = f.field.values.iter().filter(|v| v.is_finite()).count();
            println!(
                "{}: {}x{} lon {:.1}..{:.1} lat {:.1}..{:.1} finite {finite}",
                model.label(),
                f.field.nx,
                f.field.ny,
                f.field.lon_west,
                f.field.lon_east,
                f.field.lat_south,
                f.field.lat_north
            );
            // The whole point of the longitude wrap: a global field lands in −180..180.
            assert!(f.field.lon_west >= -180.5 && f.field.lon_east <= 180.5);
            assert!(finite > 0);
        }
    }
}
