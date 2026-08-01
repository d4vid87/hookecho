//! mPING — crowd-sourced precipitation-type reports.
//!
//! Radar can tell you something is falling; only someone outside can tell you whether it is
//! landing as rain, snow, sleet or freezing rain. mPING (OU/NSSL) collects exactly that from a
//! phone app, and in a winter storm the rain/snow line it draws is the whole forecast.
//!
//! The API needs a free key, requested at <https://mping.ou.edu>. Like the other keyed sources,
//! it is entered in Settings, stored in the user's own settings file, and the layer simply stays
//! empty when no key is set.

use chrono::{DateTime, Duration, Utc};

const API: &str = "https://mping.ou.edu/mping/api/v2/reports";

/// What a reporter saw falling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precip {
    Rain,
    Snow,
    /// Ice pellets / sleet.
    Sleet,
    FreezingRain,
    Hail,
    /// Mixed rain and snow, drizzle, and anything else the description doesn't pin down.
    Mixed,
    None,
}

impl Precip {
    /// One or two characters for the map dot, in the style the LSR markers already use.
    pub fn glyph(self) -> &'static str {
        match self {
            Precip::Rain => "R",
            Precip::Snow => "S",
            Precip::Sleet => "IP",
            Precip::FreezingRain => "ZR",
            Precip::Hail => "A",
            Precip::Mixed => "M",
            Precip::None => "\u{2014}",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            Precip::Rain => [60, 190, 90],
            Precip::Snow => [120, 190, 245],
            Precip::Sleet => [220, 130, 240],
            Precip::FreezingRain => [235, 70, 70],
            Precip::Hail => [250, 220, 80],
            Precip::Mixed => [200, 160, 120],
            Precip::None => [150, 150, 150],
        }
    }

    /// Classify a report description. mPING's wording is stable but wordy ("Snow and/or Graupel").
    fn from_description(d: &str) -> Precip {
        let d = d.to_ascii_lowercase();
        // Order matters: "mixed" and "freezing" both contain words that appear alone elsewhere.
        if d.contains("none") || d.contains("no precip") {
            Precip::None
        } else if d.contains("hail") {
            Precip::Hail
        } else if d.contains("freezing") {
            Precip::FreezingRain
        } else if d.contains("ice pellet") || d.contains("sleet") {
            Precip::Sleet
        } else if d.contains("mixed") || d.contains("and/or ice") {
            Precip::Mixed
        } else if d.contains("snow") || d.contains("graupel") {
            Precip::Snow
        } else if d.contains("rain") || d.contains("drizzle") {
            Precip::Rain
        } else {
            Precip::Mixed
        }
    }
}

/// One crowd report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub lat: f64,
    pub lon: f64,
    pub time: DateTime<Utc>,
    pub precip: Precip,
    /// The description as issued, for the marker popup.
    pub description: String,
}

/// Parse an mPING API response.
pub fn parse(json: &str) -> anyhow::Result<Vec<Report>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let results = v
        .get("results")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow::anyhow!("no results array"))?;
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        let coords = r
            .get("geom")
            .and_then(|g| g.get("coordinates"))
            .and_then(|c| c.as_array());
        let (Some(c), Some(desc)) = (coords, r.get("description").and_then(|d| d.as_str())) else {
            continue;
        };
        let (Some(lon), Some(lat)) = (
            c.first().and_then(|v| v.as_f64()),
            c.get(1).and_then(|v| v.as_f64()),
        ) else {
            continue;
        };
        let time = r
            .get("obtime")
            .and_then(|t| t.as_str())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map_or_else(Utc::now, |t| t.with_timezone(&Utc));
        out.push(Report {
            lat,
            lon,
            time,
            precip: Precip::from_description(desc),
            description: desc.to_string(),
        });
    }
    Ok(out)
}

/// Fetch the precipitation-type reports of the last `minutes`.
pub async fn fetch(http: &reqwest::Client, key: &str, minutes: i64) -> anyhow::Result<Vec<Report>> {
    anyhow::ensure!(!key.is_empty(), "no mPING key");
    let since = (Utc::now() - Duration::minutes(minutes)).format("%Y-%m-%d %H:%M:%S");
    let body = http
        .get(API)
        .query(&[
            ("category", "Rain/Snow"),
            ("obtime_gte", &since.to_string()),
        ])
        .header("Authorization", format!("Token {key}"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"count":3,"results":[
        {"id":1,"obtime":"2026-02-15T14:03:00Z","category":"Rain/Snow",
         "description":"Snow and/or Graupel","geom":{"type":"Point","coordinates":[-93.1,44.9]}},
        {"id":2,"obtime":"2026-02-15T14:05:00Z","category":"Rain/Snow",
         "description":"Freezing Rain","geom":{"type":"Point","coordinates":[-92.4,43.6]}},
        {"id":3,"obtime":"bogus","category":"Rain/Snow",
         "description":"Ice Pellets/Sleet","geom":{"type":"Point","coordinates":[-91.8,42.9]}}]}"#;

    #[test]
    fn reports_classify_and_locate() {
        let r = parse(SAMPLE).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].precip, Precip::Snow);
        assert!((r[0].lat - 44.9).abs() < 1e-9 && (r[0].lon - -93.1).abs() < 1e-9);
        assert_eq!(r[0].time.to_rfc3339(), "2026-02-15T14:03:00+00:00");
        assert_eq!(r[1].precip, Precip::FreezingRain);
        assert_eq!(r[2].precip, Precip::Sleet);
        // An unparsable timestamp costs the report its time, not its existence.
        assert_eq!(r[2].description, "Ice Pellets/Sleet");
    }

    #[test]
    fn descriptions_map_to_the_right_type() {
        for (d, want) in [
            ("Rain", Precip::Rain),
            ("Drizzle", Precip::Rain),
            ("Mixed Rain and Snow", Precip::Mixed),
            ("Freezing Drizzle", Precip::FreezingRain),
            ("Hail (dime size)", Precip::Hail),
            ("None (no precipitation)", Precip::None),
        ] {
            assert_eq!(Precip::from_description(d), want, "{d}");
        }
    }

    #[test]
    fn a_missing_geometry_is_skipped_not_fatal() {
        let json = r#"{"results":[{"description":"Rain"},
            {"description":"Rain","geom":{"coordinates":[-97.0,35.0]}}]}"#;
        assert_eq!(parse(json).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_empty_key_is_refused_before_any_request() {
        let http = reqwest::Client::new();
        assert!(fetch(&http, "", 60).await.is_err());
    }
}
