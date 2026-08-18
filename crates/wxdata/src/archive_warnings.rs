//! Archived storm-based warning polygons from the Iowa Environmental Mesonet (IEM).
//!
//! The `sbw.py` GeoJSON service returns every storm-based warning valid at a given instant, so
//! scrubbing the archive timeline can overlay the warnings that were actually in effect then —
//! a "time machine" for warning polygons. Decoded into the same [`GeoFeature`] type as live
//! alerts, styled via [`alerts::event_style`].

use crate::alerts;
use crate::overlay::{for_each_feature, polygons_of, AlertInfo, GeoFeature};

const SBW_URL: &str = "https://mesonet.agron.iastate.edu/geojson/sbw.py";

/// Map an IEM `phenomena` code to a human phenomenon name (as used in NWS event strings).
fn phenomenon(code: &str) -> &'static str {
    match code {
        "TO" => "Tornado",
        "SV" => "Severe Thunderstorm",
        "FF" => "Flash Flood",
        "MA" => "Marine",
        "EW" => "Extreme Wind",
        "SQ" => "Snow Squall",
        "DS" => "Dust Storm",
        _ => "Special Weather",
    }
}

/// Parse an IEM `sbw.py` GeoJSON payload into styled warning features.
pub fn parse(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let get = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let phenom = get("phenomena");
        // Storm-based warnings are always warnings; watches/statements aren't polygon-based here.
        let event = format!("{} Warning", phenomenon(phenom));
        let (kind, rgb) = alerts::event_style(&event);
        let expires = chrono::DateTime::parse_from_rfc3339(get("expire"))
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc));
        let issue = get("issue");
        // eventid arrives as a number or a string depending on the service version.
        let eventid = props
            .get("eventid")
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default();
        let id = format!("{}-{}-{}-{}", get("wfo"), phenom, eventid, issue);
        let detail = format!(
            "{}\n\nWFO: {}\nIssued: {}\nExpires: {}\n\nIEM archive",
            event,
            get("wfo"),
            issue,
            get("expire"),
        );
        let alert = AlertInfo {
            vtec: None,
            id,
            event: event.clone(),
            headline: event.clone(),
            area: get("wfo").to_string(),
            description: detail.clone(),
            instruction: String::new(),
            expires,
            max_hail_in: None,
            max_wind: None,
            tornado_detection: None,
            damage_threat: None,
            source: Some("IEM archive".into()),
            motion: None,
        };
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 45],
                stroke: [rgb[0], rgb[1], rgb[2], 235],
                kind,
                title: event.clone(),
                detail: detail.clone(),
                alert: Some(alert.clone()),
            });
        }
    })?;
    Ok(out)
}

/// Fetch the storm-based warnings valid at `ts` (an RFC3339 UTC instant).
pub async fn fetch(client: &reqwest::Client, ts: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let body = client
        .get(crate::net::fetch_url(SBW_URL))
        .query(&[("ts", ts)])
        .header("User-Agent", alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body)
}

const BYPOINT_URL: &str = "https://mesonet.agron.iastate.edu/json/vtec_events_bypoint.py";

/// One archived VTEC event that covered a point. The by-point service matches on the county/zone
/// (UGC) the point falls in, not on the storm-based polygon, so these are "county was warned"
/// counts — the same basis NWS uses when it talks about how often a place gets warned.
#[derive(Debug, Clone, PartialEq)]
pub struct PointEvent {
    pub issue: chrono::DateTime<chrono::Utc>,
    /// VTEC phenomenon code (`TO`, `SV`, `FF`, …).
    pub phenomena: String,
    /// VTEC significance (`W` warning, `A` watch, `Y` advisory).
    pub significance: String,
    /// Full product name, e.g. "Tornado Warning".
    pub name: String,
}

/// A fold of [`PointEvent`]s into the numbers the climatology card shows. Warnings only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointSummary {
    pub total: usize,
    /// Warning counts per product name, most frequent first.
    pub by_name: Vec<(String, usize)>,
    /// Earliest year with a warning on record (IEM's archive starts ~1986).
    pub first_year: Option<i32>,
    /// Calendar day (UTC) with the most warnings, and how many.
    pub worst_day: Option<(chrono::NaiveDate, usize)>,
    /// Year with the most warnings, and how many.
    pub busiest_year: Option<(i32, usize)>,
}

/// Parse a `vtec_events_bypoint.py` payload.
pub fn parse_point_events(json: &str) -> anyhow::Result<Vec<PointEvent>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let mut out = Vec::new();
    for e in v["events"].as_array().into_iter().flatten() {
        let get = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let Ok(issue) = chrono::DateTime::parse_from_rfc3339(get("issue")) else {
            continue;
        };
        out.push(PointEvent {
            issue: issue.with_timezone(&chrono::Utc),
            phenomena: get("phenomena").to_string(),
            significance: get("significance").to_string(),
            name: get("name").to_string(),
        });
    }
    Ok(out)
}

/// Fold events into per-name / per-year / worst-day counts, counting warnings (`significance == "W"`)
/// only — watches cover whole regions for hours and would swamp the picture.
pub fn summarize(events: &[PointEvent]) -> PointSummary {
    use std::collections::HashMap;
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    let mut by_year: HashMap<i32, usize> = HashMap::new();
    let mut by_day: HashMap<chrono::NaiveDate, usize> = HashMap::new();
    let mut total = 0;
    for e in events.iter().filter(|e| e.significance == "W") {
        total += 1;
        *by_name.entry(e.name.as_str()).or_default() += 1;
        *by_year.entry(chrono::Datelike::year(&e.issue)).or_default() += 1;
        *by_day.entry(e.issue.date_naive()).or_default() += 1;
    }
    let mut by_name: Vec<(String, usize)> = by_name
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    // Count descending, then name so equal counts keep a stable order.
    by_name.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    PointSummary {
        total,
        by_name,
        first_year: by_year.keys().copied().min(),
        worst_day: by_day.into_iter().max_by_key(|&(d, n)| (n, d)),
        busiest_year: by_year.into_iter().max_by_key(|&(y, n)| (n, y)),
    }
}

/// Fetch every archived VTEC event that covered `(lon, lat)`. `sdate`/`edate` are `YYYY-MM-DD`.
pub async fn fetch_point_events(
    client: &reqwest::Client,
    lon: f64,
    lat: f64,
    sdate: &str,
    edate: &str,
) -> anyhow::Result<Vec<PointEvent>> {
    let body = client
        .get(crate::net::fetch_url(BYPOINT_URL))
        .query(&[
            ("lon", lon.to_string().as_str()),
            ("lat", lat.to_string().as_str()),
            ("sdate", sdate),
            ("edate", edate),
        ])
        .header("User-Agent", alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_point_events(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::FeatureKind;

    #[test]
    fn parses_tornado_and_severe() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-98,35],[-97,35],[-97,36],[-98,35]]]},
             "properties":{"wfo":"OUN","phenomena":"TO","significance":"W","eventid":42,
               "issue":"2013-05-20T19:56:00+00:00","expire":"2013-05-20T20:39:00+00:00"}},
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-96,34],[-95,34],[-95,35],[-96,34]]]},
             "properties":{"wfo":"OUN","phenomena":"SV","significance":"W","eventid":43,
               "issue":"2013-05-20T19:50:00+00:00","expire":"2013-05-20T20:30:00+00:00"}}]}"#;
        let feats = parse(json).unwrap();
        assert_eq!(feats.len(), 2);
        let tor = &feats[0];
        assert_eq!(tor.kind, FeatureKind::Warning);
        assert_eq!(tor.stroke, [255, 0, 0, 235], "Tornado Warning red");
        assert_eq!(tor.title, "Tornado Warning");
        let a = tor.alert.as_ref().unwrap();
        assert_eq!(a.id, "OUN-TO-42-2013-05-20T19:56:00+00:00");
        assert!(a.expires.is_some());
        assert_eq!(feats[1].title, "Severe Thunderstorm Warning");
    }

    // Trimmed from a real vtec_events_bypoint.py response for -97.5,35.2 (Norman, OK).
    const BYPOINT: &str = r#"{"events":[
        {"issue":"1986-03-10T03:40:00Z","expire":"1986-03-10T04:15:00Z","eventid":5,
         "phenomena":"SV","significance":"W","wfo":"OUN","name":"Severe Thunderstorm Warning",
         "ph_name":"Severe Thunderstorm","sig_name":"Warning","ugc":"OKC027"},
        {"issue":"2013-05-20T19:56:00Z","expire":"2013-05-20T20:39:00Z","eventid":42,
         "phenomena":"TO","significance":"W","wfo":"OUN","name":"Tornado Warning",
         "ph_name":"Tornado","sig_name":"Warning","ugc":"OKC027"},
        {"issue":"2013-05-20T21:10:00Z","expire":"2013-05-20T21:45:00Z","eventid":43,
         "phenomena":"TO","significance":"W","wfo":"OUN","name":"Tornado Warning",
         "ph_name":"Tornado","sig_name":"Warning","ugc":"OKC027"},
        {"issue":"2013-05-20T14:00:00Z","expire":"2013-05-21T02:00:00Z","eventid":211,
         "phenomena":"TO","significance":"A","wfo":"OUN","name":"Tornado Watch",
         "ph_name":"Tornado","sig_name":"Watch","ugc":"OKC027"}]}"#;

    #[test]
    fn summarizes_point_warning_history() {
        let ev = parse_point_events(BYPOINT).unwrap();
        assert_eq!(ev.len(), 4);
        let s = summarize(&ev);
        assert_eq!(s.total, 3, "the watch is not counted");
        assert_eq!(
            s.by_name,
            vec![
                ("Tornado Warning".to_string(), 2),
                ("Severe Thunderstorm Warning".to_string(), 1)
            ]
        );
        assert_eq!(s.first_year, Some(1986));
        assert_eq!(s.busiest_year, Some((2013, 2)));
        let (day, n) = s.worst_day.unwrap();
        assert_eq!((day.to_string().as_str(), n), ("2013-05-20", 2));
    }
}
