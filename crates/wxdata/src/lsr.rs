//! Live Local Storm Reports from the Iowa Environmental Mesonet (IEM).
//!
//! The `lsr.geojson` service returns NWS Local Storm Reports either for the trailing few hours
//! (live view) or for an explicit UTC window (archive scrub — a "time machine" for storm
//! reports, pairing with [`crate::archive_warnings`]). Reports decode into the same
//! [`StormReport`] the SPC daily log used, so the map/digest plumbing is shared.

use crate::alerts::USER_AGENT;
use crate::spc::{ReportKind, StormReport};

const LSR_URL: &str = "https://mesonet.agron.iastate.edu/geojson/lsr.geojson";

/// Map an IEM LSR `type` code to a [`ReportKind`].
/// T = tornado, G/D = thunderstorm gust / wind damage, H = hail, F/E = flash flood / flood.
fn kind_of(code: &str) -> ReportKind {
    match code {
        "T" => ReportKind::Tornado,
        "G" | "D" => ReportKind::Wind,
        "H" => ReportKind::Hail,
        "F" | "E" => ReportKind::Flood,
        _ => ReportKind::Other,
    }
}

/// Parse an IEM `lsr.geojson` payload. Tolerant: rows missing coordinates are skipped.
pub fn parse(json: &str) -> Vec<StormReport> {
    parse_collection(json).unwrap_or_default()
}

fn parse_collection(json: &str) -> anyhow::Result<Vec<StormReport>> {
    let v = serde_json::from_str::<serde_json::Value>(json)?;
    let feats = v.get("features").and_then(|f| f.as_array())
        .ok_or_else(|| anyhow::anyhow!("storm report response is missing its features array"))?;
    let mut out = Vec::new();
    for f in feats {
        let Some(p) = f.get("properties") else {
            continue;
        };
        let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (Some(lat), Some(lon)) = (
            p.get("lat").and_then(|v| v.as_f64()),
            p.get("lon").and_then(|v| v.as_f64()),
        ) else {
            continue;
        };
        // Magnitude: numeric `magf` plus its unit ("1.75 INCH", "64 MPH"); empty when unmeasured.
        let magnitude = match p.get("magf").and_then(|v| v.as_f64()) {
            Some(m) if m > 0.0 => {
                let n = if m.fract() == 0.0 {
                    format!("{m:.0}")
                } else {
                    format!("{m:.2}")
                };
                format!("{n} {}", s("unit")).trim().to_string()
            }
            _ => String::new(),
        };
        let valid_time = p.get("valid").and_then(|value| value.as_str())
            .and_then(|time| time.parse::<chrono::DateTime<chrono::Utc>>().ok());
        out.push(StormReport {
            kind: kind_of(&s("type")),
            lat,
            lon,
            valid_time,
            time: valid_time.map(|time| time.format("%H%M").to_string()).unwrap_or_default(),
            magnitude,
            location: s("city"),
            county: s("county"),
            state: s("state"),
            comments: s("remark"),
        });
    }
    Ok(out)
}

/// Fetch LSRs: `window = None` pulls the trailing 6 h (live); `Some((sts, ets))` pulls an
/// explicit UTC window (RFC3339-ish `YYYY-MM-DDTHH:MMZ`) for archive scrubbing.
pub async fn fetch(
    client: &reqwest::Client,
    window: Option<(&str, &str)>,
) -> anyhow::Result<Vec<StormReport>> {
    let mut req = client
        .get(crate::net::fetch_url(LSR_URL))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT);
    req = match window {
        Some((sts, ets)) => req.query(&[("sts", sts), ("ets", ets)]),
        None => req.query(&[("hours", "6")]),
    };
    let body = req.send().await?.error_for_status()?.text().await?;
    parse_collection(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_lsr_shapes() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"wfo":"BIS","type":"G","magf":64.0,"county":"LaMoure",
             "typetext":"TSTM WND GST","state":"ND","remark":"10 m gust.","city":"4 SW Edgeley",
             "source":"Mesonet","unit":"MPH","valid":"2026-07-20T02:48:00Z","lon":-98.77,"lat":46.32},
             "geometry":{"type":"Point","coordinates":[-98.77,46.32]}},
            {"type":"Feature","properties":{"wfo":"OUN","type":"T","magf":null,"county":"Cleveland",
             "typetext":"TORNADO","state":"OK","remark":"","city":"Moore","source":"Trained Spotter",
             "unit":"","valid":"2013-05-20T20:01:00Z","lon":-97.5,"lat":35.33},
             "geometry":{"type":"Point","coordinates":[-97.5,35.33]}},
            {"type":"Feature","properties":{"wfo":"XXX","type":"Z","city":"nowhere","state":"??",
             "county":"","unit":"","valid":"","remark":""}}]}"#;
        let r = parse(json);
        assert_eq!(r.len(), 2, "row without lat/lon skipped");
        assert_eq!(r[0].kind, ReportKind::Wind);
        assert_eq!(r[0].magnitude, "64 MPH");
        assert_eq!(r[0].time, "0248");
        assert_eq!(r[0].valid_time, Some("2026-07-20T02:48:00Z".parse().unwrap()));
        assert!(parse_collection(r#"{"features":[]}"#).unwrap().is_empty());
        assert!(parse_collection(r#"{"error":"temporarily unavailable"}"#).is_err());
        assert_eq!(r[1].kind, ReportKind::Tornado);
        assert_eq!(r[1].magnitude, "", "null magf → unmeasured");
        assert_eq!(r[1].location, "Moore");
    }

    #[test]
    fn kind_codes() {
        assert_eq!(kind_of("T"), ReportKind::Tornado);
        assert_eq!(kind_of("D"), ReportKind::Wind);
        assert_eq!(kind_of("H"), ReportKind::Hail);
        assert_eq!(kind_of("F"), ReportKind::Flood);
        assert_eq!(kind_of("E"), ReportKind::Flood);
        assert_eq!(kind_of("M"), ReportKind::Other);
    }
}
