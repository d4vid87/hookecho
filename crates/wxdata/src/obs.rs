//! Surface observations from the nearest NWS/METAR station (api.weather.gov).
//!
//! `/points/{lat},{lon}` -> `observationStations` -> first station -> its 24h observation series.
//! All values are SI (degC, km/h, Pa, %) and any single value may be null (QC-flagged), so every
//! field is `Option<f32>`. `/points` can 404 offshore/OCONUS, surfaced as an error to the caller.

use crate::alerts::USER_AGENT;

const API: &str = "https://api.weather.gov";

/// One observation record (SI units; missing/QC-rejected values are `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub time: Option<chrono::DateTime<chrono::Utc>>,
    pub temp_c: Option<f32>,
    pub dewpoint_c: Option<f32>,
    pub rh: Option<f32>,
    pub wind_kmh: Option<f32>,
    pub gust_kmh: Option<f32>,
    pub wind_dir_deg: Option<f32>,
    pub pressure_pa: Option<f32>,
    pub slp_pa: Option<f32>,
}

/// A station and its recent observation series (newest first).
#[derive(Debug, Clone)]
pub struct StationObs {
    pub station_id: String,
    pub name: String,
    pub obs: Vec<Observation>,
}

/// `properties.<field>.value` as f32, tolerating null / QC-rejected values.
fn val(props: &serde_json::Value, field: &str) -> Option<f32> {
    props.get(field)?.get("value")?.as_f64().map(|v| v as f32)
}

/// Parse an `/observations` GeoJSON FeatureCollection into records (order preserved: newest first).
pub fn parse_observations(json: &str) -> anyhow::Result<Vec<Observation>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let feats = v.get("features").and_then(|f| f.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for f in &feats {
        let Some(p) = f.get("properties") else { continue };
        out.push(Observation {
            time: p
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc)),
            temp_c: val(p, "temperature"),
            dewpoint_c: val(p, "dewpoint"),
            rh: val(p, "relativeHumidity"),
            wind_kmh: val(p, "windSpeed"),
            gust_kmh: val(p, "windGust"),
            wind_dir_deg: val(p, "windDirection"),
            pressure_pa: val(p, "barometricPressure"),
            slp_pa: val(p, "seaLevelPressure"),
        });
    }
    Ok(out)
}

/// Fetch the nearest station's ~24h observation series for `(lat, lon)`.
///
/// `// ponytail: first station only; a nearest-N picker can follow if the closest is stale.`
pub async fn fetch_nearest(http: &reqwest::Client, lat: f64, lon: f64) -> anyhow::Result<StationObs> {
    let get = |url: String| {
        let http = http.clone();
        async move {
            http.get(&url)
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

    let points = get(format!("{API}/points/{lat:.4},{lon:.4}")).await?;
    let pv: serde_json::Value = serde_json::from_str(&points)?;
    let stations_url = pv
        .pointer("/properties/observationStations")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no observation stations for this point"))?
        .to_string();

    let stations = get(stations_url).await?;
    let sv: serde_json::Value = serde_json::from_str(&stations)?;
    let first = sv
        .pointer("/features/0/properties")
        .ok_or_else(|| anyhow::anyhow!("no nearby station"))?;
    let station_id = first
        .get("stationIdentifier")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("station has no id"))?
        .to_string();
    let name = first.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let obs_json = get(format!("{API}/stations/{station_id}/observations?limit=72")).await?;
    let obs = parse_observations(&obs_json)?;
    Ok(StationObs { station_id, name, obs })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBS: &str = r#"{"type":"FeatureCollection","features":[
        {"properties":{"timestamp":"2026-07-18T00:25:00+00:00",
          "temperature":{"unitCode":"wmoUnit:degC","value":24,"qualityControl":"V"},
          "dewpoint":{"unitCode":"wmoUnit:degC","value":22,"qualityControl":"V"},
          "relativeHumidity":{"unitCode":"wmoUnit:percent","value":88.6},
          "windSpeed":{"unitCode":"wmoUnit:km_h-1","value":11.16},
          "windGust":{"unitCode":"wmoUnit:km_h-1","value":null,"qualityControl":"Z"},
          "windDirection":{"unitCode":"wmoUnit:degree_(angle)","value":110},
          "barometricPressure":{"unitCode":"wmoUnit:Pa","value":101730},
          "seaLevelPressure":{"unitCode":"wmoUnit:Pa","value":101620}}}]}"#;

    #[test]
    fn parses_and_skips_nulls() {
        let o = parse_observations(OBS).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].temp_c, Some(24.0));
        assert_eq!(o[0].dewpoint_c, Some(22.0));
        assert_eq!(o[0].wind_kmh, Some(11.16));
        assert_eq!(o[0].gust_kmh, None, "null gust -> None");
        assert_eq!(o[0].wind_dir_deg, Some(110.0));
        assert_eq!(o[0].pressure_pa, Some(101730.0));
        assert!(o[0].time.is_some());
    }
}
