//! Forward geocoding: turn a free-text address / place name into `(label, lat, lon)` via the
//! OpenStreetMap Nominatim service (no API key). Used to add location markers by address instead
//! of typing coordinates.
//!
//! `// ponytail: Nominatim's usage policy asks for a real User-Agent (we send ours) and light use
//! (occasional interactive lookups — fine); swap for Mapbox/MapTiler geocoding if volume grows.`

/// Geocode `query` to a short label plus latitude/longitude. `Err` with a human-readable reason
/// when the query is empty, the request fails, or nothing matches.
pub async fn search(client: &reqwest::Client, query: &str) -> Result<(String, f64, f64), String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("Enter an address or place".into());
    }
    let body = client
        .get("https://nominatim.openstreetmap.org/search")
        .query(&[("q", q), ("format", "json"), ("limit", "1")])
        .header("User-Agent", crate::alerts::USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let first = v.get(0).ok_or_else(|| format!("No match for \"{q}\""))?;
    // Nominatim returns lat/lon as strings.
    let lat = first
        .get("lat")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or("Bad coordinates in result")?;
    let lon = first
        .get("lon")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or("Bad coordinates in result")?;
    // Short label: the first two comma-parts of the display name (e.g. "Norman, Cleveland County").
    let label = first
        .get("display_name")
        .and_then(|s| s.as_str())
        .map(|s| s.split(',').take(2).map(str::trim).collect::<Vec<_>>().join(", "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| q.to_string());
    Ok((label, lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nominatim_response() {
        // Shape of a real Nominatim `format=json` array (trimmed).
        let body = r#"[{"lat":"35.2225","lon":"-97.4394","display_name":"Norman, Cleveland County, Oklahoma, United States"}]"#;
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let first = v.get(0).unwrap();
        let lat: f64 = first["lat"].as_str().unwrap().parse().unwrap();
        let lon: f64 = first["lon"].as_str().unwrap().parse().unwrap();
        assert!((lat - 35.2225).abs() < 1e-6);
        assert!((lon + 97.4394).abs() < 1e-6);
        let label = first["display_name"].as_str().unwrap().split(',').take(2).map(str::trim).collect::<Vec<_>>().join(", ");
        assert_eq!(label, "Norman, Cleveland County");
    }

    // Live network check (Nominatim is public; be gentle).
    #[tokio::test]
    #[ignore = "network"]
    async fn geocodes_a_real_place() {
        let client = reqwest::Client::new();
        let (label, lat, lon) = search(&client, "Norman, OK").await.unwrap();
        eprintln!("{label} @ {lat},{lon}");
        assert!((lat - 35.2).abs() < 0.5 && (lon + 97.4).abs() < 0.5);
    }
}
