//! NHC tropical-cyclone suite: active-storm positions, forecast cones, and track points.
//!
//! `CurrentStorms.json` lists the active storms (empty in the off-season). Each storm's cone /
//! forecast-points geometry lives in the NHC tropical MapServer, whose per-storm layers are keyed
//! by a `binNumber` prefix ("AT2 Forecast Cone", …). Layer ids drift between runs, so they're
//! discovered at fetch time by name prefix rather than hardcoded.

use crate::alerts::USER_AGENT;
use crate::overlay::{polygons_of, FeatureKind, GeoFeature};
use geojson::GeoJson;

const CURRENT_STORMS: &str = "https://www.nhc.noaa.gov/CurrentStorms.json";
const MAPSERVER: &str =
    "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer";

/// One forecast/observed track point for a storm.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackPoint {
    pub lon: f64,
    pub lat: f64,
    /// Max sustained wind (kt) at this point.
    pub kt: f32,
    /// Short label (valid time / development stage).
    pub label: String,
}

/// One active tropical cyclone.
#[derive(Debug, Clone, PartialEq)]
pub struct TropicalStorm {
    pub id: String,
    pub name: String,
    /// Classification as issued (TD / TS / HU …).
    pub classification: String,
    /// Current intensity (kt).
    pub intensity_kt: f32,
    pub lat: f64,
    pub lon: f64,
    pub points: Vec<TrackPoint>,
}

/// The fetched tropical picture: cones (as overlay features) plus per-storm positions/tracks.
#[derive(Debug, Clone, Default)]
pub struct TropicalData {
    pub cones: Vec<GeoFeature>,
    pub storms: Vec<TropicalStorm>,
}

/// Saffir–Simpson category label + color for a max-wind in knots.
pub fn saffir_simpson(kt: f32) -> (&'static str, [u8; 3]) {
    match kt {
        k if k < 34.0 => ("TD", [150, 150, 160]),
        k if k < 64.0 => ("TS", [70, 200, 220]),
        k if k < 83.0 => ("Cat 1", [240, 230, 90]),
        k if k < 96.0 => ("Cat 2", [240, 160, 50]),
        k if k < 113.0 => ("Cat 3", [230, 60, 60]),
        k if k < 137.0 => ("Cat 4", [220, 60, 200]),
        _ => ("Cat 5", [180, 80, 240]),
    }
}

/// Fetch the active-storm cones + tracks. Off-season (no active storms) returns empty (`Ok`).
pub async fn fetch_active(client: &reqwest::Client) -> anyhow::Result<TropicalData> {
    let cs = client
        .get(CURRENT_STORMS)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let storms_json: serde_json::Value = serde_json::from_str(&cs)?;
    let active = storms_json.get("activeStorms").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if active.is_empty() {
        return Ok(TropicalData::default());
    }

    // Discover the per-bin layer ids by name prefix (ids drift; names are stable).
    let layers_json = client
        .get(format!("{MAPSERVER}/layers?f=json"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let layers: serde_json::Value = serde_json::from_str(&layers_json)?;
    let layer_list = layers.get("layers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let find_layer = |bin: &str, suffix: &str| -> Option<u64> {
        let want = format!("{bin} {suffix}");
        layer_list.iter().find_map(|l| {
            (l.get("name").and_then(|n| n.as_str()) == Some(want.as_str()))
                .then(|| l.get("id").and_then(|i| i.as_u64()))
                .flatten()
        })
    };

    let mut data = TropicalData::default();
    for s in &active {
        let get = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let bin = get("binNumber");
        let intensity_kt = get("intensity").parse::<f32>().unwrap_or(0.0);
        let name = get("name");
        let classification = get("classification");
        let lat = s.get("latitudeNumeric").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let lon = s.get("longitudeNumeric").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (cat, _) = saffir_simpson(intensity_kt);
        let detail = format!(
            "{name} ({classification})\nBin: {bin}\nIntensity: {intensity_kt:.0} kt ({cat})\nNHC forecast cone",
        );

        // Forecast cone → an overlay polygon.
        if let Some(id) = find_layer(&bin, "Forecast Cone") {
            if let Ok(gj) = query_layer(client, id).await {
                for f in &features(&gj) {
                    for poly in polygons_of(&f.0) {
                        data.cones.push(GeoFeature {
                            rings: poly,
                            fill: [255, 255, 255, 40],
                            stroke: [230, 230, 230, 200],
                            kind: FeatureKind::TropicalCone,
                            title: format!("{name} cone"),
                            detail: detail.clone(),
                            alert: None,
                        });
                    }
                }
            }
        }

        // Forecast points → track.
        let mut points = Vec::new();
        if let Some(id) = find_layer(&bin, "Forecast Points") {
            if let Ok(gj) = query_layer(client, id).await {
                for (geom, props) in features(&gj) {
                    if let geojson::GeometryValue::Point { coordinates } = &geom {
                        let c = coordinates.as_slice();
                        if c.len() >= 2 {
                            points.push(TrackPoint {
                                lon: c[0],
                                lat: c[1],
                                kt: props.get("maxwind").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                                label: props.get("datelbl").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            });
                        }
                    }
                }
            }
        }
        data.storms.push(TropicalStorm { id: get("id"), name, classification, intensity_kt, lat, lon, points });
    }
    Ok(data)
}

/// Query an ArcGIS MapServer layer for all features as GeoJSON.
async fn query_layer(client: &reqwest::Client, id: u64) -> anyhow::Result<GeoJson> {
    let url = format!("{MAPSERVER}/{id}/query?where=1%3D1&outFields=*&f=geojson");
    let body = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    body.parse().map_err(|e| anyhow::anyhow!("tropical geojson: {e}"))
}

/// Extract `(geometry, properties)` pairs from a parsed GeoJSON feature collection.
fn features(gj: &GeoJson) -> Vec<(geojson::GeometryValue, serde_json::Map<String, serde_json::Value>)> {
    let mut out = Vec::new();
    if let GeoJson::FeatureCollection(fc) = gj {
        for f in &fc.features {
            if let Some(g) = &f.geometry {
                out.push((g.value.clone(), f.properties.clone().unwrap_or_default()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saffir_simpson_boundaries() {
        assert_eq!(saffir_simpson(25.0).0, "TD");
        assert_eq!(saffir_simpson(34.0).0, "TS");
        assert_eq!(saffir_simpson(64.0).0, "Cat 1");
        assert_eq!(saffir_simpson(96.0).0, "Cat 3");
        assert_eq!(saffir_simpson(137.0).0, "Cat 5");
    }

    #[test]
    fn empty_season_parses_to_empty() {
        // The off-season CurrentStorms.json has an empty activeStorms list.
        let json = r#"{"activeStorms":[]}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let active = v.get("activeStorms").and_then(|a| a.as_array()).unwrap();
        assert!(active.is_empty(), "off-season → no storms");
    }

    #[test]
    fn parses_forecast_point_geojson() {
        let gj: GeoJson = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[-85.1,28.0]},
             "properties":{"maxwind":25,"datelbl":"10:00 PM Sun"}}]}"#
            .parse()
            .unwrap();
        let feats = features(&gj);
        assert_eq!(feats.len(), 1);
        let (geom, props) = &feats[0];
        assert!(matches!(geom, geojson::GeometryValue::Point { .. }));
        assert_eq!(props.get("maxwind").and_then(|v| v.as_f64()), Some(25.0));
    }
}
