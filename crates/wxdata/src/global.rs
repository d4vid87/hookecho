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
use crate::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    ModelDefinition, QualitySummary, SamplingPolicy, ValueKind,
};
use crate::mrms::MrmsField;
use chrono::{DateTime, Datelike, Timelike, Utc};

const GFS_BUCKET: &str = "https://noaa-gfs-bdp-pds.s3.amazonaws.com";
const GEFS_BUCKET: &str = "https://noaa-gefs-pds.s3.amazonaws.com";
const ECMWF_BASE: &str = "https://data.ecmwf.int/forecasts";

/// Quarter-degree source grids resample onto this. Coarser than the grid itself, so the scatter
/// fills every cell; 1440×721 at 0.25° well under the 4096 texture cap either way.
const RES_DEG: f64 = 0.3;

fn available_members(model: GlobalModel) -> Option<u16> {
    match model {
        GlobalModel::GefsMean | GlobalModel::GefsSpread => Some(31),
        GlobalModel::GefsMember(_) => Some(1),
        _ => None,
    }
}

/// Point statistics used by GEFS plumes and threshold probabilities.
#[derive(Debug, Clone, PartialEq)]
pub struct EnsembleDistribution {
    pub available: usize,
    pub expected: usize,
    pub minimum: f32,
    pub percentile_10: f32,
    pub median: f32,
    pub percentile_90: f32,
    pub maximum: f32,
    pub mean: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GefsPointDistribution {
    pub field: GlobalField,
    pub run: DateTime<Utc>,
    pub valid: DateTime<Utc>,
    pub longitude: f64,
    pub latitude: f64,
    pub statistics: EnsembleDistribution,
}

/// Summarize the members that actually supplied a finite value.
pub fn ensemble_distribution(
    members: impl IntoIterator<Item = Option<f32>>,
    expected: usize,
) -> Option<EnsembleDistribution> {
    let mut values: Vec<f32> = members
        .into_iter()
        .flatten()
        .filter(|value| value.is_finite())
        .collect();
    values.sort_by(f32::total_cmp);
    let available = values.len();
    if available == 0 {
        return None;
    }
    let percentile = |p: f32| values[((available - 1) as f32 * p).round() as usize];
    Some(EnsembleDistribution {
        available,
        expected,
        minimum: values[0],
        percentile_10: percentile(0.1),
        median: percentile(0.5),
        percentile_90: percentile(0.9),
        maximum: values[available - 1],
        mean: values.iter().sum::<f32>() / available as f32,
    })
}

/// Fraction of available members at or above `threshold`, plus the explicit sample count.
pub fn exceedance_probability(
    members: impl IntoIterator<Item = Option<f32>>,
    threshold: f32,
) -> Option<(f32, usize)> {
    let values: Vec<f32> = members
        .into_iter()
        .flatten()
        .filter(|value| value.is_finite())
        .collect();
    (!values.is_empty()).then(|| {
        let hits = values.iter().filter(|&&value| value >= threshold).count();
        (hits as f32 / values.len() as f32, values.len())
    })
}

/// Load one GEFS cycle across all 31 members and summarize a native point value.
/// Missing members stay missing and are reflected in `statistics.available`.
pub async fn fetch_gefs_point_distribution(
    http: &reqwest::Client,
    field: GlobalField,
    fh: u16,
    longitude: f64,
    latitude: f64,
) -> anyhow::Result<GefsPointDistribution> {
    GlobalModel::GefsMember(0).validate_forecast_hour(fh)?;
    let now = Utc::now();
    let step = GlobalModel::GefsMember(0).cycle_step() as i64;
    let mut last_error = None;
    for back in 0..5 {
        let hours = (now.hour() as i64 / step) * step - back * step;
        let run = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
            + chrono::Duration::hours(hours);
        if !GlobalModel::GefsMember(0).supports_forecast_hour(run.hour(), fh) {
            continue;
        }
        let mut samples = Vec::with_capacity(31);
        for batch in (0u8..=30).collect::<Vec<_>>().chunks(6) {
            let requests = batch.iter().map(|member| {
                fetch_run(http, GlobalModel::GefsMember(*member), field, run, fh)
            });
            for result in futures_util::future::join_all(requests).await {
                match result {
                    Ok(forecast) => samples.push(forecast.field.sample_bilinear(longitude, latitude)),
                    Err(error) => {
                        last_error = Some(error);
                        samples.push(None);
                    }
                }
            }
        }
        if let Some(statistics) = ensemble_distribution(samples, 31) {
            return Ok(GefsPointDistribution {
                field,
                run,
                valid: run + chrono::Duration::hours(fh as i64),
                longitude,
                latitude,
                statistics,
            });
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no GEFS member cycle found")))
}

macro_rules! descriptor {
    ($name:ident, $id:literal, $display:literal, $short:literal, $aliases:expr, $units:literal, $kind:expr) => {
        pub static $name: FieldDescriptor = FieldDescriptor {
            id: FieldId($id),
            source: "NOAA GFS / ECMWF IFS",
            family: FieldFamily::Model,
            display_name: $display,
            short_name: $short,
            search_aliases: $aliases,
            units: $units,
            value_kind: $kind,
            palette_key: $id,
            sampling: SamplingPolicy::Bilinear,
            missing: MissingData::Nan,
            time_policy: None,
            supports_contours: false,
            supports_difference: true,
        };
    };
}

descriptor!(
    MSLP_DESCRIPTOR,
    "model.global.mslp",
    "Mean sea-level pressure",
    "MSLP",
    &["pressure", "GFS", "ECMWF"],
    "Pa",
    ValueKind::Scalar
);
descriptor!(
    HEIGHT_500_DESCRIPTOR,
    "model.global.height-500",
    "500 hPa height",
    "500 hPa",
    &["geopotential", "height", "GFS", "ECMWF"],
    "m",
    ValueKind::Scalar
);
descriptor!(
    TEMP_2M_DESCRIPTOR,
    "model.global.temperature-2m",
    "2 m temperature",
    "2 m temp",
    &["temperature", "GFS", "ECMWF"],
    "K",
    ValueKind::Scalar
);
descriptor!(
    DEWPOINT_2M_DESCRIPTOR,
    "model.global.dewpoint-2m",
    "2 m dewpoint",
    "2 m dewpoint",
    &["moisture", "GFS", "ECMWF"],
    "K",
    ValueKind::Scalar
);
descriptor!(
    WIND_10M_DESCRIPTOR,
    "model.global.wind-10m",
    "10 m zonal wind",
    "10 m U wind",
    &["wind", "u component", "GFS", "ECMWF"],
    "m s-1",
    ValueKind::Scalar
);
descriptor!(
    PRECIP_DESCRIPTOR,
    "model.global.precipitable-water",
    "Precipitable water",
    "PWAT",
    &["total column water", "moisture", "GFS", "ECMWF"],
    "kg m-2",
    ValueKind::Scalar
);

/// Which global model to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlobalModel {
    #[default]
    Gfs,
    GefsMean,
    GefsSpread,
    /// NOAA GEFS control (`0`) or perturbed member (`1..=30`).
    GefsMember(u8),
    Ecmwf,
}

impl GlobalModel {
    pub fn definition(self) -> ModelDefinition {
        match self {
            Self::Gfs => ModelDefinition {
                id: "gfs",
                label: "GFS",
                provider: "NOAA",
                base_url: GFS_BUCKET,
                cycle_hours: 6,
                max_forecast_hour: 384,
                grid: "0.25 degree global latitude/longitude",
                regrid_resolution_deg: RES_DEG,
                index_suffix: ".idx",
                expected_latency_minutes: None,
                domain: "Global",
                ensemble: false,
            },
            Self::GefsMean => ModelDefinition {
                id: "gefs-mean",
                label: "GEFS mean",
                provider: "NOAA",
                base_url: GEFS_BUCKET,
                cycle_hours: 6,
                max_forecast_hour: 384,
                grid: "0.25/0.5 degree global latitude/longitude",
                regrid_resolution_deg: RES_DEG,
                index_suffix: ".idx",
                expected_latency_minutes: None,
                domain: "Global",
                ensemble: true,
            },
            Self::GefsSpread => ModelDefinition {
                id: "gefs-spread",
                label: "GEFS spread",
                provider: "NOAA",
                base_url: GEFS_BUCKET,
                cycle_hours: 6,
                max_forecast_hour: 384,
                grid: "0.25/0.5 degree global latitude/longitude",
                regrid_resolution_deg: RES_DEG,
                index_suffix: ".idx",
                expected_latency_minutes: None,
                domain: "Global",
                ensemble: true,
            },
            Self::GefsMember(_) => ModelDefinition {
                id: "gefs-member",
                label: "GEFS member",
                provider: "NOAA",
                base_url: GEFS_BUCKET,
                cycle_hours: 6,
                max_forecast_hour: 384,
                grid: "0.25/0.5 degree global latitude/longitude",
                regrid_resolution_deg: RES_DEG,
                index_suffix: ".idx",
                expected_latency_minutes: None,
                domain: "Global",
                ensemble: true,
            },
            Self::Ecmwf => ModelDefinition {
                id: "ecmwf-open-ifs",
                label: "ECMWF Open IFS",
                provider: "ECMWF",
                base_url: ECMWF_BASE,
                cycle_hours: 6,
                max_forecast_hour: 240,
                grid: "0.25 degree global latitude/longitude",
                regrid_resolution_deg: RES_DEG,
                index_suffix: ".index",
                expected_latency_minutes: None,
                domain: "Global",
                ensemble: false,
            },
        }
    }

    pub fn validate_forecast_hour(self, hour: u16) -> anyhow::Result<()> {
        if let Self::GefsMember(member) = self {
            anyhow::ensure!(member <= 30, "GEFS member {member} is outside 0..=30");
        }
        anyhow::ensure!(
            hour <= self.definition().max_forecast_hour,
            "{} forecast hour {hour} exceeds F{}",
            self.label(),
            self.definition().max_forecast_hour
        );
        Ok(())
    }

    pub fn supports_forecast_hour(self, cycle_hour: u32, hour: u16) -> bool {
        match self {
            Self::Gfs => hour <= 120 || hour <= 384 && hour.is_multiple_of(3),
            Self::GefsMean | Self::GefsSpread | Self::GefsMember(_) => {
                hour <= 240 && hour.is_multiple_of(3) || hour <= 384 && hour.is_multiple_of(6)
            }
            Self::Ecmwf if cycle_hour == 0 || cycle_hour == 12 => {
                hour <= 144 && hour.is_multiple_of(3) || hour <= 240 && hour.is_multiple_of(6)
            }
            Self::Ecmwf => hour <= 90 && hour.is_multiple_of(3),
        }
    }

    pub fn label(self) -> &'static str {
        self.definition().label
    }

    /// Hours between cycles. Both run four times a day.
    fn cycle_step(self) -> u32 {
        self.definition().cycle_hours
    }
}

/// A field a global model can draw. Kept to what both publish, so switching source keeps the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalField {
    Mslp,
    Height500,
    Temp2m,
    Dewpoint2m,
    Wind10m,
    Precip,
}

impl GlobalField {
    pub const ALL: [GlobalField; 6] = [
        GlobalField::Mslp,
        GlobalField::Height500,
        GlobalField::Temp2m,
        GlobalField::Dewpoint2m,
        GlobalField::Wind10m,
        GlobalField::Precip,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GlobalField::Mslp => "MSLP",
            GlobalField::Height500 => "500 hPa height",
            GlobalField::Temp2m => "2 m temp",
            GlobalField::Dewpoint2m => "2 m dewpoint",
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
            GlobalField::Dewpoint2m => "td2m",
            GlobalField::Wind10m => "wind10m",
            GlobalField::Precip => "precip",
        }
    }

    pub fn from_slug(s: &str) -> Option<GlobalField> {
        GlobalField::ALL.into_iter().find(|f| f.slug() == s)
    }

    pub fn descriptor(self) -> &'static FieldDescriptor {
        match self {
            GlobalField::Mslp => &MSLP_DESCRIPTOR,
            GlobalField::Height500 => &HEIGHT_500_DESCRIPTOR,
            GlobalField::Temp2m => &TEMP_2M_DESCRIPTOR,
            GlobalField::Dewpoint2m => &DEWPOINT_2M_DESCRIPTOR,
            GlobalField::Wind10m => &WIND_10M_DESCRIPTOR,
            GlobalField::Precip => &PRECIP_DESCRIPTOR,
        }
    }

    /// GFS `.idx` `(var, level)`.
    fn gfs_key(self) -> (&'static str, &'static str) {
        match self {
            GlobalField::Mslp => ("PRMSL", "mean sea level"),
            GlobalField::Height500 => ("HGT", "500 mb"),
            GlobalField::Temp2m => ("TMP", "2 m above ground"),
            GlobalField::Dewpoint2m => ("DPT", "2 m above ground"),
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
            GlobalField::Dewpoint2m => ("2d", "sfc", None),
            GlobalField::Wind10m => ("10u", "sfc", None),
            GlobalField::Precip => ("tcwv", "sfc", None),
        }
    }
}

/// One decoded global field plus the cycle it came from.
pub struct GlobalForecast {
    pub field: MrmsField,
    pub run: DateTime<Utc>,
    pub fcst_hour: u16,
    source_identity: String,
    received_at: DateTime<Utc>,
    available_members: Option<u16>,
}

impl GlobalForecast {
    pub fn valid(&self) -> DateTime<Utc> {
        self.run + chrono::Duration::hours(self.fcst_hour as i64)
    }

    pub fn into_frame(self, descriptor: &'static FieldDescriptor) -> FieldFrame {
        let valid_time = self.valid();
        FieldFrame::new(
            descriptor,
            self.field,
            DataStamp {
                source_identity: self.source_identity,
                issue_time: Some(self.run),
                run_time: Some(self.run),
                valid_time,
                received_time: self.received_at,
                class: DataClass::Forecast,
                quality: QualitySummary::Unknown,
                available_members: self.available_members,
            },
        )
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
    model.validate_forecast_hour(fh)?;
    anyhow::ensure!(
        (0..24).any(|cycle| model.supports_forecast_hour(cycle, fh)),
        "{} does not publish forecast hour {fh}",
        model.label()
    );
    let now = Utc::now();
    let mut last_err = None;
    for back in 0..5 {
        let step = model.cycle_step() as i64;
        let hours = (now.hour() as i64 / step) * step - back * step;
        let run = (now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc())
            + chrono::Duration::hours(hours);
        if !model.supports_forecast_hour(run.hour(), fh) {
            continue;
        }
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
            let source = model.definition().base_url;
            let base = format!(
                "{source}/gfs.{date}/{:02}/atmos/gfs.t{:02}z.pgrb2.0p25.f{fh:03}",
                run.hour(),
                run.hour()
            );
            let idx = get_text(http, &format!("{base}.idx")).await?;
            let (var, level) = field.gfs_key();
            let r = crate::hrrr::field_byte_range(&idx, var, level)
                .ok_or_else(|| anyhow::anyhow!("no {var}:{level} in GFS idx"))?;
            (base, r)
        }
        GlobalModel::GefsMean | GlobalModel::GefsSpread | GlobalModel::GefsMember(_) => {
            let source = model.definition().base_url;
            let product = match model {
                GlobalModel::GefsMean => "geavg".into(),
                GlobalModel::GefsSpread => "gespr".into(),
                GlobalModel::GefsMember(0) => "gec00".into(),
                GlobalModel::GefsMember(member) => format!("gep{member:02}"),
                _ => unreachable!(),
            };
            let (directory, name) = if field == GlobalField::Height500 {
                ("pgrb2ap5", "pgrb2a.0p50")
            } else {
                ("pgrb2sp25", "pgrb2s.0p25")
            };
            let base = format!(
                "{source}/gefs.{date}/{:02}/atmos/{directory}/{product}.t{:02}z.{name}.f{fh:03}",
                run.hour(),
                run.hour()
            );
            let idx = get_text(http, &format!("{base}.idx")).await?;
            let (var, level) = field.gfs_key();
            let r = crate::hrrr::field_byte_range(&idx, var, level)
                .ok_or_else(|| anyhow::anyhow!("no {var}:{level} in GEFS idx"))?;
            (base, r)
        }
        GlobalModel::Ecmwf => {
            let source = model.definition().base_url;
            let base = format!(
                "{source}/{date}/{:02}z/ifs/0p25/oper/{date}{:02}0000-{fh}h-oper-fc.grib2",
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
    let cached = crate::object_cache::fetch_range(http, &base, start, end).await?;
    let received_at = cached.received_at;

    let raw = cached.bytes;
    let field_out = crate::task::blocking(move || decode(&raw)).await??;
    Ok(GlobalForecast {
        field: field_out,
        run,
        fcst_hour: fh,
        source_identity: base,
        received_at,
        // NOAA publishes one control and 30 perturbed GEFS members. Mean/spread use the full set.
        available_members: available_members(model),
    })
}

/// `foo.grib2` → `foo`, for the sidecar whose extension replaces rather than appends.
fn strip_ext(url: &str) -> &str {
    url.strip_suffix(".grib2").unwrap_or(url)
}

async fn get_text(http: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    Ok(http
        .get(crate::net::fetch_url(url))
        .timeout(crate::net::FEED_TIMEOUT)
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
                   {\"domain\": \"g\", \"param\": \"gh\", \"levtype\": \"pl\", \"levelist\": \"500\", \"_offset\": 500, \"_length\": 75}\n\
                   {\"domain\": \"g\", \"param\": \"2d\", \"levtype\": \"sfc\", \"_offset\": 700, \"_length\": 90}\n";
        assert_eq!(
            ecmwf_byte_range(idx, GlobalField::Mslp),
            Some((100, Some(350)))
        );
        // The right pressure level, not just the right parameter.
        assert_eq!(
            ecmwf_byte_range(idx, GlobalField::Height500),
            Some((500, Some(575)))
        );
        assert_eq!(
            ecmwf_byte_range(idx, GlobalField::Dewpoint2m),
            Some((700, Some(790)))
        );
        assert_eq!(ecmwf_byte_range(idx, GlobalField::Temp2m), None);
    }

    #[test]
    fn field_slugs_round_trip() {
        for f in GlobalField::ALL {
            assert_eq!(GlobalField::from_slug(f.slug()), Some(f));
            assert_eq!(f.descriptor().family, FieldFamily::Model);
        }
        assert_eq!(GlobalField::from_slug("nope"), None);
    }

    #[test]
    fn both_models_request_compatible_column_water() {
        assert_eq!(GlobalField::Precip.gfs_key().0, "PWAT");
        assert_eq!(GlobalField::Precip.ecmwf_key().0, "tcwv");
        assert_eq!(GlobalField::Precip.descriptor().units, "kg m-2");
    }

    #[test]
    fn ensemble_statistics_report_only_available_members() {
        let distribution = ensemble_distribution(
            [Some(1.0), None, Some(f32::NAN), Some(3.0), Some(2.0)],
            5,
        )
        .unwrap();
        assert_eq!(distribution.available, 3);
        assert_eq!(distribution.expected, 5);
        assert_eq!(distribution.minimum, 1.0);
        assert_eq!(distribution.median, 2.0);
        assert_eq!(distribution.maximum, 3.0);
        assert_eq!(distribution.mean, 2.0);
        assert_eq!(
            exceedance_probability([Some(1.0), None, Some(3.0), Some(2.0)], 2.0),
            Some((2.0 / 3.0, 3))
        );
        assert!(ensemble_distribution([None, Some(f32::NAN)], 2).is_none());
        assert!(exceedance_probability([None, Some(f32::NAN)], 0.0).is_none());
    }

    #[test]
    fn global_models_share_complete_source_definitions() {
        let gfs = GlobalModel::Gfs.definition();
        let gefs = GlobalModel::GefsMean.definition();
        let spread = GlobalModel::GefsSpread.definition();
        let ecmwf = GlobalModel::Ecmwf.definition();
        assert_ne!(gfs.id, ecmwf.id);
        assert!(gefs.ensemble);
        assert!(spread.ensemble);
        assert_eq!(gfs.index_suffix, ".idx");
        assert_eq!(ecmwf.index_suffix, ".index");
        assert!(GlobalModel::Gfs.validate_forecast_hour(384).is_ok());
        assert!(GlobalModel::Gfs.validate_forecast_hour(385).is_err());
        assert!(GlobalModel::Ecmwf.validate_forecast_hour(240).is_ok());
        assert!(GlobalModel::Ecmwf.validate_forecast_hour(241).is_err());
        assert!(GlobalModel::Ecmwf.supports_forecast_hour(0, 240));
        assert!(!GlobalModel::Ecmwf.supports_forecast_hour(6, 93));
        assert!(!GlobalModel::Ecmwf.supports_forecast_hour(0, 145));
        assert!(GlobalModel::Gfs.supports_forecast_hour(18, 120));
        assert!(!GlobalModel::Gfs.supports_forecast_hour(18, 121));
        assert!(GlobalModel::Gfs.supports_forecast_hour(18, 123));
        assert!(GlobalModel::GefsMean.supports_forecast_hour(0, 240));
        assert!(!GlobalModel::GefsMean.supports_forecast_hour(0, 243));
        assert!(GlobalModel::GefsMean.supports_forecast_hour(0, 246));
        assert_eq!(available_members(GlobalModel::GefsMean), Some(31));
        assert_eq!(available_members(GlobalModel::GefsSpread), Some(31));
        assert_eq!(available_members(GlobalModel::GefsMember(0)), Some(1));
        assert!(GlobalModel::GefsMember(0).validate_forecast_hour(0).is_ok());
        assert!(
            GlobalModel::GefsMember(30)
                .validate_forecast_hour(384)
                .is_ok()
        );
        assert!(
            GlobalModel::GefsMember(31)
                .validate_forecast_hour(0)
                .is_err()
        );
        assert_eq!(available_members(GlobalModel::Gfs), None);
    }

    /// Both sources, live, at the newest usable cycle.
    /// `cargo test -p wxdata global_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn global_live() {
        let http = reqwest::Client::new();
        for model in [
            GlobalModel::Gfs,
            GlobalModel::GefsMean,
            GlobalModel::GefsSpread,
            GlobalModel::GefsMember(1),
            GlobalModel::Ecmwf,
        ] {
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

    /// `cargo test -p wxdata gefs_point_distribution_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network: downloads one field from all 31 GEFS members"]
    async fn gefs_point_distribution_live() {
        let distribution = fetch_gefs_point_distribution(
            &reqwest::Client::new(),
            GlobalField::Temp2m,
            0,
            -97.28,
            35.33,
        )
        .await
        .unwrap();
        println!("{distribution:?}");
        assert!(distribution.statistics.available >= 20);
        assert_eq!(distribution.statistics.expected, 31);
        assert!(distribution.statistics.minimum <= distribution.statistics.median);
        assert!(distribution.statistics.median <= distribution.statistics.maximum);
    }
}
