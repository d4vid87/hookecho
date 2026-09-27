//! NWS active alerts from api.weather.gov: warnings, watches, statements, advisories.
//!
//! Each alert with a polygon becomes a [`GeoFeature`] colored by event. Zone-only alerts (no inline
//! polygon, just UGC zones — heat warnings, advisories, marine) are resolved to their zone geometry;
//! see [`fetch_active`], which joins nationwide NOAA map geometry to the active NWS bulletins.

use crate::overlay::{
    for_each_feature, polygons_of, AlertInfo, FeatureKind, GeoFeature, StormMotion,
};

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
    } else if e.contains("winter")
        || e.contains("snow")
        || e.contains("ice")
        || e.contains("blizzard")
    {
        Category::Winter
    } else if e.contains("marine")
        || e.contains("small craft")
        || e.contains("gale")
        || e.contains("surf")
    {
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
    let threat = a
        .damage_threat
        .as_deref()
        .unwrap_or("")
        .to_ascii_uppercase();
    let observed = a
        .tornado_detection
        .as_deref()
        .map(|d| d.to_ascii_uppercase().contains("OBSERVED"))
        .unwrap_or(false);
    if threat.contains("DESTRUCTIVE") || threat.contains("CATASTROPHIC") || observed {
        return 2;
    }
    // SIGNIFICANT is the snow squall's own escalation tag — the one that sends a phone alert.
    if threat.contains("CONSIDERABLE") || threat.contains("SIGNIFICANT") {
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
        // NWS's own color for the product. A snow squall is a short-fuse life-threatening
        // warning and used to draw in the same generic red as everything else with "warning" in
        // its name, which is the one thing it must not look like on a winter map.
        "Snow Squall Warning" => [199, 21, 133],
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
fn build_alert(
    props: &serde_json::Map<String, serde_json::Value>,
) -> Option<(FeatureKind, [u8; 3], String, AlertInfo)> {
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
            .or_else(|| param(props, "tornadoDamageThreat"))
            // A snow squall's tag rides in the same field: it is the same kind of statement about
            // the same kind of warning, and everything downstream already reads this one.
            .or_else(|| param(props, "snowSquallImpact")),
        vtec: param(props, "VTEC"),
        source: param(props, "eventMotionDescription").or_else(|| Some("Radar indicated".into())),
        motion: param(props, "eventMotionDescription")
            .as_deref()
            .and_then(parse_motion),
    };
    Some((kind, rgb, detail, alert))
}

/// Parse one `api.weather.gov/alerts/{id}` response for callers that already own its geometry.
pub(crate) fn parse_alert_info(json: &str) -> anyhow::Result<AlertInfo> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let props = value
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("alert response has no properties"))?;
    build_alert(props)
        .map(|(_, _, _, alert)| alert)
        .ok_or_else(|| anyhow::anyhow!("alert response has no event"))
}

/// Parse an api.weather.gov alerts GeoJSON payload into features (each carries [`AlertInfo`]).
/// Only alerts with an inline polygon are returned; zone-only alerts are resolved separately.
pub fn parse_alerts(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let Some((kind, rgb, detail, alert)) = build_alert(props) else {
            return;
        };
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
static ZONE_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, ZonePolys>>,
> = std::sync::OnceLock::new();

/// Where resolved zone geometry is kept between runs. A county's shape does not change, so the
/// first heat advisory of the summer pays for the whole season. Set once at startup; unset (web,
/// headless) leaves the cache memory-only.
static ZONE_CACHE_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Point the zone geometry cache at a directory on disk. Later calls are ignored.
pub fn set_zone_cache_dir(dir: std::path::PathBuf) {
    let _ = ZONE_CACHE_DIR.set(dir);
}

/// Disk path for a zone URL: the trailing id (`.../zones/forecast/OKZ025` -> `OKZ025.json`), with
/// anything that isn't alphanumeric dropped so a malformed URL can't escape the directory.
fn zone_cache_file(url: &str) -> Option<std::path::PathBuf> {
    let dir = ZONE_CACHE_DIR.get()?;
    let id: String = url
        .rsplit('/')
        .find(|s| !s.is_empty())?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if id.is_empty() {
        return None;
    }
    Some(dir.join("zones").join(format!("{id}.json")))
}

/// Cap on zone geometries fetched per refresh, so a nationwide burst of zone-only advisories
/// can't fan out into thousands of requests.
const MAX_ZONE_FETCHES: usize = 250;

/// Fetch + cache a zone's polygon rings from its api.weather.gov zone URL.
async fn fetch_zone_geometry(client: &reqwest::Client, url: &str) -> Vec<Vec<Vec<[f64; 2]>>> {
    let cache = ZONE_CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(url).cloned() {
        return hit;
    }
    let file = zone_cache_file(url);
    let rings = |body: &str| {
        let mut out: ZonePolys = Vec::new();
        for_each_feature(body, |geom, _| out.extend(polygons_of(geom))).ok()?;
        Some(out)
    };
    if let Some(hit) = file
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|b| rings(&b))
    {
        cache.lock().unwrap().insert(url.to_string(), hit.clone());
        return hit;
    }
    let polys = async {
        let body = client
            .get(crate::net::fetch_url(url))
            .timeout(crate::net::FEED_TIMEOUT)
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
        crate::stats::net(body.len());
        let out = rings(&body)?;
        if let Some(p) = &file {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, &body);
        }
        Some(out)
    }
    .await
    .unwrap_or_default();
    cache.lock().unwrap().insert(url.to_string(), polys.clone());
    polys
}

/// Resolve zone-only alerts (no inline polygon) in `body` into features via their `affectedZones`
/// URLs. Alerts whose id is already in `seen` are skipped (dedup across the nationwide + scoped
/// passes); every resolved id is added to `seen`. `budget` caps zone fetches so a burst can't fan
/// out into thousands of requests.
async fn resolve_zone_alerts(
    client: &reqwest::Client,
    body: &str,
    mut budget: usize,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<GeoFeature> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(feats) = v.get("features").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for feat in feats {
        // Only alerts lacking an inline geometry need zone resolution.
        if !feat.get("geometry").map(|g| g.is_null()).unwrap_or(true) {
            continue;
        }
        let Some(props) = feat.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };
        let Some((kind, rgb, detail, alert)) = build_alert(props) else {
            continue;
        };
        if !seen.insert(alert.id.clone()) {
            continue; // already resolved in an earlier pass
        }
        let zones = props
            .get("affectedZones")
            .and_then(|z| z.as_array())
            .cloned()
            .unwrap_or_default();
        for zurl in zones.iter().filter_map(|z| z.as_str()) {
            if budget == 0 {
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
    }
    out
}

/// GET an api.weather.gov alerts endpoint as a GeoJSON body.
async fn get_alerts(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let body = client
        .get(crate::net::fetch_url(url))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/geo+json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    crate::stats::net(body.len());
    Ok(body)
}

/// How many `?point=` queries one refresh may make. Each is a round trip plus its zone
/// resolutions, and past a handful of saved locations the nationwide pass is the cheaper answer.
const MAX_POINTS: usize = 8;

/// Fetch only active NWS alerts carrying their own polygon.
pub async fn fetch_polygon_alerts(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    parse_alerts(&get_alerts(client, ALERTS_URL).await?)
}

// NOAA's county/zone layer supplies geometry missing from the NWS CAP feed. Fetch every page:
// selecting a radar or a saved place must not restrict which alerts can appear on the map.
const ZONE_MAP_URL: &str = "https://mapservices.weather.noaa.gov/eventdriven/rest/services/WWA/watch_warn_adv/MapServer/1/query?where=1%3D1&outFields=cap_id&geometryPrecision=4&maxAllowableOffset=0.002&orderByFields=objectid&resultRecordCount=1000&f=geojson";

/// Attach each county/zone part to its current bulletin. Keep inline warning polygons intact;
/// county geometry is only a fallback for alerts without geometry in the authoritative feed.
fn parse_zone_map(body: &str, active: &serde_json::Value) -> anyhow::Result<Vec<GeoFeature>> {
    let bulletins: std::collections::HashMap<_, _> = active["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f.get("geometry").is_none_or(serde_json::Value::is_null))
        .filter_map(|f| {
            let props = f["properties"].as_object()?;
            let alert = build_alert(props)?;
            Some((alert.3.id.clone(), alert))
        })
        .collect();
    let mut out = Vec::new();
    for_each_feature(body, |geom, props| {
        let Some((kind, rgb, detail, alert)) = props
            .get("cap_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| bulletins.get(id))
        else { return; };
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 45],
                stroke: [rgb[0], rgb[1], rgb[2], 235],
                kind: *kind,
                title: alert.event.clone(),
                detail: detail.clone(),
                alert: Some(alert.clone()),
            });
        }
    })?;
    Ok(out)
}

async fn fetch_zone_map(client: &reqwest::Client, active: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let active = serde_json::from_str(active)?;
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let body = get_alerts(client, &format!("{ZONE_MAP_URL}&resultOffset={offset}")).await?;
        let page: serde_json::Value = serde_json::from_str(&body)?;
        out.extend(parse_zone_map(&body, &active)?);
        if page["exceededTransferLimit"].as_bool() != Some(true)
            && page["properties"]["exceededTransferLimit"].as_bool() != Some(true)
        {
            break;
        }
        let count = page["features"].as_array().map_or(0, Vec::len);
        anyhow::ensure!(count > 0, "nationwide alert geometry pagination made no progress");
        offset += count;
    }
    Ok(out)
}

/// Fetch all active NWS polygons nationwide, including zone-only watches, advisories and
/// statements. Local zone lookups remain a fallback when NOAA's bulk geometry feed is unavailable.
pub async fn fetch_active(
    client: &reqwest::Client,
    points: &[(f64, f64)],
) -> anyhow::Result<Vec<GeoFeature>> {
    let body = get_alerts(client, ALERTS_URL).await?;
    let mut feats = parse_alerts(&body)?;
    match fetch_zone_map(client, &body).await {
        Ok(zones) => {
            feats.extend(zones);
            return Ok(feats);
        }
        Err(e) => log::warn!("nationwide alert geometry failed ({e}); resolving local zones"),
    }
    let mut seen: std::collections::HashSet<String> = feats
        .iter()
        .filter_map(|f| f.alert.as_ref().map(|a| a.id.clone()))
        .collect();
    if points.is_empty() {
        feats.extend(resolve_zone_alerts(client, &body, MAX_ZONE_FETCHES, &mut seen).await);
    }
    for (lat, lon) in points.iter().take(MAX_POINTS) {
        let url = format!("{ALERTS_URL}?point={lat:.4},{lon:.4}");
        match get_alerts(client, &url).await {
            Ok(point_body) => {
                feats.extend(resolve_zone_alerts(client, &point_body, 400, &mut seen).await);
            }
            // A point query can 400 (e.g. a marine site just off the coast) — fall back so the
            // user still gets the feed-top zone alerts rather than none.
            Err(e) => {
                log::warn!("scoped alert fetch failed ({e}); using nationwide zone pass");
                feats.extend(resolve_zone_alerts(client, &body, MAX_ZONE_FETCHES, &mut seen).await);
            }
        }
    }
    Ok(feats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nationwide_zone_geometry_keeps_all_parts_and_bulletins_without_replacing_inline_polygons() {
        use serde_json::json;
        let polygon = json!({"type":"Polygon","coordinates":[[[-80.,40.],[-79.,40.],[-79.,41.],[-80.,40.]]]});
        let events = ["Flood Watch", "Flood Warning", "Wind Advisory", "Special Weather Statement"];
        let mut active = Vec::new();
        let mut zones = Vec::new();
        for event in events {
            active.push(json!({"type":"Feature","geometry":null,"properties":{
                "id":event,"event":event,"headline":"Official headline","description":"Official details",
                "areaDesc":"Far from the selected radar","expires":"2026-09-28T00:00:00Z"
            }}));
            // Two county parts of the same bulletin must both survive the join.
            for _ in 0..2 {
                zones.push(json!({"type":"Feature","geometry":polygon,"properties":{"cap_id":event}}));
            }
        }
        active.push(json!({"type":"Feature","geometry":polygon,"properties":{"id":"inline","event":"Tornado Warning"}}));
        for id in ["inline", "expired-or-unknown"] {
            zones.push(json!({"type":"Feature","geometry":polygon,"properties":{"cap_id":id}}));
        }
        let active = json!({"type":"FeatureCollection","features":active});
        let zones = json!({"type":"FeatureCollection","features":zones}).to_string();
        let features = parse_zone_map(&zones, &active).unwrap();
        assert_eq!(features.len(), 8);
        for event in events {
            let parts: Vec<_> = features.iter().filter(|f| f.title == event).collect();
            assert_eq!(parts.len(), 2);
            let alert = parts[0].alert.as_ref().unwrap();
            assert_eq!(alert.headline, "Official headline");
            assert_eq!(alert.description, "Official details");
            assert!(alert.expires.is_some());
        }
        assert_eq!(parse_alerts(&active.to_string()).unwrap().len(), 1);
        assert!(parse_zone_map(r#"{"error":{"message":"unavailable"}}"#, &active).is_err());
    }

    #[test]
    fn parses_one_watch_bulletin_without_geometry() {
        let json = r#"{"type":"Feature","geometry":null,"properties":{"id":"watch-669","event":"Tornado Watch","headline":"Tornado Watch 669","areaDesc":"Central Iowa","description":"A couple tornadoes possible.","instruction":null,"expires":"2026-09-15T05:00:00+00:00","parameters":{}}}"#;
        let alert = parse_alert_info(json).unwrap();
        assert_eq!(alert.area, "Central Iowa");
        assert_eq!(alert.description, "A couple tornadoes possible.");
    }

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
        let ids: std::collections::HashSet<_> = hits
            .iter()
            .filter_map(|f| f.alert.as_ref().map(|a| a.id.as_str()))
            .collect();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn parse_motion_single_point() {
        let m =
            parse_motion("2023-03-31T20:30:00-00:00...storm...234DEG...52KT...3540,9012").unwrap();
        assert_eq!(m.deg, 234.0);
        assert_eq!(m.kt, 52.0);
        assert_eq!(m.points.len(), 1);
        assert!((m.points[0][1] - 35.40).abs() < 1e-6, "lat");
        assert!(
            (m.points[0][0] - -90.12).abs() < 1e-6,
            "lon west-positive negated"
        );
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
            vtec: None,
            motion: None,
        };
        assert_eq!(escalation(&mk("plain warning", None, None)), 0);
        assert_eq!(escalation(&mk("", Some("CONSIDERABLE"), None)), 1);
        assert_eq!(escalation(&mk("", Some("DESTRUCTIVE"), None)), 2);
        assert_eq!(escalation(&mk("", None, Some("OBSERVED"))), 2);
        assert_eq!(
            escalation(&mk("THIS IS A TORNADO EMERGENCY", None, None)),
            3
        );
    }

    // Live network check (nation-wide there are essentially always active alerts).
    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_live_alerts() {
        let client = reqwest::Client::new();
        let feats = fetch_active(&client, &[(32.57, -97.30)]).await.unwrap();
        eprintln!("fetched {} alert polygons", feats.len());
    }

    #[test]
    fn a_snow_squall_gets_its_own_color_and_its_impact_tag_escalates() {
        let (kind, rgb) = event_style("Snow Squall Warning");
        assert_eq!(kind, FeatureKind::Warning);
        assert_ne!(
            rgb,
            event_style("Winter Storm Warning").1,
            "not generic red"
        );

        let mut a = AlertInfo {
            id: "urn:x".into(),
            event: "Snow Squall Warning".into(),
            headline: String::new(),
            area: String::new(),
            description: String::new(),
            instruction: String::new(),
            expires: None,
            max_hail_in: None,
            max_wind: None,
            tornado_detection: None,
            damage_threat: Some("SIGNIFICANT".into()),
            source: None,
            motion: None,
            vtec: None,
        };
        assert_eq!(
            escalation(&a),
            1,
            "a tagged squall sorts above plain warnings"
        );
        a.damage_threat = None;
        assert_eq!(escalation(&a), 0);
    }
}
