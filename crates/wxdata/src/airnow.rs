//! Air-quality observations from AirNow (EPA), the feed the "purple sky" apps show.
//!
//! Needs a free key from <https://docs.airnowapi.org>; without one the layer stays off. The
//! bbox "data" endpoint is the only one that returns every monitor in a view rather than the
//! single nearest city, which is what a map layer needs.

use crate::alerts::USER_AGENT;
use serde::Deserialize;

const BASE: &str = "https://www.airnowapi.org/aq/data/";

/// One monitoring site's current AQI for one pollutant.
#[derive(Debug, Clone, PartialEq)]
pub struct AqiOb {
    pub lat: f64,
    pub lon: f64,
    pub aqi: i32,
    /// EPA category number (1 Good … 6 Hazardous).
    pub category: u8,
    /// Pollutant, as reported ("OZONE", "PM2.5", …).
    pub param: String,
    pub site: String,
}

impl AqiOb {
    /// EPA's published category color for this observation.
    pub fn color(&self) -> [u8; 3] {
        match self.category {
            1 => [0, 228, 0],
            2 => [255, 255, 0],
            3 => [255, 126, 0],
            4 => [255, 0, 0],
            5 => [143, 63, 151],
            _ => [126, 0, 35],
        }
    }

    pub fn category_name(&self) -> &'static str {
        match self.category {
            1 => "Good",
            2 => "Moderate",
            3 => "Unhealthy for sensitive groups",
            4 => "Unhealthy",
            5 => "Very unhealthy",
            _ => "Hazardous",
        }
    }
}

#[derive(Deserialize)]
struct Row {
    #[serde(rename = "Latitude")]
    lat: f64,
    #[serde(rename = "Longitude")]
    lon: f64,
    #[serde(rename = "AQI")]
    aqi: f64,
    /// Signed: the feed sends -1 on rows with no valid value.
    #[serde(rename = "Category")]
    category: Option<i32>,
    #[serde(rename = "Parameter")]
    parameter: Option<String>,
    #[serde(rename = "SiteName")]
    site_name: Option<String>,
}

/// AQI category for a value, when the feed omits `Category` (it does on some rows).
fn category_of(aqi: i32) -> u8 {
    match aqi {
        i32::MIN..=50 => 1,
        51..=100 => 2,
        101..=150 => 3,
        151..=200 => 4,
        201..=300 => 5,
        _ => 6,
    }
}

/// Parse an AirNow `aq/data` JSON array. Rows with a negative AQI are monitors reporting no
/// valid value this hour; they're dropped rather than drawn as a green dot.
pub fn parse(json: &str) -> anyhow::Result<Vec<AqiOb>> {
    let rows: Vec<Row> = serde_json::from_str(json)?;
    Ok(rows
        .into_iter()
        .filter(|r| r.aqi >= 0.0)
        .map(|r| {
            let aqi = r.aqi.round() as i32;
            AqiOb {
                lat: r.lat,
                lon: r.lon,
                aqi,
                category: r
                    .category
                    .filter(|c| (1..=6).contains(c))
                    .map(|c| c as u8)
                    .unwrap_or_else(|| category_of(aqi)),
                param: r.parameter.unwrap_or_default(),
                site: r.site_name.unwrap_or_default(),
            }
        })
        .collect())
}

/// Fetch current AQI observations inside `bbox` (`[west, south, east, north]`).
pub async fn fetch_bbox(
    client: &reqwest::Client,
    key: &str,
    bbox: [f64; 4],
) -> anyhow::Result<Vec<AqiOb>> {
    let [w, s, e, n] = bbox;
    // The endpoint wants an explicit hour window; "the last hour, in UTC" is what a live map means.
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::hours(1);
    let url = format!(
        "{BASE}?startDate={}&endDate={}&parameters=OZONE,PM25,PM10&BBOX={w:.4},{s:.4},{e:.4},{n:.4}\
         &dataType=A&format=application/json&verbose=1&monitorType=2&includerawconcentrations=0&API_KEY={key}",
        start.format("%Y-%m-%dT%H"),
        now.format("%Y-%m-%dT%H"),
    );
    let body = client
        .get(crate::net::fetch_url(&url))
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

    #[test]
    fn parses_rows_and_drops_missing() {
        let json = r#"[
          {"Latitude":34.05,"Longitude":-118.24,"UTC":"2026-08-01T18:00","Parameter":"PM2.5",
           "AQI":152,"Category":4,"SiteName":"Los Angeles - N. Main"},
          {"Latitude":34.1,"Longitude":-118.0,"Parameter":"OZONE","AQI":-999,"Category":-1,
           "SiteName":"Broken"},
          {"Latitude":40.0,"Longitude":-105.0,"Parameter":"OZONE","AQI":42}
        ]"#;
        let obs = parse(json).unwrap();
        assert_eq!(obs.len(), 2, "the -999 monitor is dropped");
        assert_eq!(obs[0].aqi, 152);
        assert_eq!(obs[0].category, 4);
        assert_eq!(obs[0].color(), [255, 0, 0]);
        // Category missing → derived from the value.
        assert_eq!(obs[1].category, 1);
        assert_eq!(obs[1].site, "");
    }
}
