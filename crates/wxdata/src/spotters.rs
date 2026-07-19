//! Live storm-spotter positions from Spotter Network's public GRLevelX placefile.
//!
//! Feed: <https://www.spotternetwork.org/feeds/gr.txt> — public, no key, 1-minute refresh,
//! ~1400 spotters CONUS-wide. The generic [`crate::placefile`] parser deliberately skips
//! `Object:` blocks, so this feed needs a dedicated parser.
//!
//! Privacy: many entries embed an `Email: …` line inside the icon hover text. Those segments
//! are stripped at parse time and never stored, so no address can reach the UI.

use chrono::{DateTime, NaiveDateTime, Utc};

const SPOTTERS_URL: &str = "https://www.spotternetwork.org/feeds/gr.txt";

/// One reporting spotter's current position.
#[derive(Debug, Clone)]
pub struct Spotter {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Last report time (UTC).
    pub time: DateTime<Utc>,
    /// `Some(deg)` when moving (arrow icon), heading clockwise from north; `None` when stationary.
    pub heading: Option<f32>,
    /// Free-text status line ("STATIONARY", "25 mph NE", …).
    pub status: String,
}

/// Fetch and parse the current Spotter Network positions.
pub async fn fetch_spotters(client: &reqwest::Client) -> anyhow::Result<Vec<Spotter>> {
    let body = client
        .get(SPOTTERS_URL)
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_spotters(&body))
}

/// In-progress spotter block accumulated across an `Object:`…`End:` span.
#[derive(Default)]
struct Acc {
    coords: Option<(f64, f64)>,
    heading: Option<f32>,
    name: Option<String>,
    time: Option<DateTime<Utc>>,
    status: String,
}

/// Parse the placefile text into spotters. A line state machine over `Object:` / `Icon:` / `End:`.
pub fn parse_spotters(text: &str) -> Vec<Spotter> {
    let mut out = Vec::new();
    let mut acc = Acc::default();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Object:") {
            acc = Acc::default();
            let mut it = rest.split(',');
            if let (Some(la), Some(lo)) = (it.next(), it.next()) {
                if let (Ok(lat), Ok(lon)) = (la.trim().parse(), lo.trim().parse()) {
                    acc.coords = Some((lat, lon));
                }
            }
        } else if let Some(rest) = line.strip_prefix("Icon:") {
            if acc.coords.is_none() {
                continue;
            }
            // Fields: 0,0,<rot>,<sheet>,<idx>[,"hover"].
            let sheet = rest.split(',').nth(3).and_then(|s| s.trim().parse::<u32>().ok());
            let rot = rest.split(',').nth(2).and_then(|s| s.trim().parse::<f32>().ok());
            if sheet == Some(2) {
                // Movement arrow: its rotation is the heading.
                acc.heading = rot;
                continue;
            }
            // Identity icon: parse the quoted hover text.
            if let (Some(a), Some(b)) = (rest.find('"'), rest.rfind('"')) {
                if b > a {
                    let segs: Vec<&str> = rest[a + 1..b]
                        .split("\\n")
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && !s.starts_with("Email:"))
                        .collect();
                    if let Some(n) = segs.first() {
                        acc.name = Some(n.to_string());
                    }
                    if let Some(t) = segs.get(1) {
                        let t = t.trim_end_matches(" UTC");
                        acc.time = NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S")
                            .ok()
                            .map(|dt| dt.and_utc());
                    }
                    // Only segment 3 is the movement status ("STATIONARY"/"25 mph NE"). Any later
                    // segments (Phone/Twitter/Ham/Web/Email/…) are personal contact info — never kept.
                    if let Some(st) = segs.get(2) {
                        acc.status = st.to_string();
                    }
                }
            }
        } else if line.starts_with("End:") {
            if let (Some((lat, lon)), Some(name), Some(time)) =
                (acc.coords, acc.name.take(), acc.time)
            {
                out.push(Spotter {
                    name,
                    lat,
                    lon,
                    time,
                    heading: acc.heading,
                    status: std::mem::take(&mut acc.status),
                });
            }
            acc = Acc::default();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spotters_and_strips_email() {
        let feed = "Refresh: 1\n\
            Object: 44.7412376,-89.0691071\n\
            Icon: 0,0,000,6,10,\"Jane Doe\\n2026-07-18 22:54:53 UTC\\nSTATIONARY\\nPhone: 5551234567\\nTwitter: https://twitter.com/@jane\\nEmail: jane@example.com\"\n\
            Text: 15, 10, 1, \"Jane Doe\"\n\
            End:\n\
            Object: 38.3074760,-89.0871582\n\
            Icon: 0,0,225,2,15,\n\
            Icon: 0,0,000,6,10,\"Sam Rider\\n2026-07-18 21:57:52 UTC\\nMOVING\"\n\
            Text: 15, 10, 1, \"Sam Rider\"\n\
            End:\n\
            Object: 41.6641617,-96.6784897\n\
            Icon: 0,0,000,6,10,\"Pat Smith\\n2026-07-18 20:00:00 UTC\\nSTATIONARY\"\n\
            End:\n";
        let s = parse_spotters(feed);
        assert_eq!(s.len(), 3);

        assert_eq!(s[0].name, "Jane Doe");
        assert!((s[0].lat - 44.7412376).abs() < 1e-6);
        assert!((s[0].lon - -89.0691071).abs() < 1e-6);
        assert_eq!(s[0].heading, None);
        assert_eq!(s[0].time.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-07-18 22:54:53");
        // Status is only the movement line — no phone/twitter/email contact info.
        assert_eq!(s[0].status, "STATIONARY");

        assert_eq!(s[1].name, "Sam Rider");
        assert_eq!(s[1].heading, Some(225.0), "mover arrow (sheet 2) sets heading");

        assert_eq!(s[2].name, "Pat Smith");

        // No contact PII (email @, twitter handle, phone digits) may survive parsing.
        let dump = format!("{s:?}");
        assert!(!dump.contains('@'), "email/handle leaked into parsed spotters");
        assert!(!dump.contains("5551234567"), "phone leaked into parsed spotters");
    }
}
