//! Area Forecast Discussion (AFD) text from api.weather.gov.
//!
//! Two-step: resolve the point to its county-warning area (`/points/{lat},{lon}` → `cwa`),
//! then pull the newest AFD product for that office and its full text. The AFD is the
//! forecaster's own reasoning — the first thing to read before a chase day.

use crate::alerts::USER_AGENT;

const API: &str = "https://api.weather.gov";

/// One fetched discussion.
#[derive(Debug, Clone)]
pub struct Afd {
    /// Issuing office id (e.g. "OUN").
    pub office: String,
    /// Issuance time as returned (RFC3339).
    pub issued: String,
    /// Full product text.
    pub text: String,
}

/// The `cwa` (county-warning area / WFO id) of a `/points` response. GeoJSON nests it under
/// `properties`; the JSON-LD representation puts it at the root — accept both.
pub fn cwa_of(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("properties")
        .and_then(|p| p.get("cwa"))
        .or_else(|| v.get("cwa"))?
        .as_str()
        .map(str::to_string)
}

/// The newest product id in an AFD listing (`@graph` is newest-first).
pub fn first_product_id(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("@graph")?.as_array()?.first()?.get("id")?.as_str().map(str::to_string)
}

/// `(issuanceTime, productText)` of a product response.
pub fn product_text(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let issued = v.get("issuanceTime")?.as_str().unwrap_or("").to_string();
    let text = v.get("productText")?.as_str()?.to_string();
    Some((issued, text))
}

async fn get(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    Ok(client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/ld+json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Fetch the latest AFD for the office covering `(lat, lon)`.
pub async fn fetch(client: &reqwest::Client, lat: f64, lon: f64) -> anyhow::Result<Afd> {
    let points = get(client, &format!("{API}/points/{lat:.4},{lon:.4}")).await?;
    let office = cwa_of(&points).ok_or_else(|| anyhow::anyhow!("no cwa for point"))?;
    let listing = get(client, &format!("{API}/products/types/AFD/locations/{office}")).await?;
    let id = first_product_id(&listing).ok_or_else(|| anyhow::anyhow!("no AFD issued for {office}"))?;
    let product = get(client, &format!("{API}/products/{id}")).await?;
    let (issued, text) =
        product_text(&product).ok_or_else(|| anyhow::anyhow!("AFD product has no text"))?;
    Ok(Afd { office, issued, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_steps() {
        assert_eq!(
            cwa_of(r#"{"properties":{"cwa":"OUN","gridId":"OUN"}}"#).as_deref(),
            Some("OUN")
        );
        // JSON-LD flattens properties to the root.
        assert_eq!(cwa_of(r#"{"@context":{},"cwa":"OUN"}"#).as_deref(), Some("OUN"));
        assert_eq!(
            first_product_id(r#"{"@graph":[{"id":"abc-123","issuanceTime":"t1"},{"id":"old"}]}"#)
                .as_deref(),
            Some("abc-123")
        );
        let (issued, text) = product_text(
            r#"{"id":"abc-123","issuanceTime":"2026-07-20T03:00:00+00:00","productText":"FXUS64...\n.DISCUSSION...\nSupercells possible."}"#,
        )
        .unwrap();
        assert_eq!(issued, "2026-07-20T03:00:00+00:00");
        assert!(text.contains("Supercells"));
    }

    #[test]
    fn missing_pieces_are_none() {
        assert!(cwa_of(r#"{"properties":{}}"#).is_none());
        assert!(first_product_id(r#"{"@graph":[]}"#).is_none());
        assert!(product_text(r#"{"id":"x"}"#).is_none());
    }
}
