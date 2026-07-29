//! Live surface stations for the telemetry cards: airport METARs plus personal weather stations.
//!
//! The station cards want one shape regardless of who reported it, so every network lands in
//! [`StationOb`]: SI-ish metric values, `Option` for anything the network may not carry, and a
//! timestamp so the card can say how old the number on it is.
//!
//! Three networks, and they differ in what they cost the user:
//!
//! * **METAR** (aviationweather.gov) — keyless, nationwide, already fetched for the station-plot
//!   overlay. Reported hourly-ish, so its sparklines step rather than glide.
//! * **WeatherFlow Tempest** — a personal token lists the stations that token can see, then one
//!   call per station for the current observation. Reports every minute.
//! * **Weather Underground PWS** — an API key plus a geocode finds the nearby stations, then one
//!   call per station. Reports every few minutes.
//!
//! Both keyed networks are opt-in and stay empty until the user pastes a key into Settings, same
//! as the Mapbox token.

use crate::alerts::USER_AGENT;
use chrono::{DateTime, TimeZone, Utc};

/// Which network an observation came from. Shown on the card so a one-minute Tempest reading is
/// never mistaken for an hourly airport one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    Metar,
    Tempest,
    WeatherUnderground,
}

impl Network {
    pub fn label(self) -> &'static str {
        match self {
            Network::Metar => "METAR",
            Network::Tempest => "Tempest",
            Network::WeatherUnderground => "Weather Underground",
        }
    }
}

/// One station's current conditions.
#[derive(Debug, Clone, PartialEq)]
pub struct StationOb {
    pub id: String,
    pub name: String,
    pub network: Network,
    pub lat: f64,
    pub lon: f64,
    pub time: Option<DateTime<Utc>>,
    pub temp_c: Option<f32>,
    pub dewp_c: Option<f32>,
    /// Relative humidity (%). Derived from temperature and dewpoint when the network omits it.
    pub rh_pct: Option<f32>,
    pub wdir_deg: Option<f32>,
    pub wspd_kt: Option<f32>,
    pub gust_kt: Option<f32>,
    pub pressure_mb: Option<f32>,
    /// Rain rate (mm/h) — Tempest and WU report it; METAR does not.
    pub precip_rate_mmh: Option<f32>,
    pub elev_m: Option<f32>,
}

impl StationOb {
    /// A stable key for the per-station ring buffer, unique across networks.
    pub fn key(&self) -> String {
        format!("{}:{}", self.network.label(), self.id)
    }
}

/// Relative humidity from temperature and dewpoint (Magnus, over water). Clamped to 0..100 — the
/// formula overshoots slightly when a station reports a dewpoint above its own temperature.
pub fn rh_from_dewpoint(temp_c: f32, dewp_c: f32) -> f32 {
    let e = |t: f32| 6.112 * ((17.67 * t) / (t + 243.5)).exp();
    (100.0 * e(dewp_c) / e(temp_c)).clamp(0.0, 100.0)
}

const MS_TO_KT: f32 = 1.943_844;
const MPH_TO_KT: f32 = 0.868_976;

/// Convert the METAR station plots we already fetch into station cards. Free — no extra request.
pub fn from_metars(obs: &[crate::metar::SurfaceOb]) -> Vec<StationOb> {
    obs.iter()
        .map(|m| StationOb {
            id: m.icao.clone(),
            name: if m.name.is_empty() {
                m.icao.clone()
            } else {
                m.name.clone()
            },
            network: Network::Metar,
            lat: m.lat,
            lon: m.lon,
            time: m.obs_time.and_then(|t| Utc.timestamp_opt(t, 0).single()),
            temp_c: m.temp_c,
            dewp_c: m.dewp_c,
            rh_pct: match (m.temp_c, m.dewp_c) {
                (Some(t), Some(d)) => Some(rh_from_dewpoint(t, d)),
                _ => None,
            },
            wdir_deg: m.wdir_deg,
            wspd_kt: Some(m.wspd_kt),
            gust_kt: m.wgst_kt,
            pressure_mb: m.altim_mb,
            precip_rate_mmh: None,
            elev_m: m.elev_m,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// WeatherFlow Tempest
// ---------------------------------------------------------------------------------------------

const TEMPEST_API: &str = "https://swd.weatherflow.com/swd/rest";

/// A Tempest station from `/stations`: enough to place a marker before any observation arrives.
#[derive(Debug, Clone, PartialEq)]
pub struct TempestStation {
    pub id: u64,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

pub fn parse_tempest_stations(json: &str) -> Vec<TempestStation> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v.get("stations")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some(TempestStation {
                        id: s.get("station_id")?.as_u64()?,
                        name: s
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("Tempest")
                            .to_string(),
                        lat: s.get("latitude")?.as_f64()?,
                        lon: s.get("longitude")?.as_f64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `/observations/station/{id}`. Tempest reports SI: °C, m/s, mb, mm/min.
pub fn parse_tempest_obs(json: &str, station: &TempestStation) -> Option<StationOb> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let o = v.get("obs")?.as_array()?.first()?;
    let f = |k: &str| o.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
    let temp_c = f("air_temperature");
    let dewp_c = f("dew_point");
    Some(StationOb {
        id: station.id.to_string(),
        name: v
            .get("station_name")
            .and_then(|n| n.as_str())
            .unwrap_or(&station.name)
            .to_string(),
        network: Network::Tempest,
        lat: v.get("latitude").and_then(|x| x.as_f64()).unwrap_or(station.lat),
        lon: v
            .get("longitude")
            .and_then(|x| x.as_f64())
            .unwrap_or(station.lon),
        time: o
            .get("timestamp")
            .and_then(|t| t.as_i64())
            .and_then(|t| Utc.timestamp_opt(t, 0).single()),
        temp_c,
        dewp_c,
        rh_pct: f("relative_humidity").or(match (temp_c, dewp_c) {
            (Some(t), Some(d)) => Some(rh_from_dewpoint(t, d)),
            _ => None,
        }),
        wdir_deg: f("wind_direction"),
        wspd_kt: f("wind_avg").map(|v| v * MS_TO_KT),
        gust_kt: f("wind_gust").map(|v| v * MS_TO_KT),
        pressure_mb: f("sea_level_pressure").or(f("station_pressure")),
        // Tempest reports accumulation over the last minute; scale to an hourly rate.
        precip_rate_mmh: f("precip").map(|mm| mm * 60.0),
        elev_m: v
            .get("elevation")
            .and_then(|x| x.as_f64())
            .map(|x| x as f32),
    })
}

/// List the Tempest stations the token can see.
pub async fn tempest_stations(
    client: &reqwest::Client,
    token: &str,
) -> anyhow::Result<Vec<TempestStation>> {
    let body = client
        .get(format!("{TEMPEST_API}/stations"))
        .query(&[("token", token)])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_tempest_stations(&body))
}

/// Current observation for one Tempest station. `None` when the station has reported nothing.
pub async fn tempest_ob(
    client: &reqwest::Client,
    token: &str,
    station: &TempestStation,
) -> anyhow::Result<Option<StationOb>> {
    let body = client
        .get(format!("{TEMPEST_API}/observations/station/{}", station.id))
        .query(&[("token", token)])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_tempest_obs(&body, station))
}

// ---------------------------------------------------------------------------------------------
// Weather Underground PWS
// ---------------------------------------------------------------------------------------------

const WU_API: &str = "https://api.weather.com";

/// Station ids near a point, from the WU location service. WU has no bbox query, so the card
/// layer asks around the view centre and lets the map cull whatever falls outside.
pub fn parse_wu_near(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v.get("location")
        .and_then(|l| l.get("stationId"))
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `/v2/pws/observations/current` (metric units requested, so the `metric` block carries
/// the values).
pub fn parse_wu_ob(json: &str) -> Option<StationOb> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let o = v.get("observations")?.as_array()?.first()?;
    let m = o.get("metric");
    let f = |k: &str| {
        m.and_then(|m| m.get(k))
            .and_then(|x| x.as_f64())
            .map(|x| x as f32)
    };
    let temp_c = f("temp");
    let dewp_c = f("dewpt");
    let rh_pct = o
        .get("humidity")
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .or(match (temp_c, dewp_c) {
            (Some(t), Some(d)) => Some(rh_from_dewpoint(t, d)),
            _ => None,
        });
    Some(StationOb {
        id: o.get("stationID")?.as_str()?.to_string(),
        name: o
            .get("neighborhood")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| o.get("stationID").and_then(|s| s.as_str()).unwrap_or("PWS"))
            .to_string(),
        network: Network::WeatherUnderground,
        lat: o.get("lat")?.as_f64()?,
        lon: o.get("lon")?.as_f64()?,
        time: o
            .get("obsTimeUtc")
            .and_then(|t| t.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc)),
        temp_c,
        dewp_c,
        rh_pct,
        wdir_deg: o.get("winddir").and_then(|x| x.as_f64()).map(|x| x as f32),
        // WU's "metric" block reports wind in km/h.
        wspd_kt: f("windSpeed").map(|v| v * 0.539_957),
        gust_kt: f("windGust").map(|v| v * 0.539_957),
        pressure_mb: f("pressure"),
        precip_rate_mmh: f("precipRate"),
        elev_m: f("elev"),
    })
}

/// Station ids within WU's own idea of "near" a point.
pub async fn wu_stations_near(
    client: &reqwest::Client,
    key: &str,
    lat: f64,
    lon: f64,
) -> anyhow::Result<Vec<String>> {
    let geocode = format!("{lat:.4},{lon:.4}");
    let body = client
        .get(format!("{WU_API}/v3/location/near"))
        .query(&[
            ("geocode", geocode.as_str()),
            ("product", "pws"),
            ("format", "json"),
            ("apiKey", key),
        ])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_wu_near(&body))
}

/// Current conditions at one PWS.
pub async fn wu_ob(
    client: &reqwest::Client,
    key: &str,
    station_id: &str,
) -> anyhow::Result<Option<StationOb>> {
    let body = client
        .get(format!("{WU_API}/v2/pws/observations/current"))
        .query(&[
            ("stationId", station_id),
            ("format", "json"),
            ("units", "m"),
            ("apiKey", key),
        ])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_wu_ob(&body))
}

/// Fetch every configured network for a view, keyless ones included. Errors from one network are
/// logged and skipped — a bad Tempest token must not blank out the METAR cards.
///
/// `tempest` and `wu` are the API keys; empty strings switch those networks off. WU is asked
/// around `(lat, lon)`, the centre of the current view.
pub async fn fetch_all(
    client: &reqwest::Client,
    metars: &[crate::metar::SurfaceOb],
    tempest: &str,
    wu: &str,
    lat: f64,
    lon: f64,
) -> Vec<StationOb> {
    let mut out = from_metars(metars);

    if !tempest.is_empty() {
        match tempest_stations(client, tempest).await {
            Ok(stations) => {
                for s in &stations {
                    match tempest_ob(client, tempest, s).await {
                        Ok(Some(ob)) => out.push(ob),
                        Ok(None) => {}
                        Err(e) => log::warn!("tempest station {}: {e}", s.id),
                    }
                }
            }
            Err(e) => log::warn!("tempest stations: {e}"),
        }
    }

    if !wu.is_empty() {
        match wu_stations_near(client, wu, lat, lon).await {
            // ponytail: WU bills per call, so cap the neighbours; raise it if users ask.
            Ok(ids) => {
                for id in ids.iter().take(12) {
                    match wu_ob(client, wu, id).await {
                        Ok(Some(ob)) => out.push(ob),
                        Ok(None) => {}
                        Err(e) => log::warn!("wu station {id}: {e}"),
                    }
                }
            }
            Err(e) => log::warn!("wu stations near {lat},{lon}: {e}"),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humidity_from_dewpoint_is_sane() {
        // Saturated air: dewpoint equals temperature.
        assert!((rh_from_dewpoint(20.0, 20.0) - 100.0).abs() < 0.01);
        // A 15 °C spread at 35 °C is dry Plains air, not desert.
        let rh = rh_from_dewpoint(35.0, 20.0);
        assert!((38.0..44.0).contains(&rh), "rh was {rh}");
        // A dewpoint above the temperature is bad data, not >100 % humidity.
        assert_eq!(rh_from_dewpoint(10.0, 20.0), 100.0);
    }

    #[test]
    fn metar_plots_become_cards() {
        let json = r#"[{"icaoId":"KJWG","lat":35.863,"lon":-98.423,"temp":35.6,"dewp":15,
          "wdir":140,"wspd":10,"wgst":15,"altim":1010.2,"elev":469,"obsTime":1785287700,
          "name":"Watonga Rgnl, OK, US","fltCat":"VFR","rawOb":"METAR KJWG ..."}]"#;
        let cards = from_metars(&crate::metar::parse(json));
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(c.network, Network::Metar);
        assert_eq!(c.key(), "METAR:KJWG");
        assert_eq!(c.gust_kt, Some(15.0));
        assert_eq!(c.pressure_mb, Some(1010.2));
        assert_eq!(c.time.unwrap().timestamp(), 1785287700);
        // Humidity is derived, since METAR never reports it directly: 35.6 °C over a 15 °C
        // dewpoint is dry Plains air, near 30 %.
        let rh = c.rh_pct.unwrap();
        assert!((28.0..32.0).contains(&rh), "rh was {rh}");
    }

    #[test]
    fn parses_tempest_station_and_observation() {
        let stations = parse_tempest_stations(
            r#"{"stations":[{"station_id":4242,"name":"Backyard","latitude":34.9,"longitude":-84.3}]}"#,
        );
        assert_eq!(stations.len(), 1);
        let ob = parse_tempest_obs(
            r#"{"station_id":4242,"station_name":"Backyard","latitude":34.9,"longitude":-84.3,
              "elevation":412.0,"obs":[{"timestamp":1785287700,"air_temperature":26.5,
              "relative_humidity":74,"dew_point":21.6,"wind_avg":3.1,"wind_gust":7.2,
              "wind_direction":190,"sea_level_pressure":1012.4,"precip":0.5}]}"#,
            &stations[0],
        )
        .unwrap();
        assert_eq!(ob.key(), "Tempest:4242");
        assert_eq!(ob.rh_pct, Some(74.0));
        // m/s in, knots out.
        assert!((ob.wspd_kt.unwrap() - 6.03).abs() < 0.01);
        assert!((ob.gust_kt.unwrap() - 13.99).abs() < 0.01);
        // 0.5 mm in a minute is 30 mm/h.
        assert_eq!(ob.precip_rate_mmh, Some(30.0));
    }

    #[test]
    fn parses_wu_neighbours_and_observation() {
        assert_eq!(
            parse_wu_near(r#"{"location":{"stationId":["KGAELLIJ12","KGAJASPE7"]}}"#),
            vec!["KGAELLIJ12", "KGAJASPE7"]
        );
        let ob = parse_wu_ob(
            r#"{"observations":[{"stationID":"KGAELLIJ12","neighborhood":"Silver City",
              "obsTimeUtc":"2026-07-29T01:35:00Z","lat":34.75,"lon":-84.42,"humidity":81,
              "winddir":175,"metric":{"temp":24.0,"dewpt":20.6,"windSpeed":9.7,"windGust":16.1,
              "pressure":1011.9,"precipRate":2.5,"elev":430}}]}"#,
        )
        .unwrap();
        assert_eq!(ob.key(), "Weather Underground:KGAELLIJ12");
        assert_eq!(ob.name, "Silver City");
        assert_eq!(ob.rh_pct, Some(81.0));
        // km/h in, knots out.
        assert!((ob.wspd_kt.unwrap() - 5.24).abs() < 0.01);
        assert_eq!(ob.precip_rate_mmh, Some(2.5));
        // A payload without observations is a quiet station, not a panic.
        assert!(parse_wu_ob(r#"{"observations":[]}"#).is_none());
    }
}
