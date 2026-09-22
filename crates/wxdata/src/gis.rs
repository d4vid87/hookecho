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
        if item.anchor.is_some() {
            continue;
        }
        let (geometry, label) = match &item.kind {
            PlaceKind::Line { pts, .. } if pts.len() >= 2 && pts.iter().all(valid_pos) => (
                serde_json::json!({"type":"LineString", "coordinates":pts}),
                None,
            ),
            PlaceKind::Polygon { rings, .. } if !rings.is_empty() => {
                let mut closed = Vec::with_capacity(rings.len());
                for ring in rings {
                    anyhow::ensure!(
                        ring.len() >= 3 && ring.iter().all(valid_pos),
                        "overlay polygon has invalid coordinates"
                    );
                    let mut ring = ring.clone();
                    if ring.first() != ring.last() {
                        ring.push(ring[0]);
                    }
                    closed.push(ring);
                }
                (
                    serde_json::json!({"type":"Polygon", "coordinates":closed}),
                    None,
                )
            }
            PlaceKind::Icon { pos, hover, .. } if valid_pos(pos) => (
                serde_json::json!({"type":"Point", "coordinates":pos}),
                Some(hover),
            ),
            PlaceKind::Text { pos, text, .. } if valid_pos(pos) => (
                serde_json::json!({"type":"Point", "coordinates":pos}),
                Some(text),
            ),
            _ => continue,
        };
        let mut properties = item.properties.as_deref().cloned().unwrap_or_default();
        properties
            .entry("source")
            .or_insert_with(|| file.title.clone().into());
        if let Some(label) = label {
            properties
                .entry("label")
                .or_insert_with(|| label.clone().into());
        }
        if let Some((start, end)) = item.time {
            properties
                .entry("valid_from")
                .or_insert_with(|| start.to_rfc3339().into());
            properties
                .entry("valid_until")
                .or_insert_with(|| end.to_rfc3339().into());
        }
        features.push(serde_json::json!({
            "type": "Feature", "geometry": geometry, "properties": properties
        }));
    }
    anyhow::ensure!(
        !features.is_empty(),
        "overlay has no geographic vectors to export"
    );
    let json = serde_json::to_string(&serde_json::json!({
        "type":"FeatureCollection", "features":features
    }))?;
    anyhow::ensure!(
        json.len() <= 64 * 1024 * 1024,
        "GeoJSON export exceeds 64 MB"
    );
    Ok(json)
}

fn valid_pos(pos: &[f64; 2]) -> bool {
    pos[0].is_finite()
        && pos[1].is_finite()
        && (-180.0..=180.0).contains(&pos[0])
        && (-90.0..=90.0).contains(&pos[1])
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
            let properties = feature.properties.unwrap_or_default();
            let label = feature_label(Some(&properties)).unwrap_or(name).to_string();
            if let Some(geometry) = feature.geometry {
                append_properties(&mut file.items, geometry.value, &label, properties);
            }
        }
        GeoJson::FeatureCollection(collection) => {
            for feature in collection.features {
                let properties = feature.properties.unwrap_or_default();
                let label = feature_label(Some(&properties)).unwrap_or(name).to_string();
                if let Some(geometry) = feature.geometry {
                    append_properties(&mut file.items, geometry.value, &label, properties);
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
        properties: None,
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

fn append_properties(
    items: &mut Vec<PlaceItem>,
    geometry: GeometryValue,
    label: &str,
    properties: serde_json::Map<String, serde_json::Value>,
) {
    let first = items.len();
    append(items, geometry, label);
    let properties = std::sync::Arc::new(properties);
    for item in &mut items[first..] {
        item.properties = Some(properties.clone());
    }
}

/// Read a KMZ or zipped Shapefile. Archives are bounded before decompression and Shapefiles must
/// declare a supported CRS in a matching `.prj`.
pub fn archive(name: &str, bytes: &[u8]) -> anyhow::Result<Placefile> {
    archive_with_options(name, bytes, GisImportOptions::default()).map(|(file, _)| file)
}

/// DBF columns selected for a zipped Shapefile. Time filtering applies when both bounds are set.
#[derive(Default, Clone, Copy)]
pub struct GisImportOptions<'a> {
    pub label_field: Option<&'a str>,
    pub color_field: Option<&'a str>,
    pub valid_start_field: Option<&'a str>,
    pub valid_end_field: Option<&'a str>,
}

/// Import a zipped Shapefile with optional DBF-backed styling and validity. The returned names
/// are the available DBF columns for the layer controls.
pub fn archive_with_options(
    name: &str,
    bytes: &[u8],
    options: GisImportOptions<'_>,
) -> anyhow::Result<(Placefile, Vec<String>)> {
    let files = zip_entries(bytes)?;
    if let Some((entry, content)) = files.iter().find(|(entry, _)| entry.ends_with(".kml")) {
        return Ok((kml(entry, std::str::from_utf8(content)?)?, Vec::new()));
    }
    let (shp_name, shp) = files
        .iter()
        .find(|(entry, _)| entry.ends_with(".shp"))
        .ok_or_else(|| anyhow::anyhow!("GIS archive contains no .kml or .shp file"))?;
    let stem = shp_name.strip_suffix(".shp").unwrap_or(shp_name);
    let projection = files
        .iter()
        .find(|(entry, _)| entry == &format!("{stem}.prj"))
        .map(|(_, content)| String::from_utf8_lossy(content))
        .ok_or_else(|| {
            anyhow::anyhow!("Shapefile archive is missing its matching .prj CRS file")
        })?;
    let dbf = files
        .iter()
        .find(|(entry, _)| entry == &format!("{stem}.dbf"))
        .map(|(_, content)| DbfTable::new(content))
        .transpose()?;
    let fields = dbf.as_ref().map_or_else(Vec::new, |table| {
        table
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect()
    });
    let style = dbf
        .as_ref()
        .map(|table| DbfStyle::new(table, options))
        .transpose()?;
    Ok((
        shapefile(
            name,
            shp,
            &CrsTransform::from_wkt(&projection)?,
            dbf.as_ref(),
            style.as_ref(),
        )?,
        fields,
    ))
}

struct DbfField {
    name: String,
    kind: u8,
    offset: usize,
    len: usize,
}

struct DbfTable<'a> {
    bytes: &'a [u8],
    fields: Vec<DbfField>,
    count: usize,
    header: usize,
    record: usize,
}

impl<'a> DbfTable<'a> {
    fn new(bytes: &'a [u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(bytes.len() >= 33, "DBF header is truncated");
        let count = le_u32(bytes, 4)? as usize;
        let header = le_u16(bytes, 8)? as usize;
        let record = le_u16(bytes, 10)? as usize;
        anyhow::ensure!(
            count <= 100_000 && header >= 33 && record > 0 && header <= bytes.len(),
            "invalid DBF dimensions"
        );
        anyhow::ensure!(
            count
                .checked_mul(record)
                .and_then(|size| header.checked_add(size))
                .is_some_and(|end| end <= bytes.len()),
            "DBF records are truncated"
        );
        let mut fields = Vec::new();
        let mut offset = 1usize;
        let mut at = 32usize;
        while at < header && bytes[at] != 0x0d {
            anyhow::ensure!(at + 32 <= header, "DBF field descriptor is truncated");
            let descriptor = &bytes[at..at + 32];
            let name_end = descriptor[..11]
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(11);
            let name = String::from_utf8_lossy(&descriptor[..name_end])
                .trim()
                .to_string();
            let len = descriptor[16] as usize;
            anyhow::ensure!(
                !name.is_empty() && len > 0 && offset + len <= record,
                "invalid DBF field"
            );
            fields.push(DbfField {
                name,
                kind: descriptor[11],
                offset,
                len,
            });
            offset += len;
            at += 32;
        }
        anyhow::ensure!(
            at < header && bytes[at] == 0x0d,
            "DBF field list is unterminated"
        );
        Ok(Self {
            bytes,
            fields,
            count,
            header,
            record,
        })
    }

    fn value(&self, row: usize, field: usize) -> std::borrow::Cow<'_, str> {
        let column = &self.fields[field];
        let at = self.header + row * self.record + column.offset;
        String::from_utf8_lossy(self.bytes[at..at + column.len].trim_ascii())
    }

    fn deleted(&self, row: usize) -> bool {
        self.bytes[self.header + row * self.record] == b'*'
    }

    fn field(&self, name: &str) -> anyhow::Result<usize> {
        self.fields
            .iter()
            .position(|field| field.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| anyhow::anyhow!("DBF has no attribute named {name}"))
    }

    fn properties(&self, row: usize) -> serde_json::Map<String, serde_json::Value> {
        self.fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| {
                let value = self.value(row, index);
                if value.is_empty() {
                    return None;
                }
                let value = match field.kind {
                    b'N' | b'F' => value
                        .parse::<f64>()
                        .ok()
                        .and_then(serde_json::Number::from_f64)
                        .map_or_else(
                            || serde_json::Value::String(value.into_owned()),
                            serde_json::Value::Number,
                        ),
                    b'L' => match value.as_bytes().first().map(u8::to_ascii_uppercase) {
                        Some(b'T' | b'Y') => serde_json::Value::Bool(true),
                        Some(b'F' | b'N') => serde_json::Value::Bool(false),
                        _ => serde_json::Value::String(value.into_owned()),
                    },
                    _ => serde_json::Value::String(value.into_owned()),
                };
                Some((field.name.clone(), value))
            })
            .collect()
    }
}

struct DbfStyle {
    label: Option<usize>,
    color: Option<usize>,
    numeric: Option<(f64, f64)>,
    valid_start: Option<usize>,
    valid_end: Option<usize>,
}

impl DbfStyle {
    fn new(table: &DbfTable<'_>, options: GisImportOptions<'_>) -> anyhow::Result<Self> {
        let label = match options.label_field {
            Some(name) => Some(table.field(name)?),
            None => table.fields.iter().position(|field| {
                ["name", "title", "label"]
                    .iter()
                    .any(|candidate| field.name.eq_ignore_ascii_case(candidate))
            }),
        };
        let color = options
            .color_field
            .map(|name| table.field(name))
            .transpose()?;
        let valid_start = options
            .valid_start_field
            .map(|name| table.field(name))
            .transpose()?;
        let valid_end = options
            .valid_end_field
            .map(|name| table.field(name))
            .transpose()?;
        let numeric = color
            .filter(|field| matches!(table.fields[*field].kind, b'N' | b'F'))
            .and_then(|field| {
                (0..table.count)
                    .filter(|row| !table.deleted(*row))
                    .filter_map(|row| {
                        table
                            .value(row, field)
                            .parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite())
                    })
                    .fold(None, |range: Option<(f64, f64)>, value| {
                        Some(range.map_or((value, value), |(min, max)| {
                            (min.min(value), max.max(value))
                        }))
                    })
            });
        Ok(Self {
            label,
            color,
            numeric,
            valid_start,
            valid_end,
        })
    }

    fn time(
        &self,
        table: &DbfTable<'_>,
        row: usize,
    ) -> anyhow::Result<Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>>
    {
        let (Some(start), Some(end)) = (self.valid_start, self.valid_end) else {
            return Ok(None);
        };
        let start = dbf_time(&table.value(row, start), false)?;
        let end = dbf_time(&table.value(row, end), true)?;
        anyhow::ensure!(
            start <= end,
            "DBF validity ends before it starts at record {}",
            row + 1
        );
        Ok(Some((start, end)))
    }

    fn color(&self, table: &DbfTable<'_>, row: usize) -> Option<[u8; 3]> {
        let field = self.color?;
        let value = table.value(row, field);
        if value.is_empty() {
            return None;
        }
        if matches!(table.fields[field].kind, b'N' | b'F') {
            if let (Some((min, max)), Ok(number)) = (self.numeric, value.parse::<f64>()) {
                let fraction = if max > min {
                    ((number - min) / (max - min)).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                return Some([
                    (40.0 + 210.0 * fraction) as u8,
                    (170.0 - 90.0 * fraction) as u8,
                    (235.0 - 190.0 * fraction) as u8,
                ]);
            }
            return None;
        }
        let hex = value.trim_start_matches('#');
        if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Some([
                u8::from_str_radix(&hex[..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..], 16).ok()?,
            ]);
        }
        const COLORS: [[u8; 3]; 6] = [
            [84, 192, 232],
            [250, 181, 83],
            [124, 205, 128],
            [204, 136, 222],
            [236, 111, 106],
            [120, 179, 245],
        ];
        let hash = value.bytes().fold(0u32, |hash, byte| {
            hash.wrapping_mul(16777619) ^ u32::from(byte)
        });
        Some(COLORS[(hash as usize) % COLORS.len()])
    }
}

fn dbf_time(value: &str, end_of_day: bool) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDate, TimeZone, Utc};
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(time.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(value, "%Y%m%d")
        .or_else(|_| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .map_err(|_| {
            anyhow::anyhow!(
                "invalid DBF UTC date/time '{value}'; use YYYYMMDD, YYYY-MM-DD, or RFC3339"
            )
        })?;
    let seconds = if end_of_day { 86_399 } else { 0 };
    Ok(Utc.from_utc_datetime(
        &(date.and_hms_opt(0, 0, 0).unwrap() + chrono::Duration::seconds(seconds)),
    ))
}

enum CrsTransform {
    Wgs84,
    Projected {
        from: Box<proj4rs::Proj>,
        to: Box<proj4rs::Proj>,
    },
}

impl CrsTransform {
    fn from_wkt(wkt: &str) -> anyhow::Result<Self> {
        let upper = wkt.trim().to_ascii_uppercase();
        // Root geographic WGS84 is already lon/lat. Do not mistake the nested GEOGCS in a
        // projected WKT for an unprojected layer.
        if (upper.starts_with("GEOGCS[") || upper.starts_with("GEOGCRS["))
            && (upper.contains("WGS_1984") || upper.contains("WGS 84"))
        {
            return Ok(Self::Wgs84);
        }
        anyhow::ensure!(
            upper.contains("WGS_1984")
                || upper.contains("WGS 84")
                || upper.contains("NAD83")
                || upper.contains("NORTH_AMERICAN_DATUM_1983"),
            "unsupported Shapefile datum; WGS84 or NAD83 is required"
        );
        let definition = proj4wkt::wkt_to_projstring(wkt)
            .map_err(|error| anyhow::anyhow!("unsupported Shapefile CRS: {error}"))?;
        let from = proj4rs::Proj::from_proj_string(&definition)
            .map_err(|error| anyhow::anyhow!("invalid Shapefile projection: {error}"))?;
        let to = proj4rs::Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
            .map_err(|error| anyhow::anyhow!("WGS84 projection unavailable: {error}"))?;
        Ok(Self::Projected {
            from: Box::new(from),
            to: Box::new(to),
        })
    }

    fn point(&self, x: f64, y: f64) -> anyhow::Result<[f64; 2]> {
        anyhow::ensure!(
            x.is_finite() && y.is_finite(),
            "Shapefile coordinate is not finite"
        );
        let pos = match self {
            Self::Wgs84 => [x, y],
            Self::Projected { from, to } => {
                let mut point = if from.is_latlong() {
                    (x.to_radians(), y.to_radians(), 0.0)
                } else {
                    (x, y, 0.0)
                };
                proj4rs::transform::transform(from, to, &mut point).map_err(|error| {
                    anyhow::anyhow!("Shapefile coordinate transformation failed: {error}")
                })?;
                [point.0.to_degrees(), point.1.to_degrees()]
            }
        };
        anyhow::ensure!(
            valid_pos(&pos),
            "Shapefile coordinate is outside WGS84 lon/lat bounds"
        );
        Ok(pos)
    }
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

fn shapefile(
    name: &str,
    bytes: &[u8],
    crs: &CrsTransform,
    dbf: Option<&DbfTable<'_>>,
    style: Option<&DbfStyle>,
) -> anyhow::Result<Placefile> {
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
        anyhow::ensure!(
            dbf.is_none_or(|table| features < table.count),
            "Shapefile has more records than DBF"
        );
        if !dbf.is_some_and(|table| table.deleted(features)) {
            let label = dbf
                .zip(style)
                .and_then(|(table, style)| style.label.map(|field| table.value(features, field)))
                .filter(|value| !value.is_empty());
            let first = file.items.len();
            parse_shape(
                &bytes[offset..end],
                label.as_deref().unwrap_or(name),
                crs,
                &mut file.items,
            )?;
            if let Some((table, style)) = dbf.zip(style) {
                let properties = std::sync::Arc::new(table.properties(features));
                for item in &mut file.items[first..] {
                    item.properties = Some(properties.clone());
                }
                if let Some(time) = style.time(table, features)? {
                    for item in &mut file.items[first..] {
                        item.time = Some(time);
                    }
                }
                if let Some(rgb) = style.color(table, features) {
                    for item in &mut file.items[first..] {
                        match &mut item.kind {
                            PlaceKind::Line { color, .. } | PlaceKind::Icon { color, .. } => {
                                *color = [rgb[0], rgb[1], rgb[2], 255]
                            }
                            PlaceKind::Polygon { color, .. } => {
                                *color = [rgb[0], rgb[1], rgb[2], 72]
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        offset = end;
        features += 1;
        anyhow::ensure!(features <= 100_000, "Shapefile exceeds 100,000 features");
    }
    anyhow::ensure!(
        dbf.is_none_or(|table| features == table.count),
        "Shapefile and DBF record counts differ"
    );
    anyhow::ensure!(
        !file.items.is_empty(),
        "Shapefile contains no supported geometry"
    );
    Ok(file)
}

fn parse_shape(
    bytes: &[u8],
    label: &str,
    crs: &CrsTransform,
    items: &mut Vec<PlaceItem>,
) -> anyhow::Result<()> {
    let kind = le_u32(bytes, 0)?;
    match kind {
        0 | 31 => {}
        1 | 11 | 21 => push_shp_point(items, shp_point(bytes, 4, crs)?, label),
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
                    .map(|index| shp_point(bytes, point_start + index * 16, crs))
                    .collect::<anyhow::Result<_>>()?;
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
                push_shp_point(items, shp_point(bytes, 40 + index * 16, crs)?, label);
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

fn shp_point(bytes: &[u8], offset: usize, crs: &CrsTransform) -> anyhow::Result<[f64; 2]> {
    crs.point(le_f64(bytes, offset)?, le_f64(bytes, offset + 8)?)
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
          {"type":"Feature","properties":{"name":"Storm area","priority":3},"geometry":{"type":"Polygon","coordinates":[[[-98,34],[-96,34],[-96,36],[-98,34]],[[-97.5,34.5],[-97,34.5],[-97,35],[-97.5,34.5]]]}},
          {"type":"Feature","properties":{"name":"Track"},"geometry":{"type":"LineString","coordinates":[[-98,34],[-97,35]]}},
          {"type":"Feature","properties":{"name":"Site"},"geometry":{"type":"Point","coordinates":[-97,35]}}
        ]}"#;
        let file = geojson("analyst.geojson", source).unwrap();
        let exported = export_geojson(&file).unwrap();
        let json: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(json["features"].as_array().unwrap().len(), 3);
        assert_eq!(
            json["features"][0]["geometry"]["coordinates"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(json["features"][2]["properties"]["label"], "Site");
        assert_eq!(json["features"][0]["properties"]["name"], "Storm area");
        assert_eq!(json["features"][0]["properties"]["priority"], 3);
        assert_eq!(json["features"][1]["properties"]["name"], "Track");
        assert_eq!(
            geojson("roundtrip.geojson", &exported).unwrap().items.len(),
            3
        );
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

    #[test]
    fn transforms_projected_shapefile_coordinates_before_import() {
        let wkt = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1]]"#;
        let mut shp = vec![0u8; 128];
        shp[0..4].copy_from_slice(&9994u32.to_be_bytes());
        shp[24..28].copy_from_slice(&64u32.to_be_bytes());
        shp[28..32].copy_from_slice(&1000u32.to_le_bytes());
        shp[32..36].copy_from_slice(&1u32.to_le_bytes());
        shp[100..104].copy_from_slice(&1u32.to_be_bytes());
        shp[104..108].copy_from_slice(&10u32.to_be_bytes());
        shp[108..112].copy_from_slice(&1u32.to_le_bytes());
        shp[112..120].copy_from_slice(&(-10_798_000.0f64).to_le_bytes());
        shp[120..128].copy_from_slice(&(4_139_370.0f64).to_le_bytes());
        let mut dbf = vec![0u8; 117];
        dbf[0] = 3;
        dbf[4..8].copy_from_slice(&1u32.to_le_bytes());
        dbf[8..10].copy_from_slice(&97u16.to_le_bytes());
        dbf[10..12].copy_from_slice(&20u16.to_le_bytes());
        dbf[32..36].copy_from_slice(b"NAME");
        dbf[43] = b'C';
        dbf[48] = 12;
        dbf[64..69].copy_from_slice(b"COLOR");
        dbf[75] = b'C';
        dbf[80] = 7;
        dbf[96] = 0x0d;
        dbf[97] = b' ';
        dbf[98..110].copy_from_slice(b"County test ");
        dbf[110..117].copy_from_slice(b"#ff8000");
        let bytes = test_zip(
            &[
                ("layer.shp", &shp),
                ("layer.prj", wkt.as_bytes()),
                ("layer.dbf", &dbf),
            ],
            false,
        );
        let (file, fields) = archive_with_options(
            "layer.zip",
            &bytes,
            GisImportOptions {
                label_field: Some("NAME"),
                color_field: Some("COLOR"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fields, ["NAME", "COLOR"]);
        let PlaceKind::Icon { color, hover, .. } = &file.items[0].kind else {
            panic!("expected icon")
        };
        assert_eq!(color, &[255, 128, 0, 255]);
        assert_eq!(hover, "County test");
        let exported: serde_json::Value =
            serde_json::from_str(&export_geojson(&file).unwrap()).unwrap();
        let pos = &exported["features"][0]["geometry"]["coordinates"];
        assert_eq!(exported["features"][0]["properties"]["NAME"], "County test");
        assert_eq!(exported["features"][0]["properties"]["COLOR"], "#ff8000");
        let lon = pos[0].as_f64().unwrap();
        let lat = pos[1].as_f64().unwrap();
        assert!((-98.0..-96.0).contains(&lon), "longitude: {lon}");
        assert!((34.0..36.0).contains(&lat), "latitude: {lat}");
        assert!(archive_with_options(
            "layer.zip",
            &bytes,
            GisImportOptions {
                color_field: Some("MISSING"),
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("no attribute"));
        dbf[75] = b'N';
        dbf[110..117].copy_from_slice(b"     42");
        let table = DbfTable::new(&dbf).unwrap();
        assert_eq!(table.properties(0)["COLOR"], 42.0);
        let numeric = test_zip(
            &[
                ("layer.shp", &shp),
                ("layer.prj", wkt.as_bytes()),
                ("layer.dbf", &dbf),
            ],
            false,
        );
        let (file, _) = archive_with_options(
            "layer.zip",
            &numeric,
            GisImportOptions {
                color_field: Some("COLOR"),
                ..Default::default()
            },
        )
        .unwrap();
        let PlaceKind::Icon { color, .. } = &file.items[0].kind else {
            panic!("expected icon")
        };
        assert_eq!(color, &[145, 125, 140, 255]);
        dbf.truncate(115);
        let truncated = test_zip(
            &[
                ("layer.shp", &shp),
                ("layer.prj", wkt.as_bytes()),
                ("layer.dbf", &dbf),
            ],
            false,
        );
        assert!(archive("layer.zip", &truncated)
            .unwrap_err()
            .to_string()
            .contains("DBF records are truncated"));
    }

    #[test]
    fn dbf_date_columns_set_feature_validity_and_reject_reversed_intervals() {
        let mut shp = vec![0u8; 128];
        shp[0..4].copy_from_slice(&9994u32.to_be_bytes());
        shp[100..104].copy_from_slice(&1u32.to_be_bytes());
        shp[104..108].copy_from_slice(&10u32.to_be_bytes());
        shp[108..112].copy_from_slice(&1u32.to_le_bytes());
        shp[112..120].copy_from_slice(&(-97.0f64).to_le_bytes());
        shp[120..128].copy_from_slice(&(35.0f64).to_le_bytes());

        let mut dbf = vec![0u8; 114];
        dbf[4..8].copy_from_slice(&1u32.to_le_bytes());
        dbf[8..10].copy_from_slice(&97u16.to_le_bytes());
        dbf[10..12].copy_from_slice(&17u16.to_le_bytes());
        dbf[32..36].copy_from_slice(b"FROM");
        dbf[43] = b'D';
        dbf[48] = 8;
        dbf[64..69].copy_from_slice(b"UNTIL");
        dbf[75] = b'D';
        dbf[80] = 8;
        dbf[96] = 0x0d;
        dbf[97] = b' ';
        dbf[98..106].copy_from_slice(b"20260917");
        dbf[106..114].copy_from_slice(b"20260919");
        let table = DbfTable::new(&dbf).unwrap();
        let style = DbfStyle::new(
            &table,
            GisImportOptions {
                valid_start_field: Some("FROM"),
                valid_end_field: Some("UNTIL"),
                ..Default::default()
            },
        )
        .unwrap();
        let file = shapefile(
            "case.shp",
            &shp,
            &CrsTransform::Wgs84,
            Some(&table),
            Some(&style),
        )
        .unwrap();
        let (start, end) = file.items[0].time.unwrap();
        assert_eq!(start.to_rfc3339(), "2026-09-17T00:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-09-19T23:59:59+00:00");
        dbf[98..106].copy_from_slice(b"        ");
        let table = DbfTable::new(&dbf).unwrap();
        let style = DbfStyle::new(
            &table,
            GisImportOptions {
                valid_start_field: Some("FROM"),
                valid_end_field: Some("UNTIL"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(shapefile(
            "case.shp",
            &shp,
            &CrsTransform::Wgs84,
            Some(&table),
            Some(&style)
        )
        .unwrap_err()
        .to_string()
        .contains("invalid DBF UTC date/time"));
        dbf[98..106].copy_from_slice(b"20260917");
        dbf[106..114].copy_from_slice(b"20260916");
        let table = DbfTable::new(&dbf).unwrap();
        let style = DbfStyle::new(
            &table,
            GisImportOptions {
                valid_start_field: Some("FROM"),
                valid_end_field: Some("UNTIL"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(shapefile(
            "case.shp",
            &shp,
            &CrsTransform::Wgs84,
            Some(&table),
            Some(&style)
        )
        .unwrap_err()
        .to_string()
        .contains("ends before it starts"));
    }
}
