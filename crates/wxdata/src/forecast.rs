//! NWS point forecast: the plain "what's the weather going to do here" product.
//!
//! Everything else in this app answers a radar question. This one answers the question a person
//! actually asks when they open a weather app, and it's the only NWS endpoint we use that returns
//! a forecast rather than an observation or a warning.
//!
//! The NWS grid stops at the US border, so when `/points` doesn't answer, [`fetch`] falls back to
//! the global [`crate::openmeteo`] forecast. That also covers a transient NWS 500 for a US point,
//! which now yields somebody's forecast instead of an error — the `office` field says which.

use crate::alerts::USER_AGENT;
use chrono::{DateTime, Utc};

const API: &str = "https://api.weather.gov";

/// One forecast period — a named half-day for the daily view, or one hour for the hourly strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Period {
    pub start: DateTime<Utc>,
    /// "This Afternoon", "Tuesday Night" — empty for hourly periods.
    pub name: String,
    pub temp_f: f32,
    /// Chance of precipitation, when the grid publishes one.
    pub precip_pct: Option<u8>,
    /// "Scattered Showers And Thunderstorms".
    pub short: String,
    /// "10 to 15 mph".
    pub wind: String,
    pub is_day: bool,
}

/// A point forecast: named periods out ~7 days, plus hour-by-hour for the near term.
#[derive(Debug, Clone, Default)]
pub struct PointForecast {
    /// Which office and grid cell answered — worth showing, it's the provenance.
    pub office: String,
    pub daily: Vec<Period>,
    pub hourly: Vec<Period>,
}

fn parse_periods(body: &str) -> anyhow::Result<Vec<Period>> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    let arr = v
        .pointer("/properties/periods")
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow::anyhow!("forecast has no periods"))?;
    Ok(arr
        .iter()
        .filter_map(|p| {
            let start = p
                .get("startTime")
                .and_then(|s| s.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())?
                .with_timezone(&Utc);
            Some(Period {
                start,
                name: p
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string(),
                // Hourly periods are always °F from this API; the unit field says so explicitly.
                temp_f: p
                    .get("temperature")
                    .and_then(|t| t.as_f64())
                    .unwrap_or(f64::NAN) as f32,
                precip_pct: p
                    .pointer("/probabilityOfPrecipitation/value")
                    .and_then(|x| x.as_u64())
                    .map(|x| x as u8),
                short: p
                    .get("shortForecast")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string(),
                wind: match (
                    p.get("windSpeed").and_then(|s| s.as_str()),
                    p.get("windDirection").and_then(|s| s.as_str()),
                ) {
                    (Some(s), Some(d)) if !d.is_empty() => format!("{d} {s}"),
                    (Some(s), _) => s.to_string(),
                    _ => String::new(),
                },
                is_day: p.get("isDaytime").and_then(|b| b.as_bool()).unwrap_or(true),
            })
        })
        .collect())
}

/// Fetch the forecast for `(lat, lon)`: `/points` resolves the grid cell, then the two forecast
/// URLs it hands back. Two round trips, same shape as [`crate::obs::fetch_nearest`].
pub async fn fetch(http: &reqwest::Client, lat: f64, lon: f64) -> anyhow::Result<PointForecast> {
    let get = |url: String| {
        let http = http.clone();
        async move {
            http.get(crate::net::fetch_url(&url))
                .header("User-Agent", USER_AGENT)
                .header("Accept", "application/geo+json")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await
                .map_err(anyhow::Error::from)
        }
    };

    let points = match get(format!("{API}/points/{lat:.4},{lon:.4}")).await {
        Ok(body) => body,
        Err(e) => {
            log::info!("NWS has no forecast for {lat:.3},{lon:.3} ({e}); using Open-Meteo");
            return crate::openmeteo::fetch(http, lat, lon).await;
        }
    };
    let pv: serde_json::Value = serde_json::from_str(&points)?;
    let str_at = |ptr: &str| pv.pointer(ptr).and_then(|v| v.as_str()).map(str::to_string);
    let Some(daily_url) = str_at("/properties/forecast") else {
        log::info!("NWS point {lat:.3},{lon:.3} carries no forecast grid; using Open-Meteo");
        return crate::openmeteo::fetch(http, lat, lon).await;
    };
    let hourly_url = str_at("/properties/forecastHourly");
    // Provenance the window prints verbatim, so it has to name the provider, not just the office.
    let office = match str_at("/properties/cwa") {
        Some(cwa) if !cwa.is_empty() => format!("NWS {cwa}"),
        _ => "NWS".to_string(),
    };

    let daily = parse_periods(&get(daily_url).await?)?;
    // The hourly grid 500s more often than the daily one; a missing strip shouldn't sink the
    // whole window.
    let hourly = match hourly_url {
        Some(u) => match get(u).await {
            Ok(body) => parse_periods(&body).unwrap_or_default(),
            Err(e) => {
                log::warn!("hourly forecast unavailable: {e}");
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    Ok(PointForecast {
        office,
        daily,
        hourly,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "properties": {
        "periods": [
          {
            "number": 1,
            "name": "This Afternoon",
            "startTime": "2026-07-25T15:00:00-05:00",
            "isDaytime": true,
            "temperature": 96,
            "temperatureUnit": "F",
            "probabilityOfPrecipitation": { "unitCode": "wmoUnit:percent", "value": 40 },
            "windSpeed": "10 to 15 mph",
            "windDirection": "S",
            "shortForecast": "Scattered Showers And Thunderstorms"
          },
          {
            "number": 2,
            "name": "Tonight",
            "startTime": "2026-07-25T19:00:00-05:00",
            "isDaytime": false,
            "temperature": 74,
            "temperatureUnit": "F",
            "probabilityOfPrecipitation": { "unitCode": "wmoUnit:percent", "value": null },
            "windSpeed": "5 mph",
            "windDirection": "",
            "shortForecast": "Mostly Clear"
          }
        ]
      }
    }"#;

    #[test]
    fn parses_periods() {
        let p = parse_periods(SAMPLE).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "This Afternoon");
        assert_eq!(p[0].temp_f, 96.0);
        assert_eq!(p[0].precip_pct, Some(40));
        assert_eq!(p[0].wind, "S 10 to 15 mph");
        assert!(p[0].is_day);
        assert_eq!(p[0].short, "Scattered Showers And Thunderstorms");
    }

    #[test]
    fn null_precip_and_blank_direction_survive() {
        let p = parse_periods(SAMPLE).unwrap();
        assert_eq!(
            p[1].precip_pct, None,
            "null probability is absent, not zero"
        );
        assert_eq!(p[1].wind, "5 mph", "no direction, no leading space");
        assert!(!p[1].is_day);
    }

    #[test]
    fn start_times_are_utc_normalized() {
        let p = parse_periods(SAMPLE).unwrap();
        assert_eq!(p[0].start.to_rfc3339(), "2026-07-25T20:00:00+00:00");
    }

    #[test]
    fn missing_periods_is_an_error_not_a_panic() {
        assert!(parse_periods("{}").is_err());
        assert!(parse_periods("not json").is_err());
    }
}
