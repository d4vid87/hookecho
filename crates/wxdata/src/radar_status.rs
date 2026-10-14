//! Public NWS radar-station status, separate from the scan displayed in a pane.
//!
//! The API reports RDA operating state and when the Level II distribution system last
//! received data. Neither field proves that HookEcho has rendered that latest scan.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

const URL: &str = "https://api.weather.gov/radar/stations";

#[derive(Debug, Clone)]
pub struct StationStatus {
    pub rda_state: Option<String>,
    pub rda_time: Option<DateTime<Utc>>,
    pub level_two_received: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub fetched_at: DateTime<Utc>,
    pub stations: HashMap<String, StationStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Recent,
    Delayed,
    Stale,
    NotOperating,
    Unknown,
}

impl Report {
    /// A report is allowed two missed refreshes before every pin returns to unknown.
    pub fn availability(&self, id: &str, now: DateTime<Utc>) -> Availability {
        if now.signed_duration_since(self.fetched_at) > Duration::minutes(5) {
            return Availability::Unknown;
        }
        let Some(station) = self.stations.get(id) else {
            return Availability::Unknown;
        };
        let rda_current = station.rda_time.is_some_and(|time| {
            let age = now.signed_duration_since(time);
            age >= Duration::minutes(-2) && age <= Duration::minutes(30)
        });
        if rda_current
            && matches!(station.rda_state.as_deref(),
                Some("Offline" | "Standby" | "Restart" | "Start-Up"))
        {
            return Availability::NotOperating;
        }
        let Some(received) = station.level_two_received else {
            return Availability::Unknown;
        };
        let age = now.signed_duration_since(received);
        if age < Duration::minutes(-2) {
            return Availability::Unknown;
        }
        if age <= Duration::minutes(10) {
            Availability::Recent
        } else if age <= Duration::minutes(25) {
            Availability::Delayed
        } else {
            Availability::Stale
        }
    }
}

pub async fn fetch(client: &reqwest::Client) -> anyhow::Result<Report> {
    let body = client
        .get(crate::net::fetch_url(URL))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body, Utc::now())
}

pub fn parse(body: &str, fetched_at: DateTime<Utc>) -> anyhow::Result<Report> {
    let json: serde_json::Value = serde_json::from_str(body)?;
    let features = json["features"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("NWS radar-stations response has no features"))?;
    let mut stations = HashMap::with_capacity(features.len());
    for feature in features {
        let properties = &feature["properties"];
        let Some(id) = properties["id"].as_str().filter(|id| id.len() == 4) else {
            continue;
        };
        let rda_state = properties["rda"]["properties"]["status"]
            .as_str()
            .map(str::to_owned);
        let rda_time = properties["rda"]["timestamp"]
            .as_str()
            .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
            .map(|time| time.with_timezone(&Utc));
        let level_two_received = properties["latency"]["levelTwoLastReceivedTime"]
            .as_str()
            .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
            .map(|time| time.with_timezone(&Utc));
        stations.insert(id.to_owned(), StationStatus { rda_state, rda_time, level_two_received });
    }
    Ok(Report { fetched_at, stations })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_rda_and_receipt_time_without_guessing_missing_values() {
        let now = DateTime::parse_from_rfc3339("2026-10-14T23:30:00Z").unwrap().to_utc();
        let body = r#"{"features":[
            {"properties":{"id":"KTLX","rda":{"properties":{"status":"Operate"}},"latency":{"levelTwoLastReceivedTime":"2026-10-14T23:27:00Z"}}},
            {"properties":{"id":"KFWS","rda":{"timestamp":"2026-10-14T23:25:00Z","properties":{"status":"Start-Up"}},"latency":{"levelTwoLastReceivedTime":"2026-10-14T23:28:00Z"}}},
            {"properties":{"id":"KMUX","rda":{"timestamp":"2026-10-14T16:14:00Z","properties":{"status":"Start-Up"}},"latency":{"levelTwoLastReceivedTime":"2026-10-14T23:28:00Z"}}},
            {"properties":{"id":"KICT","rda":{"properties":{"status":"Operate"}},"latency":{"levelTwoLastReceivedTime":"2026-10-14T23:15:00Z"}}},
            {"properties":{"id":"KMRX","rda":{"properties":{"status":"Operate"}},"latency":{"levelTwoLastReceivedTime":"2026-10-14T22:55:00Z"}}},
            {"properties":{"id":"KJAX","rda":null,"latency":{}}}
        ]}"#;
        let report = parse(body, now).unwrap();
        assert_eq!(report.availability("KTLX", now), Availability::Recent);
        assert_eq!(report.availability("KFWS", now), Availability::NotOperating);
        assert_eq!(report.availability("KMUX", now), Availability::Recent);
        assert_eq!(report.availability("KICT", now), Availability::Delayed);
        assert_eq!(report.availability("KMRX", now), Availability::Stale);
        assert_eq!(report.availability("KJAX", now), Availability::Unknown);
        assert_eq!(report.availability("XXXX", now), Availability::Unknown);
        assert_eq!(report.availability("KTLX", now + Duration::minutes(6)), Availability::Unknown);
    }
}
