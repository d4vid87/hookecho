//! NOAA CONUS RTMA/URMA surface-analysis source contract and RTMA range fetches.

use crate::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    QualitySummary, SamplingPolicy, ValueKind,
};
use chrono::{DateTime, Timelike, Utc};

const RTMA_ROOT: &str = "https://nomads.ncep.noaa.gov/pub/data/nccf/com/rtma/v2.10";
const URMA_ROOT: &str = "https://nomads.ncep.noaa.gov/pub/data/nccf/com/urma/prod";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Rtma,
    Urma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceField {
    Temperature2m,
    Dewpoint2m,
    Pressure,
    WindU10m,
}

macro_rules! descriptor {
    ($name:ident, $id:literal, $label:literal, $short:literal, $units:literal) => {
        pub static $name: FieldDescriptor = FieldDescriptor {
            id: FieldId($id),
            source: "NOAA RTMA",
            family: FieldFamily::Analysis,
            display_name: $label,
            short_name: $short,
            search_aliases: &["RTMA", "surface analysis"],
            units: $units,
            value_kind: ValueKind::Scalar,
            palette_key: $id,
            sampling: SamplingPolicy::Bilinear,
            missing: MissingData::Nan,
            time_policy: None,
            supports_contours: true,
            supports_difference: true,
        };
    };
}

descriptor!(
    TEMP_DESCRIPTOR,
    "analysis.rtma.temperature-2m",
    "RTMA 2 m temperature",
    "RTMA temp",
    "K"
);
descriptor!(
    DEWPOINT_DESCRIPTOR,
    "analysis.rtma.dewpoint-2m",
    "RTMA 2 m dewpoint",
    "RTMA dewpoint",
    "K"
);
descriptor!(
    PRESSURE_DESCRIPTOR,
    "analysis.rtma.pressure",
    "RTMA surface pressure",
    "RTMA pressure",
    "Pa"
);
descriptor!(
    WIND_U_DESCRIPTOR,
    "analysis.rtma.wind-u-10m",
    "RTMA 10 m U wind",
    "RTMA U wind",
    "m s-1"
);

impl SurfaceField {
    fn index(self) -> (&'static str, &'static str) {
        match self {
            Self::Temperature2m => ("TMP", "2 m above ground"),
            Self::Dewpoint2m => ("DPT", "2 m above ground"),
            Self::Pressure => ("PRES", "surface"),
            Self::WindU10m => ("UGRD", "10 m above ground"),
        }
    }

    pub fn descriptor(self) -> &'static FieldDescriptor {
        match self {
            Self::Temperature2m => &TEMP_DESCRIPTOR,
            Self::Dewpoint2m => &DEWPOINT_DESCRIPTOR,
            Self::Pressure => &PRESSURE_DESCRIPTOR,
            Self::WindU10m => &WIND_U_DESCRIPTOR,
        }
    }
}

/// Exact immutable analysis object for a UTC valid hour.
pub fn object_url(source: Source, valid: DateTime<Utc>) -> String {
    let day = valid.format("%Y%m%d");
    let (root, prefix) = match source {
        Source::Rtma => (RTMA_ROOT, "rtma2p5"),
        Source::Urma => (URMA_ROOT, "urma2p5"),
    };
    format!(
        "{root}/{prefix}.{day}/{prefix}.t{:02}z.2dvaranl_ndfd.grb2_wexp",
        valid.hour()
    )
}

/// Fetch one RTMA message by its official text-index byte range and wrap native provenance.
pub async fn fetch_rtma(
    http: &reqwest::Client,
    field: SurfaceField,
    valid: DateTime<Utc>,
) -> anyhow::Result<FieldFrame> {
    let url = object_url(Source::Rtma, valid);
    let index = http
        .get(crate::net::fetch_url(&format!("{url}.idx")))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let (variable, level) = field.index();
    let (start, end) = crate::hrrr::field_byte_range(&index, variable, level)
        .ok_or_else(|| anyhow::anyhow!("RTMA index has no {variable}:{level}"))?;
    let cached = crate::object_cache::fetch_range(http, &url, start, end).await?;
    let grid = crate::task::guarded(|| {
        crate::hrrr::decode_regrid_at_resolution(&cached.bytes, 0.03, f64::NEG_INFINITY)
    })
    .unwrap_or_else(|_| anyhow::bail!("RTMA GRIB decode panicked"))?;
    let valid_time = grid.time;
    let source_identity = end.map_or_else(
        || format!("{url}#bytes={start}-"),
        |end| format!("{url}#bytes={start}-{}", end - 1),
    );
    Ok(FieldFrame::new(
        field.descriptor(),
        grid,
        DataStamp {
            source_identity,
            issue_time: Some(valid_time),
            run_time: None,
            valid_time,
            received_time: cached.received_at,
            class: DataClass::Analysis,
            quality: QualitySummary::Unknown,
            available_members: None,
        },
    ))
}

/// Fetch the newest available hourly RTMA analysis, allowing for normal publication latency.
pub async fn fetch_latest_rtma(
    http: &reqwest::Client,
    field: SurfaceField,
) -> anyhow::Result<FieldFrame> {
    let now = Utc::now();
    let hour = now
        .with_minute(0)
        .and_then(|time| time.with_second(0))
        .and_then(|time| time.with_nanosecond(0))
        .unwrap_or(now);
    let mut last_error = None;
    for age in 1..=4 {
        match fetch_rtma(http, field, hour - chrono::Duration::hours(age)).await {
            Ok(frame) => return Ok(frame),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no recent RTMA analysis")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_paths_and_index_names_are_stable() {
        let at = "2026-09-20T20:00:00Z".parse().unwrap();
        assert_eq!(
            object_url(Source::Rtma, at),
            "https://nomads.ncep.noaa.gov/pub/data/nccf/com/rtma/v2.10/rtma2p5.20260920/rtma2p5.t20z.2dvaranl_ndfd.grb2_wexp"
        );
        assert_eq!(
            object_url(Source::Urma, at),
            "https://nomads.ncep.noaa.gov/pub/data/nccf/com/urma/prod/urma2p5.20260920/urma2p5.t20z.2dvaranl_ndfd.grb2_wexp"
        );
        let index = "1:0:d=2026092020:HGT:surface:anl:\n2:7490118:d=2026092020:PRES:surface:anl:\n3:14980236:d=2026092020:TMP:2 m above ground:anl:\n4:21065993:d=2026092020:DPT:2 m above ground:anl:";
        assert_eq!(
            crate::hrrr::field_byte_range(index, "TMP", "2 m above ground"),
            Some((14_980_236, Some(21_065_993)))
        );
    }
}
