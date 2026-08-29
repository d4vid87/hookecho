//! ECCC — the public weather alerts Environment and Climate Change Canada has in force.
//!
//! Canada's alerts arrive as the same [`GeoFeature`] the NWS and MeteoAlarm ones do, so the panel,
//! banner, rules, watch radius, rollup and webhooks downstream need no idea a warning came from
//! Canada.
//!
//! # Why WFS and not CAP
//!
//! ECCC publishes the alerts three ways, and only one of them is usable here. The Datamart CAP
//! tree (`dd.weather.gc.ca/.../alerts/cap/`) is a date/office/hour directory of individual XML
//! bulletins with no index of what is currently in force, so finding the active set means walking
//! the tree — dozens of requests per refresh, and the recommended way to follow it is an AMQP
//! subscription this app has no business holding open. `api.weather.gc.ca` (OGC API - Features)
//! serves hydrometric, climate and marine collections but publishes no alerts collection at all.
//!
//! GeoMet's WFS does: `Current-Alerts` is exactly the set in force, every feature carries its own
//! polygon, and the whole of Canada came to 427 KB / 42 features on 2026-08-29 in under a second.
//! It is also `geo.weather.gc.ca`, already in the allowlist from the radar composites, so this
//! wave adds no host and no new trust boundary.
//!
//! ponytail: the whole country in one request, filtered to the view by the panel like every other
//! overlay. The server's `bbox=` parameter was tried and returns zero features (WFS 2.0 axis-order
//! trouble); at 427 KB for the country it is not worth chasing.

use crate::overlay::{for_each_feature, polygons_of, AlertInfo, FeatureKind, GeoFeature};

/// `Current-Alerts` is the layer of alerts in force right now — no time parameter, no paging.
const FEED: &str = "https://geo.weather.gc.ca/geomet/?service=WFS&version=2.0.0\
&request=GetFeature&typeNames=Current-Alerts&outputFormat=geojson";

/// Canada, generously boxed to the Arctic islands and both coasts. A false positive costs one
/// 427 KB fetch that draws nothing; a false negative silently hides real warnings.
const CANADA: (f64, f64, f64, f64) = (-141.5, 41.0, -52.0, 84.0);

/// A broken or hostile feed must not spend the whole frame budget. Far above anything observed
/// (42 features, the largest polygon 850 vertices) — this only bounds the work.
const MAX_ALERTS: usize = 2000;

/// Whether the view touches Canada at all. Public so the caller skips the request entirely when
/// it does not.
pub fn in_view(bounds: (f64, f64, f64, f64)) -> bool {
    let (x0, y0, x1, y1) = bounds;
    let (a0, b0, a1, b1) = CANADA;
    a0 <= x1 && a1 >= x0 && b0 <= y1 && b1 >= y0
}

/// Every Canadian alert in force, or nothing at all when the view is elsewhere.
pub async fn fetch_in_view(
    client: &reqwest::Client,
    bounds: (f64, f64, f64, f64),
) -> anyhow::Result<Vec<GeoFeature>> {
    if !in_view(bounds) {
        return Ok(Vec::new());
    }
    let body = client
        .get(crate::net::fetch_url(FEED))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse(&body))
}

/// Colour, feature kind and escalation tag for one alert.
///
/// The colour is ECCC's own `risk_colour_en`, so an alert reads on the map the way it reads on
/// weather.gc.ca rather than in NWS red, and it maps onto the escalation tiers the rest of the app
/// sorts and sounds on. The kind comes from `alert_type` instead, because a red *watch* is still a
/// watch. Tier 3 (tornado emergency / PDS) stays US-only: Canada issues no equivalent product.
fn style(risk_colour: &str, alert_type: &str) -> ([u8; 3], FeatureKind, Option<&'static str>) {
    let (rgb, threat) = match risk_colour {
        "red" => ([220, 0, 0], Some("DESTRUCTIVE")),
        "orange" => ([255, 136, 0], Some("CONSIDERABLE")),
        _ => ([245, 205, 0], None),
    };
    let kind = match alert_type {
        "warning" => FeatureKind::Warning,
        "watch" => FeatureKind::Watch,
        "statement" => FeatureKind::Statement,
        _ => FeatureKind::Advisory,
    };
    (rgb, kind, threat)
}

/// `"severe thunderstorm warning"` -> `"Severe Thunderstorm Warning"`.
///
/// ECCC writes the event name lower case. `severity_rank` and the user's rules both lower case
/// before matching, so this is display only — but the panel sits next to NWS events that are
/// title cased, and a lower-case row reads like a bug.
fn title_case(name: &str) -> String {
    name.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse the `Current-Alerts` GeoJSON into features. Anything malformed is skipped, never
/// panicked on.
pub fn parse(body: &str) -> Vec<GeoFeature> {
    let mut out = Vec::new();
    let _ = for_each_feature(body, |geom, props| {
        if out.len() >= MAX_ALERTS {
            return;
        }
        let str_of = |k: &str| {
            props
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let event = title_case(&str_of("alert_name_en"));
        if event.is_empty() {
            return;
        }
        let (rgb, kind, threat) = style(&str_of("risk_colour_en"), &str_of("alert_type"));
        let area = str_of("feature_name_en");
        let province = str_of("province");
        let expires_raw = str_of("expiration_datetime");
        let expires = chrono::DateTime::parse_from_rfc3339(&expires_raw)
            .ok()
            .map(|t| t.with_timezone(&chrono::Utc));
        let description = str_of("alert_text_en");
        let id = str_of("id");
        let detail = format!(
            "{event}\n\nIssued by: Environment and Climate Change Canada\nStatus: {}\nExpires: \
             {expires_raw}\nArea: {area}, {province}\n\n{description}",
            str_of("status_en"),
        );
        for rings in polygons_of(geom) {
            // A ring needs three corners to enclose anything; less is a feed artefact that would
            // render as an invisible sliver.
            if rings.first().is_none_or(|r| r.len() < 3) {
                continue;
            }
            out.push(GeoFeature {
                rings,
                fill: [rgb[0], rgb[1], rgb[2], 60],
                stroke: [rgb[0], rgb[1], rgb[2], 255],
                kind,
                title: event.clone(),
                detail: detail.clone(),
                alert: Some(AlertInfo {
                    id: id.clone(),
                    event: event.clone(),
                    headline: format!("{event} for {area}"),
                    area: format!("{area}, {province}"),
                    description: description.clone(),
                    instruction: String::new(),
                    expires,
                    max_hail_in: None,
                    max_wind: None,
                    tornado_detection: None,
                    damage_threat: threat.map(str::to_string),
                    source: Some("Environment and Climate Change Canada".into()),
                    motion: None,
                    // Canada issues no VTEC, so `dedupe_key` falls back to this id. ECCC keeps it
                    // stable across an alert's updates, so a continued alert does not re-announce.
                    vtec: None,
                }),
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two alerts cut down from the live `Current-Alerts` feed: an orange warning and a yellow
    /// watch, plus a degenerate polygon that must not reach the map.
    const FEED_JSON: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"id":"19072915712931888792","alert_code":"RFW",
       "alert_type":"warning","alert_name_en":"rainfall warning","risk_colour_en":"orange",
       "expiration_datetime":"2026-08-30T00:32:34.747Z","status_en":"issued",
       "alert_text_en":"Heavy rain.","feature_name_en":"Nelson","province":"BC"},
       "geometry":{"type":"Polygon","coordinates":[[[-117.1,49.6],[-117.0,49.6],[-117.0,49.7],
       [-117.1,49.6]]]}},
      {"type":"Feature","properties":{"id":"22222","alert_type":"watch",
       "alert_name_en":"severe thunderstorm watch","risk_colour_en":"yellow",
       "expiration_datetime":"2026-08-29T20:00:00.000Z","status_en":"continued",
       "alert_text_en":"Conditions favourable.","feature_name_en":"Kenora","province":"ON"},
       "geometry":{"type":"Polygon","coordinates":[[[-94.5,49.7],[-94.4,49.7],[-94.4,49.8],
       [-94.5,49.7]]]}},
      {"type":"Feature","properties":{"id":"33333","alert_type":"warning",
       "alert_name_en":"wind warning","risk_colour_en":"yellow","feature_name_en":"Sliver",
       "province":"YT"},
       "geometry":{"type":"Polygon","coordinates":[[[-135.0,60.0],[-135.0,60.1]]]}}
    ]}"#;

    #[test]
    fn parses_the_alerts_and_drops_the_degenerate_polygon() {
        let feats = parse(FEED_JSON);
        assert_eq!(feats.len(), 2, "the two-point ring must not render");

        let rain = &feats[0];
        assert_eq!(rain.title, "Rainfall Warning");
        assert_eq!(rain.kind, FeatureKind::Warning);
        assert_eq!(rain.stroke, [255, 136, 0, 255]);
        let info = rain.alert.as_ref().unwrap();
        assert_eq!(info.id, "19072915712931888792");
        assert_eq!(info.area, "Nelson, BC");
        assert_eq!(info.damage_threat.as_deref(), Some("CONSIDERABLE"));
        assert!(info.vtec.is_none());
        assert_eq!(
            info.expires.map(|t| t.to_rfc3339()),
            Some("2026-08-30T00:32:34.747+00:00".to_string())
        );
        // Ring order is GeoJSON's own, so it is already [lon, lat] — no swap anywhere here.
        assert_eq!(rain.rings[0][0], [-117.1, 49.6]);

        // A watch stays a watch whatever its colour, which is what keeps `severity_rank` honest.
        assert_eq!(feats[1].kind, FeatureKind::Watch);
        assert_eq!(feats[1].title, "Severe Thunderstorm Watch");
        assert_eq!(feats[1].alert.as_ref().unwrap().damage_threat, None);
    }

    #[test]
    fn risk_colours_map_onto_the_escalation_tiers() {
        assert_eq!(style("red", "warning").2, Some("DESTRUCTIVE"));
        assert_eq!(style("orange", "warning").2, Some("CONSIDERABLE"));
        assert_eq!(style("yellow", "warning").2, None);
        // An unknown colour must degrade to the mildest tier, not to a missing arm.
        assert_eq!(style("chartreuse", "statement").0, [245, 205, 0]);
        assert_eq!(style("yellow", "statement").1, FeatureKind::Statement);
    }

    #[test]
    fn only_a_view_over_canada_asks_for_anything() {
        assert!(in_view((-114.0, 50.0, -113.0, 51.0)), "Calgary");
        assert!(
            in_view((-124.0, 48.0, -122.0, 49.5)),
            "the border straddles"
        );
        assert!(!in_view((-97.5, 35.0, -97.0, 35.5)), "Oklahoma City");
        assert!(!in_view((7.0, 46.0, 8.0, 47.0)), "Switzerland");
    }

    #[test]
    fn malformed_feeds_are_skipped_not_fatal() {
        assert!(parse("").is_empty());
        assert!(parse("{").is_empty());
        assert!(parse(r#"{"type":"FeatureCollection","features":[]}"#).is_empty());
        // A feature with no event name has nothing to label a row with.
        assert!(parse(
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},
            "geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}]}"#
        )
        .is_empty());
    }
}
