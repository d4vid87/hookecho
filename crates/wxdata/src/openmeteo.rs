//! Open-Meteo point forecast — the forecast for everywhere the NWS doesn't cover.
//!
//! [`crate::forecast`] is the NWS gridpoint product, which stops at the US border: tap the map on
//! London and `/points` 404s. Open-Meteo serves a global forecast from ECMWF/GFS/ICON with no key
//! and no account, so this fills that hole without asking the user for anything.
//!
//! Deliberately returns [`crate::forecast::PointForecast`] rather than a type of its own — the
//! window renders whichever provider answered, and there is nothing about the shape of a forecast
//! that differs between them. What does differ is vocabulary: Open-Meteo reports a numeric WMO
//! weather code where the NWS writes prose, and reports wind as a number and a bearing where the
//! NWS writes "S 10 to 15 mph". Both are translated here so the UI stays provider-blind.

use crate::forecast::{Period, PointForecast};
use chrono::{TimeZone, Utc};

const BASE: &str = "https://api.open-meteo.com/v1/forecast";

/// Parse an Open-Meteo response into the shared forecast type.
pub fn parse_forecast(body: &str) -> anyhow::Result<PointForecast> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    if let Some(reason) = v.get("reason").and_then(|r| r.as_str()) {
        anyhow::bail!("open-meteo: {reason}");
    }

    let daily = v
        .get("daily")
        .ok_or_else(|| anyhow::anyhow!("no daily forecast in response"))?;
    let times = arr_i64(daily, "time");
    let temps = arr_f64(daily, "temperature_2m_max");
    let precip = arr_f64(daily, "precipitation_probability_max");
    let codes = arr_f64(daily, "weather_code");
    let wind = arr_f64(daily, "wind_speed_10m_max");
    let wind_dir = arr_f64(daily, "wind_direction_10m_dominant");

    let mut out = Vec::with_capacity(times.len());
    for (i, t) in times.iter().enumerate() {
        let Some(start) = Utc.timestamp_opt(*t, 0).single() else {
            continue;
        };
        out.push(Period {
            start,
            // Open-Meteo's daily rows are calendar days, so the weekday is the name; the NWS's
            // "This Afternoon" / "Tuesday Night" split doesn't exist here.
            name: start.format("%A").to_string(),
            temp_f: temps.get(i).copied().unwrap_or(f64::NAN) as f32,
            precip_pct: precip.get(i).map(|p| *p as u8),
            short: wmo_label(codes.get(i).copied().unwrap_or(-1.0)).to_string(),
            wind: match (wind.get(i), wind_dir.get(i)) {
                (Some(s), Some(d)) => format!("{} {s:.0} mph", compass(*d)),
                (Some(s), None) => format!("{s:.0} mph"),
                _ => String::new(),
            },
            wind_mph: wind.get(i).map(|s| *s as f32),
            wind_deg: wind_dir.get(i).map(|d| *d as f32),
            // A daily row covers a whole day; the window colors it as a day period.
            is_day: true,
        });
    }

    let hourly = v.get("hourly").map(parse_hourly).unwrap_or_default();

    Ok(PointForecast {
        office: "Open-Meteo".to_string(),
        daily: out,
        hourly,
    })
}

/// Hourly rows, trimmed to the ones still ahead — the response starts at midnight local, and the
/// strip is meant to read "next 24 hours" like the NWS one does.
fn parse_hourly(hourly: &serde_json::Value) -> Vec<Period> {
    let times = arr_i64(hourly, "time");
    let temps = arr_f64(hourly, "temperature_2m");
    let precip = arr_f64(hourly, "precipitation_probability");
    let wind = arr_f64(hourly, "wind_speed_10m");
    let wind_dir = arr_f64(hourly, "wind_direction_10m");
    let now = Utc::now().timestamp();
    times
        .iter()
        .enumerate()
        .filter(|(_, t)| **t >= now - 3600)
        .filter_map(|(i, t)| {
            Some(Period {
                start: Utc.timestamp_opt(*t, 0).single()?,
                name: String::new(),
                temp_f: temps.get(i).copied().unwrap_or(f64::NAN) as f32,
                precip_pct: precip.get(i).map(|p| *p as u8),
                short: String::new(),
                wind: match (wind.get(i), wind_dir.get(i)) {
                    (Some(s), Some(d)) => format!("{} {s:.0} mph", compass(*d)),
                    (Some(s), None) => format!("{s:.0} mph"),
                    _ => String::new(),
                },
                wind_mph: wind.get(i).map(|s| *s as f32),
                wind_deg: wind_dir.get(i).map(|d| *d as f32),
                is_day: true,
            })
        })
        .collect()
}

fn arr_f64(v: &serde_json::Value, key: &str) -> Vec<f64> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
        .unwrap_or_default()
}

fn arr_i64(v: &serde_json::Value, key: &str) -> Vec<i64> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default()
}

/// 16-point compass label for a bearing in degrees.
fn compass(deg: f64) -> &'static str {
    const D: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    D[((deg / 22.5).round() as usize) % 16]
}

/// WMO 4677 present-weather code as the phrase the forecast window prints. Codes Open-Meteo
/// doesn't emit are simply absent, so anything unmatched reads as an empty short forecast rather
/// than as a guess.
fn wmo_label(code: f64) -> &'static str {
    match code as i32 {
        0 => "Clear",
        1 => "Mainly Clear",
        2 => "Partly Cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 => "Light Drizzle",
        53 => "Drizzle",
        55 => "Heavy Drizzle",
        56 | 57 => "Freezing Drizzle",
        61 => "Light Rain",
        63 => "Rain",
        65 => "Heavy Rain",
        66 | 67 => "Freezing Rain",
        71 => "Light Snow",
        73 => "Snow",
        75 => "Heavy Snow",
        77 => "Snow Grains",
        80 => "Light Showers",
        81 => "Showers",
        82 => "Heavy Showers",
        85 => "Light Snow Showers",
        86 => "Snow Showers",
        95 => "Thunderstorms",
        96 | 99 => "Thunderstorms With Hail",
        _ => "",
    }
}

/// Fetch the global forecast for `(lat, lon)`. One request, no key.
pub async fn fetch(http: &reqwest::Client, lat: f64, lon: f64) -> anyhow::Result<PointForecast> {
    // Fahrenheit/mph so the shared `Period` needs no per-provider unit flag; unixtime in UTC so
    // parsing is a timestamp rather than a local-time string with an implied zone.
    let url = format!(
        "{BASE}?latitude={lat:.4}&longitude={lon:.4}\
         &temperature_unit=fahrenheit&wind_speed_unit=mph&timeformat=unixtime&timezone=UTC\
         &hourly=temperature_2m,precipitation_probability,wind_speed_10m,wind_direction_10m\
         &daily=temperature_2m_max,precipitation_probability_max,weather_code,\
wind_speed_10m_max,wind_direction_10m_dominant&forecast_days=7"
    );
    let body = http
        .get(crate::net::fetch_url(&url))
        .timeout(crate::net::FEED_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_forecast(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "latitude": 51.5,
      "longitude": -0.12,
      "timezone": "GMT",
      "hourly": {
        "time": [4102444800, 4102448400],
        "temperature_2m": [48.2, 47.1],
        "precipitation_probability": [10, 35]
      },
      "daily": {
        "time": [4102444800, 4102531200],
        "temperature_2m_max": [54.3, 49.8],
        "precipitation_probability_max": [40, 0],
        "weather_code": [61, 3],
        "wind_speed_10m_max": [12.4, 7.6],
        "wind_direction_10m_dominant": [202, 350]
      }
    }"#;

    #[test]
    fn daily_rows_read_like_the_nws_ones() {
        let f = parse_forecast(SAMPLE).unwrap();
        assert_eq!(f.office, "Open-Meteo");
        assert_eq!(f.daily.len(), 2);
        assert_eq!(f.daily[0].temp_f, 54.3);
        assert_eq!(f.daily[0].precip_pct, Some(40));
        assert_eq!(f.daily[0].short, "Light Rain");
        // Same "DIR speed unit" shape the NWS parser produces.
        assert_eq!(f.daily[0].wind, "SSW 12 mph");
        assert_eq!(f.daily[1].wind, "N 8 mph");
        assert_eq!(f.daily[1].short, "Overcast");
        // The name is the weekday: 4102444800 is 2100-01-01, a Friday.
        assert_eq!(f.daily[0].name, "Friday");
    }

    #[test]
    fn hourly_rows_in_the_future_survive_the_now_filter() {
        // The fixture's timestamps are in 2100, so nothing is filtered out.
        let f = parse_forecast(SAMPLE).unwrap();
        assert_eq!(f.hourly.len(), 2);
        assert_eq!(f.hourly[0].temp_f, 48.2);
        assert_eq!(f.hourly[1].precip_pct, Some(35));
        assert!(f.hourly[0].name.is_empty(), "hourly periods are unnamed");
    }

    #[test]
    fn compass_covers_the_wrap() {
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(359.0), "N");
        assert_eq!(compass(180.0), "S");
        assert_eq!(compass(225.0), "SW");
        assert_eq!(compass(202.5), "SSW");
    }

    #[test]
    fn unknown_weather_codes_say_nothing_rather_than_guess() {
        assert_eq!(wmo_label(95.0), "Thunderstorms");
        assert_eq!(wmo_label(-1.0), "");
        assert_eq!(wmo_label(7.0), "");
    }

    #[test]
    fn an_error_response_is_an_error() {
        let body = r#"{"error": true, "reason": "Latitude must be in range of -90 to 90"}"#;
        assert!(parse_forecast(body).is_err());
        assert!(parse_forecast("{}").is_err());
        assert!(parse_forecast("not json").is_err());
    }

    #[test]
    #[ignore = "network"]
    fn live_london_forecast() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let f = rt.block_on(async {
            let http = reqwest::Client::new();
            fetch(&http, 51.5074, -0.1278).await.unwrap()
        });
        assert!(!f.daily.is_empty() && !f.hourly.is_empty());
        assert_eq!(f.office, "Open-Meteo");
    }
}
