//! NOAA REFS combined ensemble products from the RRFS operational bucket.

use crate::alerts::USER_AGENT;
use crate::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    QualitySummary, SamplingPolicy, ValueKind,
};
use chrono::{DateTime, Datelike, Timelike, Utc};

const BUCKET: &str = "https://noaa-rrfs-ops-pds.s3.amazonaws.com";

pub static REFLECTIVITY_40_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("model.refs.reflectivity-probability-40dbz"),
    source: "NOAA REFS v1 parallel",
    family: FieldFamily::Model,
    display_name: "REFS probability of composite reflectivity above 40 dBZ",
    short_name: "REFS >40 dBZ",
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

/// Fetch the latest REFS neighborhood probability of composite reflectivity above 40 dBZ.
pub async fn fetch_reflectivity_40(
    http: &reqwest::Client,
    forecast_hour: u8,
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
        match fetch_run(http, run, forecast_hour).await {
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
) -> anyhow::Result<FieldFrame> {
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
    let (start, end, members) = probability_range(&index, "REFC", "prob >40")
        .ok_or_else(|| anyhow::anyhow!("REFS index lacks REFC probability above 40 dBZ"))?;
    let range = end.map_or_else(
        || format!("bytes={start}-"),
        |end| format!("bytes={start}-{}", end - 1),
    );
    let bytes = http
        .get(crate::net::fetch_url(&base))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .header("Range", range)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let received_time = Utc::now();
    let field = crate::task::guarded(|| {
        crate::hrrr::decode_regrid(&bytes, crate::hrrr::Model::Rrfs, 0.0)
    })
    .unwrap_or_else(|_| anyhow::bail!("REFS GRIB decode panicked"))?;
    let valid_time = run + chrono::Duration::hours(forecast_hour.into());
    Ok(FieldFrame::new(
        &REFLECTIVITY_40_DESCRIPTOR,
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

fn probability_range(index: &str, variable: &str, threshold: &str) -> Option<(u64, Option<u64>, u16)> {
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
    }

    /// `cargo test -p wxdata refs_probability_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network"]
    async fn refs_probability_live() {
        let frame = fetch_reflectivity_40(&reqwest::Client::new(), 1)
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
            "REFS {} members, {finite} finite cells",
            frame.stamp.available_members.unwrap()
        );
        assert!(finite > 1000);
    }
}
