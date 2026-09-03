//! Geographic overlay features (NWS alerts, SPC outlooks/MDs/watches).
//!
//! All layers decode into a common [`GeoFeature`] — lon/lat polygon rings plus fill/stroke
//! colors and click-through text — so the renderer and hit-tester treat them uniformly.

use geojson::{GeoJson, GeometryValue};

/// What kind of feature this is; also its click-priority tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FeatureKind {
    Warning,
    Watch,
    WatchBox,
    Statement,
    Advisory,
    MesoDiscussion,
    Outlook,
    ProbSevere,
    /// NHC tropical-cyclone forecast cone (feature V).
    TropicalCone,
    /// Aviation SIGMET/AIRMET hazard area (feature GG).
    Sigmet,
    /// FAA Temporary Flight Restriction — airspace that is closed, not weather.
    Tfr,
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
            FeatureKind::TropicalCone => 2,
            FeatureKind::Sigmet => 2,
            // Above the other airspace layers: a TFR is a legal boundary, and if one is under
            // the cursor it is the thing the click was about.
            FeatureKind::Tfr => 3,
            FeatureKind::Outlook => 1,
        }
    }
}

/// Warned-storm motion parsed from the alert's `eventMotionDescription`.
/// Direction is stored *as issued* — the FROM bearing; the heading a storm travels toward is
/// `(deg + 180) % 360`, flipped at draw/ETA time.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StormMotion {
    /// FROM direction in degrees (meteorological, as issued).
    pub deg: f32,
    /// Speed in knots.
    pub kt: f32,
    /// Storm centroid track points as `[lon, lat]` (usually one).
    pub points: Vec<[f64; 2]>,
}

/// Structured NWS alert metadata for the warning window (parsed from the alert `parameters`).
/// `None` on non-alert features (SPC outlooks, mesoscale discussions).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Parsed storm motion (direction/speed/track) from `eventMotionDescription`; not persisted.
    pub motion: Option<StormMotion>,
    /// Raw P-VTEC string, when the product carries one. [`Self::event_key`] is what uses it.
    #[serde(default)]
    pub vtec: Option<String>,
}

impl AlertInfo {
    /// Identity of the *event*, for a seen-set to key on.
    ///
    /// `id` identifies a *message*, not an event. Continuing a tornado warning issues a fresh
    /// message with a fresh id and the same warning underneath, so an id-keyed seen-set
    /// re-announces the same warning every few minutes. The VTEC event key does not move.
    /// Products with no VTEC (most non-warning-tier alerts) fall back to the id, which is the
    /// old behaviour.
    pub fn event_key(&self) -> String {
        self.vtec
            .as_deref()
            .and_then(crate::vtec::Vtec::parse)
            .map(|v| v.event_key())
            .unwrap_or_else(|| self.id.clone())
    }

    /// The string the app dedupes announcements on. Normally one entry per *event*, so a
    /// continuation of a tornado warning is silent; an upgrade earns its own entry (keyed by
    /// message id) so it announces once and only once.
    pub fn dedupe_key(&self) -> String {
        let k = self.event_key();
        if self.is_upgrade() {
            format!("{k}#{}", self.id)
        } else {
            k
        }
    }

    /// Should this message re-announce an event we have already announced? Only an upgrade does.
    pub fn is_upgrade(&self) -> bool {
        self.vtec
            .as_deref()
            .and_then(crate::vtec::Vtec::parse)
            .is_some_and(|v| v.action.is_newsworthy_repeat())
    }
}

/// One renderable, clickable overlay polygon.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
                    Some((x0, y0, x1, y1)) => {
                        (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]))
                    }
                });
            }
        }
        b
    }

    /// Great-circle kilometers from `(lon, lat)` to the nearest point on this feature's boundary,
    /// or 0 when the point is inside it. Used by the watched-location radius: an NWS polygon three
    /// counties wide still matters when its edge is a few miles from your house.
    ///
    /// Distance to each edge is computed by projecting onto the segment in a local flat frame
    /// (longitude scaled by cos(lat)), which is exact enough at the scales this is asked about.
    pub fn distance_km(&self, lon: f64, lat: f64) -> f64 {
        if self.contains(lon, lat) {
            return 0.0;
        }
        const KM_PER_DEG: f64 = 111.194_927;
        let scale = lat.to_radians().cos().max(0.01);
        let to_xy = |p: [f64; 2]| [(p[0] - lon) * scale * KM_PER_DEG, (p[1] - lat) * KM_PER_DEG];
        let mut best = f64::INFINITY;
        for ring in &self.rings {
            for pair in ring.windows(2) {
                let (a, b) = (to_xy(pair[0]), to_xy(pair[1]));
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dy * dy;
                // Clamp the projection to the segment, so endpoints win for a point "past" it.
                let t = if len2 <= f64::EPSILON {
                    0.0
                } else {
                    ((-a[0] * dx - a[1] * dy) / len2).clamp(0.0, 1.0)
                };
                let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
                best = best.min((cx * cx + cy * cy).sqrt());
            }
        }
        best
    }

    /// Is `(lon, lat)` inside this feature's outer ring (minus holes)?
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        let Some(outer) = self.rings.first() else {
            return false;
        };
        if !point_in_ring(outer, lon, lat) {
            return false;
        }
        !self.rings[1..]
            .iter()
            .any(|hole| point_in_ring(hole, lon, lat))
    }
}

/// The highest-priority feature containing `(lon, lat)`, if any.
pub fn hit(features: &[GeoFeature], lon: f64, lat: f64) -> Option<&GeoFeature> {
    hit_all(features, lon, lat).into_iter().next()
}

/// All features containing `(lon, lat)`, highest click-priority first.
pub fn hit_all(features: &[GeoFeature], lon: f64, lat: f64) -> Vec<&GeoFeature> {
    let mut hits: Vec<&GeoFeature> = features.iter().filter(|f| f.contains(lon, lat)).collect();
    hits.sort_by_key(|f| std::cmp::Reverse(f.kind.z()));
    hits
}

/// Do two `[lon, lat]` rings overlap at all — crossing edges, or one wholly inside the other?
///
/// This is what "a warning touches my zone" means: a polygon that merely clips a corner of the
/// zone counts, and so does a warning big enough to swallow it whole.
///
/// ponytail: brute-force O(n·m) segment pairs behind a bbox reject. Warning polygons run to a few
/// dozen vertices and user zones to a handful, so the pair count is trivial; a sweep line is the
/// upgrade if either ever grows.
pub fn rings_intersect(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    let bbox = |r: &[[f64; 2]]| {
        r.iter().fold(
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |(x0, y0, x1, y1), p| (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])),
        )
    };
    let (ax0, ay0, ax1, ay1) = bbox(a);
    let (bx0, by0, bx1, by1) = bbox(b);
    if ax1 < bx0 || bx1 < ax0 || ay1 < by0 || by1 < ay0 {
        return false;
    }
    // Containment either way (no edge crosses when one ring is wholly inside the other).
    if point_in_ring(b, a[0][0], a[0][1]) || point_in_ring(a, b[0][0], b[0][1]) {
        return true;
    }
    // Any pair of edges crossing. Rings are treated as closed, so the last→first edge counts.
    for i in 0..a.len() {
        let (p1, p2) = (a[i], a[(i + 1) % a.len()]);
        for j in 0..b.len() {
            let (q1, q2) = (b[j], b[(j + 1) % b.len()]);
            if segments_cross(p1, p2, q1, q2) {
                return true;
            }
        }
    }
    false
}

/// Sign of the cross product of `(b-a) × (c-a)`: >0 left turn, <0 right turn, 0 collinear.
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Do segments `p1p2` and `q1q2` properly cross or touch?
fn segments_cross(p1: [f64; 2], p2: [f64; 2], q1: [f64; 2], q2: [f64; 2]) -> bool {
    let (d1, d2) = (orient(q1, q2, p1), orient(q1, q2, p2));
    let (d3, d4) = (orient(p1, p2, q1), orient(p1, p2, q2));
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        return true;
    }
    // Collinear touching: an endpoint sitting on the other segment still counts as a touch.
    let on = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        orient(a, b, c) == 0.0
            && c[0] >= a[0].min(b[0])
            && c[0] <= a[0].max(b[0])
            && c[1] >= a[1].min(b[1])
            && c[1] <= a[1].max(b[1])
    };
    on(q1, q2, p1) || on(q1, q2, p2) || on(p1, p2, q1) || on(p1, p2, q2)
}

/// Even-odd point-in-polygon test on a `[lon, lat]` ring.
pub fn point_in_ring(ring: &[[f64; 2]], lon: f64, lat: f64) -> bool {
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
        GeometryValue::MultiPolygon { coordinates } => {
            coordinates.iter().map(|p| poly(p)).collect()
        }
        _ => Vec::new(),
    }
}

/// The message from a body that is an error report rather than GeoJSON, if it is one.
///
/// ArcGIS REST — which is behind the watch boxes, the mesoscale discussions, WSSI, the ERO, the
/// fire perimeters and the damage surveys — reports failure as **HTTP 200 with an error object**:
///
/// ```json
/// {"error":{"code":404,"message":"Layer or Table not found","details":[]}}
/// ```
///
/// `error_for_status()` sees a 200 and passes it straight to the parser, so a renumbered layer or
/// an expired token surfaced as `geojson parse: missing field 'type' at line 1 column 72` — the
/// length of that exact body, and a message that sends you looking at the parser instead of at
/// the service. Checked here rather than at twelve call sites, because every one of them arrives
/// through this function.
fn upstream_error(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let err = v.get("error")?;
    // Both shapes seen in the wild: an object with a message, or a bare string.
    let msg = match err {
        serde_json::Value::String(s) => s.clone(),
        _ => {
            let text = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
            match err.get("code").and_then(|c| c.as_i64()) {
                Some(code) if !text.is_empty() => format!("{text} ({code})"),
                Some(code) => format!("code {code}"),
                None if !text.is_empty() => text.to_string(),
                None => return None,
            }
        }
    };
    Some(msg)
}

/// Iterate the features of a GeoJSON document, yielding (geometry value, properties).
pub fn for_each_feature<F>(json: &str, mut f: F) -> anyhow::Result<()>
where
    F: FnMut(&GeometryValue, &serde_json::Map<String, serde_json::Value>),
{
    if let Some(msg) = upstream_error(json) {
        anyhow::bail!("upstream error: {msg}");
    }
    let gj: GeoJson = json
        .parse()
        .map_err(|e| anyhow::anyhow!("geojson parse: {e}"))?;
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
            rings: vec![vec![
                [0.0, 0.0],
                [2.0, 0.0],
                [2.0, 2.0],
                [0.0, 2.0],
                [0.0, 0.0],
            ]],
            fill: [0, 0, 0, 0],
            stroke: [0, 0, 0, 0],
            kind,
            title: String::new(),
            detail: String::new(),
            alert: None,
        }
    }

    #[test]
    fn distance_is_zero_inside_and_grows_outside() {
        let f = square(FeatureKind::Warning);
        assert_eq!(f.distance_km(1.0, 1.0), 0.0);
        // One degree of latitude due north of the top edge is ~111 km.
        let north = f.distance_km(1.0, 3.0);
        assert!((north - 111.2).abs() < 1.0, "{north}");
        // Just outside the east edge at the equator: 0.1 deg lon is ~11 km.
        let east = f.distance_km(2.1, 1.0);
        assert!((east - 11.1).abs() < 0.5, "{east}");
        // Diagonally off a corner, so the nearest point is the vertex, not an edge projection.
        let corner = f.distance_km(3.0, 3.0);
        assert!((corner - 157.0).abs() < 2.0, "{corner}");
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
            assert_eq!(
                props.get("event").and_then(|v| v.as_str()),
                Some("Tornado Warning")
            );
        })
        .unwrap();
        assert_eq!(count, 1);
    }

    /// Unit square, and squares placed relative to it.
    fn sq(x: f64, y: f64, w: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x + w, y], [x + w, y + w], [x, y + w]]
    }

    #[test]
    fn rings_intersect_covers_the_four_cases() {
        let base = sq(0.0, 0.0, 1.0);
        // Disjoint, and far enough that the bbox alone rejects it.
        assert!(!rings_intersect(&base, &sq(5.0, 5.0, 1.0)));
        // Overlapping bboxes but still disjoint (diagonal neighbours sharing only a corner point
        // count as touching, so step clear of it).
        assert!(!rings_intersect(&base, &sq(1.1, 1.1, 1.0)));
        // Wholly contained, either way round.
        let small = sq(0.25, 0.25, 0.25);
        assert!(rings_intersect(&base, &small));
        assert!(rings_intersect(&small, &base));
        // Edges crossing.
        assert!(rings_intersect(&base, &sq(0.5, 0.5, 1.0)));
        // Shared edge: a warning whose boundary runs along the zone's still touches it.
        assert!(rings_intersect(&base, &sq(1.0, 0.0, 1.0)));
        // Degenerate rings are never a match.
        assert!(!rings_intersect(&base, &[[0.5, 0.5], [0.6, 0.6]]));
    }
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    fn alert(id: &str, vtec: &str) -> AlertInfo {
        AlertInfo {
            id: id.into(),
            event: "Tornado Warning".into(),
            headline: String::new(),
            area: String::new(),
            description: String::new(),
            instruction: String::new(),
            expires: None,
            max_hail_in: None,
            max_wind: None,
            tornado_detection: None,
            damage_threat: None,
            source: None,
            motion: None,
            vtec: Some(vtec.into()),
        }
    }

    #[test]
    fn a_continuation_dedupes_against_the_new_warning() {
        // Same event, two messages: the second must not announce.
        let new = alert("urn:a", "/O.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/");
        let con = alert("urn:b", "/O.CON.KOUN.TO.W.0071.000000T0000Z-130520T2045Z/");
        assert_eq!(new.dedupe_key(), con.dedupe_key());
    }

    #[test]
    fn an_upgrade_announces_once_and_only_once() {
        let new = alert("urn:a", "/O.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/");
        let upg = alert("urn:c", "/O.UPG.KOUN.TO.W.0071.000000T0000Z-130520T2045Z/");
        assert_ne!(new.dedupe_key(), upg.dedupe_key());
        assert_eq!(upg.dedupe_key(), upg.dedupe_key());
    }

    #[test]
    fn no_vtec_falls_back_to_the_message_id() {
        let mut a = alert("urn:z", "");
        a.vtec = None;
        assert_eq!(a.dedupe_key(), "urn:z");
    }

    /// The body that sent someone looking at the parser: ArcGIS reports failure with HTTP 200 and
    /// an error object, so it must be named as an upstream failure, not as malformed GeoJSON.
    #[test]
    fn an_error_body_is_reported_as_the_service_failing() {
        let body = r#"{"error":{"code":404,"message":"Layer or Table not found","details":[]}}"#;
        let err = super::for_each_feature(body, |_, _| {})
            .expect_err("an error body is not a feature collection");
        let msg = err.to_string();
        assert!(msg.contains("upstream error"), "{msg}");
        assert!(msg.contains("Layer or Table not found"), "{msg}");
        assert!(msg.contains("404"), "{msg}");
        // A bare-string `error` is the other shape these services use.
        let bare = super::for_each_feature(r#"{"error":"Token Required"}"#, |_, _| {})
            .expect_err("a string error body is still an error");
        assert!(bare.to_string().contains("Token Required"));
    }

    /// Real GeoJSON must be unaffected — including a feature whose *properties* happen to carry an
    /// `error` key, which is a shape the DAT damage surveys actually publish.
    #[test]
    fn ordinary_geojson_still_parses() {
        let body = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"error":"none"},
             "geometry":{"type":"Point","coordinates":[-97.0,35.0]}}]}"#;
        let mut seen = 0;
        super::for_each_feature(body, |_, props| {
            seen += 1;
            assert_eq!(props.get("error").and_then(|v| v.as_str()), Some("none"));
        })
        .expect("a feature collection parses");
        assert_eq!(seen, 1);
    }
}
