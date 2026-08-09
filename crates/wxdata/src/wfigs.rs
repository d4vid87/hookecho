//! Wildfire perimeters and incident points from WFIGS (the interagency fire GIS feed).
//!
//! Two ArcGIS FeatureServers, both queried over the current view's bbox: daily fire perimeters
//! (polygons) and incident locations (points). Perimeters nationwide are far too much geometry to
//! pull in one go, hence the envelope query and the record cap.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const PERIMETERS: &str = "https://services3.arcgis.com/T4QMspbfLg3qTGWY/arcgis/rest/services/WFIGS_Interagency_Perimeters_Current/FeatureServer/0/query";
const INCIDENTS: &str = "https://services3.arcgis.com/T4QMspbfLg3qTGWY/arcgis/rest/services/WFIGS_Incident_Locations_Current/FeatureServer/0/query";

/// How many features one bbox query may return. A busy fire season in the West can exceed this;
// ponytail: no paging — the cap keeps the request bounded, and the biggest fires come first
// because the query sorts by size.
const LIMIT: usize = 400;

/// One active fire incident (the point feed, not the perimeter).
#[derive(Debug, Clone, PartialEq)]
pub struct FireIncident {
    pub lon: f64,
    pub lat: f64,
    pub name: String,
    /// Reported size in acres.
    pub acres: Option<f64>,
    /// Containment percent, when reported.
    pub containment: Option<f64>,
}

fn bbox_query(bbox: [f64; 4]) -> String {
    let [w, s, e, n] = bbox;
    format!("&geometry={w},{s},{e},{n}&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&resultRecordCount={LIMIT}&f=geojson")
}

/// Parse the perimeter GeoJSON into fill/stroke polygons with a name/acres popup.
pub fn parse_perimeters(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let num_of = |k: &str| props.get(k).and_then(|v| v.as_f64());
        let name = {
            let n = str_of("poly_IncidentName");
            if n.is_empty() {
                str_of("attr_IncidentName")
            } else {
                n
            }
        };
        let title = if name.is_empty() {
            "Fire perimeter".to_string()
        } else {
            format!("{name} Fire")
        };
        let mut detail = title.clone();
        if let Some(a) = num_of("poly_GISAcres").or_else(|| num_of("attr_IncidentSize")) {
            detail.push_str(&format!("\n{a:.0} acres"));
        }
        if let Some(c) = num_of("attr_PercentContained") {
            detail.push_str(&format!("\n{c:.0}% contained"));
        }
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [200, 60, 20, 70],
                stroke: [235, 110, 40, 230],
                kind: FeatureKind::Outlook,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// Parse the incident-locations GeoJSON into point incidents.
pub fn parse_incidents(json: &str) -> anyhow::Result<Vec<FireIncident>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let geojson::GeometryValue::Point { coordinates: c } = geom else {
            return;
        };
        let (Some(&lon), Some(&lat)) = (c.as_slice().first(), c.as_slice().get(1)) else {
            return;
        };
        out.push(FireIncident {
            lon,
            lat,
            name: props
                .get("IncidentName")
                .and_then(|v| v.as_str())
                .unwrap_or("Fire")
                .to_string(),
            acres: props.get("IncidentSize").and_then(|v| v.as_f64()),
            containment: props.get("PercentContained").and_then(|v| v.as_f64()),
        });
    })?;
    Ok(out)
}

async fn get(client: &reqwest::Client, url: String) -> anyhow::Result<String> {
    Ok(client
        .get(crate::net::fetch_url(&url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Fetch current fire perimeters intersecting `bbox` (`[west, south, east, north]`).
pub async fn fetch_perimeters(
    client: &reqwest::Client,
    bbox: [f64; 4],
) -> anyhow::Result<Vec<GeoFeature>> {
    let url = format!(
        "{PERIMETERS}?where=1%3D1&outFields=poly_IncidentName,attr_IncidentName,poly_GISAcres,attr_IncidentSize,attr_PercentContained&orderByFields=poly_GISAcres%20DESC{}",
        bbox_query(bbox)
    );
    parse_perimeters(&get(client, url).await?)
}

/// Fetch wildfire incidents intersecting `bbox` that haven't been declared out.
pub async fn fetch_incidents(
    client: &reqwest::Client,
    bbox: [f64; 4],
) -> anyhow::Result<Vec<FireIncident>> {
    let where_clause = "IncidentTypeCategory%3D%27WF%27%20AND%20FireOutDateTime%20IS%20NULL";
    let url = format!(
        "{INCIDENTS}?where={where_clause}&outFields=IncidentName,IncidentSize,PercentContained&orderByFields=IncidentSize%20DESC{}",
        bbox_query(bbox)
    );
    parse_incidents(&get(client, url).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_perimeter() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-120,38],[-119,38],[-119,39],[-120,38]]]},
             "properties":{"poly_IncidentName":"Summit","poly_GISAcres":2690,"attr_PercentContained":35}}]}"#;
        let f = parse_perimeters(json).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].title, "Summit Fire");
        assert!(f[0].detail.contains("2690 acres"), "{}", f[0].detail);
        assert!(f[0].detail.contains("35% contained"), "{}", f[0].detail);
    }

    #[test]
    fn parses_incidents_and_skips_non_points() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[-117.48,33.46]},
             "properties":{"IncidentName":"MATEO","IncidentSize":1335,"PercentContained":100}},
            {"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[-120,38],[-119,38],[-119,39],[-120,38]]]},
             "properties":{"IncidentName":"NOTAPOINT"}}]}"#;
        let inc = parse_incidents(json).unwrap();
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].name, "MATEO");
        assert_eq!(inc[0].acres, Some(1335.0));
        assert_eq!(inc[0].containment, Some(100.0));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_california() {
        let c = reqwest::Client::new();
        let bbox = [-124.0, 32.0, -114.0, 42.0];
        assert!(fetch_incidents(&c, bbox).await.is_ok());
        assert!(fetch_perimeters(&c, bbox).await.is_ok());
    }
}
