//! Synoptic Data's mesonet aggregation — every state, university and DOT network at once.
//!
//! METAR covers airports and the personal-weather networks cover back gardens; between them sit
//! the mesonets — Oklahoma's, West Texas's, every state DOT's roadside kit — which are the
//! stations actually inside the storm. Synoptic aggregates about thirty thousand of them behind
//! one query.
//!
//! Opt-in and keyed, like Tempest and Weather Underground: empty token, no requests. The token is
//! a user secret, so this talks to `api.synopticdata.com` **directly** on every target — the web
//! build's CORS proxy is a shared edge cache, and a key in a cacheable URL is a key stored on
//! someone else's disk. Synoptic sends `Access-Control-Allow-Origin: *`, so direct works.

use crate::alerts::USER_AGENT;
use crate::stations::{Network, StationOb};
use chrono::{DateTime, Utc};

const API: &str = "https://api.synopticdata.com/v2/stations/latest";

/// Feet to metres — `ELEVATION` is in feet whatever the unit system asked for.
const FT_TO_M: f32 = 0.3048;

/// Parse a `/stations/latest` response into station observations.
///
/// Every observation lives under `OBSERVATIONS` as `{var}_value_1: {date_time, value}`, and any
/// of them may be missing on any station — a roadside sensor that reports only temperature and
/// wind is normal, not an error.
pub fn parse_latest(json: &str) -> Vec<StationOb> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(stations) = v.get("STATION").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    stations.iter().filter_map(station).collect()
}

fn station(s: &serde_json::Value) -> Option<StationOb> {
    let obs = s.get("OBSERVATIONS");
    // Values arrive as numbers, but a few networks report them as strings; take either.
    let num = |key: &str| -> Option<f32> {
        let v = obs?.get(format!("{key}_value_1"))?.get("value")?;
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .map(|x| x as f32)
    };
    let str_num = |key: &str| -> Option<f64> {
        let v = s.get(key)?;
        v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    };
    let temp_c = num("air_temp");
    let dewp_c = num("dew_point_temperature");
    let time = obs
        .and_then(|o| o.get("air_temp_value_1"))
        .or_else(|| obs.and_then(|o| o.get("wind_speed_value_1")))
        .and_then(|o| o.get("date_time"))
        .and_then(|t| t.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    Some(StationOb {
        id: s.get("STID")?.as_str()?.to_string(),
        name: s
            .get("NAME")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| s.get("STID").and_then(|x| x.as_str()).unwrap_or("Mesonet"))
            .to_string(),
        network: Network::Synoptic,
        lat: str_num("LATITUDE")?,
        lon: str_num("LONGITUDE")?,
        time,
        temp_c,
        dewp_c,
        rh_pct: num("relative_humidity").or(match (temp_c, dewp_c) {
            (Some(t), Some(d)) => Some(crate::stations::rh_from_dewpoint(t, d)),
            _ => None,
        }),
        wdir_deg: num("wind_direction"),
        wspd_kt: num("wind_speed"),
        gust_kt: num("wind_gust"),
        pressure_mb: num("sea_level_pressure").or_else(|| num("pressure")),
        precip_rate_mmh: None,
        elev_m: str_num("ELEVATION").map(|f| f as f32 * FT_TO_M),
    })
}

/// Stations reporting within `radius_miles` of a point, most recent observation each.
///
/// `limit` caps what comes back: a thirty-mile circle over the Front Range is hundreds of
/// stations, and the cards can only show so many before they stop being readable.
pub async fn fetch_near(
    client: &reqwest::Client,
    token: &str,
    lat: f64,
    lon: f64,
    radius_miles: u32,
    limit: usize,
) -> anyhow::Result<Vec<StationOb>> {
    anyhow::ensure!(!token.is_empty(), "no Synoptic token");
    let radius = format!("{lat:.4},{lon:.4},{radius_miles}");
    let body = client
        // Deliberately not `net::fetch_url`: the token must never enter the shared proxy cache.
        .get(API)
        .query(&[
            ("token", token),
            ("radius", radius.as_str()),
            // Anything older than half an hour is not "live" on a station card.
            ("within", "30"),
            ("status", "active"),
            ("limit", &limit.to_string()),
            (
                "vars",
                "air_temp,dew_point_temperature,relative_humidity,wind_speed,wind_direction,\
                 wind_gust,sea_level_pressure",
            ),
            // Knots and millibars, so nothing downstream has to convert.
            ("units", "metric,speed|kts,pres|mb"),
        ])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_latest(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"STATION":[
      {"STID":"NRMN","NAME":"Norman Mesonet","LATITUDE":"35.2362","LONGITUDE":"-97.4643",
       "ELEVATION":"1180",
       "OBSERVATIONS":{
         "air_temp_value_1":{"date_time":"2026-08-25T18:05:00Z","value":31.5},
         "dew_point_temperature_value_1":{"date_time":"2026-08-25T18:05:00Z","value":21.0},
         "wind_speed_value_1":{"date_time":"2026-08-25T18:05:00Z","value":14.0},
         "wind_gust_value_1":{"date_time":"2026-08-25T18:05:00Z","value":25.0},
         "wind_direction_value_1":{"date_time":"2026-08-25T18:05:00Z","value":170}}},
      {"STID":"ROAD","NAME":"","LATITUDE":"36.0","LONGITUDE":"-98.0","ELEVATION":"1000",
       "OBSERVATIONS":{"wind_speed_value_1":{"date_time":"2026-08-25T18:00:00Z","value":8}}},
      {"NAME":"no id, dropped","LATITUDE":"36.0","LONGITUDE":"-98.0"}
    ]}"#;

    #[test]
    fn a_mesonet_station_parses_with_what_it_reports_and_nothing_it_does_not() {
        let obs = parse_latest(SAMPLE);
        assert_eq!(obs.len(), 2, "the station with no id is dropped");

        let n = &obs[0];
        assert_eq!(n.id, "NRMN");
        assert_eq!(n.network, Network::Synoptic);
        assert_eq!(n.temp_c, Some(31.5));
        assert_eq!(n.gust_kt, Some(25.0));
        // No humidity reported, so it comes from temperature and dewpoint.
        assert!(n.rh_pct.is_some_and(|r| (50.0..60.0).contains(&r)));
        // Elevation is feet on the wire whatever units were asked for.
        assert!(n.elev_m.is_some_and(|m| (355.0..362.0).contains(&m)));
        assert!(n.time.is_some());

        // A wind-only roadside sensor is a normal station, not a parse failure.
        let r = &obs[1];
        assert_eq!(r.name, "ROAD", "an empty name falls back to the id");
        assert_eq!(r.temp_c, None);
        assert_eq!(r.wspd_kt, Some(8.0));
        assert!(r.time.is_some(), "the timestamp can come off any variable");
    }

    #[test]
    fn junk_and_error_responses_come_back_empty_rather_than_panicking() {
        assert!(parse_latest("").is_empty());
        assert!(parse_latest(r#"{"SUMMARY":{"RESPONSE_MESSAGE":"Invalid token"}}"#).is_empty());
    }
}
