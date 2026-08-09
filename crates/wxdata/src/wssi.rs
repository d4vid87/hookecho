//! Winter Storm Severity Index — how disruptive a winter storm will be, not how much falls.
//!
//! WPC publishes the WSSI as a map service; this reads the overall-impact layer for a given day
//! and decodes it into the shared [`GeoFeature`] type, so clicking a band gives its impact level
//! and valid window like any other outlook polygon.
//!
//! Layers are discovered by name rather than by id: the service has separate per-day and
//! combined-day layers and the ids have moved before.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const SERVICE: &str =
    "https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/wpc_wssi/MapServer";

/// Fill for an impact level. The scale is WPC's own: limited → extreme.
fn impact_color(label: &str) -> [u8; 3] {
    match label.to_ascii_uppercase().as_str() {
        "LIMITED" => [120, 200, 120],
        "MINOR" => [245, 235, 110],
        "MODERATE" => [240, 160, 60],
        "MAJOR" => [225, 70, 70],
        "EXTREME" => [175, 60, 195],
        // "WINTER WEATHER AREA" — the outermost band, where something wintry happens but the
        // index doesn't rate it an impact.
        _ => [140, 160, 190],
    }
}

/// Parse the WSSI overall-impact GeoJSON into features.
pub fn parse(json: &str, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| {
            props
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let impact = str_of("impact");
        let valid = str_of("valid_time");
        let rgb = impact_color(&impact);
        for rings in polygons_of(geom) {
            out.push(GeoFeature {
                rings,
                fill: [rgb[0], rgb[1], rgb[2], 70],
                stroke: [rgb[0], rgb[1], rgb[2], 220],
                kind: FeatureKind::Outlook,
                title: format!("WSSI D{day}: {impact}"),
                detail: format!(
                    "Winter Storm Severity Index — Day {day}\nImpact: {impact}\nValid: {valid}"
                ),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// The map-service layer id whose name is `name`, from a `?f=json` service description.
fn layer_id(service_json: &str, name: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(service_json).ok()?;
    v.get("layers")?.as_array()?.iter().find_map(|l| {
        (l.get("name")?.as_str()? == name).then(|| l.get("id")?.as_u64().map(|i| i as u32))?
    })
}

/// Fetch the WSSI overall impact for `day` (1-3).
pub async fn fetch(client: &reqwest::Client, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    let day = day.clamp(1, 3);
    let meta = client
        .get(crate::net::fetch_url(&format!("{SERVICE}?f=json")))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let want = format!("Overall_Impact_Day_{day}");
    let id = layer_id(&meta, &want).ok_or_else(|| anyhow::anyhow!("no WSSI layer named {want}"))?;
    let body = client
        .get(crate::net::fetch_url(&format!(
            "{SERVICE}/{id}/query?where=1%3D1&outFields=impact,valid_time&f=geojson"
        )))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE_JSON: &str = r#"{"mapName":"Winter Storm Severity Index","layers":[
        {"id":0,"name":"Overall Impact"},
        {"id":1,"name":"Overall_Impact_Day_1"},
        {"id":2,"name":"Overall_Impact_Day_2"},
        {"id":5,"name":"Snow Amount"}]}"#;

    const FEATURES: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"impact":"MAJOR","valid_time":"14Z 02/15/26 - 12Z 02/16/26"},
         "geometry":{"type":"Polygon","coordinates":[[[-90.0,44.0],[-89.0,44.0],[-89.0,45.0],[-90.0,44.0]]]}},
        {"type":"Feature","properties":{"impact":"WINTER WEATHER AREA","valid_time":"14Z 02/15/26 - 12Z 02/16/26"},
         "geometry":{"type":"Polygon","coordinates":[[[-92.0,43.0],[-91.0,43.0],[-91.0,44.0],[-92.0,43.0]]]}}]}"#;

    #[test]
    fn layers_resolve_by_name_not_by_id() {
        assert_eq!(layer_id(SERVICE_JSON, "Overall_Impact_Day_1"), Some(1));
        assert_eq!(layer_id(SERVICE_JSON, "Overall_Impact_Day_2"), Some(2));
        assert_eq!(layer_id(SERVICE_JSON, "Overall_Impact_Day_3"), None);
    }

    #[test]
    fn features_carry_impact_and_window() {
        let f = parse(FEATURES, 1).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].title, "WSSI D1: MAJOR");
        assert!(f[0].detail.contains("14Z 02/15/26"));
        assert_eq!(f[0].fill[..3], impact_color("MAJOR"));
        // The outermost band is not an impact level and gets the neutral color.
        assert_eq!(f[1].fill[..3], impact_color("WINTER WEATHER AREA"));
        assert_eq!(f[0].rings[0].len(), 4);
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_the_live_service() {
        let c = reqwest::Client::new();
        // Out of season this is legitimately empty; the fetch still has to succeed.
        let f = fetch(&c, 1).await.expect("WSSI day 1");
        for feat in &f {
            assert!(feat.title.starts_with("WSSI D1:"));
        }
    }
}
