//! Warning verification: did the warnings verify, and how much lead time did they give?
//!
//! The matching is IEM's "Cow" (Command Of Warnings) service, not ours. Deciding whether a storm
//! report verifies a polygon is a pile of judgement calls — a 15 km report buffer, shared county
//! borders, hail-size and wind-speed floors, the SVR-with-tornado-possible carve-out — and IEM
//! has been maintaining that arbitration, publicly and for the whole warning archive, for years.
//! Reimplementing it would produce numbers that disagree with everyone else's for no benefit.
//!
//! So this module asks a question and reads an answer: POD / FAR / CSI and lead times for a WFO
//! over a window, plus the individual warnings and reports so the map can draw who verified.

use chrono::{DateTime, Utc};

const API: &str = "https://mesonet.agron.iastate.edu/api/1/cow.json";

/// Aggregate skill for the queried window.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Probability of detection: the share of storm reports that were warned.
    pub pod: f64,
    /// False alarm ratio: the share of warnings that nothing verified.
    pub far: f64,
    /// Critical success index — the one number that punishes both misses and false alarms.
    pub csi: f64,
    pub avg_lead_min: f64,
    pub max_lead_min: i64,
    pub events_total: u32,
    pub events_verified: u32,
    pub reports_total: u32,
    pub warned_reports: u32,
    pub unwarned_reports: u32,
    /// Mean warning area (km²) — context for FAR, since a big polygon is easier to verify.
    pub avg_size_km2: f64,
}

/// One warning, with IEM's verdict on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub wfo: String,
    /// `TO` (tornado) or `SV` (severe thunderstorm).
    pub phenomena: String,
    pub eventid: u32,
    pub issue: DateTime<Utc>,
    pub expire: DateTime<Utc>,
    pub verified: bool,
    /// Minutes between issuance and the first verifying report.
    pub lead_min: Option<i64>,
    pub area_km2: f64,
    /// Centroid, for putting a marker on the map without carrying the polygon.
    pub lon: f64,
    pub lat: f64,
    pub counties: Vec<String>,
}

/// One storm report, and whether a warning covered it.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub valid: DateTime<Utc>,
    pub kind: String,
    pub city: String,
    pub county: String,
    pub magnitude: Option<f64>,
    pub warned: bool,
    pub lead_min: Option<i64>,
    pub lon: f64,
    pub lat: f64,
}

/// A finished verification run.
#[derive(Debug, Clone, PartialEq)]
pub struct Verification {
    pub wfo: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub stats: Stats,
    pub warnings: Vec<Warning>,
    pub reports: Vec<Report>,
}

fn f(v: &serde_json::Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn s(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn time(v: &serde_json::Value, k: &str) -> Option<DateTime<Utc>> {
    let raw = v.get(k)?.as_str()?;
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .ok()
}

/// Parse a Cow response. Tolerant: a warning with no issue time or position is dropped rather
/// than faked, because a verification table with invented rows is worse than a shorter one.
pub fn parse(
    body: &str,
    wfo: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<Verification> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    let st = v
        .get("stats")
        .ok_or_else(|| anyhow::anyhow!("verification response has no stats block"))?;
    let stats = Stats {
        pod: f(st, "POD[1]"),
        far: f(st, "FAR[1]"),
        csi: f(st, "CSI[1]"),
        avg_lead_min: f(st, "avg_leadtime[min]"),
        max_lead_min: f(st, "max_leadtime[min]") as i64,
        events_total: f(st, "events_total") as u32,
        events_verified: f(st, "events_verified") as u32,
        reports_total: f(st, "reports_total") as u32,
        warned_reports: f(st, "warned_reports") as u32,
        unwarned_reports: f(st, "unwarned_reports") as u32,
        avg_size_km2: f(st, "avg_size[sq km]"),
    };

    let mut warnings = Vec::new();
    if let Some(feats) = v.pointer("/events/features").and_then(|x| x.as_array()) {
        for feat in feats {
            let Some(p) = feat.get("properties") else {
                continue;
            };
            let (Some(issue), Some(expire)) = (time(p, "issue"), time(p, "expire")) else {
                continue;
            };
            warnings.push(Warning {
                wfo: s(p, "wfo"),
                phenomena: s(p, "phenomena"),
                eventid: f(p, "eventid") as u32,
                issue,
                expire,
                verified: p.get("verify").and_then(|x| x.as_bool()).unwrap_or(false),
                lead_min: p.get("lead0").and_then(|x| x.as_i64()),
                area_km2: f(p, "parea"),
                lon: f(p, "lon0"),
                lat: f(p, "lat0"),
                counties: p
                    .get("ar_ugcname")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    warnings.sort_by_key(|w| w.issue);

    let mut reports = Vec::new();
    if let Some(feats) = v.pointer("/stormreports/features").and_then(|x| x.as_array()) {
        for feat in feats {
            let Some(p) = feat.get("properties") else {
                continue;
            };
            let Some(valid) = time(p, "valid") else {
                continue;
            };
            reports.push(Report {
                valid,
                kind: s(p, "typetext"),
                city: s(p, "city"),
                county: s(p, "county"),
                magnitude: p.get("magnitude").and_then(|x| x.as_f64()),
                warned: p.get("warned").and_then(|x| x.as_bool()).unwrap_or(false),
                lead_min: p.get("leadtime").and_then(|x| x.as_i64()),
                lon: f(p, "lon0"),
                lat: f(p, "lat0"),
            });
        }
    }
    reports.sort_by_key(|r| r.valid);

    Ok(Verification {
        wfo: wfo.to_string(),
        start,
        end,
        stats,
        warnings,
        reports,
    })
}

/// Run a verification for one WFO over a window. Tornado and severe-thunderstorm warnings are
/// scored together against tornado, wind and hail reports — the same default the Cow web UI uses,
/// so the numbers here can be checked against it.
pub async fn fetch(
    client: &reqwest::Client,
    wfo: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> anyhow::Result<Verification> {
    let body = client
        .get(API)
        .query(&[
            ("wfo", wfo),
            ("phenomena", "TO"),
            ("phenomena", "SV"),
            ("lsrtype", "TO"),
            ("lsrtype", "SV"),
            ("begints", &start.to_rfc3339()),
            ("endts", &end.to_rfc3339()),
        ])
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body, wfo, start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real Cow run for OUN on 20 May 2013 — one verified tornado warning, one
    /// unwarned report, plus a warning with no issue time that must be dropped.
    const COW: &str = r#"{
      "stats": {"area_verify[%]":6.29,"max_leadtime[min]":20,"avg_leadtime[min]":18.0,
        "unwarned_reports":1,"warned_reports":2,"events_verified":2,"events_total":15,
        "reports_total":3,"POD[1]":0.6666,"FAR[1]":0.8666,"CSI[1]":0.125,
        "avg_size[sq km]":874.18},
      "events":{"type":"FeatureCollection","features":[
        {"properties":{"wfo":"OUN","phenomena":"TO","eventid":24,
          "issue":"2013-05-20T19:40:00Z","expire":"2013-05-20T20:15:00Z",
          "verify":true,"lead0":16,"parea":533.0,"lon0":-97.4,"lat0":35.43,
          "ar_ugcname":["Cleveland OK","Oklahoma OK"]}},
        {"properties":{"wfo":"OUN","phenomena":"SV","eventid":310,
          "issue":"2013-05-20T00:26:00Z","expire":"2013-05-20T01:00:00Z",
          "verify":false,"parea":706.2,"lon0":-96.62,"lat0":35.82}},
        {"properties":{"wfo":"OUN","phenomena":"TO","eventid":99,"verify":false}}
      ]},
      "stormreports":{"type":"FeatureCollection","features":[
        {"properties":{"valid":"2013-05-20T00:08:00Z","typetext":"TORNADO","city":"2 NW PRAGUE",
          "county":"LINCOLN","magnitude":null,"warned":false,"leadtime":null,
          "lon0":-96.72,"lat0":35.51}}
      ]}}"#;

    #[test]
    fn parses_a_cow_run() {
        let start = DateTime::parse_from_rfc3339("2013-05-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = start + chrono::Duration::days(1);
        let v = parse(COW, "OUN", start, end).unwrap();
        assert!((v.stats.pod - 0.6666).abs() < 1e-3);
        assert_eq!(v.stats.events_verified, 2);
        assert_eq!(v.warnings.len(), 2, "the warning with no issue time drops");
        // Sorted by issue time, so the 00:26Z severe warning comes first.
        assert_eq!(v.warnings[0].phenomena, "SV");
        assert!(!v.warnings[0].verified);
        let tor = &v.warnings[1];
        assert!(tor.verified);
        assert_eq!(tor.lead_min, Some(16));
        assert_eq!(tor.counties.len(), 2);
        assert_eq!(v.reports.len(), 1);
        assert!(!v.reports[0].warned, "an unwarned report is the miss");
    }

    #[test]
    fn a_response_without_stats_is_an_error() {
        assert!(parse("{}", "OUN", Utc::now(), Utc::now()).is_err());
    }
}
