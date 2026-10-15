//! WPC Excessive Rainfall Outlook — where rainfall is likely to exceed flash-flood guidance.
//!
//! The SPC outlook answers "where will storms be severe"; this answers "where will they flood",
//! which is the other half of a warm-season day and the only one that covers the tropics. Same
//! risk ladder (marginal → high), same map-service plumbing, same [`GeoFeature`] output.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const SERVICE: &str =
    "https://mapservices.weather.noaa.gov/vector/rest/services/hazards/wpc_precip_hazards/MapServer";

#[derive(Debug, Clone)]
pub struct Discussion {
    pub day: u8,
    pub text: String,
    pub source_url: &'static str,
}

pub fn discussion_url(day: u8) -> Option<&'static str> {
    match day {
        1..=3 => Some("https://www.wpc.ncep.noaa.gov/discussions/hpcdiscussions.php?disc=qpferd&fmt=print"),
        4..=5 => Some("https://www.wpc.ncep.noaa.gov/discussions/hpcdiscussions.php?disc=pmdepd&fmt=print"),
        _ => None,
    }
}

pub fn parse_discussion(body: &str, day: u8) -> anyhow::Result<Discussion> {
    let source_url = discussion_url(day).ok_or_else(|| anyhow::anyhow!("rainfall outlook day out of range"))?;
    anyhow::ensure!(body.len() <= 1024 * 1024, "WPC discussion exceeds 1 MB");
    let lower = body.to_ascii_lowercase();
    let start = lower.find("<pre").ok_or_else(|| anyhow::anyhow!("WPC discussion body missing"))?;
    let text_start = start + lower[start..].find('>').ok_or_else(|| anyhow::anyhow!("WPC discussion body malformed"))? + 1;
    let end = text_start + lower[text_start..].find("</pre>").ok_or_else(|| anyhow::anyhow!("WPC discussion body incomplete"))?;
    let text = crate::cpc::plain_text(&body[text_start..end]);
    let expected = if day <= 3 { "Excessive Rainfall Discussion" } else { "Extended Forecast Discussion" };
    anyhow::ensure!(text.contains(expected) && text.len() > 100, "WPC discussion text unavailable");
    Ok(Discussion { day, text, source_url })
}

pub async fn fetch_discussion(client: &reqwest::Client, day: u8) -> anyhow::Result<Discussion> {
    let url = discussion_url(day).ok_or_else(|| anyhow::anyhow!("rainfall outlook day out of range"))?;
    let body = client.get(crate::net::fetch_url(url)).timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT).send().await?.error_for_status()?.text().await?;
    parse_discussion(&body, day)
}

/// Fill for a risk level. Mirrors [`crate::spc::risk_color`] so a "moderate" reads the same
/// whichever outlook drew it.
fn risk_color(outlook: &str) -> [u8; 3] {
    let o = outlook.to_ascii_uppercase();
    match () {
        _ if o.starts_with("MARGINAL") => [127, 197, 127],
        _ if o.starts_with("SLIGHT") => [246, 246, 131],
        _ if o.starts_with("MODERATE") => [230, 152, 90],
        _ if o.starts_with("HIGH") => [204, 102, 204],
        _ => [150, 150, 150],
    }
}

/// Short label ("MRGL") from the service's spelled-out risk ("Marginal (At Least 5%)").
fn risk_short(outlook: &str) -> &'static str {
    let o = outlook.to_ascii_uppercase();
    match () {
        _ if o.starts_with("MARGINAL") => "MRGL",
        _ if o.starts_with("SLIGHT") => "SLGT",
        _ if o.starts_with("MODERATE") => "MDT",
        _ if o.starts_with("HIGH") => "HIGH",
        _ => "ERO",
    }
}

/// Parse an Excessive Rainfall Outlook GeoJSON payload.
pub fn parse(json: &str, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let s = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let outlook = s("outlook");
        if outlook.is_empty() {
            return;
        }
        let rgb = risk_color(outlook);
        for rings in polygons_of(geom) {
            out.push(GeoFeature {
                rings,
                fill: [rgb[0], rgb[1], rgb[2], 60],
                stroke: [rgb[0], rgb[1], rgb[2], 220],
                kind: FeatureKind::Outlook,
                title: format!("ERO D{day} {}", risk_short(outlook)),
                detail: format!(
                    "Excessive Rainfall Outlook — Day {day}\nRisk: {outlook}\nValid: {}",
                    s("valid_time")
                ),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// Fetch the Excessive Rainfall Outlook for `day` (1-5; the service publishes five).
pub async fn fetch(client: &reqwest::Client, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    let day = day.clamp(1, 5);
    // Layer ids are Day 1..5 in order from zero, and unlike the WSSI service they have not moved.
    let body = client
        .get(crate::net::fetch_url(&format!(
            "{SERVICE}/{}/query?where=1%3D1&outFields=outlook,valid_time&f=geojson",
            day - 1
        )))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discussion_selects_public_product_by_day() {
        for day in 1..=5 {
            let title = if day <= 3 { "Excessive Rainfall Discussion" } else { "Extended Forecast Discussion" };
            let html = format!("<html><pre><font>{title}\nNWS Weather Prediction Center\n{}</font></pre></html>", "Forecast details. ".repeat(10));
            let parsed = parse_discussion(&html, day).unwrap();
            assert!(parsed.text.contains(title));
            assert_eq!(parsed.day, day);
        }
        assert!(discussion_url(0).is_none());
        assert!(discussion_url(6).is_none());
        assert!(parse_discussion("<html>no discussion</html>", 1).is_err());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_public_discussion_products() {
        let client = reqwest::Client::new();
        for day in [1, 4] {
            let discussion = fetch_discussion(&client, day).await.unwrap();
            assert!(discussion.text.len() > 100);
        }
    }

    const SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"outlook":"Marginal (At Least 5%)","valid_time":"16Z 08/01/26 - 12Z 08/02/26"},
       "geometry":{"type":"Polygon","coordinates":[[[-95.0,30.0],[-94.0,30.0],[-94.0,31.0],[-95.0,30.0]]]}},
      {"type":"Feature","properties":{"outlook":"High (At Least 50%)","valid_time":"16Z 08/01/26 - 12Z 08/02/26"},
       "geometry":{"type":"Polygon","coordinates":[[[-90.0,29.0],[-89.0,29.0],[-89.0,30.0],[-90.0,29.0]]]}},
      {"type":"Feature","properties":{"valid_time":"x"},"geometry":null}]}"#;

    #[test]
    fn risks_label_and_color() {
        let f = parse(SAMPLE, 1).unwrap();
        assert_eq!(f.len(), 2, "the risk-less feature is skipped");
        assert_eq!(f[0].title, "ERO D1 MRGL");
        assert_eq!(f[1].title, "ERO D1 HIGH");
        assert!(f[0].detail.contains("Marginal (At Least 5%)"));
        // The ladder matches the SPC colors the app already uses for the same words.
        assert_eq!(f[0].fill[..3], crate::spc::risk_color("MRGL"));
        assert_eq!(f[1].fill[..3], crate::spc::risk_color("HIGH"));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_day_one() {
        let c = reqwest::Client::new();
        let f = fetch(&c, 1).await.expect("ERO day 1");
        for feat in &f {
            assert!(feat.title.starts_with("ERO D1"));
        }
    }
}
