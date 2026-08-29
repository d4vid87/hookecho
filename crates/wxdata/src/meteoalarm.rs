//! MeteoAlarm — the European severe-weather warnings EUMETNET's members publish in common.
//!
//! Every national met service in Europe issues its own warnings in its own words; MeteoAlarm is
//! the one place they arrive in a single shape (CAP 1.2, wrapped in JSON) with a single severity
//! scale. That scale is the "awareness level": 1 green (no warning in force), 2 yellow, 3 orange,
//! 4 red. This module turns the levels that matter into the same [`GeoFeature`] the NWS alerts
//! produce, so the panel, banner, rules, watch radius, rollup and webhooks downstream need no
//! idea a warning came from Europe.
//!
//! # What this covers, and what it does not
//!
//! A CAP area carries geometry either as an inline `polygon` or as a `geocode` naming a region in
//! some registry. A census of all 31 country feeds on 2026-08-29 found 481 areas with a polygon
//! and 20545 with a geocode only — and the geocodes are in six different registries (EMMA_ID,
//! NUTS2, NUTS3, WARNCELLID, CISORP, FIPS). Resolving them needs a boundary table per registry.
//! MeteoAlarm publishes one, at `api.meteoalarm.org/metadata/v1/regions`, and it answers 401: the
//! metadata API is key-gated, and no key ships in this app. Eurostat's GISCO covers the NUTS
//! codes, but not EMMA_ID, which is most of them — and France's NUTS3 codes match neither the
//! 2024 nor the 2016 vintage, so even that half needs vintage archaeology.
//!
//! ponytail: so this ships the warnings that carry their own geometry, and skips the rest rather
//! than drawing them in the wrong place or guessing a shape. [`COUNTRIES`] is therefore the list
//! of feeds observed to publish polygons, not the list of countries MeteoAlarm serves — Germany,
//! Poland, Austria, Spain and Italy are all geocode-only today and are deliberately not fetched,
//! because their feeds are megabytes that would yield nothing. The upgrade path is a boundary
//! table keyed by EMMA_ID; the day one is publicly available, this list becomes every country.

use crate::overlay::{AlertInfo, FeatureKind, GeoFeature};

const FEED: &str = "https://feeds.meteoalarm.org/api/v1/warnings/feeds-";

/// The feeds that publish inline polygons, with the bounding box used to decide whether the
/// current view needs one. Fetching is bbox-gated because these payloads are not small — the
/// Swiss feed alone is about 12 MB, since a warning there carries a boundary at full survey
/// resolution — and a pan across Europe must not pull all of them.
///
/// Boxes are deliberately generous (whole-country, coasts included); a false positive costs one
/// fetch that parses to nothing, a false negative silently hides real warnings.
const COUNTRIES: &[(&str, f64, f64, f64, f64)] = &[
    ("estonia", 21.5, 57.5, 28.3, 59.8),
    ("france", -5.3, 41.2, 9.7, 51.2),
    ("latvia", 20.9, 55.6, 28.3, 58.1),
    ("lithuania", 20.9, 53.8, 26.9, 56.5),
    ("luxembourg", 5.7, 49.4, 6.6, 50.2),
    ("netherlands", 3.3, 50.7, 7.3, 53.6),
    ("norway", 4.0, 57.9, 31.2, 71.2),
    ("slovenia", 13.3, 45.4, 16.7, 46.9),
    ("sweden", 10.9, 55.3, 24.2, 69.1),
    ("switzerland", 5.9, 45.8, 10.5, 47.9),
    ("united-kingdom", -8.7, 49.8, 1.8, 61.0),
];

/// A hostile or broken feed must not be able to spend the whole frame budget. Both caps are far
/// above anything observed (the busiest feed carried 589 warnings, the largest polygon ~1200
/// vertices) and exist only to bound the work.
const MAX_WARNINGS: usize = 2000;
const MAX_VERTICES: usize = 20_000;

/// Which country feeds a view needs. Public so the caller can skip the whole overlay when the
/// view is not in Europe at all.
pub fn countries_in_view(bounds: (f64, f64, f64, f64)) -> Vec<&'static str> {
    let (x0, y0, x1, y1) = bounds;
    COUNTRIES
        .iter()
        .filter(|(_, a0, b0, a1, b1)| *a0 <= x1 && *a1 >= x0 && *b0 <= y1 && *b1 >= y0)
        .map(|(slug, ..)| *slug)
        .collect()
}

/// Every MeteoAlarm warning with a polygon, for the countries the view touches.
///
/// One failing country does not fail the overlay: a met service can take its feed down without
/// taking the rest of Europe's warnings off the map.
pub async fn fetch_in_view(
    client: &reqwest::Client,
    bounds: (f64, f64, f64, f64),
) -> anyhow::Result<Vec<GeoFeature>> {
    // Up to eleven countries can be in view, and they were fetched one after another — eleven
    // round trips deep instead of one wide, on a 120 s refresh.
    let bodies = futures_util::future::join_all(countries_in_view(bounds).into_iter().map(|slug| {
        let url = crate::net::fetch_url(&format!("{FEED}{slug}"));
        async move {
            match client.get(&url).send().await {
                Ok(resp) => match resp.text().await {
                    Ok(body) => Some(body),
                    Err(e) => {
                        log::warn!("meteoalarm {slug}: body read failed ({e})");
                        None
                    }
                },
                Err(e) => {
                    log::warn!("meteoalarm {slug}: fetch failed ({e})");
                    None
                }
            }
        }
    }))
    .await;
    let mut out = Vec::new();
    for body in bodies.into_iter().flatten() {
        out.extend(parse(&body));
    }
    Ok(out)
}

/// The awareness level (1..=4) of an info block, from its `awareness_level` parameter.
///
/// The value reads `"3; orange; Severe"`; only the leading digit is relied on, because the colour
/// word is localised in some feeds and the CAP `severity` field does not always agree with it.
fn awareness_level(info: &serde_json::Value) -> Option<u8> {
    let params = info.get("parameter")?.as_array()?;
    let raw = params.iter().find_map(|p| {
        (p.get("valueName")?.as_str()? == "awareness_level")
            .then(|| p.get("value")?.as_str())
            .flatten()
    })?;
    raw.split(';').next()?.trim().parse().ok()
}

/// The hazard from the `awareness_type` parameter (`"13; rain-flood"` -> `"rain-flood"`), title
/// cased for display. `None` when the feed omits it.
fn awareness_type(info: &serde_json::Value) -> Option<String> {
    let params = info.get("parameter")?.as_array()?;
    let raw = params.iter().find_map(|p| {
        (p.get("valueName")?.as_str()? == "awareness_type")
            .then(|| p.get("value")?.as_str())
            .flatten()
    })?;
    let name = raw.split(';').nth(1)?.trim().replace('-', " ");
    if name.is_empty() {
        return None;
    }
    let mut c = name.chars();
    Some(c.next()?.to_uppercase().collect::<String>() + c.as_str())
}

/// Colour, feature kind and escalation tag for an awareness level.
///
/// The colours are MeteoAlarm's own, so a European warning reads on the map the way it reads on
/// the issuing service's site rather than in NWS red. Orange and red map onto the escalation
/// tiers the rest of the app already sorts and sounds on: tier 3 (tornado emergency / PDS) stays
/// US-only, because Europe has no equivalent product to map onto it.
fn level_style(level: u8) -> ([u8; 3], FeatureKind, Option<&'static str>) {
    match level {
        4 => ([220, 0, 0], FeatureKind::Warning, Some("DESTRUCTIVE")),
        3 => ([255, 136, 0], FeatureKind::Warning, Some("CONSIDERABLE")),
        _ => ([245, 205, 0], FeatureKind::Advisory, None),
    }
}

/// The English info block of an alert, falling back to the first one.
///
/// Every alert repeats itself once per language. English is not guaranteed to be present — a few
/// services publish only their own — so the fallback keeps the warning on the map with a headline
/// the reader may not be able to read, which beats no warning at all.
fn english(infos: &[serde_json::Value]) -> Option<&serde_json::Value> {
    infos
        .iter()
        .find(|i| {
            i.get("language")
                .and_then(|l| l.as_str())
                .is_some_and(|l| l.starts_with("en"))
        })
        .or_else(|| infos.first())
}

/// One CAP `polygon` string — `"lat,lon lat,lon ..."` — as a ring of `[lon, lat]`.
///
/// CAP writes coordinates latitude first; every ring in this app is `[lon, lat]`, and getting the
/// order wrong puts Norway in the Indian Ocean rather than producing an error.
fn ring(poly: &str) -> Vec<[f64; 2]> {
    poly.split_whitespace()
        .take(MAX_VERTICES)
        .filter_map(|pair| {
            let (lat, lon) = pair.split_once(',')?;
            Some([lon.trim().parse().ok()?, lat.trim().parse().ok()?])
        })
        .collect()
}

/// Parse one country feed into features. Anything malformed is skipped, never panicked on: this
/// is a document from eleven different national services and it is not this app's job to be
/// right about all of them.
pub fn parse(body: &str) -> Vec<GeoFeature> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(warnings) = doc.get("warnings").and_then(|w| w.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for w in warnings.iter().take(MAX_WARNINGS) {
        let Some(alert) = w.get("alert") else {
            continue;
        };
        let Some(infos) = alert.get("info").and_then(|i| i.as_array()) else {
            continue;
        };
        let Some(info) = english(infos) else { continue };
        // Level 1 is "green": the all-clear a service publishes to say nothing is in force. It is
        // the bulk of most feeds and there is nothing to draw.
        let level = awareness_level(info).unwrap_or(1);
        if level < 2 {
            continue;
        }
        let Some(areas) = info.get("area").and_then(|a| a.as_array()) else {
            continue;
        };
        let (rgb, kind, threat) = level_style(level);
        let colour = match level {
            4 => "Red",
            3 => "Orange",
            _ => "Yellow",
        };
        let str_of = |k: &str| {
            info.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        // The synthesized name has to contain "warning" whatever the level, because that word is
        // what `severity_rank` and the user's `RuleTrigger::Warning { event_contains }` match on.
        let hazard = awareness_type(info).unwrap_or_else(|| "Weather".into());
        let event = format!("{colour} {hazard} Warning");
        let expires = info
            .get("expires")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        let id = alert
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let sender = str_of("senderName");
        for area in areas {
            let Some(polys) = area.get("polygon").and_then(|p| p.as_array()) else {
                continue;
            };
            let desc = area
                .get("areaDesc")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            for poly in polys {
                let Some(rings) = poly.as_str().map(ring) else {
                    continue;
                };
                // A ring needs three distinct corners to enclose anything; anything less is a
                // feed artefact that would render as an invisible sliver.
                if rings.len() < 3 {
                    continue;
                }
                let detail = format!(
                    "{}\n\n{event}\n\nIssued by: {sender}\nExpires: {}\nArea: {desc}\n\n{}\n\n{}",
                    str_of("headline"),
                    str_of("expires"),
                    str_of("description"),
                    str_of("instruction"),
                );
                out.push(GeoFeature {
                    rings: vec![rings],
                    fill: [rgb[0], rgb[1], rgb[2], 60],
                    stroke: [rgb[0], rgb[1], rgb[2], 255],
                    kind,
                    title: event.clone(),
                    detail,
                    alert: Some(AlertInfo {
                        id: id.clone(),
                        event: event.clone(),
                        headline: str_of("headline"),
                        area: desc.to_string(),
                        description: str_of("description"),
                        instruction: str_of("instruction"),
                        expires,
                        max_hail_in: None,
                        max_wind: None,
                        tornado_detection: None,
                        damage_threat: threat.map(str::to_string),
                        source: Some(sender.clone()),
                        motion: None,
                        // Europe issues no VTEC, so `dedupe_key` falls back to the CAP identifier.
                        // That is a message id, and MeteoAlarm mints a fresh one per update, so an
                        // extended warning announces again — the same behaviour every non-VTEC US
                        // product already has.
                        vtec: None,
                    }),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A red thunderstorm warning in Switzerland, cut down from a live feed: two languages, a
    /// green all-clear alongside it, and one area that carries a geocode instead of a polygon.
    const FEED_JSON: &str = r#"{"warnings":[
      {"alert":{"identifier":"2.49.0.0.756.0.CH.1","sent":"2026-08-29T12:00:00+00:00","info":[
        {"language":"de-CH","event":"Gewitter","area":[]},
        {"language":"en-GB","event":"Severe thunderstorm warning","senderName":"MeteoSwiss",
         "headline":"Severe thunderstorms expected","description":"Large hail.",
         "instruction":"Stay indoors.","expires":"2026-08-29T18:00:00+00:00",
         "parameter":[{"valueName":"awareness_level","value":"4; red; Extreme"},
                      {"valueName":"awareness_type","value":"11; thunderstorm"}],
         "area":[{"areaDesc":"Verzasca","polygon":["46.5,8.8 46.5,8.9 46.6,8.9 46.5,8.8"]},
                 {"areaDesc":"Coded only","geocode":[{"valueName":"EMMA_ID","value":"CH001"}]}]}]}},
      {"alert":{"identifier":"2.49.0.0.756.0.CH.2","info":[
        {"language":"en-GB","event":"No warning","parameter":[{"valueName":"awareness_level","value":"1; green; Minor"}],
         "area":[{"areaDesc":"Nowhere","polygon":["46.0,8.0 46.0,8.1 46.1,8.1 46.0,8.0"]}]}]}}
    ]}"#;

    #[test]
    fn parses_a_red_warning_and_drops_the_green_one() {
        let f = parse(FEED_JSON);
        // One feature: the green all-clear and the geocode-only area both produce nothing.
        assert_eq!(
            f.len(),
            1,
            "{:?}",
            f.iter().map(|x| &x.title).collect::<Vec<_>>()
        );
        let a = f[0].alert.as_ref().expect("a warning carries AlertInfo");
        assert_eq!(a.event, "Red Thunderstorm Warning");
        // The word the rest of the app matches on has to survive the synthesis.
        assert!(a.event.to_ascii_lowercase().contains("warning"));
        assert_eq!(a.damage_threat.as_deref(), Some("DESTRUCTIVE"));
        assert_eq!(
            crate::alerts::escalation(a),
            2,
            "red is the second escalation tier"
        );
        assert_eq!(a.area, "Verzasca");
        assert_eq!(a.id, "2.49.0.0.756.0.CH.1");
        // No VTEC in Europe, so the dedupe key falls back to the CAP identifier.
        assert!(a.vtec.is_none());
        assert_eq!(a.dedupe_key(), a.id);
        assert_eq!(
            a.expires.map(|t| t.to_rfc3339()),
            Some("2026-08-29T18:00:00+00:00".into())
        );
        // The English block is preferred over the German one that comes first.
        assert_eq!(a.headline, "Severe thunderstorms expected");
    }

    /// CAP writes `lat,lon`; every ring in this app is `[lon, lat]`. Swapping them is silent —
    /// the polygon renders, just in the wrong hemisphere.
    #[test]
    fn cap_coordinates_are_latitude_first() {
        let r = ring("46.5,8.8 46.4,8.9");
        assert_eq!(r, vec![[8.8, 46.5], [8.9, 46.4]]);
        let f = parse(FEED_JSON);
        let (x0, y0, ..) = f[0].bbox().expect("a parsed ring has a bbox");
        assert!(
            (5.0..11.0).contains(&x0) && (45.0..48.0).contains(&y0),
            "{x0},{y0}"
        );
    }

    #[test]
    fn awareness_levels_map_onto_the_escalation_tiers() {
        assert_eq!(level_style(4).2, Some("DESTRUCTIVE"));
        assert_eq!(level_style(3).2, Some("CONSIDERABLE"));
        assert_eq!(level_style(2).2, None);
        // Yellow is an advisory; orange and red draw as warnings.
        assert_eq!(level_style(2).1, FeatureKind::Advisory);
        assert_eq!(level_style(3).1, FeatureKind::Warning);
    }

    /// The bbox gate is what keeps a pan across the Atlantic from pulling tens of megabytes of
    /// European CAP, and what keeps a view over Kansas from fetching anything at all.
    #[test]
    fn only_the_countries_under_the_view_are_fetched() {
        // Generous boxes overlap at a border, and that is the safe direction: a view near Geneva
        // asks France as well as Switzerland rather than missing a warning on the other bank.
        assert_eq!(
            countries_in_view((7.0, 46.0, 8.0, 47.0)),
            vec!["france", "switzerland"]
        );
        assert_eq!(countries_in_view((8.0, 60.0, 9.0, 61.0)), vec!["norway"]);
        assert!(
            countries_in_view((-98.0, 35.0, -97.0, 36.0)).is_empty(),
            "Oklahoma is not Europe"
        );
        // A view spanning the Channel needs both sides of it.
        let c = countries_in_view((-2.0, 49.0, 3.0, 52.0));
        assert!(
            c.contains(&"france") && c.contains(&"united-kingdom"),
            "{c:?}"
        );
    }

    /// Garbage in must return nothing, not panic: this parses documents from eleven national
    /// services with no schema enforcement between them and the map.
    #[test]
    fn malformed_feeds_are_skipped_not_fatal() {
        for bad in [
            "",
            "not json",
            "{}",
            r#"{"warnings":"nope"}"#,
            r#"{"warnings":[{"alert":{}}]}"#,
            r#"{"warnings":[{"alert":{"info":[{"language":"en","parameter":[{"valueName":"awareness_level","value":"4"}],"area":[{"polygon":["oops"]}]}]}}]}"#,
        ] {
            assert!(parse(bad).is_empty(), "{bad}");
        }
    }
}
