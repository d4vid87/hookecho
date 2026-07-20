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
            .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
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
        .get(SBW_URL)
        .query(&[("ts", ts)])
        .header("User-Agent", alerts::USER_AGENT)
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
}
