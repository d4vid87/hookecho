//! Geographic overlay features (NWS alerts, SPC outlooks/MDs/watches).
//!
//! All layers decode into a common [`GeoFeature`] — lon/lat polygon rings plus fill/stroke
//! colors and click-through text — so the renderer and hit-tester treat them uniformly.

use geojson::{GeoJson, GeometryValue};

/// What kind of feature this is; also its click-priority tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureKind {
    Warning,
    Watch,
    WatchBox,
    Statement,
    Advisory,
    MesoDiscussion,
    Outlook,
    ProbSevere,
}

impl FeatureKind {
    /// Hit-test priority: a click returns the highest-priority feature under the cursor
    /// (a warning beats the outlook it sits inside).
    pub fn z(self) -> u8 {
        match self {
            FeatureKind::Warning => 6,
            FeatureKind::Statement => 5,
            FeatureKind::Advisory => 4,
            FeatureKind::Watch => 3,
            FeatureKind::WatchBox => 3,
            FeatureKind::ProbSevere => 5,
            FeatureKind::MesoDiscussion => 2,
            FeatureKind::Outlook => 1,
        }
    }
}

/// Structured NWS alert metadata for the warning window (parsed from the alert `parameters`).
/// `None` on non-alert features (SPC outlooks, mesoscale discussions).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertInfo {
    /// Stable alert id — used to dedupe the one-GeoFeature-per-MultiPolygon-part case.
    pub id: String,
    pub event: String,
    pub headline: String,
    pub area: String,
    pub description: String,
    pub instruction: String,
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
    pub max_hail_in: Option<f32>,
    /// Raw wind string as issued, e.g. "60 MPH".
    pub max_wind: Option<String>,
    pub tornado_detection: Option<String>,
    pub damage_threat: Option<String>,
    /// The "SOURCE..." line, else "Radar indicated".
    pub source: Option<String>,
}

/// One renderable, clickable overlay polygon.
#[derive(Debug, Clone)]
pub struct GeoFeature {
    /// Rings in `[lon, lat]`; ring 0 is the outer boundary, any others are holes.
    pub rings: Vec<Vec<[f64; 2]>>,
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub kind: FeatureKind,
    /// Short label for lists/legend, e.g. "Tornado Warning" or "SLGT".
    pub title: String,
    /// Full text shown in the detail window on click.
    pub detail: String,
    /// Structured NWS alert metadata (warnings/watches); `None` for SPC layers.
    pub alert: Option<AlertInfo>,
}

impl GeoFeature {
    /// Bounding box of the outer rings as `(min_lon, min_lat, max_lon, max_lat)`, or `None` when
    /// the feature has no vertices.
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        for ring in &self.rings {
            for p in ring {
                b = Some(match b {
                    None => (p[0], p[1], p[0], p[1]),
                    Some((x0, y0, x1, y1)) => (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])),
                });
            }
        }
        b
    }

    /// Is `(lon, lat)` inside this feature's outer ring (minus holes)?
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        let Some(outer) = self.rings.first() else { return false };
        if !point_in_ring(outer, lon, lat) {
            return false;
        }
        !self.rings[1..].iter().any(|hole| point_in_ring(hole, lon, lat))
    }
}

/// The highest-priority feature containing `(lon, lat)`, if any.
pub fn hit<'a>(features: &'a [GeoFeature], lon: f64, lat: f64) -> Option<&'a GeoFeature> {
    hit_all(features, lon, lat).into_iter().next()
}

/// All features containing `(lon, lat)`, highest click-priority first.
pub fn hit_all<'a>(features: &'a [GeoFeature], lon: f64, lat: f64) -> Vec<&'a GeoFeature> {
    let mut hits: Vec<&GeoFeature> = features.iter().filter(|f| f.contains(lon, lat)).collect();
    hits.sort_by(|a, b| b.kind.z().cmp(&a.kind.z()));
    hits
}

/// Even-odd point-in-polygon test on a `[lon, lat]` ring.
fn point_in_ring(ring: &[[f64; 2]], lon: f64, lat: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0], ring[i][1]);
        let (xj, yj) = (ring[j][0], ring[j][1]);
        if (yi > lat) != (yj > lat) {
            let x_cross = (xj - xi) * (lat - yi) / (yj - yi) + xi;
            if lon < x_cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Extract polygon rings from a GeoJSON geometry value (Polygon or MultiPolygon).
///
/// Returns one `Vec<ring>` group per polygon so multipolygon parts stay independent.
pub fn polygons_of(value: &GeometryValue) -> Vec<Vec<Vec<[f64; 2]>>> {
    fn ring(r: &[geojson::Position]) -> Vec<[f64; 2]> {
        r.iter()
            .filter_map(|p| {
                let s = p.as_slice();
                (s.len() >= 2).then(|| [s[0], s[1]])
            })
            .collect()
    }
    fn poly(p: &[Vec<geojson::Position>]) -> Vec<Vec<[f64; 2]>> {
        p.iter().map(|r| ring(r)).collect()
    }
    match value {
        GeometryValue::Polygon { coordinates } => vec![poly(coordinates)],
        GeometryValue::MultiPolygon { coordinates } => coordinates.iter().map(|p| poly(p)).collect(),
        _ => Vec::new(),
    }
}

/// Iterate the features of a GeoJSON document, yielding (geometry value, properties).
pub fn for_each_feature<F>(json: &str, mut f: F) -> anyhow::Result<()>
where
    F: FnMut(&GeometryValue, &serde_json::Map<String, serde_json::Value>),
{
    let gj: GeoJson = json.parse().map_err(|e| anyhow::anyhow!("geojson parse: {e}"))?;
    let empty = serde_json::Map::new();
    match gj {
        GeoJson::FeatureCollection(fc) => {
            for feat in &fc.features {
                if let Some(geom) = &feat.geometry {
                    let props = feat.properties.as_ref().unwrap_or(&empty);
                    f(&geom.value, props);
                }
            }
        }
        GeoJson::Feature(feat) => {
            if let Some(geom) = &feat.geometry {
                let props = feat.properties.as_ref().unwrap_or(&empty);
                f(&geom.value, props);
            }
        }
        GeoJson::Geometry(geom) => f(&geom.value, &empty),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(kind: FeatureKind) -> GeoFeature {
        GeoFeature {
            rings: vec![vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]]],
            fill: [0, 0, 0, 0],
            stroke: [0, 0, 0, 0],
            kind,
            title: String::new(),
            detail: String::new(),
            alert: None,
        }
    }

    #[test]
    fn point_in_polygon_and_priority() {
        let outlook = square(FeatureKind::Outlook);
        let warning = square(FeatureKind::Warning);
        assert!(outlook.contains(1.0, 1.0));
        assert!(!outlook.contains(3.0, 1.0));
        // Overlapping features: the warning wins the click.
        let feats = vec![outlook, warning];
        assert_eq!(hit(&feats, 1.0, 1.0).unwrap().kind, FeatureKind::Warning);
        assert!(hit(&feats, 5.0, 5.0).is_none());
    }

    #[test]
    fn parses_geojson_polygon() {
        let json = r#"{"type":"Feature","geometry":{"type":"Polygon",
            "coordinates":[[[-97.5,35.0],[-97.0,35.0],[-97.0,35.5],[-97.5,35.0]]]},
            "properties":{"event":"Tornado Warning"}}"#;
        let mut count = 0;
        for_each_feature(json, |geom, props| {
            count += 1;
            let polys = polygons_of(geom);
            assert_eq!(polys.len(), 1);
            assert_eq!(polys[0][0].len(), 4);
            assert_eq!(props.get("event").and_then(|v| v.as_str()), Some("Tornado Warning"));
        })
        .unwrap();
        assert_eq!(count, 1);
    }
}
