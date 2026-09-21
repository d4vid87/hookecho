//! NOAA REFS combined ensemble products from the RRFS operational bucket.

use crate::alerts::USER_AGENT;
use crate::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    QualitySummary, SamplingPolicy, ValueKind,
};
use chrono::{DateTime, Datelike, Timelike, Utc};

const BUCKET: &str = "https://noaa-rrfs-ops-pds.s3.amazonaws.com";

macro_rules! descriptor {
    ($name:ident, $id:literal, $display:literal, $short:literal) => {
        pub static $name: FieldDescriptor = FieldDescriptor {
            id: FieldId($id),
            source: "NOAA REFS v1 parallel",
            family: FieldFamily::Model,
            display_name: $display,
            short_name: $short,
            search_aliases: &["ensemble", "probability", "storms", "reflectivity"],
            units: "%",
            value_kind: ValueKind::Probability,
            palette_key: "thunder-probability",
            sampling: SamplingPolicy::Bilinear,
            missing: MissingData::Nan,
            time_policy: None,
            supports_contours: true,
            supports_difference: false,
        };
    };
}

descriptor!(
    REFLECTIVITY_10_DESCRIPTOR,
    "model.refs.reflectivity-probability-10dbz",
    "REFS probability of composite reflectivity above 10 dBZ",
    "REFS >10 dBZ"
);
descriptor!(
    REFLECTIVITY_20_DESCRIPTOR,
    "model.refs.reflectivity-probability-20dbz",
    "REFS probability of composite reflectivity above 20 dBZ",
    "REFS >20 dBZ"
);
descriptor!(
    REFLECTIVITY_30_DESCRIPTOR,
    "model.refs.reflectivity-probability-30dbz",
    "REFS probability of composite reflectivity above 30 dBZ",
    "REFS >30 dBZ"
);
descriptor!(
    REFLECTIVITY_40_DESCRIPTOR,
    "model.refs.reflectivity-probability-40dbz",
    "REFS probability of composite reflectivity above 40 dBZ",
    "REFS >40 dBZ"
);
descriptor!(
    REFLECTIVITY_50_DESCRIPTOR,
    "model.refs.reflectivity-probability-50dbz",
    "REFS probability of composite reflectivity above 50 dBZ",
    "REFS >50 dBZ"
);

pub const REFLECTIVITY_THRESHOLDS: [u8; 5] = [10, 20, 30, 40, 50];

pub fn reflectivity_descriptor(threshold: u8) -> Option<&'static FieldDescriptor> {
    Some(match threshold {
        10 => &REFLECTIVITY_10_DESCRIPTOR,
        20 => &REFLECTIVITY_20_DESCRIPTOR,
        30 => &REFLECTIVITY_30_DESCRIPTOR,
        40 => &REFLECTIVITY_40_DESCRIPTOR,
        50 => &REFLECTIVITY_50_DESCRIPTOR,
        _ => return None,
    })
}

/// Fetch the latest REFS neighborhood probability for a published reflectivity threshold.
pub async fn fetch_reflectivity(
    http: &reqwest::Client,
    forecast_hour: u8,
    threshold: u8,
) -> anyhow::Result<FieldFrame> {
    anyhow::ensure!(
        (1..=60).contains(&forecast_hour),
        "REFS forecast hour must be F01..F60"
    );
    let now = Utc::now();
    let mut last_err = None;
    for run in crate::hrrr::recent_cycles(crate::hrrr::Model::Rrfs, now)
        .into_iter()
        .filter(|run| run.hour().is_multiple_of(6))
    {
        match fetch_run(http, run, forecast_hour, threshold).await {
            Ok(frame) => return Ok(frame),
            Err(error) => last_err = Some(error),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no REFS cycle found")))
}

async fn fetch_run(
    http: &reqwest::Client,
    run: DateTime<Utc>,
    forecast_hour: u8,
    threshold: u8,
) -> anyhow::Result<FieldFrame> {
    let descriptor = reflectivity_descriptor(threshold).ok_or_else(|| {
        anyhow::anyhow!("unsupported REFS reflectivity threshold {threshold} dBZ")
    })?;
    let date = format!("{:04}{:02}{:02}", run.year(), run.month(), run.day());
    let base = format!(
        "{BUCKET}/refs.{date}/{:02}/ensprod/refs.t{:02}z.prob.f{forecast_hour:02}.conus.grib2",
        run.hour(),
        run.hour()
    );
    let index = http
        .get(crate::net::fetch_url(&format!("{base}.idx")))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let key = format!("prob >{threshold}");
    let (start, end, members) = probability_range(&index, "REFC", &key).ok_or_else(|| {
        anyhow::anyhow!("REFS index lacks REFC probability above {threshold} dBZ")
    })?;
    let cached = crate::object_cache::fetch_range(http, &base, start, end).await?;
    let received_time = cached.received_at;
    let field = crate::task::guarded(|| {
        crate::hrrr::decode_regrid(&cached.bytes, crate::hrrr::Model::Rrfs, 0.0)
    })
    .unwrap_or_else(|_| anyhow::bail!("REFS GRIB decode panicked"))?;
    let valid_time = run + chrono::Duration::hours(forecast_hour.into());
    Ok(FieldFrame::new(
        descriptor,
        field,
        DataStamp {
            source_identity: base,
            issue_time: Some(run),
            run_time: Some(run),
            valid_time,
            received_time,
            class: DataClass::Forecast,
            quality: QualitySummary::Unknown,
            available_members: Some(members),
        },
    ))
}

fn probability_range(
    index: &str,
    variable: &str,
    threshold: &str,
) -> Option<(u64, Option<u64>, u16)> {
    let lines: Vec<&str> = index.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.contains(&format!(":{variable}:")) && line.contains(threshold))?;
    let start = lines[at].split(':').nth(1)?.parse().ok()?;
    let end = lines[at + 1..]
        .iter()
        .filter_map(|line| line.split(':').nth(1)?.parse::<u64>().ok())
        .find(|offset| *offset > start);
    let members = lines[at]
        .split("prob fcst ")
        .nth(1)?
        .split(':')
        .next()?
        .split('/')
        .nth(1)?
        .parse()
        .ok()?;
    Some((start, end, members))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_range_selects_threshold_and_member_count() {
        let index = "1:0:d=x:REFC:entire atmosphere:1 hour fcst:prob >30:prob fcst 0/14:\n\
                     2:100:d=x:REFC:entire atmosphere:1 hour fcst:prob >40:prob fcst 0/13:\n\
                     3:250:d=x:REFC:entire atmosphere:1 hour fcst:prob >50:prob fcst 0/14:\n";
        assert_eq!(
            probability_range(index, "REFC", "prob >40"),
            Some((100, Some(250), 13))
        );
        assert_eq!(REFLECTIVITY_THRESHOLDS.len(), 5);
        for threshold in REFLECTIVITY_THRESHOLDS {
            let descriptor = reflectivity_descriptor(threshold).unwrap();
            assert_eq!(descriptor.value_kind, ValueKind::Probability);
            assert!(descriptor.id.0.contains(&threshold.to_string()));
        }
        assert!(reflectivity_descriptor(35).is_none());
    }

    /// `cargo test -p wxdata refs_probability_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn refs_probability_live() {
        let http = reqwest::Client::new();
        for threshold in REFLECTIVITY_THRESHOLDS {
            let frame = fetch_reflectivity(&http, 1, threshold)
                .await
                .expect("REFS probability");
            assert!(frame.stamp.available_members.unwrap_or_default() > 0);
            let finite = frame
                .field()
                .values
                .iter()
                .filter(|value| value.is_finite())
                .count();
            eprintln!(
                "REFS >{threshold} dBZ, {} members, {finite} finite cells",
                frame.stamp.available_members.unwrap()
            );
            assert!(finite > 1000);
        }
    }
}
