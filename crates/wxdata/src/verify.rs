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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlgorithmEvent {
    pub valid: DateTime<Utc>,
    pub lon: f64,
    pub lat: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlgorithmStats {
    pub hits: u32,
    pub misses: u32,
    pub false_alarms: u32,
    pub pod: f64,
    pub far: f64,
    pub csi: f64,
}

/// Score algorithm detections against observed reports with one-to-one space/time matching.
pub fn score_algorithm(
    detections: &[AlgorithmEvent],
    reports: &[Report],
    max_distance_km: f64,
    max_time: chrono::Duration,
) -> AlgorithmStats {
    let mut used = vec![false; detections.len()];
    let mut hits = 0u32;
    for report in reports {
        let nearest = detections
            .iter()
            .enumerate()
            .filter(|(i, detection)| {
                !used[*i]
                    && (detection.valid - report.valid).abs() <= max_time
                    && distance_km(detection.lon, detection.lat, report.lon, report.lat)
                        <= max_distance_km
            })
            .min_by_key(|(_, detection)| (detection.valid - report.valid).abs());
        if let Some((index, _)) = nearest {
            used[index] = true;
            hits += 1;
        }
    }
    let misses = reports.len() as u32 - hits;
    let false_alarms = detections.len() as u32 - hits;
    let ratio = |num: u32, den: u32| {
        if den > 0 { num as f64 / den as f64 } else { 0.0 }
    };
    AlgorithmStats {
        hits,
        misses,
        false_alarms,
        pod: ratio(hits, hits + misses),
        far: ratio(false_alarms, hits + false_alarms),
        csi: ratio(hits, hits + misses + false_alarms),
    }
}

fn distance_km(a_lon: f64, a_lat: f64, b_lon: f64, b_lat: f64) -> f64 {
    let (a_lat, b_lat) = (a_lat.to_radians(), b_lat.to_radians());
    let dlat = b_lat - a_lat;
    let dlon = (b_lon - a_lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2)
        + a_lat.cos() * b_lat.cos() * (dlon / 2.0).sin().powi(2);
    6371.0 * 2.0 * h.sqrt().asin()
}

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

impl Verification {
    /// The whole run as CSV: the skill line, then every warning, then every report. Three
    /// differently-shaped tables in one file, separated by blank lines — a spreadsheet imports it
    /// as three blocks, which is how people read it anyway.
    pub fn to_csv(&self) -> String {
        fn o<T: std::fmt::Display>(v: Option<T>) -> String {
            v.map(|x| x.to_string()).unwrap_or_default()
        }
        let s = &self.stats;
        let mut out = format!(
            "wfo,start,end,pod,far,csi,avg_lead_min,max_lead_min,events_total,events_verified,\
             reports_total,warned_reports,unwarned_reports,avg_size_km2\n\
             {},{},{},{:.4},{:.4},{:.4},{:.1},{},{},{},{},{},{},{:.1}\n\n",
            self.wfo,
            self.start.to_rfc3339(),
            self.end.to_rfc3339(),
            s.pod,
            s.far,
            s.csi,
            s.avg_lead_min,
            s.max_lead_min,
            s.events_total,
            s.events_verified,
            s.reports_total,
            s.warned_reports,
            s.unwarned_reports,
            s.avg_size_km2,
        );
        out.push_str(
            "wfo,phenomena,eventid,issue,expire,verified,lead_min,area_km2,lon,lat,counties\n",
        );
        for w in &self.warnings {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{:.1},{:.4},{:.4},{}\n",
                w.wfo,
                w.phenomena,
                w.eventid,
                w.issue.to_rfc3339(),
                w.expire.to_rfc3339(),
                u8::from(w.verified),
                o(w.lead_min),
                w.area_km2,
                w.lon,
                w.lat,
                // Semicolons, so the county list survives a comma-separated file unquoted.
                w.counties.join(";"),
            ));
        }
        out.push_str("\nvalid,kind,city,county,magnitude,warned,lead_min,lon,lat\n");
        for r in &self.reports {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{:.4},{:.4}\n",
                r.valid.to_rfc3339(),
                r.kind,
                r.city.replace(',', " "),
                r.county.replace(',', " "),
                o(r.magnitude),
                u8::from(r.warned),
                o(r.lead_min),
                r.lon,
                r.lat,
            ));
        }
        out
    }
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
    if let Some(feats) = v
        .pointer("/stormreports/features")
        .and_then(|x| x.as_array())
    {
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
        .get(crate::net::fetch_url(API))
        .timeout(crate::net::FEED_TIMEOUT)
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
    fn csv_has_all_three_blocks() {
        let start = DateTime::parse_from_rfc3339("2013-05-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let v = parse(COW, "OUN", start, start + chrono::Duration::days(1)).unwrap();
        let csv = v.to_csv();
        let blocks: Vec<&str> = csv.split("\n\n").collect();
        assert_eq!(blocks.len(), 3, "stats, warnings, reports");
        assert!(blocks[0].contains("OUN,2013-05-20T00:00:00+00:00"));
        assert!(
            blocks[1].contains("OUN,TO,24,"),
            "the verified tornado warning"
        );
        assert!(
            blocks[1].contains("Cleveland OK;Oklahoma OK"),
            "counties keep out of the comma columns"
        );
        assert!(blocks[2].contains("TORNADO,2 NW PRAGUE,LINCOLN,,0,,"));
    }

    #[test]
    fn a_response_without_stats_is_an_error() {
        assert!(parse("{}", "OUN", Utc::now(), Utc::now()).is_err());
    }

    #[test]
    fn algorithm_scoring_is_one_to_one_in_space_and_time() {
        let at = DateTime::parse_from_rfc3339("2013-05-20T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let detections = [
            AlgorithmEvent { valid: at, lon: -97.0, lat: 35.0 },
            AlgorithmEvent { valid: at + chrono::Duration::minutes(2), lon: -97.01, lat: 35.0 },
            AlgorithmEvent { valid: at, lon: -100.0, lat: 35.0 },
        ];
        let report = Report {
            valid: at + chrono::Duration::minutes(1),
            kind: "TORNADO".into(),
            city: String::new(),
            county: String::new(),
            magnitude: None,
            warned: false,
            lead_min: None,
            lon: -97.0,
            lat: 35.0,
        };
        let score = score_algorithm(
            &detections,
            std::slice::from_ref(&report),
            20.0,
            chrono::Duration::minutes(10),
        );
        assert_eq!((score.hits, score.misses, score.false_alarms), (1, 0, 2));
        assert_eq!(score.pod, 1.0);
        assert!((score.far - 2.0 / 3.0).abs() < 1e-9);
        assert!((score.csi - 1.0 / 3.0).abs() < 1e-9);
    }
}
