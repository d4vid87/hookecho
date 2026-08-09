//! NWS damage surveys from the Damage Assessment Toolkit (DAT).
//!
//! After a tornado, NWS survey crews walk the path and rate what they find: one point per damage
//! indicator, each with an EF rating, a degree-of-damage description and an estimated wind speed;
//! the fitted path is published as a line. That is ground truth — the only layer in this app that
//! says what the storm actually did, rather than what the radar or a warning thought it would.
//!
//! Source: the public ArcGIS Feature Server the DAT's own viewer calls. No key. Queried as GeoJSON
//! with a bbox and a storm-date window, which is why the layer is archive-aware: scrub the timeline
//! to 2013-05-20 over Moore and the survey lands on top of the hook.
//!
//! Two service quirks, both handled below: the server caps a response at `maxRecordCount` (2000)
//! and sets `exceededTransferLimit`, so pages are pulled with `resultOffset`; and a date comparison
//! against an epoch-millisecond literal is rejected — the `where` clause has to use SQL `timestamp`
//! literals.

use crate::alerts::USER_AGENT;

const SERVER: &str =
    "https://services.dat.noaa.gov/arcgis/rest/services/nws_damageassessmenttoolkit/DamageViewer/FeatureServer";
/// The server's own cap. Asking for more per page is silently clamped to it.
const PAGE: usize = 2000;
/// Stop after this many pages (~20k features) — a continental multi-day query would otherwise
/// page for a long time over a slow link.
const MAX_PAGES: usize = 10;

/// One surveyed damage indicator.
#[derive(Debug, Clone, PartialEq)]
pub struct DamagePoint {
    pub lon: f64,
    pub lat: f64,
    /// "EF0".."EF5", or "EFU" for unknown. Empty when the survey left it blank.
    pub efscale: String,
    /// The damage indicator surveyed, e.g. "One- or Two-Family Residence (FR12)".
    pub damage: String,
    /// The degree of damage observed on that indicator.
    pub dod: String,
    /// Estimated wind speed, mph.
    pub windspeed: Option<i32>,
    pub deaths: i32,
    pub injuries: i32,
    /// Issuing office (WFO) that did the survey.
    pub office: String,
    pub comments: Option<String>,
    /// URL of a survey photo, when one was attached.
    pub image: Option<String>,
    pub storm: Option<chrono::DateTime<chrono::Utc>>,
}

/// A surveyed tornado path.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageTrack {
    /// Path vertices as `[lon, lat]`, in order.
    pub path: Vec<[f64; 2]>,
    pub efscale: String,
    /// Path length in miles and max width in yards, as the survey recorded them.
    pub length_mi: Option<f64>,
    pub width_yd: Option<f64>,
    pub max_wind: Option<i32>,
    pub fatalities: i32,
    pub injuries: i32,
    pub wfo: String,
    pub comments: Option<String>,
    pub storm: Option<chrono::DateTime<chrono::Utc>>,
}

/// The rating as a 0–5 number, or `None` when the survey didn't rate it. Surveys write the scale
/// field freehand, so besides "EF3" the real data carries "EF3+" (at least EF3), "EFU", "UNKNOWN",
/// "N/A" and "TSTM/Wind" (straight-line wind, not a tornado) — everything unrated draws grey.
pub fn ef_number(efscale: &str) -> Option<u8> {
    let digits: String = efscale
        .trim()
        .strip_prefix("EF")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u8>().ok().filter(|n| *n <= 5)
}

fn text(p: &serde_json::Value, k: &str) -> String {
    match p.get(k).and_then(|v| v.as_str()) {
        // The service writes the string "null" into empty text columns.
        Some(s) if !s.is_empty() && s != "null" => s.to_string(),
        _ => String::new(),
    }
}

fn opt_text(p: &serde_json::Value, k: &str) -> Option<String> {
    let s = text(p, k);
    (!s.is_empty() && s != "None").then_some(s)
}

fn num(p: &serde_json::Value, k: &str) -> Option<f64> {
    let v = p.get(k)?;
    // Numeric columns come back as numbers; `windspeed` is a string of digits.
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// Epoch-millisecond columns → UTC.
fn stamp(p: &serde_json::Value, k: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp_millis(p.get(k)?.as_i64()?)
}

/// Parse a GeoJSON page of the damage-point layer.
pub fn parse_points(json: &str) -> anyhow::Result<Vec<DamagePoint>> {
    let mut out = Vec::new();
    crate::overlay::for_each_feature(json, |geom, props| {
        let geojson::GeometryValue::Point { coordinates: c } = geom else {
            return;
        };
        let (Some(&lon), Some(&lat)) = (c.as_slice().first(), c.as_slice().get(1)) else {
            return;
        };
        let props = serde_json::Value::Object(props.clone());
        out.push(DamagePoint {
            lon,
            lat,
            efscale: text(&props, "efscale"),
            damage: text(&props, "damage_txt"),
            dod: text(&props, "dod_txt"),
            windspeed: num(&props, "windspeed").map(|w| w as i32),
            deaths: num(&props, "deaths").unwrap_or(0.0) as i32,
            injuries: num(&props, "injuries").unwrap_or(0.0) as i32,
            office: text(&props, "office"),
            comments: opt_text(&props, "comments"),
            image: opt_text(&props, "image"),
            storm: stamp(&props, "stormdate"),
        });
    })?;
    Ok(out)
}

/// Parse a GeoJSON page of the damage-line layer. Handles both `LineString` and `MultiLineString`
/// (the service returns either depending on how the path was digitised).
pub fn parse_tracks(json: &str) -> anyhow::Result<Vec<DamageTrack>> {
    let mut out = Vec::new();
    crate::overlay::for_each_feature(json, |geom, props| {
        let lines: Vec<&Vec<geojson::Position>> = match geom {
            geojson::GeometryValue::LineString { coordinates: l } => vec![l],
            geojson::GeometryValue::MultiLineString { coordinates: ls } => ls.iter().collect(),
            _ => return,
        };
        let props = serde_json::Value::Object(props.clone());
        for line in lines {
            let path: Vec<[f64; 2]> = line
                .iter()
                .filter_map(|p| Some([*p.as_slice().first()?, *p.as_slice().get(1)?]))
                .collect();
            if path.len() < 2 {
                continue;
            }
            out.push(DamageTrack {
                path,
                efscale: text(&props, "efscale"),
                length_mi: num(&props, "length"),
                width_yd: num(&props, "width"),
                max_wind: num(&props, "maxwind").map(|w| w as i32),
                fatalities: num(&props, "fatalities").unwrap_or(0.0) as i32,
                injuries: num(&props, "injuries").unwrap_or(0.0) as i32,
                wfo: text(&props, "wfo"),
                comments: opt_text(&props, "comments"),
                storm: stamp(&props, "stormdate"),
            });
        }
    })?;
    Ok(out)
}

/// A storm-date `where` clause. The service rejects epoch-millisecond comparisons on date columns,
/// so the window is written with SQL `timestamp` literals (UTC, as the column is stored).
fn date_where(start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "stormdate BETWEEN timestamp '{}' AND timestamp '{}'",
        start.format("%Y-%m-%d %H:%M:%S"),
        end.format("%Y-%m-%d %H:%M:%S")
    )
}

/// Pull every page of one layer as GeoJSON, following `exceededTransferLimit`.
async fn fetch_layer(
    client: &reqwest::Client,
    layer: u8,
    bbox: (f64, f64, f64, f64),
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Vec<String>> {
    let (min_lon, min_lat, max_lon, max_lat) = bbox;
    let envelope = format!("{min_lon},{min_lat},{max_lon},{max_lat}");
    let where_clause = date_where(start, end);
    let mut pages = Vec::new();
    for page in 0..MAX_PAGES {
        let offset = (page * PAGE).to_string();
        let body = client
            .get(crate::net::fetch_url(&format!("{SERVER}/{layer}/query")))
            .query(&[
                ("where", where_clause.as_str()),
                ("geometry", envelope.as_str()),
                ("geometryType", "esriGeometryEnvelope"),
                ("inSR", "4326"),
                ("outSR", "4326"),
                ("outFields", "*"),
                ("returnGeometry", "true"),
                ("resultOffset", offset.as_str()),
                ("resultRecordCount", "2000"),
                ("f", "geojson"),
            ])
            .header("User-Agent", USER_AGENT)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let more = exceeded_limit(&body);
        pages.push(body);
        if !more {
            break;
        }
    }
    Ok(pages)
}

/// Whether a GeoJSON page was truncated by the server's record cap.
fn exceeded_limit(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("properties")?.get("exceededTransferLimit")?.as_bool())
        .unwrap_or(false)
}

/// Fetch the damage points and surveyed tracks for storms in `[start, end]` inside `bbox`.
pub async fn fetch(
    client: &reqwest::Client,
    bbox: (f64, f64, f64, f64),
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<(Vec<DamagePoint>, Vec<DamageTrack>)> {
    let (pt_pages, ln_pages) = futures_util::try_join!(
        fetch_layer(client, 0, bbox, start, end),
        fetch_layer(client, 1, bbox, start, end),
    )?;
    let mut points = Vec::new();
    for p in &pt_pages {
        points.extend(parse_points(p)?);
    }
    let mut tracks = Vec::new();
    for p in &ln_pages {
        tracks.extend(parse_tracks(p)?);
    }
    Ok((points, tracks))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a live query over Moore, OK for 2013-05-20.
    const POINTS: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","id":136273,"geometry":{"type":"Point","coordinates":[-97.4863,35.3391]},
       "properties":{"objectid":136273,"stormdate":1369079760000,"efscale":"EF5",
         "damage_txt":"One- or Two-Family Residence (FR12)","dod_txt":"Total destruction of building",
         "windspeed":"200","deaths":2,"injuries":11,"office":"OUN","comments":"None",
         "image":"null"}},
      {"type":"Feature","id":136274,"geometry":{"type":"Point","coordinates":[-97.51,35.33]},
       "properties":{"objectid":136274,"stormdate":1369079760000,"efscale":"EFU",
         "damage_txt":"Trees - Softwood (TS)","dod_txt":"Trees uprooted","windspeed":"85",
         "deaths":0,"injuries":0,"office":"OUN","comments":"Snapped at the base",
         "image":"https://apps.dat.noaa.gov/photo/1.jpg"}},
      {"type":"Feature","id":136275,"geometry":null,"properties":{}}]}"#;

    const TRACK: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","id":9,"geometry":{"type":"LineString",
        "coordinates":[[-97.6,35.30],[-97.4,35.35],[-97.2,35.38]]},
       "properties":{"objectid":9,"stormdate":1369079760000,"efscale":"EF5","efnum":5,
         "length":13.85,"width":1900,"maxwind":210,"fatalities":24,"injuries":212,"wfo":"OUN",
         "comments":"null"}}]}"#;

    #[test]
    fn parses_damage_points() {
        let p = parse_points(POINTS).unwrap();
        assert_eq!(p.len(), 2, "the geometry-less feature must drop");
        assert_eq!(p[0].efscale, "EF5");
        assert_eq!(ef_number(&p[0].efscale), Some(5));
        assert_eq!(p[0].windspeed, Some(200));
        assert_eq!((p[0].deaths, p[0].injuries), (2, 11));
        // "None" comments and "null" images are the service's way of saying nothing.
        assert_eq!(p[0].comments, None);
        assert_eq!(p[0].image, None);
        assert_eq!(p[1].comments.as_deref(), Some("Snapped at the base"));
        assert!(p[1].image.is_some());
        // EFU is a real rating value meaning "unrateable", not a number.
        assert_eq!(ef_number(&p[1].efscale), None);
        // The other scale values the live service actually returns.
        assert_eq!(ef_number("EF3+"), Some(3));
        for unrated in ["TSTM/Wind", "UNKNOWN", "N/A", ""] {
            assert_eq!(ef_number(unrated), None, "{unrated}");
        }
        assert_eq!(
            p[0].storm.unwrap().to_rfc3339(),
            "2013-05-20T19:56:00+00:00"
        );
    }

    #[test]
    fn parses_damage_track() {
        let t = parse_tracks(TRACK).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].path.len(), 3);
        assert_eq!(t[0].length_mi, Some(13.85));
        assert_eq!(t[0].width_yd, Some(1900.0));
        assert_eq!(t[0].fatalities, 24);
        assert_eq!(t[0].comments, None);
    }

    #[test]
    fn date_window_uses_sql_timestamps() {
        let s = chrono::DateTime::from_timestamp(1369008000, 0).unwrap();
        let e = chrono::DateTime::from_timestamp(1369094400, 0).unwrap();
        assert_eq!(
            date_where(s, e),
            "stormdate BETWEEN timestamp '2013-05-20 00:00:00' AND timestamp '2013-05-21 00:00:00'"
        );
    }

    #[test]
    fn detects_the_transfer_limit() {
        assert!(exceeded_limit(
            r#"{"type":"FeatureCollection","features":[],"properties":{"exceededTransferLimit":true}}"#
        ));
        assert!(!exceeded_limit(
            r#"{"type":"FeatureCollection","features":[]}"#
        ));
    }
}
