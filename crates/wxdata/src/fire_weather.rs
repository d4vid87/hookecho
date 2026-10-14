//! Public NOAA SPC fire-weather outlook polygons.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const BASE: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/fire_weather/SPC_firewx/MapServer";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Conditions,
    DryThunderstorms,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Conditions => "Conditions",
            Self::DryThunderstorms => "Dry thunderstorms",
        }
    }
}

/// The service has separate condition and dry-thunderstorm layers for each forecast day.
pub fn layer_id(day: u8, kind: Kind) -> Option<u8> {
    if !(1..=8).contains(&day) { return None; }
    Some(match (day, kind) {
        (1, Kind::Conditions) => 1,
        (1, Kind::DryThunderstorms) => 2,
        (2, Kind::Conditions) => 4,
        (2, Kind::DryThunderstorms) => 5,
        (d, Kind::Conditions) => 8 + (d - 3) * 3,
        (d, Kind::DryThunderstorms) => 7 + (d - 3) * 3,
    })
}

fn category(day: u8, kind: Kind, props: &serde_json::Map<String, serde_json::Value>) -> Option<(&'static str, [u8; 3])> {
    if day <= 2 {
        let dn = props.get("dn")?.as_i64()?;
        return match (kind, dn) {
            (Kind::Conditions, 5) => Some(("Elevated", [230, 152, 0])),
            (Kind::Conditions, 8) => Some(("Critical", [255, 0, 0])),
            (Kind::Conditions, 10) => Some(("Extreme", [230, 0, 169])),
            (Kind::DryThunderstorms, 5) => Some(("Isolated dry thunderstorms", [115, 76, 0])),
            (Kind::DryThunderstorms, 8) => Some(("Scattered dry thunderstorms", [220, 55, 35])),
            _ => None,
        };
    }
    let value = props.get("label").and_then(|v| v.as_str()).unwrap_or("");
    match (kind, value) {
        (Kind::Conditions, "0.40") => Some(("Marginal (40% wind/RH)", [255, 170, 0])),
        (Kind::Conditions, "0.70") => Some(("Critical (70% wind/RH)", [230, 0, 0])),
        (Kind::DryThunderstorms, "0.10") => Some(("Marginal (10%) dry thunderstorms", [115, 76, 0])),
        (Kind::DryThunderstorms, "0.40") => Some(("Critical (40%) dry thunderstorms", [0, 112, 255])),
        _ => None,
    }
}

pub fn parse(json: &str, day: u8, kind: Kind) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let Some((label, rgb)) = category(day, kind, props) else { return; };
        let text = |key| props.get(key).and_then(|v| v.as_str()).unwrap_or("");
        let title = format!("Day {day} fire weather: {label}");
        let detail = format!("NOAA Storm Prediction Center fire-weather outlook\n{label}\nIssued: {} UTC\nValid: {} UTC\nExpires: {} UTC\nSource: {}", text("issue"), text("valid"), text("expire"), text("idp_source"));
        for rings in polygons_of(geom) {
            out.push(GeoFeature { rings, fill: [rgb[0], rgb[1], rgb[2], 55], stroke: [rgb[0], rgb[1], rgb[2], 230], kind: FeatureKind::Outlook, title: title.clone(), detail: detail.clone(), alert: None });
        }
    })?;
    Ok(out)
}

pub async fn fetch(client: &reqwest::Client, day: u8, kind: Kind) -> anyhow::Result<Vec<GeoFeature>> {
    let layer = layer_id(day, kind).ok_or_else(|| anyhow::anyhow!("fire-weather day out of range"))?;
    // Days 1–2 and days 3–8 expose different field schemas. Their union is rejected by ArcGIS.
    let fields = if day <= 2 { "dn%2Cvalid%2Cexpire%2Cidp_source" } else { "label%2Cvalid%2Cexpire%2Cissue%2Cidp_source" };
    let url = format!("{BASE}/{layer}/query?where=1%3D1&outFields={fields}&geometryPrecision=4&maxAllowableOffset=0.002&f=geojson");
    let body = client.get(crate::net::fetch_url(&url)).timeout(crate::net::FEED_TIMEOUT).header("User-Agent", USER_AGENT).send().await?.error_for_status()?.text().await?;
    parse(&body, day, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_layers_cover_all_days_and_categories() {
        assert_eq!(layer_id(1, Kind::Conditions), Some(1));
        assert_eq!(layer_id(8, Kind::Conditions), Some(23));
        assert_eq!(layer_id(8, Kind::DryThunderstorms), Some(22));
        assert_eq!(layer_id(9, Kind::Conditions), None);
    }
    #[test]
    fn parses_only_real_outlook_polygons_with_validity() {
        let json = r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":null,"properties":{"dn":0}},{"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[-100,35],[-99,35],[-99,36],[-100,35]]]},"properties":{"dn":8,"valid":"202610141700","expire":"202610151200","idp_source":"SPC"}}]}"#;
        let f = parse(json, 1, Kind::Conditions).unwrap();
        assert_eq!(f.len(), 1);
        assert!(f[0].title.contains("Critical"));
        assert!(f[0].detail.contains("202610141700"));
    }
}
