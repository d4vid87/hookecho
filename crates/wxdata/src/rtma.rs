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
    WindV10m,
}

macro_rules! descriptor {
    ($name:ident, $source:literal, $id:literal, $label:literal, $short:literal, $units:literal, $alias:literal) => {
        pub static $name: FieldDescriptor = FieldDescriptor {
            id: FieldId($id),
            source: $source,
            family: FieldFamily::Analysis,
            display_name: $label,
            short_name: $short,
            search_aliases: &[$alias, "surface analysis"],
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
    "NOAA RTMA",
    "analysis.rtma.temperature-2m",
    "RTMA 2 m temperature",
    "RTMA temp",
    "K",
    "RTMA"
);
descriptor!(
    DEWPOINT_DESCRIPTOR,
    "NOAA RTMA",
    "analysis.rtma.dewpoint-2m",
    "RTMA 2 m dewpoint",
    "RTMA dewpoint",
    "K",
    "RTMA"
);
descriptor!(
    PRESSURE_DESCRIPTOR,
    "NOAA RTMA",
    "analysis.rtma.pressure",
    "RTMA surface pressure",
    "RTMA pressure",
    "Pa",
    "RTMA"
);
descriptor!(
    WIND_U_DESCRIPTOR,
    "NOAA RTMA",
    "analysis.rtma.wind-u-10m",
    "RTMA 10 m U wind",
    "RTMA U wind",
    "m s-1",
    "RTMA"
);

descriptor!(WIND_V_DESCRIPTOR, "NOAA RTMA", "analysis.rtma.wind-v-10m", "RTMA 10 m V wind", "RTMA V wind", "m s-1", "RTMA");
descriptor!(URMA_TEMP_DESCRIPTOR, "NOAA URMA", "analysis.urma.temperature-2m", "URMA 2 m temperature", "URMA temp", "K", "URMA");
descriptor!(URMA_DEWPOINT_DESCRIPTOR, "NOAA URMA", "analysis.urma.dewpoint-2m", "URMA 2 m dewpoint", "URMA dewpoint", "K", "URMA");
descriptor!(URMA_PRESSURE_DESCRIPTOR, "NOAA URMA", "analysis.urma.pressure", "URMA surface pressure", "URMA pressure", "Pa", "URMA");
descriptor!(URMA_WIND_U_DESCRIPTOR, "NOAA URMA", "analysis.urma.wind-u-10m", "URMA 10 m U wind", "URMA U wind", "m s-1", "URMA");
descriptor!(URMA_WIND_V_DESCRIPTOR, "NOAA URMA", "analysis.urma.wind-v-10m", "URMA 10 m V wind", "URMA V wind", "m s-1", "URMA");

impl SurfaceField {
    fn index(self) -> (&'static str, &'static str) {
        match self {
            Self::Temperature2m => ("TMP", "2 m above ground"),
            Self::Dewpoint2m => ("DPT", "2 m above ground"),
            Self::Pressure => ("PRES", "surface"),
            Self::WindU10m => ("UGRD", "10 m above ground"),
            Self::WindV10m => ("VGRD", "10 m above ground"),
        }
    }

    pub fn descriptor(self) -> &'static FieldDescriptor {
        match self {
            Self::Temperature2m => &TEMP_DESCRIPTOR,
            Self::Dewpoint2m => &DEWPOINT_DESCRIPTOR,
            Self::Pressure => &PRESSURE_DESCRIPTOR,
            Self::WindU10m => &WIND_U_DESCRIPTOR,
            Self::WindV10m => &WIND_V_DESCRIPTOR,
        }
    }

    pub fn descriptor_for(self, source: Source) -> &'static FieldDescriptor {
        if source == Source::Rtma {
            return self.descriptor();
        }
        match self {
            Self::Temperature2m => &URMA_TEMP_DESCRIPTOR,
            Self::Dewpoint2m => &URMA_DEWPOINT_DESCRIPTOR,
            Self::Pressure => &URMA_PRESSURE_DESCRIPTOR,
            Self::WindU10m => &URMA_WIND_U_DESCRIPTOR,
            Self::WindV10m => &URMA_WIND_V_DESCRIPTOR,
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

fn grib_message(bytes: &[u8]) -> Option<(u64, u8, u8, u8)> {
    if bytes.get(..4)? != b"GRIB" || bytes.get(7) != Some(&2) {
        return None;
    }
    let length = u64::from_be_bytes(bytes.get(8..16)?.try_into().ok()?);
    let mut offset = 16usize;
    while offset + 5 <= bytes.len() {
        let section_length = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        if section_length < 5 || offset + section_length > bytes.len() {
            return None;
        }
        if bytes[offset + 4] == 4 {
            let section = &bytes[offset..offset + section_length];
            return Some((length, *section.get(9)?, *section.get(10)?, *section.get(22)?));
        }
        offset += section_length;
    }
    None
}

async fn urma_range(
    http: &reqwest::Client,
    url: &str,
    field: SurfaceField,
) -> anyhow::Result<(u64, Option<u64>)> {
    let target = match field {
        SurfaceField::Temperature2m => (0, 0, 103),
        SurfaceField::Dewpoint2m => (0, 6, 103),
        SurfaceField::Pressure => (3, 0, 1),
        SurfaceField::WindU10m => (2, 2, 103),
        SurfaceField::WindV10m => (2, 3, 103),
    };
    let mut offset = 0u64;
    for _ in 0..32 {
        let header = crate::object_cache::fetch_range(http, url, offset, Some(offset + 512)).await?;
        let (length, category, parameter, surface) = grib_message(&header.bytes)
            .ok_or_else(|| anyhow::anyhow!("URMA object has an invalid GRIB message at {offset}"))?;
        anyhow::ensure!(length >= 512, "URMA GRIB message is too short");
        if (category, parameter, surface) == target {
            return Ok((offset, Some(offset + length)));
        }
        offset = offset
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("URMA GRIB offsets overflowed"))?;
    }
    anyhow::bail!("URMA object has no requested surface field")
}

/// Fetch one RTMA or URMA message by byte range and wrap native provenance.
pub async fn fetch_analysis(
    http: &reqwest::Client,
    source: Source,
    field: SurfaceField,
    valid: DateTime<Utc>,
) -> anyhow::Result<FieldFrame> {
    let (cached, source_identity) = fetch_message(http, source, field, valid).await?;
    let grid = crate::task::guarded(|| {
        crate::hrrr::decode_regrid_at_resolution(&cached.bytes, 0.03, f64::NEG_INFINITY)
    })
    .unwrap_or_else(|_| anyhow::bail!("RTMA GRIB decode panicked"))?;
    let valid_time = grid.time;
    Ok(FieldFrame::new(
        field.descriptor_for(source),
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

async fn fetch_message(
    http: &reqwest::Client,
    source: Source,
    field: SurfaceField,
    valid: DateTime<Utc>,
) -> anyhow::Result<(crate::object_cache::CachedObject, String)> {
    let url = object_url(source, valid);
    let (start, end) = if source == Source::Urma {
        urma_range(http, &url, field).await?
    } else {
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
        crate::hrrr::field_byte_range(&index, variable, level)
            .ok_or_else(|| anyhow::anyhow!("RTMA index has no {variable}:{level}"))?
    };
    let cached = crate::object_cache::fetch_range(http, &url, start, end).await?;
    let source_identity = end.map_or_else(
        || format!("{url}#bytes={start}-"),
        |end| format!("{url}#bytes={start}-{}", end - 1),
    );
    Ok((cached, source_identity))
}

/// Two exact native-grid samples from one immutable message, for a point and its observation.
pub struct NativeSamplePair {
    pub source_identity: String,
    pub valid_time: DateTime<Utc>,
    pub received_time: DateTime<Utc>,
    pub point: f32,
    pub station: f32,
}

pub async fn fetch_native_pair(
    http: &reqwest::Client,
    source: Source,
    field: SurfaceField,
    valid: DateTime<Utc>,
    point: (f64, f64),
    station: (f64, f64),
) -> anyhow::Result<NativeSamplePair> {
    let (cached, source_identity) = fetch_message(http, source, field, valid).await?;
    let received_time = cached.received_at;
    let (actual, values) = crate::task::blocking(move || {
        crate::task::guarded(|| crate::sounding::sample_grib_points(&cached.bytes, &[point, station]))
            .unwrap_or_else(|_| anyhow::bail!("analysis native decode panicked"))
    }).await??;
    let actual = actual.ok_or_else(|| anyhow::anyhow!("analysis valid time unavailable"))?;
    anyhow::ensure!(values.iter().all(|(distance, _)| *distance <= 0.2_f64.powi(2)),
        "analysis point outside native grid");
    anyhow::ensure!(actual == valid, "analysis valid time differs from requested hour");
    Ok(NativeSamplePair {
        source_identity, valid_time: actual, received_time,
        point: values[0].1 as f32, station: values[1].1 as f32,
    })
}

pub async fn fetch_rtma(
    http: &reqwest::Client,
    field: SurfaceField,
    valid: DateTime<Utc>,
) -> anyhow::Result<FieldFrame> {
    fetch_analysis(http, Source::Rtma, field, valid).await
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

/// Fetch the newest delayed URMA analysis without downloading its full aggregate object.
pub async fn fetch_latest_urma(
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
    for age in 6..=36 {
        match fetch_analysis(http, Source::Urma, field, hour - chrono::Duration::hours(age)).await {
            Ok(frame) => return Ok(frame),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no recent URMA analysis")))
}

/// One native 2 m temperature sample at a station from an exact RTMA valid hour.
#[derive(Debug, Clone)]
pub struct PointTemperature {
    pub stamp: DataStamp,
    pub kelvin: f32,
}

/// Fetch up to six consecutive hourly analyses, ending at the newest published RTMA hour.
/// Missing hours remain gaps rather than being replaced with a different valid time.
pub async fn fetch_point_temperature_history(
    http: &reqwest::Client,
    lon: f64,
    lat: f64,
) -> anyhow::Result<Vec<PointTemperature>> {
    let latest = fetch_latest_rtma(http, SurfaceField::Temperature2m).await?;
    let newest = latest.stamp.valid_time;
    let sample = |frame: FieldFrame| {
        Some(PointTemperature { kelvin: frame.sample(lon, lat).value?, stamp: frame.stamp })
    };
    let mut points: Vec<_> = sample(latest).into_iter().collect();
    for hour in (1..=5).rev() {
        let requested = newest - chrono::Duration::hours(hour);
        if let Ok(frame) = fetch_rtma(http, SurfaceField::Temperature2m, requested).await {
            if frame.stamp.valid_time == requested {
                points.extend(sample(frame));
            }
        }
    }
    points.sort_by_key(|point| point.stamp.valid_time);
    anyhow::ensure!(!points.is_empty(), "no RTMA temperature data at station");
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "network"]
    async fn live_station_temperature_history_keeps_exact_hours() {
        let points = fetch_point_temperature_history(&reqwest::Client::new(), -97.3, 32.6)
            .await.expect("RTMA station temperature");
        eprintln!("RTMA station temperature: {} hours", points.len());
        assert!(points.len() >= 2);
        assert!(points.iter().all(|point| point.stamp.class == DataClass::Analysis
            && (240.0..330.0).contains(&point.kelvin)));
        assert!(points.windows(2).all(|pair| pair[0].stamp.valid_time < pair[1].stamp.valid_time));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_native_pair_reads_exact_analysis_hour() {
        let http = reqwest::Client::new();
        let latest = fetch_latest_rtma(&http, SurfaceField::Temperature2m).await.unwrap();
        let at = latest.stamp.valid_time;
        let point = fetch_native_pair(&http, Source::Rtma, SurfaceField::Temperature2m,
            at, (-97.3, 32.6), (-97.04, 32.9)).await.unwrap();
        assert_eq!(point.valid_time, at);
        assert_eq!(point.source_identity, latest.stamp.source_identity);
        assert!((240.0..330.0).contains(&point.point));
        assert!((240.0..330.0).contains(&point.station));
        for field in [SurfaceField::Dewpoint2m, SurfaceField::Pressure,
            SurfaceField::WindU10m, SurfaceField::WindV10m] {
            let other = fetch_native_pair(&http, Source::Rtma, field,
                at, (-97.3, 32.6), (-97.04, 32.9)).await.unwrap();
            assert_eq!(other.valid_time, at);
            assert_eq!(other.source_identity.split_once("#bytes=").unwrap().0,
                point.source_identity.split_once("#bytes=").unwrap().0);
            assert!(other.point.is_finite() && other.station.is_finite());
        }
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_v_wind_has_signed_native_values() {
        let frame = fetch_latest_rtma(&reqwest::Client::new(), SurfaceField::WindV10m)
            .await.expect("latest RTMA 10 m V wind");
        assert_eq!(frame.descriptor.id.0, "analysis.rtma.wind-v-10m");
        assert!(frame.field().values.iter().any(|value| value.is_finite()));
        let temperature = fetch_rtma(&reqwest::Client::new(), SurfaceField::Temperature2m,
            frame.stamp.valid_time).await.expect("same-hour RTMA temperature");
        let u = fetch_rtma(&reqwest::Client::new(), SurfaceField::WindU10m,
            frame.stamp.valid_time).await.expect("same-hour RTMA U wind");
        fn object(identity: &str) -> &str { identity.split_once("#bytes=").unwrap().0 }
        assert_eq!(object(&frame.stamp.source_identity), object(&temperature.stamp.source_identity));
        assert_eq!(object(&frame.stamp.source_identity), object(&u.stamp.source_identity));
        assert!(!crate::contour::contour_lines(temperature.field(), 2.0).is_empty(),
            "the decoded native temperature grid should yield analysis isolines");
        eprintln!("RTMA matched wind and temperature: {}", frame.stamp.source_identity);
    }

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
        assert_eq!(
            SurfaceField::Temperature2m.descriptor_for(Source::Urma).source_id().0,
            "noaa.urma"
        );
        assert_eq!(SurfaceField::WindV10m.index(), ("VGRD", "10 m above ground"));
        assert_ne!(
            SurfaceField::Temperature2m.descriptor().id,
            SurfaceField::Temperature2m.descriptor_for(Source::Urma).id
        );
    }

    #[test]
    fn urma_grib_headers_locate_parameter_and_level() {
        let mut bytes = vec![0u8; 56];
        bytes[..4].copy_from_slice(b"GRIB");
        bytes[7] = 2;
        bytes[8..16].copy_from_slice(&6_085_757u64.to_be_bytes());
        bytes[16..20].copy_from_slice(&5u32.to_be_bytes());
        bytes[20] = 1;
        bytes[21..25].copy_from_slice(&35u32.to_be_bytes());
        bytes[25] = 4;
        bytes[30] = 0;
        bytes[31] = 6;
        bytes[43] = 103;
        assert_eq!(grib_message(&bytes), Some((6_085_757, 0, 6, 103)));
        bytes[0] = 0;
        assert_eq!(grib_message(&bytes), None);
    }

    /// `cargo test -p wxdata urma_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn urma_live() {
        let frame = fetch_analysis(
            &reqwest::Client::new(),
            Source::Urma,
            SurfaceField::Temperature2m,
            "2026-09-20T20:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(frame.descriptor.id.0, "analysis.urma.temperature-2m");
        assert!(frame.field().values.iter().any(|value| value.is_finite()));
        let v_wind = fetch_analysis(
            &reqwest::Client::new(), Source::Urma, SurfaceField::WindV10m,
            "2026-09-20T20:00:00Z".parse().unwrap(),
        ).await.unwrap();
        assert_eq!(v_wind.descriptor.id.0, "analysis.urma.wind-v-10m");
        assert!(v_wind.field().values.iter().any(|value| value.is_finite()));
        println!("{}", frame.stamp.source_identity);
    }
}
