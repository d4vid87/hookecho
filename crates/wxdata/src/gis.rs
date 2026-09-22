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

/// Export the geographic vector subset of an imported overlay as RFC 7946 GeoJSON.
/// Screen-anchored objects and raster/image meshes have no geographic vector equivalent.
pub fn export_geojson(file: &Placefile) -> anyhow::Result<String> {
    let mut features = Vec::new();
    for item in &file.items {
        if item.anchor.is_some() { continue; }
        let (geometry, label) = match &item.kind {
            PlaceKind::Line { pts, .. } if pts.len() >= 2 && pts.iter().all(valid_pos) =>
                (serde_json::json!({"type":"LineString", "coordinates":pts}), None),
            PlaceKind::Polygon { rings, .. } if !rings.is_empty() => {
                let mut closed = Vec::with_capacity(rings.len());
                for ring in rings {
                    anyhow::ensure!(ring.len() >= 3 && ring.iter().all(valid_pos),
                        "overlay polygon has invalid coordinates");
                    let mut ring = ring.clone();
                    if ring.first() != ring.last() { ring.push(ring[0]); }
                    closed.push(ring);
                }
                (serde_json::json!({"type":"Polygon", "coordinates":closed}), None)
            }
            PlaceKind::Icon { pos, hover, .. } if valid_pos(pos) =>
                (serde_json::json!({"type":"Point", "coordinates":pos}), Some(hover)),
            PlaceKind::Text { pos, text, .. } if valid_pos(pos) =>
                (serde_json::json!({"type":"Point", "coordinates":pos}), Some(text)),
            _ => continue,
        };
        features.push(serde_json::json!({
            "type": "Feature", "geometry": geometry,
            "properties": {
                "source": file.title,
                "label": label,
                "valid_from": item.time.map(|(start, _)| start),
                "valid_until": item.time.map(|(_, end)| end),
            }
        }));
    }
    anyhow::ensure!(!features.is_empty(), "overlay has no geographic vectors to export");
    let json = serde_json::to_string(&serde_json::json!({
        "type":"FeatureCollection", "features":features
    }))?;
    anyhow::ensure!(json.len() <= 64 * 1024 * 1024, "GeoJSON export exceeds 64 MB");
    Ok(json)
}

fn valid_pos(pos: &[f64; 2]) -> bool {
    pos[0].is_finite() && pos[1].is_finite()
        && (-180.0..=180.0).contains(&pos[0]) && (-90.0..=90.0).contains(&pos[1])
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
    anyhow::ensure!(text.len() <= 64 * 1024 * 1024, "KML exceeds 64 MB");
    let mut file = Placefile {
        title: name.to_string(),
        ..Default::default()
    };
    for placemark in xml_elements(text, "Placemark") {
        let label = xml_elements(placemark, "name")
            .first()
            .map(|value| xml_text(value))
            .unwrap_or_else(|| name.to_string());
        for point in xml_elements(placemark, "Point") {
            if let Some(pos) = xml_coordinates(point).first().copied() {
                file.items.push(item(PlaceKind::Icon {
                    color: LINE,
                    pos,
                    angle: 0.0,
                    sheet: None,
                    hover: label.clone(),
                }));
            }
        }
        for line in xml_elements(placemark, "LineString") {
            let pts = xml_coordinates(line);
            if pts.len() >= 2 {
                file.items.push(item(PlaceKind::Line {
                    color: LINE,
                    width: 2.0,
                    pts,
                }));
            }
        }
        for polygon in xml_elements(placemark, "Polygon") {
            let rings: Vec<_> = xml_elements(polygon, "LinearRing")
                .into_iter()
                .map(xml_coordinates)
                .filter(|ring| ring.len() >= 3)
                .collect();
            if !rings.is_empty() {
                file.items
                    .push(item(PlaceKind::Polygon { color: FILL, rings }));
            }
        }
    }
    anyhow::ensure!(!file.items.is_empty(), "KML contains no supported geometry");
    Ok(file)
}

fn xml_elements<'a>(text: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = text;
    let mut out = Vec::new();
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(body_at) = after.find('>') else {
            break;
        };
        let body = &after[body_at + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(&body[..end]);
        rest = &body[end + close.len()..];
    }
    out
}

fn xml_text(text: &str) -> String {
    text.trim()
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn xml_coordinates(text: &str) -> Vec<[f64; 2]> {
    xml_elements(text, "coordinates")
        .into_iter()
        .flat_map(kml_coordinates)
        .collect()
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

/// Read a KMZ or zipped Shapefile. Archives are bounded before decompression and Shapefiles must
/// declare WGS84 in a `.prj`; silently assuming a projected layer is lon/lat is worse than failing.
pub fn archive(name: &str, bytes: &[u8]) -> anyhow::Result<Placefile> {
    let files = zip_entries(bytes)?;
    if let Some((entry, content)) = files.iter().find(|(entry, _)| entry.ends_with(".kml")) {
        return kml(entry, std::str::from_utf8(content)?);
    }
    let (shp_name, shp) = files
        .iter()
        .find(|(entry, _)| entry.ends_with(".shp"))
        .ok_or_else(|| anyhow::anyhow!("GIS archive contains no .kml or .shp file"))?;
    let stem = shp_name.strip_suffix(".shp").unwrap_or(shp_name);
    let projection = files
        .iter()
        .find(|(entry, _)| entry == &format!("{stem}.prj"))
        .map(|(_, content)| String::from_utf8_lossy(content).to_ascii_uppercase())
        .ok_or_else(|| {
            anyhow::anyhow!("Shapefile archive is missing its matching .prj CRS file")
        })?;
    anyhow::ensure!(
        projection.contains("WGS_1984")
            || projection.contains("WGS 84")
            || projection.contains("EPSG\",4326"),
        "unsupported Shapefile CRS; convert the layer to WGS84 (EPSG:4326)"
    );
    shapefile(name, shp)
}

fn zip_entries(bytes: &[u8]) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    use std::io::Read;
    const MAX_UNPACKED: usize = 64 * 1024 * 1024;

    let eocd = bytes
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .ok_or_else(|| anyhow::anyhow!("invalid ZIP archive"))?;
    let count = le_u16(bytes, eocd + 10)? as usize;
    anyhow::ensure!(count <= 128, "GIS archive has too many files");
    let mut offset = le_u32(bytes, eocd + 16)? as usize;
    let mut total = 0usize;
    let mut out = Vec::new();
    for _ in 0..count {
        anyhow::ensure!(
            le_u32(bytes, offset)? == 0x0201_4b50,
            "invalid ZIP directory"
        );
        let flags = le_u16(bytes, offset + 8)?;
        anyhow::ensure!(flags & 1 == 0, "encrypted GIS archives are not supported");
        let method = le_u16(bytes, offset + 10)?;
        let compressed = le_u32(bytes, offset + 20)? as usize;
        let unpacked = le_u32(bytes, offset + 24)? as usize;
        let name_len = le_u16(bytes, offset + 28)? as usize;
        let extra_len = le_u16(bytes, offset + 30)? as usize;
        let comment_len = le_u16(bytes, offset + 32)? as usize;
        let local = le_u32(bytes, offset + 42)? as usize;
        let name_bytes = slice(bytes, offset + 46, name_len)?;
        let name = std::str::from_utf8(name_bytes)?.to_ascii_lowercase();
        let local_name = le_u16(bytes, local + 26)? as usize;
        let local_extra = le_u16(bytes, local + 28)? as usize;
        let data_at = local
            .checked_add(30 + local_name + local_extra)
            .ok_or_else(|| anyhow::anyhow!("ZIP entry is too large"))?;
        let data = slice(bytes, data_at, compressed)?;
        total = total
            .checked_add(unpacked)
            .ok_or_else(|| anyhow::anyhow!("GIS archive is too large"))?;
        anyhow::ensure!(total <= MAX_UNPACKED, "GIS archive exceeds 64 MB unpacked");
        if !name.ends_with('/') {
            let content = match method {
                0 => data.to_vec(),
                8 => {
                    let mut decoded = Vec::with_capacity(unpacked);
                    flate2::read::DeflateDecoder::new(data).read_to_end(&mut decoded)?;
                    decoded
                }
                _ => anyhow::bail!("unsupported ZIP compression method {method}"),
            };
            anyhow::ensure!(content.len() == unpacked, "ZIP entry size mismatch");
            out.push((name, content));
        }
        offset = offset
            .checked_add(46 + name_len + extra_len + comment_len)
            .ok_or_else(|| anyhow::anyhow!("ZIP directory is too large"))?;
    }
    Ok(out)
}

fn slice(bytes: &[u8], offset: usize, length: usize) -> anyhow::Result<&[u8]> {
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| anyhow::anyhow!("ZIP entry is truncated"))?;
    Ok(&bytes[offset..end])
}

fn le_u16(bytes: &[u8], offset: usize) -> anyhow::Result<u16> {
    let value = slice(bytes, offset, 2)?;
    Ok(u16::from_le_bytes(value.try_into().expect("two bytes")))
}

fn shapefile(name: &str, bytes: &[u8]) -> anyhow::Result<Placefile> {
    anyhow::ensure!(bytes.len() >= 100, "Shapefile header is truncated");
    anyhow::ensure!(be_u32(bytes, 0)? == 9994, "invalid Shapefile header");
    let mut file = Placefile {
        title: name.to_string(),
        ..Default::default()
    };
    let mut offset = 100usize;
    let mut features = 0usize;
    while offset < bytes.len() {
        anyhow::ensure!(
            offset + 8 <= bytes.len(),
            "Shapefile record header is truncated"
        );
        let words = be_u32(bytes, offset + 4)? as usize;
        let length = words
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("Shapefile record is too large"))?;
        offset += 8;
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| anyhow::anyhow!("Shapefile record is truncated"))?;
        parse_shape(&bytes[offset..end], name, &mut file.items)?;
        offset = end;
        features += 1;
        anyhow::ensure!(features <= 100_000, "Shapefile exceeds 100,000 features");
    }
    anyhow::ensure!(
        !file.items.is_empty(),
        "Shapefile contains no supported geometry"
    );
    Ok(file)
}

fn parse_shape(bytes: &[u8], label: &str, items: &mut Vec<PlaceItem>) -> anyhow::Result<()> {
    let kind = le_u32(bytes, 0)?;
    match kind {
        0 | 31 => {}
        1 | 11 | 21 => push_shp_point(items, shp_point(bytes, 4)?, label),
        3 | 5 | 13 | 15 | 23 | 25 => {
            anyhow::ensure!(bytes.len() >= 44, "Shapefile path is truncated");
            let parts = le_u32(bytes, 36)? as usize;
            let count = le_u32(bytes, 40)? as usize;
            anyhow::ensure!(
                parts <= 100_000 && count <= 4_000_000,
                "Shapefile geometry exceeds limits"
            );
            let part_start = 44usize;
            let point_start = part_start
                .checked_add(parts * 4)
                .ok_or_else(|| anyhow::anyhow!("Shapefile path is too large"))?;
            let point_end = count
                .checked_mul(16)
                .and_then(|length| point_start.checked_add(length))
                .ok_or_else(|| anyhow::anyhow!("Shapefile path is too large"))?;
            anyhow::ensure!(point_end <= bytes.len(), "Shapefile path is truncated");
            let polygon = kind == 5 || kind == 15 || kind == 25;
            let mut rings = Vec::new();
            for part in 0..parts {
                let first = le_u32(bytes, part_start + part * 4)? as usize;
                let last = if part + 1 < parts {
                    le_u32(bytes, part_start + (part + 1) * 4)? as usize
                } else {
                    count
                };
                anyhow::ensure!(
                    first <= last && last <= count,
                    "invalid Shapefile part index"
                );
                let points: Vec<_> = (first..last)
                    .filter_map(|index| shp_point(bytes, point_start + index * 16).ok())
                    .collect();
                if polygon {
                    if points.len() >= 3 {
                        rings.push(points);
                    }
                } else if points.len() >= 2 {
                    items.push(item(PlaceKind::Line {
                        color: LINE,
                        width: 2.0,
                        pts: points,
                    }));
                }
            }
            if !rings.is_empty() {
                items.push(item(PlaceKind::Polygon { color: FILL, rings }));
            }
        }
        8 | 18 | 28 => {
            anyhow::ensure!(bytes.len() >= 40, "Shapefile multipoint is truncated");
            let count = le_u32(bytes, 36)? as usize;
            anyhow::ensure!(count <= 4_000_000, "Shapefile geometry exceeds limits");
            let point_end = count
                .checked_mul(16)
                .and_then(|length| 40usize.checked_add(length))
                .ok_or_else(|| anyhow::anyhow!("Shapefile multipoint is too large"))?;
            anyhow::ensure!(
                point_end <= bytes.len(),
                "Shapefile multipoint is truncated"
            );
            for index in 0..count {
                if let Ok(point) = shp_point(bytes, 40 + index * 16) {
                    push_shp_point(items, point, label);
                }
            }
        }
        other => anyhow::bail!("unsupported Shapefile shape type {other}"),
    }
    Ok(())
}

fn push_shp_point(items: &mut Vec<PlaceItem>, pos: [f64; 2], label: &str) {
    items.push(item(PlaceKind::Icon {
        color: LINE,
        pos,
        angle: 0.0,
        sheet: None,
        hover: label.to_string(),
    }));
}

fn shp_point(bytes: &[u8], offset: usize) -> anyhow::Result<[f64; 2]> {
    let lon = le_f64(bytes, offset)?;
    let lat = le_f64(bytes, offset + 8)?;
    anyhow::ensure!(
        lon.is_finite()
            && lat.is_finite()
            && (-180.0..=180.0).contains(&lon)
            && (-90.0..=90.0).contains(&lat),
        "Shapefile coordinate is outside WGS84 lon/lat bounds"
    );
    Ok([lon, lat])
}

fn le_u32(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("Shapefile value is truncated"))?;
    Ok(u32::from_le_bytes(value.try_into().expect("four bytes")))
}

fn be_u32(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("Shapefile value is truncated"))?;
    Ok(u32::from_be_bytes(value.try_into().expect("four bytes")))
}

fn le_f64(bytes: &[u8], offset: usize) -> anyhow::Result<f64> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| anyhow::anyhow!("Shapefile value is truncated"))?;
    Ok(f64::from_le_bytes(value.try_into().expect("eight bytes")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_imported_vectors_with_holes_labels_and_wgs84_coordinates() {
        let source = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","properties":{"name":"Storm area"},"geometry":{"type":"Polygon","coordinates":[[[-98,34],[-96,34],[-96,36],[-98,34]],[[-97.5,34.5],[-97,34.5],[-97,35],[-97.5,34.5]]]}},
          {"type":"Feature","properties":{"name":"Track"},"geometry":{"type":"LineString","coordinates":[[-98,34],[-97,35]]}},
          {"type":"Feature","properties":{"name":"Site"},"geometry":{"type":"Point","coordinates":[-97,35]}}
        ]}"#;
        let file = geojson("analyst.geojson", source).unwrap();
        let exported = export_geojson(&file).unwrap();
        let json: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(json["features"].as_array().unwrap().len(), 3);
        assert_eq!(json["features"][0]["geometry"]["coordinates"].as_array().unwrap().len(), 2);
        assert_eq!(json["features"][2]["properties"]["label"], "Site");
        assert_eq!(geojson("roundtrip.geojson", &exported).unwrap().items.len(), 3);
    }

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

    fn test_zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        use std::io::Write;
        fn u16(out: &mut Vec<u8>, value: u16) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        fn u32(out: &mut Vec<u8>, value: u32) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, content) in entries {
            let compressed = if deflate {
                let mut encoder =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(content).unwrap();
                encoder.finish().unwrap()
            } else {
                content.to_vec()
            };
            let method = if deflate { 8 } else { 0 };
            let local = out.len() as u32;
            u32(&mut out, 0x0403_4b50);
            u16(&mut out, 20);
            u16(&mut out, 0);
            u16(&mut out, method);
            u16(&mut out, 0);
            u16(&mut out, 0);
            u32(&mut out, 0);
            u32(&mut out, compressed.len() as u32);
            u32(&mut out, content.len() as u32);
            u16(&mut out, name.len() as u16);
            u16(&mut out, 0);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&compressed);

            u32(&mut central, 0x0201_4b50);
            u16(&mut central, 20);
            u16(&mut central, 20);
            u16(&mut central, 0);
            u16(&mut central, method);
            u16(&mut central, 0);
            u16(&mut central, 0);
            u32(&mut central, 0);
            u32(&mut central, compressed.len() as u32);
            u32(&mut central, content.len() as u32);
            u16(&mut central, name.len() as u16);
            u16(&mut central, 0);
            u16(&mut central, 0);
            u16(&mut central, 0);
            u16(&mut central, 0);
            u32(&mut central, 0);
            u32(&mut central, local);
            central.extend_from_slice(name.as_bytes());
        }
        let central_at = out.len() as u32;
        out.extend_from_slice(&central);
        u32(&mut out, 0x0605_4b50);
        u16(&mut out, 0);
        u16(&mut out, 0);
        u16(&mut out, entries.len() as u16);
        u16(&mut out, entries.len() as u16);
        u32(&mut out, central.len() as u32);
        u32(&mut out, central_at);
        u16(&mut out, 0);
        out
    }

    #[test]
    fn imports_kmz_and_requires_a_shapefile_projection() {
        let bytes = test_zip(&[(
            "doc.kml",
            b"<kml><Placemark><Point><coordinates>-97,35</coordinates></Point></Placemark></kml>",
        )], true);
        assert_eq!(archive("case.kmz", &bytes).unwrap().items.len(), 1);

        let bytes = test_zip(&[("layer.shp", b"not a shapefile")], false);
        assert!(archive("layer.zip", &bytes)
            .unwrap_err()
            .to_string()
            .contains("missing its matching .prj"));

        let mut shp = vec![0u8; 128];
        shp[0..4].copy_from_slice(&9994u32.to_be_bytes());
        shp[24..28].copy_from_slice(&64u32.to_be_bytes());
        shp[28..32].copy_from_slice(&1000u32.to_le_bytes());
        shp[32..36].copy_from_slice(&1u32.to_le_bytes());
        shp[100..104].copy_from_slice(&1u32.to_be_bytes());
        shp[104..108].copy_from_slice(&10u32.to_be_bytes());
        shp[108..112].copy_from_slice(&1u32.to_le_bytes());
        shp[112..120].copy_from_slice(&(-97.0f64).to_le_bytes());
        shp[120..128].copy_from_slice(&(35.0f64).to_le_bytes());
        let bytes = test_zip(
            &[
                ("layer.shp", shp.as_slice()),
                ("layer.prj", b"GEOGCS[\"WGS_1984\"]"),
            ],
            false,
        );
        assert_eq!(archive("layer.zip", &bytes).unwrap().items.len(), 1);
    }
}
