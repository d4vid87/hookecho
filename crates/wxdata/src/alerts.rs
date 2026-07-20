//! NWS active alerts from api.weather.gov: warnings, watches, statements, advisories.
//!
//! Each alert with a polygon becomes a [`GeoFeature`] colored by event. Zone-only alerts
//! (no polygon, just UGC zones) are skipped for now — resolving zone geometry is a follow-up.

use crate::overlay::{for_each_feature, polygons_of, AlertInfo, FeatureKind, GeoFeature, StormMotion};

const ALERTS_URL: &str = "https://api.weather.gov/alerts/active";
/// weather.gov requires a User-Agent identifying the app + a contact.
pub const USER_AGENT: &str = "hookecho (github.com/d4vid87/hookecho, davidmay87@gmail.com)";

/// Broad phenomenon group, for the toolbox filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Tornado,
    SevereThunderstorm,
    Flood,
    Winter,
    Marine,
    Other,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Tornado,
        Category::SevereThunderstorm,
        Category::Flood,
        Category::Winter,
        Category::Marine,
        Category::Other,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Category::Tornado => "Tornado",
            Category::SevereThunderstorm => "Severe Tstm",
            Category::Flood => "Flood",
            Category::Winter => "Winter",
            Category::Marine => "Marine",
            Category::Other => "Other",
        }
    }
    pub fn index(self) -> usize {
        Category::ALL.iter().position(|c| *c == self).unwrap()
    }
}

/// Classify an event name into a phenomenon group.
pub fn category(event: &str) -> Category {
    let e = event.to_ascii_lowercase();
    if e.contains("tornado") {
        Category::Tornado
    } else if e.contains("thunderstorm") {
        Category::SevereThunderstorm
    } else if e.contains("flood") || e.contains("flash flood") {
        Category::Flood
    } else if e.contains("winter") || e.contains("snow") || e.contains("ice") || e.contains("blizzard") {
        Category::Winter
    } else if e.contains("marine") || e.contains("small craft") || e.contains("gale") || e.contains("surf") {
        Category::Marine
    } else {
        Category::Other
    }
}

/// Parse an NWS `eventMotionDescription` into a [`StormMotion`].
///
/// Format is `...`-delimited, e.g. `2023-03-31T20:30:00-00:00...storm...234DEG...52KT...3540,9012`.
/// Direction/speed are the tokens ending in `DEG`/`KT`; trailing `lat,lon` pairs are hundredths of
/// a degree with lon stored west-positive (so negated). Direction is kept *as issued* (FROM). Any
/// missing piece (no DEG, no KT, no points) makes this `None` so the caller simply doesn't draw.
pub fn parse_motion(desc: &str) -> Option<StormMotion> {
    let mut deg = None;
    let mut kt = None;
    let mut points = Vec::new();
    for tok in desc.split("...").map(str::trim).filter(|t| !t.is_empty()) {
        let up = tok.to_ascii_uppercase();
        if let Some(n) = up.strip_suffix("DEG") {
            deg = n.trim().parse::<f32>().ok().or(deg);
        } else if let Some(n) = up.strip_suffix("KT") {
            kt = n.trim().parse::<f32>().ok().or(kt);
        } else if tok.contains(',') {
            // One or more space-separated `lat,lon` centroid pairs (hundredths of a degree).
            for pair in tok.split_whitespace() {
                if let Some((a, b)) = pair.split_once(',') {
                    if let (Ok(lat), Ok(lon)) = (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
                        points.push([-(lon / 100.0), lat / 100.0]);
                    }
                }
            }
        }
    }
    let (deg, kt) = (deg?, kt?);
    if points.is_empty() {
        return None;
    }
    Some(StormMotion { deg, kt, points })
}

/// Escalation tier for a warning: 0 plain, 1 CONSIDERABLE, 2 DESTRUCTIVE/observed-tornado,
/// 3 Tornado Emergency / PDS. Higher tiers sort to the top and trigger the emergency sound.
pub fn escalation(a: &AlertInfo) -> u8 {
    let head = format!("{} {}", a.headline, a.description).to_ascii_uppercase();
    if head.contains("TORNADO EMERGENCY") || head.contains("PARTICULARLY DANGEROUS SITUATION") {
        return 3;
    }
    let threat = a.damage_threat.as_deref().unwrap_or("").to_ascii_uppercase();
    let observed = a
        .tornado_detection
        .as_deref()
        .map(|d| d.to_ascii_uppercase().contains("OBSERVED"))
        .unwrap_or(false);
    if threat.contains("DESTRUCTIVE") || threat.contains("CATASTROPHIC") || observed {
        return 2;
    }
    if threat.contains("CONSIDERABLE") {
        return 1;
    }
    0
}

/// (FeatureKind, base RGB) for an event; fill is this at low alpha, stroke at full.
pub(crate) fn event_style(event: &str) -> (FeatureKind, [u8; 3]) {
    let e = event.to_ascii_lowercase();
    let kind = if e.contains("warning") {
        FeatureKind::Warning
    } else if e.contains("watch") {
        FeatureKind::Watch
    } else if e.contains("advisory") {
        FeatureKind::Advisory
    } else {
        FeatureKind::Statement
    };
    let rgb = match event {
        "Tornado Warning" => [255, 0, 0],
        "Severe Thunderstorm Warning" => [255, 165, 0],
        "Flash Flood Warning" => [0, 200, 0],
        "Flood Warning" => [0, 160, 90],
        "Tornado Watch" => [255, 255, 0],
        "Severe Thunderstorm Watch" => [219, 112, 147],
        "Special Weather Statement" => [255, 228, 181],
        "Flood Advisory" => [0, 180, 120],
        _ => match kind {
            FeatureKind::Warning => [230, 60, 60],
            FeatureKind::Watch => [200, 180, 60],
            FeatureKind::Advisory => [120, 180, 200],
            _ => [180, 180, 180],
        },
    };
    (kind, rgb)
}

/// First string of `parameters[key]` (alert parameter values are arrays of strings).
fn param(props: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    props
        .get("parameters")?
        .get(key)?
        .as_array()?
        .first()?
        .as_str()
        .map(str::to_string)
}

/// Build the styling + [`AlertInfo`] + detail text for one alert's properties. `None` if it has
/// no `event`.
fn build_alert(props: &serde_json::Map<String, serde_json::Value>) -> Option<(FeatureKind, [u8; 3], String, AlertInfo)> {
    let get = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let event = get("event");
    if event.is_empty() {
        return None;
    }
    let (kind, rgb) = event_style(event);
    let detail = format!(
        "{}\n\n{}\n\nEffective: {}\nExpires: {}\nArea: {}\n\n{}\n\n{}",
        get("headline"),
        event,
        get("effective"),
        get("expires"),
        get("areaDesc"),
        get("description"),
        get("instruction"),
    );
    let max_hail_in = param(props, "maxHailSize").and_then(|s| s.trim().parse::<f32>().ok());
    let alert = AlertInfo {
        id: get("id").to_string(),
        event: event.to_string(),
        headline: get("headline").to_string(),
        area: get("areaDesc").to_string(),
        description: get("description").to_string(),
        instruction: get("instruction").to_string(),
        expires: chrono::DateTime::parse_from_rfc3339(get("expires"))
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc)),
        max_hail_in,
        max_wind: param(props, "maxWindGust"),
        tornado_detection: param(props, "tornadoDetection"),
        damage_threat: param(props, "thunderstormDamageThreat")
            .or_else(|| param(props, "tornadoDamageThreat")),
        source: param(props, "eventMotionDescription").or_else(|| Some("Radar indicated".into())),
        motion: param(props, "eventMotionDescription").as_deref().and_then(parse_motion),
    };
    Some((kind, rgb, detail, alert))
}

/// Parse an api.weather.gov alerts GeoJSON payload into features (each carries [`AlertInfo`]).
/// Only alerts with an inline polygon are returned; zone-only alerts are resolved separately.
pub fn parse_alerts(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let Some((kind, rgb, detail, alert)) = build_alert(props) else { return };
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 45],
                stroke: [rgb[0], rgb[1], rgb[2], 235],
                kind,
                title: alert.event.clone(),
                detail: detail.clone(),
                alert: Some(alert.clone()),
            });
        }
    })?;
    Ok(out)
}

/// One zone's polygon groups (rings per polygon part), as returned by [`polygons_of`].
type ZonePolys = Vec<Vec<Vec<[f64; 2]>>>;

/// Process-lifetime cache of resolved zone geometries (rings), keyed by zone URL. Zone polygons
/// are effectively static, so one fetch per zone per run is plenty.
/// `// ponytail: in-memory only; a disk cache would survive restarts if it ever matters.`
static ZONE_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, ZonePolys>>> =
    std::sync::OnceLock::new();

/// Cap on zone geometries fetched per refresh, so a nationwide burst of zone-only advisories
/// can't fan out into thousands of requests.
const MAX_ZONE_FETCHES: usize = 250;

/// Fetch + cache a zone's polygon rings from its api.weather.gov zone URL.
async fn fetch_zone_geometry(client: &reqwest::Client, url: &str) -> Vec<Vec<Vec<[f64; 2]>>> {
    let cache = ZONE_CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(url).cloned() {
        return hit;
    }
    let polys = async {
        let body = client
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/geo+json")
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()?;
        let mut out = Vec::new();
        for_each_feature(&body, |geom, _| out.extend(polygons_of(geom))).ok()?;
        Some(out)
    }
    .await
    .unwrap_or_default();
    cache.lock().unwrap().insert(url.to_string(), polys.clone());
    polys
}

/// Resolve zone-only alerts (no inline polygon) into features via their `affectedZones` URLs.
async fn fetch_zone_alerts(client: &reqwest::Client, body: &str) -> Vec<GeoFeature> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    let Some(feats) = v.get("features").and_then(|f| f.as_array()) else { return Vec::new() };
    let mut out = Vec::new();
    let mut budget = MAX_ZONE_FETCHES;
    let mut truncated = false;
    for feat in feats {
        // Only alerts lacking an inline geometry need zone resolution.
        if !feat.get("geometry").map(|g| g.is_null()).unwrap_or(true) {
            continue;
        }
        let Some(props) = feat.get("properties").and_then(|p| p.as_object()) else { continue };
        let Some((kind, rgb, detail, alert)) = build_alert(props) else { continue };
        let zones = props.get("affectedZones").and_then(|z| z.as_array()).cloned().unwrap_or_default();
        for zurl in zones.iter().filter_map(|z| z.as_str()) {
            if budget == 0 {
                truncated = true;
                break;
            }
            budget -= 1;
            for poly in fetch_zone_geometry(client, zurl).await {
                out.push(GeoFeature {
                    rings: poly,
                    fill: [rgb[0], rgb[1], rgb[2], 45],
                    stroke: [rgb[0], rgb[1], rgb[2], 235],
                    kind,
                    title: alert.event.clone(),
                    detail: detail.clone(),
                    alert: Some(alert.clone()),
                });
            }
        }
        if truncated {
            break;
        }
    }
    if truncated {
        log::warn!("zone-only alert resolution capped at {MAX_ZONE_FETCHES} zones");
    }
    out
}

/// Fetch all active NWS alerts as overlay features: inline-polygon alerts plus zone-only alerts
/// resolved to their UGC zone geometry.
pub async fn fetch_active(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    let resp = client
        .get(ALERTS_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/geo+json")
        .send()
        .await?
        .error_for_status()?;
    let body = resp.text().await?;
    let mut feats = parse_alerts(&body)?;
    feats.extend(fetch_zone_alerts(client, &body).await);
    Ok(feats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_styles_warning() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-98,35],[-97,35],[-97,36],[-98,35]]]},
             "properties":{"id":"urn:oid:tor1","event":"Tornado Warning","headline":"TOR until 5pm","description":"...","areaDesc":"Cleveland, OK",
               "parameters":{"maxHailSize":["1.00"],"maxWindGust":["60 MPH"],"tornadoDetection":["RADAR INDICATED"]}}}]}"#;
        let feats = parse_alerts(json).unwrap();
        assert_eq!(feats.len(), 1);
        assert_eq!(feats[0].kind, FeatureKind::Warning);
        assert_eq!(feats[0].stroke, [255, 0, 0, 235]);
        assert!(feats[0].detail.contains("TOR until 5pm"));
        assert_eq!(category("Tornado Warning"), Category::Tornado);
        let a = feats[0].alert.as_ref().expect("alert info");
        assert_eq!(a.id, "urn:oid:tor1");
        assert_eq!(a.max_hail_in, Some(1.0));
        assert_eq!(a.max_wind.as_deref(), Some("60 MPH"));
        assert_eq!(a.tornado_detection.as_deref(), Some("RADAR INDICATED"));
    }

    #[test]
    fn hit_all_dedupe_by_id() {
        use crate::overlay::hit_all;
        // A MultiPolygon warning yields two GeoFeatures sharing one alert id.
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "geometry":{"type":"MultiPolygon","coordinates":[
                [[[-98,35],[-97,35],[-97,36],[-98,35]]],
                [[[-98,35],[-97,35],[-97,36],[-98,35]]]]},
             "properties":{"id":"urn:oid:x","event":"Severe Thunderstorm Warning","areaDesc":"A"}}]}"#;
        let feats = parse_alerts(json).unwrap();
        assert_eq!(feats.len(), 2, "one feature per polygon part");
        let hits = hit_all(&feats, -97.5, 35.2);
        // Both parts contain the point; the caller dedupes by alert id.
        let ids: std::collections::HashSet<_> =
            hits.iter().filter_map(|f| f.alert.as_ref().map(|a| a.id.as_str())).collect();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn parse_motion_single_point() {
        let m = parse_motion("2023-03-31T20:30:00-00:00...storm...234DEG...52KT...3540,9012").unwrap();
        assert_eq!(m.deg, 234.0);
        assert_eq!(m.kt, 52.0);
        assert_eq!(m.points.len(), 1);
        assert!((m.points[0][1] - 35.40).abs() < 1e-6, "lat");
        assert!((m.points[0][0] - -90.12).abs() < 1e-6, "lon west-positive negated");
    }

    #[test]
    fn parse_motion_multi_point() {
        let m = parse_motion("...storm...100DEG...20KT...3540,9012 3548,9020").unwrap();
        assert_eq!(m.points.len(), 2);
    }

    #[test]
    fn parse_motion_garbage_is_none() {
        assert!(parse_motion("no motion here").is_none());
        assert!(parse_motion("...234DEG...52KT...").is_none(), "no points");
    }

    #[test]
    fn escalation_tiers() {
        let mk = |head: &str, threat: Option<&str>, det: Option<&str>| AlertInfo {
            id: String::new(),
            event: "Severe Thunderstorm Warning".into(),
            headline: head.into(),
            area: String::new(),
            description: String::new(),
            instruction: String::new(),
            expires: None,
            max_hail_in: None,
            max_wind: None,
            tornado_detection: det.map(str::to_string),
            damage_threat: threat.map(str::to_string),
            source: None,
            motion: None,
        };
        assert_eq!(escalation(&mk("plain warning", None, None)), 0);
        assert_eq!(escalation(&mk("", Some("CONSIDERABLE"), None)), 1);
        assert_eq!(escalation(&mk("", Some("DESTRUCTIVE"), None)), 2);
        assert_eq!(escalation(&mk("", None, Some("OBSERVED"))), 2);
        assert_eq!(escalation(&mk("THIS IS A TORNADO EMERGENCY", None, None)), 3);
    }

    // Live network check (nation-wide there are essentially always active alerts).
    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_live_alerts() {
        let client = reqwest::Client::new();
        let feats = fetch_active(&client).await.unwrap();
        eprintln!("fetched {} alert polygons", feats.len());
    }
}
