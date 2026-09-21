//! User-supplied GIS overlays normalized into the existing placefile renderer.

use crate::placefile::{PlaceItem, PlaceKind, Placefile};
use geojson::{GeoJson, GeometryValue};

const LINE: [u8; 4] = [80, 210, 240, 255];
const FILL: [u8; 4] = [80, 210, 240, 72];

pub fn parse(name: &str, text: &str) -> anyhow::Result<Placefile> {
    if name.to_ascii_lowercase().ends_with(".kml") {
        kml(name, text)
    } else {
        geojson(name, text)
    }
}

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

/// Parse the portable geometry subset of OGC KML. KML coordinates are always WGS84 lon/lat;
/// styles remain deliberately local so imported data cannot fetch arbitrary remote resources.
pub fn kml(name: &str, text: &str) -> anyhow::Result<Placefile> {
    use quick_xml::events::Event;

    #[derive(Clone, Copy)]
    enum Geometry {
        Point,
        Line,
        Ring,
    }

    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut file = Placefile {
        title: name.to_string(),
        ..Default::default()
    };
    let mut geometry = None;
    let mut reading_name = false;
    let mut label = name.to_string();
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(start) => match start.name().local_name().as_ref() {
                b"Placemark" => {
                    label = name.to_string();
                    rings.clear();
                }
                b"name" => reading_name = geometry.is_none(),
                b"Point" => geometry = Some(Geometry::Point),
                b"LineString" => geometry = Some(Geometry::Line),
                b"LinearRing" => geometry = Some(Geometry::Ring),
                _ => {}
            },
            Event::Text(text) if reading_name => label = text.decode()?.into_owned(),
            Event::Text(text) if geometry.is_some() => {
                let points = kml_coordinates(&text.decode()?);
                match geometry {
                    Some(Geometry::Point) => {
                        if let Some(pos) = points.first().copied() {
                            file.items.push(item(PlaceKind::Icon {
                                color: LINE,
                                pos,
                                angle: 0.0,
                                sheet: None,
                                hover: label.clone(),
                            }));
                        }
                    }
                    Some(Geometry::Line) if points.len() >= 2 => {
                        file.items.push(item(PlaceKind::Line {
                            color: LINE,
                            width: 2.0,
                            pts: points,
                        }));
                    }
                    Some(Geometry::Ring) if points.len() >= 3 => rings.push(points),
                    _ => {}
                }
            }
            Event::End(end) => match end.name().local_name().as_ref() {
                b"name" => reading_name = false,
                b"Point" | b"LineString" | b"LinearRing" => geometry = None,
                b"Polygon" if !rings.is_empty() => file.items.push(item(PlaceKind::Polygon {
                    color: FILL,
                    rings: std::mem::take(&mut rings),
                })),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    anyhow::ensure!(!file.items.is_empty(), "KML contains no supported geometry");
    Ok(file)
}

fn kml_coordinates(text: &str) -> Vec<[f64; 2]> {
    text.split_ascii_whitespace()
        .filter_map(|tuple| {
            let mut values = tuple.split(',');
            let lon = values.next()?.parse::<f64>().ok()?;
            let lat = values.next()?.parse::<f64>().ok()?;
            (lon.is_finite()
                && lat.is_finite()
                && (-180.0..=180.0).contains(&lon)
                && (-90.0..=90.0).contains(&lat))
            .then_some([lon, lat])
        })
        .collect()
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

    #[test]
    fn imports_kml_points_lines_and_polygon_rings() {
        let text = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
          <Placemark><name>Point</name><Point><coordinates>-97,35,0</coordinates></Point></Placemark>
          <Placemark><LineString><coordinates>-98,34 -97,35</coordinates></LineString></Placemark>
          <Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>-98,34 -96,34 -96,36 -98,34</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark>
        </Document></kml>"#;
        assert_eq!(kml("analysis.kml", text).unwrap().items.len(), 3);
    }
}
