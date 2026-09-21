//! User-supplied GIS overlays normalized into the existing placefile renderer.

use crate::placefile::{PlaceItem, PlaceKind, Placefile};
use geojson::{GeoJson, GeometryValue};

const LINE: [u8; 4] = [80, 210, 240, 255];
const FILL: [u8; 4] = [80, 210, 240, 72];

/// Parse RFC 7946 GeoJSON. Coordinates are WGS84 lon/lat; legacy documents that explicitly
/// declare another CRS fail rather than being silently plotted in the wrong place.
pub fn geojson(name: &str, text: &str) -> anyhow::Result<Placefile> {
    reject_unsupported_crs(text)?;
    let document: GeoJson = text
        .parse()
        .map_err(|error| anyhow::anyhow!("GeoJSON parse: {error}"))?;
    let mut file = Placefile {
        title: name.to_string(),
        ..Default::default()
    };
    match document {
        GeoJson::Geometry(geometry) => append(&mut file.items, geometry.value, name),
        GeoJson::Feature(feature) => {
            let label = feature_label(feature.properties.as_ref()).unwrap_or(name);
            if let Some(geometry) = feature.geometry {
                append(&mut file.items, geometry.value, label);
            }
        }
        GeoJson::FeatureCollection(collection) => {
            for feature in collection.features {
                let label = feature_label(feature.properties.as_ref()).unwrap_or(name);
                if let Some(geometry) = feature.geometry {
                    append(&mut file.items, geometry.value, label);
                }
            }
        }
    }
    anyhow::ensure!(
        !file.items.is_empty(),
        "GeoJSON contains no supported geometry"
    );
    Ok(file)
}

fn reject_unsupported_crs(text: &str) -> anyhow::Result<()> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(crs) = value.get("crs") {
        let named = crs.to_string();
        anyhow::ensure!(
            named.contains("4326") || named.to_ascii_uppercase().contains("CRS84"),
            "unsupported CRS; convert the layer to WGS84 (EPSG:4326)"
        );
    }
    Ok(())
}

fn feature_label(properties: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<&str> {
    let properties = properties?;
    ["name", "title", "label"]
        .into_iter()
        .find_map(|key| properties.get(key)?.as_str())
}

fn item(kind: PlaceKind) -> PlaceItem {
    PlaceItem {
        threshold_nmi: 0.0,
        time: None,
        anchor: None,
        kind,
    }
}

fn point(coordinates: &geojson::Position) -> Option<[f64; 2]> {
    (coordinates.len() >= 2
        && coordinates[0].is_finite()
        && coordinates[1].is_finite()
        && (-180.0..=180.0).contains(&coordinates[0])
        && (-90.0..=90.0).contains(&coordinates[1]))
    .then(|| [coordinates[0], coordinates[1]])
}

fn line(coordinates: &[geojson::Position]) -> Vec<[f64; 2]> {
    coordinates.iter().filter_map(point).collect()
}

fn append(items: &mut Vec<PlaceItem>, geometry: GeometryValue, label: &str) {
    match geometry {
        GeometryValue::Point { coordinates } => {
            if let Some(pos) = point(&coordinates) {
                items.push(item(PlaceKind::Icon {
                    color: LINE,
                    pos,
                    angle: 0.0,
                    sheet: None,
                    hover: label.to_string(),
                }));
            }
        }
        GeometryValue::MultiPoint { coordinates } => {
            for coordinates in coordinates {
                append(items, GeometryValue::Point { coordinates }, label);
            }
        }
        GeometryValue::LineString { coordinates } => {
            let pts = line(&coordinates);
            if pts.len() >= 2 {
                items.push(item(PlaceKind::Line {
                    color: LINE,
                    width: 2.0,
                    pts,
                }));
            }
        }
        GeometryValue::MultiLineString { coordinates } => {
            for coordinates in coordinates {
                append(items, GeometryValue::LineString { coordinates }, label);
            }
        }
        GeometryValue::Polygon { coordinates } => {
            let rings: Vec<_> = coordinates
                .iter()
                .map(|ring| line(ring))
                .filter(|ring| ring.len() >= 3)
                .collect();
            if !rings.is_empty() {
                items.push(item(PlaceKind::Polygon { color: FILL, rings }));
            }
        }
        GeometryValue::MultiPolygon { coordinates } => {
            for coordinates in coordinates {
                append(items, GeometryValue::Polygon { coordinates }, label);
            }
        }
        GeometryValue::GeometryCollection { geometries } => {
            for geometry in geometries {
                append(items, geometry.value, label);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_supported_geometry_and_rejects_an_unknown_crs() {
        let text = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","properties":{"name":"Target"},"geometry":{"type":"Point","coordinates":[-97,35]}},
          {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[-98,34],[-96,34],[-96,36],[-98,34]]]}}
        ]}"#;
        let file = geojson("analysis.geojson", text).unwrap();
        assert_eq!(file.items.len(), 2);
        let bad =
            r#"{"type":"Point","coordinates":[0,0],"crs":{"properties":{"name":"EPSG:3857"}}}"#;
        assert!(geojson("bad.geojson", bad)
            .unwrap_err()
            .to_string()
            .contains("unsupported CRS"));
    }
}
