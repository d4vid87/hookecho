//! Public WPC Day 1–3 probabilities for 4-inch snow and quarter-inch ice.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const SERVICE: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/precip/wpc_prob_winter_precip/MapServer";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Snow,
    Ice,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self { Self::Snow => "Snow ≥4 in", Self::Ice => "Ice ≥0.25 in" }
    }
}

pub fn layer_id(day: u8, kind: Kind) -> Option<u8> {
    if !(1..=3).contains(&day) { return None; }
    Some((day - 1) * 5 + match kind { Kind::Snow => 1, Kind::Ice => 4 })
}

fn color(kind: Kind, outlook: &str) -> Option<[u8; 3]> {
    let high = outlook.starts_with("High");
    let moderate = outlook.starts_with("Moderate");
    let slight = outlook.starts_with("Slight");
    match (kind, high, moderate, slight) {
        (Kind::Snow, true, _, _) => Some([167, 37, 243]),
        (Kind::Snow, _, true, _) => Some([78, 123, 227]),
        (Kind::Snow, _, _, true) => Some([109, 215, 251]),
        (Kind::Ice, true, _, _) => Some([234, 53, 179]),
        (Kind::Ice, _, true, _) => Some([237, 113, 46]),
        (Kind::Ice, _, _, true) => Some([243, 171, 62]),
        _ => None,
    }
}

pub fn parse(json: &str, day: u8, kind: Kind) -> anyhow::Result<Vec<GeoFeature>> {
    anyhow::ensure!(layer_id(day, kind).is_some(), "winter probability day out of range");
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let text = |name| props.get(name).and_then(|value| value.as_str()).unwrap_or("");
        let outlook = text("outlook");
        let Some(rgb) = color(kind, outlook) else { return; };
        let title = format!("Day {day} {}: {outlook}", kind.label());
        let detail = format!("NOAA WPC winter precipitation outlook\n{} probability: {outlook}\nIssued: {}\nValid: {}\nSource: {}", kind.label(), text("issue_time"), text("valid_time"), text("idp_source"));
        for rings in polygons_of(geom) {
            out.push(GeoFeature { rings, fill: [rgb[0], rgb[1], rgb[2], 65], stroke: [rgb[0], rgb[1], rgb[2], 220], kind: FeatureKind::Outlook, title: title.clone(), detail: detail.clone(), alert: None });
        }
    })?;
    Ok(out)
}

pub async fn fetch(client: &reqwest::Client, day: u8, kind: Kind) -> anyhow::Result<Vec<GeoFeature>> {
    let layer = layer_id(day, kind).ok_or_else(|| anyhow::anyhow!("winter probability day out of range"))?;
    let url = format!("{SERVICE}/{layer}/query?where=1%3D1&outFields=outlook%2Cvalid_time%2Cissue_time%2Cidp_source&geometryPrecision=3&maxAllowableOffset=0.02&f=geojson");
    let body = client.get(crate::net::fetch_url(&url)).timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT).send().await?.error_for_status()?.text().await?;
    parse(&body, day, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_public_layers_and_outlook_categories() {
        assert_eq!(layer_id(1, Kind::Snow), Some(1));
        assert_eq!(layer_id(3, Kind::Snow), Some(11));
        assert_eq!(layer_id(3, Kind::Ice), Some(14));
        assert_eq!(layer_id(4, Kind::Ice), None);
        let body = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"outlook":"Slight (10-39%)","valid_time":"00Z 10/15/26 - 00Z 10/16/26","issue_time":"2026-10-14 19:17:44","idp_source":"wwd_day1_psnow_gt_04"},"geometry":{"type":"Polygon","coordinates":[[[-100,35],[-99,35],[-99,36],[-100,35]]]}}]}"#;
        let features = parse(body, 1, Kind::Snow).unwrap();
        assert_eq!(features.len(), 1);
        assert!(features[0].title.contains("Slight (10-39%)"));
        assert!(features[0].detail.contains("00Z 10/15/26"));
        assert_eq!(features[0].fill[..3], [109, 215, 251]);
    }
}
