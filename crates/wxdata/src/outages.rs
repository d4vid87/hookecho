//! County power outages, from ODIN (Outage Data Initiative Nationwide, ORNL/DOE).
//!
//! The radar says where the storm is; this says where the lights went out behind it. ODIN
//! aggregates participating utilities' outage feeds to the county and publishes them through an
//! Opendatasoft API that also carries a ready-simplified county polygon per record — so the map
//! needs no county geometry of its own, and no FIPS table to join on.
//!
//! Records are per-incident (utility × county), so several rows share a county on a busy day.
//! They are summed here into one [`GeoFeature`] per county, colored by how many meters are out.
//!
//! Coverage is opt-in: utilities that do not report to ODIN are simply absent, so a dark county
//! can also mean an unreporting utility. That caveat belongs in the UI, not in a filter here.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

/// One request: data, geometry, and the significance floor, all server-side. The floor is well
/// under the first draw tier — a county is only drawn once its *summed* incidents clear
/// [`MIN_METERS`], and small incidents still have to be fetched to be summed.
const URL: &str = "https://ornl.opendatasoft.com/api/explore/v2.1/catalog/datasets/\
odin-real-time-outages-county/exports/geojson?where=metersaffected%3E%3D25\
&select=county,state,metersaffected,cause,estimatedrestorationtime,utility_id";

/// Below this many meters out, a county is not drawn at all. A few hundred customers is a
/// blown fuse on one street, and shading a whole county for it makes every quiet day look bad.
pub const MIN_METERS: u32 = 500;

/// Tier color and label for a county total, or `None` when it is not worth drawing.
///
/// Absolute counts, not share of households: the question a chaser asks is "how big is this",
/// and a share needs a Census denominator this crate would have to ship and keep current.
// ponytail: hand-picked breakpoints; revisit only if rural counties visibly under-light.
fn tier(meters: u32) -> Option<([u8; 3], &'static str)> {
    match meters {
        m if m < MIN_METERS => None,
        m if m < 2_000 => Some(([246, 224, 110], "scattered")),
        m if m < 10_000 => Some(([240, 160, 60], "significant")),
        m if m < 50_000 => Some(([225, 70, 70], "major")),
        _ => Some(([190, 80, 205], "widespread")),
    }
}

/// A number that may arrive as a JSON number or as a string.
fn number(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: Option<&serde_json::Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    (!s.is_empty() && !s.eq_ignore_ascii_case("null")).then(|| s.to_string())
}

/// One county's running total while parsing.
#[derive(Default)]
struct County {
    meters: u32,
    incidents: u32,
    utilities: std::collections::HashSet<String>,
    /// Cause and restoration estimate of the county's *largest* incident: with several utilities
    /// out at once, the biggest one is the story.
    largest: u32,
    cause: Option<String>,
    restore: Option<String>,
    rings: Vec<Vec<Vec<[f64; 2]>>>,
}

/// Parse an ODIN county-outage GeoJSON export into one feature per significant county.
pub fn parse(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut by_county: std::collections::HashMap<(String, String), County> =
        std::collections::HashMap::new();
    for_each_feature(json, |geom, props| {
        let county = text(props.get("county")).unwrap_or_default();
        let state = text(props.get("state")).unwrap_or_default();
        if county.is_empty() {
            return;
        }
        let meters = number(props.get("metersaffected")).unwrap_or(0.0).max(0.0) as u32;
        let e = by_county.entry((state, county)).or_default();
        e.meters = e.meters.saturating_add(meters);
        e.incidents += 1;
        if let Some(u) = text(props.get("utility_id")) {
            e.utilities.insert(u);
        }
        if meters >= e.largest {
            e.largest = meters;
            e.cause = text(props.get("cause"));
            e.restore = text(props.get("estimatedrestorationtime"));
        }
        // Every incident in a county repeats the same county polygon; keep the first one that
        // actually has geometry.
        if e.rings.is_empty() {
            e.rings = polygons_of(geom);
        }
    })?;

    let mut out = Vec::new();
    for ((state, county), c) in by_county {
        let Some((rgb, label)) = tier(c.meters) else {
            continue;
        };
        let where_ = if state.is_empty() {
            county.clone()
        } else {
            format!("{county}, {state}")
        };
        let mut detail = format!(
            "{where_}\n{} customers without power ({label})\n{} incident{}, {} utilit{}",
            thousands(c.meters),
            c.incidents,
            if c.incidents == 1 { "" } else { "s" },
            c.utilities.len().max(1),
            if c.utilities.len() == 1 { "y" } else { "ies" },
        );
        if let Some(cause) = &c.cause {
            detail.push_str(&format!("\nCause: {cause}"));
        }
        if let Some(restore) = &c.restore {
            detail.push_str(&format!("\nEstimated restoration: {restore}"));
        }
        detail.push_str("\n\nSource: ODIN (DOE/ORNL) — participating utilities only.");
        let title = format!("{where_}: {} out", thousands(c.meters));
        for rings in c.rings {
            out.push(GeoFeature {
                rings,
                // No fill: the app cross-hatches these instead (`hookecho::outage_draw`). A
                // translucent fill is how outlooks and warnings are drawn, and an outage that
                // looks like a warning is worse than one that is harder to see.
                fill: [rgb[0], rgb[1], rgb[2], 0],
                stroke: [rgb[0], rgb[1], rgb[2], 200],
                kind: FeatureKind::Outlook,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    }
    // Stable order so the biggest outage draws last (on top) and repeat renders don't shuffle.
    out.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(out)
}

/// `4120` → `4,120`.
fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Fetch current county outages nationwide.
pub async fn fetch(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    let body = client
        .get(crate::net::fetch_url(URL))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(county: &str, meters: f64, utility: &str, cause: &str, geom: bool) -> String {
        let g = if geom {
            r#"{"type":"Polygon","coordinates":[[[-81.0,41.0],[-81.0,42.0],[-80.0,42.0],[-80.0,41.0],[-81.0,41.0]]]}"#
        } else {
            "null"
        };
        format!(
            r#"{{"type":"Feature","geometry":{g},"properties":{{"county":"{county}","state":"OH","metersaffected":{meters},"utility_id":"{utility}","cause":"{cause}","estimatedrestorationtime":null}}}}"#
        )
    }

    fn collection(feats: &[String]) -> String {
        format!(
            r#"{{"type":"FeatureCollection","features":[{}]}}"#,
            feats.join(",")
        )
    }

    #[test]
    fn incidents_in_one_county_are_summed_once() {
        let json = collection(&[
            feature("Cuyahoga", 1_200.0, "A", "storm", true),
            feature("Cuyahoga", 3_000.0, "B", "equipment failure", true),
        ]);
        let out = parse(&json).unwrap();
        // One polygon, not one per incident.
        assert_eq!(out.len(), 1);
        assert!(out[0].title.contains("4,200"), "{}", out[0].title);
        assert!(out[0].detail.contains("2 incidents, 2 utilities"));
        // Cause comes from the larger of the two incidents.
        assert!(out[0].detail.contains("Cause: equipment failure"));
    }

    #[test]
    fn small_outages_are_not_drawn() {
        let json = collection(&[feature("Geauga", 120.0, "A", "storm", true)]);
        assert!(parse(&json).unwrap().is_empty());
    }

    #[test]
    fn rows_without_geometry_contribute_no_polygon() {
        let json = collection(&[feature("Lake", 9_000.0, "A", "storm", false)]);
        assert!(parse(&json).unwrap().is_empty());
    }

    #[test]
    fn tiers_step_at_their_boundaries() {
        assert!(tier(MIN_METERS - 1).is_none());
        assert_eq!(tier(MIN_METERS).unwrap().1, "scattered");
        assert_eq!(tier(2_000).unwrap().1, "significant");
        assert_eq!(tier(10_000).unwrap().1, "major");
        assert_eq!(tier(50_000).unwrap().1, "widespread");
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(4_120), "4,120");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn numbers_may_arrive_as_strings() {
        assert_eq!(number(Some(&serde_json::json!("1200"))), Some(1200.0));
        assert_eq!(number(Some(&serde_json::json!(7.0))), Some(7.0));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_live_outages() {
        let c = reqwest::Client::new();
        for f in fetch(&c).await.expect("ODIN outages") {
            assert!(f.detail.contains("without power"));
            assert!(!f.rings.is_empty());
        }
    }
}

