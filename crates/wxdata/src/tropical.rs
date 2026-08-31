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
    /// Minimum central pressure (mb) at this point, when the forecast carries one.
    pub mb: Option<f32>,
    /// Forecast hour this point is valid at (+0/+12/+24 …), when present.
    pub tau: Option<f32>,
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
    /// Minimum central pressure (mb) as issued, when the feed carries one.
    pub pressure_mb: Option<f32>,
    pub points: Vec<TrackPoint>,
    /// Public advisory (TCP) page for this storm, straight from the feed.
    pub advisory_url: Option<String>,
    /// Forecast discussion (TCD) page — the hurricane specialist's own reasoning.
    pub discussion_url: Option<String>,
}

/// The fetched tropical picture: cones (as overlay features) plus per-storm positions/tracks.
#[derive(Debug, Clone, Default)]
pub struct TropicalData {
    pub cones: Vec<GeoFeature>,
    pub storms: Vec<TropicalStorm>,
    /// Forecast wind-field polygons at the requested threshold (34/50/64 kt), if one was asked
    /// for. Drawn beneath the cones: the cone is where the centre might go, this is how far out
    /// the damaging wind reaches.
    pub wind_radii: Vec<GeoFeature>,
    /// Potential storm surge flooding polygons (NHC P-Surge), when requested.
    pub surge: Vec<GeoFeature>,
}

/// A JSON number that NHC sometimes ships as a string.
fn number(v: Option<&serde_json::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// A sea-level pressure in mb, or `None` for a missing value. The feeds use sentinels (`0`,
/// `-9999`) rather than nulls, so anything off the physical scale is missing, not data.
fn mb_value(v: Option<&serde_json::Value>) -> Option<f32> {
    number(v)
        .filter(|p| (850.0..=1050.0).contains(p))
        .map(|p| p as f32)
}

/// Fill for a wind threshold: tropical-storm force through hurricane force.
fn wind_color(kt: u8) -> [u8; 3] {
    match kt {
        64 => [230, 60, 60],
        50 => [240, 160, 50],
        _ => [70, 200, 220],
    }
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
    fetch_active_opts(client, None, false).await
}

/// Fetch the tropical picture, optionally adding the forecast wind field at `wind_kt`
/// (34/50/64) and the potential storm-surge flooding polygons.
pub async fn fetch_active_opts(
    client: &reqwest::Client,
    wind_kt: Option<u8>,
    surge: bool,
) -> anyhow::Result<TropicalData> {
    let cs = client
        .get(crate::net::fetch_url(CURRENT_STORMS))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let storms_json: serde_json::Value = serde_json::from_str(&cs)?;
    let active = storms_json
        .get("activeStorms")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if active.is_empty() {
        return Ok(TropicalData::default());
    }

    // Discover the per-bin layer ids by name prefix (ids drift; names are stable).
    let layers_json = client
        .get(crate::net::fetch_url(&format!("{MAPSERVER}/layers?f=json")))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let layers: serde_json::Value = serde_json::from_str(&layers_json)?;
    let layer_list = layers
        .get("layers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
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
        let lat = s
            .get("latitudeNumeric")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let lon = s
            .get("longitudeNumeric")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
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
                            // The outline is drawn dashed by the painter (`tropical_draw`);
                            // a solid tessellated stroke underneath would fill the gaps back in.
                            fill: [255, 255, 255, 28],
                            stroke: [255, 255, 255, 0],
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
                                kt: number(props.get("maxwind")).unwrap_or(0.0) as f32,
                                label: props
                                    .get("datelbl")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                mb: mb_value(props.get("mslp")),
                                tau: number(props.get("tau")).map(|t| t as f32),
                            });
                        }
                    }
                }
            }
        }
        // Forecast wind field at the requested threshold. `radii` is the threshold in knots and
        // `tau` the forecast hour; the union over all hours is the swath people want to see.
        if let Some(kt) = wind_kt {
            if let Some(id) = find_layer(&bin, "Forecast Wind Radii") {
                if let Ok(gj) = query_layer(client, id).await {
                    let rgb = wind_color(kt);
                    for (geom, props) in features(&gj) {
                        let radii = number(props.get("radii")).unwrap_or(0.0);
                        if (radii - f64::from(kt)).abs() > 0.5 {
                            continue;
                        }
                        let tau = props.get("tau").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        for poly in polygons_of(&geom) {
                            data.wind_radii.push(GeoFeature {
                                rings: poly,
                                fill: [rgb[0], rgb[1], rgb[2], 35],
                                stroke: [rgb[0], rgb[1], rgb[2], 170],
                                kind: FeatureKind::TropicalCone,
                                title: format!("{name} {kt} kt wind"),
                                detail: format!(
                                    "{name} \u{2014} forecast {kt} kt wind field\nForecast hour: +{tau:.0} h"
                                ),
                                alert: None,
                            });
                        }
                    }
                }
            }
        }

        // The feed nests each product's page under its own key; take the URL and nothing else.
        let product_url = |key: &str| {
            s.get(key)
                .and_then(|p| p.get("url"))
                .and_then(|u| u.as_str())
                .map(str::to_string)
        };
        data.storms.push(TropicalStorm {
            id: get("id"),
            name,
            classification,
            intensity_kt,
            lat,
            lon,
            pressure_mb: mb_value(s.get("pressure")),
            points,
            advisory_url: product_url("publicAdvisory"),
            discussion_url: product_url("forecastDiscussion"),
        });
    }
    if surge {
        match fetch_surge(client).await {
            // Surge is a separate service; its outage must not cost the cones.
            Ok(f) => data.surge = f,
            Err(e) => log::warn!("storm surge: {e}"),
        }
    }
    Ok(data)
}

/// Potential Storm Surge Flooding (NHC P-Surge): how deep water could get above ground.
const SURGE_SERVICE: &str =
    "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_PeakStormSurge/MapServer";

/// Fill for a surge depth band, read from the band's own label ("greater than 3 feet").
fn surge_color(label: &str) -> [u8; 3] {
    let l = label.to_ascii_lowercase();
    match () {
        _ if l.contains('9') => [150, 40, 190],
        _ if l.contains('6') => [225, 60, 60],
        _ if l.contains('3') => [240, 160, 50],
        _ => [240, 230, 90],
    }
}

/// Fetch the potential storm-surge flooding polygons. Empty (`Ok`) when nothing is threatened.
pub async fn fetch_surge(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    let url = format!("{SURGE_SERVICE}/2/query?where=1%3D1&outFields=name,snippet&f=geojson");
    let body = client
        .get(crate::net::fetch_url(&url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_surge(&body)
}

/// Parse a P-Surge polygons payload.
pub fn parse_surge(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let gj: GeoJson = body_geojson(json)?;
    let mut out = Vec::new();
    for (geom, props) in features(&gj) {
        let label = props
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Storm surge");
        let rgb = surge_color(label);
        for poly in polygons_of(&geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 90],
                stroke: [rgb[0], rgb[1], rgb[2], 220],
                kind: FeatureKind::TropicalCone,
                title: format!("Surge: {label}"),
                detail: format!(
                    "Potential storm surge flooding\n{label}\n\nDepths are above ground, in the \
                     areas that could be inundated if the peak surge arrives at high tide."
                ),
                alert: None,
            });
        }
    }
    Ok(out)
}

fn body_geojson(json: &str) -> anyhow::Result<GeoJson> {
    json.parse()
        .map_err(|e| anyhow::anyhow!("tropical geojson: {e}"))
}

/// Query an ArcGIS MapServer layer for all features as GeoJSON.
async fn query_layer(client: &reqwest::Client, id: u64) -> anyhow::Result<GeoJson> {
    let url = format!("{MAPSERVER}/{id}/query?where=1%3D1&outFields=*&f=geojson");
    let body = client
        .get(crate::net::fetch_url(&url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    body.parse()
        .map_err(|e| anyhow::anyhow!("tropical geojson: {e}"))
}

/// Extract `(geometry, properties)` pairs from a parsed GeoJSON feature collection.
fn features(
    gj: &GeoJson,
) -> Vec<(
    geojson::GeometryValue,
    serde_json::Map<String, serde_json::Value>,
)> {
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

/// One fetched NHC text product.
#[derive(Debug, Clone)]
pub struct Advisory {
    /// Storm name plus which product this is, for the window title.
    pub title: String,
    pub text: String,
}

/// The product text out of an NHC text page.
///
/// These pages are `.shtml` with the product in a single `<pre>` block — there is no `.txt`
/// variant (they 404), so the block is what there is to take. Returns `None` rather than a page
/// full of navigation chrome if the layout ever changes.
pub fn advisory_text(html: &str) -> Option<String> {
    let start = html.find("<pre>")? + "<pre>".len();
    let end = html[start..].find("</pre>")? + start;
    let body = html[start..end].trim();
    if body.is_empty() {
        return None;
    }
    // The products are plain text inside the block, but the page still escapes the handful of
    // characters HTML reserves.
    Some(
        body.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&"),
    )
}

/// Fetch one of a storm's text products by URL.
pub async fn fetch_advisory(
    client: &reqwest::Client,
    title: &str,
    url: &str,
) -> anyhow::Result<Advisory> {
    let body = client
        .get(crate::net::fetch_url(url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let text =
        advisory_text(&body).ok_or_else(|| anyhow::anyhow!("no product text in the NHC page"))?;
    Ok(Advisory {
        title: title.to_string(),
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURGE: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"name":"greater than 3 feet above ground"},
       "geometry":{"type":"Polygon","coordinates":[[[-90.0,29.0],[-89.5,29.0],[-89.5,29.5],[-90.0,29.0]]]}},
      {"type":"Feature","properties":{"name":"greater than 9 feet above ground"},
       "geometry":{"type":"Polygon","coordinates":[[[-89.4,29.1],[-89.2,29.1],[-89.2,29.3],[-89.4,29.1]]]}}]}"#;

    #[test]
    fn surge_bands_label_and_deepen() {
        let f = parse_surge(SURGE).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f[0].title.contains("3 feet"));
        assert_eq!(f[0].fill[..3], surge_color("greater than 3 feet"));
        assert_ne!(
            f[0].fill[..3],
            f[1].fill[..3],
            "deeper bands read differently"
        );
        assert!(f[1].detail.contains("above ground"));
    }

    #[test]
    fn wind_thresholds_have_distinct_colors() {
        assert_ne!(wind_color(34), wind_color(50));
        assert_ne!(wind_color(50), wind_color(64));
    }

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
    fn advisory_text_comes_out_of_the_pre_block() {
        let page = "<html><body><div>nav junk</div><pre>\nWTPZ44 KNHC 242032\nTCDEP4\n\n\
                    Tropical Storm Iselle Discussion Number 6\n</pre><footer>more junk</footer>";
        let t = advisory_text(page).expect("pre block");
        assert!(t.starts_with("WTPZ44"), "{t}");
        assert!(t.contains("Discussion Number 6"));
        assert!(!t.contains("junk"), "page chrome must not come through");
    }

    #[test]
    fn a_page_without_a_product_is_none_not_chrome() {
        assert!(advisory_text("<html><body>no product here</body></html>").is_none());
        assert!(advisory_text("<pre>   </pre>").is_none());
    }

    #[test]
    fn escaped_characters_are_unescaped() {
        assert_eq!(
            advisory_text("<pre>winds &gt; 50 kt &amp; rising</pre>").unwrap(),
            "winds > 50 kt & rising"
        );
    }

    #[test]
    fn pressure_takes_numbers_and_strings_but_not_sentinels() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"a":985,"b":"1004.5","c":-9999,"d":0,"e":"n/a"}"#).unwrap();
        assert_eq!(mb_value(v.get("a")), Some(985.0));
        assert_eq!(mb_value(v.get("b")), Some(1004.5));
        for k in ["c", "d", "e"] {
            assert_eq!(mb_value(v.get(k)), None, "{k} is missing, not a pressure");
        }
        assert_eq!(mb_value(None), None);
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
