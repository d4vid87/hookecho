//! `--status`: what the weather is doing at the places you care about, printed to a terminal.
//!
//! The spots are your saved markers (home first), a `LAT,LON` given on the command line, or the
//! default radar site if you have neither. For each one: the nearest station's current conditions
//! and any active NWS alert whose polygon comes within that marker's watch radius.
//!
//! The report is a plain serializable struct so the `--serve` HTTP endpoint and the Home Assistant
//! component read exactly what the terminal prints — one collector, three renderings.

use crate::geo::KM_PER_MILE;
use serde::Serialize;
use wxdata::overlay::AlertInfo;

/// A place to report on.
#[derive(Debug, Clone)]
pub struct Spot {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub radius_mi: f64,
    pub home: bool,
}

/// One alert, trimmed to what a status line can show.
#[derive(Debug, Clone, Serialize)]
pub struct AlertBrief {
    pub event: String,
    /// Expiry in the machine's local time, `HH:MM`, when the alert carries one.
    pub until: Option<String>,
    /// 0 if the polygon covers the spot, else how far its edge is.
    pub distance_km: f64,
    /// 0 plain … 3 tornado emergency / PDS (see `wxdata::alerts::escalation`).
    pub escalation: u8,
}

/// Everything known about one spot right now.
#[derive(Debug, Clone, Serialize)]
pub struct SpotStatus {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub home: bool,
    pub station: Option<String>,
    pub temp_f: Option<f32>,
    pub dewpoint_f: Option<f32>,
    pub rh: Option<f32>,
    pub wind_kt: Option<f32>,
    pub gust_kt: Option<f32>,
    pub wind_dir: Option<String>,
    pub pressure_in: Option<f32>,
    pub alerts: Vec<AlertBrief>,
}

/// How to print the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Multi-line, one block per spot.
    Human,
    /// A single line for the home spot — status bars (polybar, i3, tmux).
    Line,
    /// The full report as JSON.
    Json,
}

/// The spots to report on: an explicit `LAT,LON`, else the saved markers (home first), else the
/// default radar site.
pub fn spots(settings: &crate::settings::Settings, point: Option<&str>) -> Vec<Spot> {
    if let Some((lat, lon)) = point.and_then(parse_point) {
        return vec![Spot {
            name: format!("{lat:.3},{lon:.3}"),
            lat,
            lon,
            radius_mi: crate::settings::default_alert_radius_mi(),
            home: true,
        }];
    }
    if !settings.markers.is_empty() {
        let mut out: Vec<Spot> = settings
            .markers
            .iter()
            .map(|m| Spot {
                name: m.name.clone(),
                lat: m.lat,
                lon: m.lon,
                radius_mi: m.alert_radius_mi,
                home: m.home,
            })
            .collect();
        // Home first: `--line` reports it, and it leads the human output.
        out.sort_by_key(|s| !s.home);
        return out;
    }
    let site = wxdata::sites::site_by_id(&settings.default_site)
        .or_else(|| wxdata::sites::site_by_id("KTLX"));
    site.map(|s| {
        vec![Spot {
            name: s.id.to_string(),
            lat: s.latitude as f64,
            lon: s.longitude as f64,
            radius_mi: crate::settings::default_alert_radius_mi(),
            home: true,
        }]
    })
    .unwrap_or_default()
}

/// `"35.3,-97.5"` → `(lat, lon)`.
fn parse_point(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Fetch conditions and alerts for every spot.
///
/// The nationwide alert feed is pulled once and distance-filtered per spot rather than queried per
/// marker — the whole point of this mode is one quick round of requests.
///
// ponytail: nationwide zone pass is capped; per-marker `?point=` queries if edge-of-range
// advisories matter.
pub async fn collect(http: &reqwest::Client, spots: &[Spot]) -> anyhow::Result<Vec<SpotStatus>> {
    let feats = wxdata::alerts::fetch_active(http, None).await?;
    let mut out = Vec::with_capacity(spots.len());
    for spot in spots {
        let mut alerts: Vec<AlertBrief> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in &feats {
            let Some(a) = f.alert.as_ref() else { continue };
            let km = f.distance_km(spot.lon, spot.lat);
            if km > spot.radius_mi * KM_PER_MILE || !seen.insert(a.id.clone()) {
                continue;
            }
            alerts.push(brief(a, km));
        }
        // Worst first — a status bar with room for one alert should show the tornado warning.
        alerts.sort_by(|a, b| {
            b.escalation
                .cmp(&a.escalation)
                .then(a.distance_km.total_cmp(&b.distance_km))
        });
        out.push(status_for(spot, observation(http, spot).await, alerts));
    }
    Ok(out)
}

/// The nearest station's latest observation, or `None` when there isn't one (offshore, OCONUS,
/// API down) — conditions are worth reporting when available and worth skipping when not.
///
// ponytail: `fetch_nearest` pulls 24 h of obs to use the newest one; trim if this ever feels slow.
async fn observation(
    http: &reqwest::Client,
    spot: &Spot,
) -> Option<(String, wxdata::obs::Observation)> {
    match wxdata::obs::fetch_nearest(http, spot.lat, spot.lon).await {
        Ok(s) => s.obs.first().map(|o| (s.station_id, o.clone())),
        Err(e) => {
            log::debug!("no observations for {}: {e}", spot.name);
            None
        }
    }
}

fn brief(a: &AlertInfo, distance_km: f64) -> AlertBrief {
    AlertBrief {
        event: a.event.clone(),
        until: a
            .expires
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string()),
        distance_km,
        escalation: wxdata::alerts::escalation(a),
    }
}

fn status_for(
    spot: &Spot,
    ob: Option<(String, wxdata::obs::Observation)>,
    alerts: Vec<AlertBrief>,
) -> SpotStatus {
    let (station, o) = match ob {
        Some((id, o)) => (Some(id), Some(o)),
        None => (None, None),
    };
    SpotStatus {
        name: spot.name.clone(),
        lat: spot.lat,
        lon: spot.lon,
        home: spot.home,
        station,
        temp_f: o.as_ref().and_then(|o| o.temp_c).map(c_to_f),
        dewpoint_f: o.as_ref().and_then(|o| o.dewpoint_c).map(c_to_f),
        rh: o.as_ref().and_then(|o| o.rh),
        wind_kt: o.as_ref().and_then(|o| o.wind_kmh).map(kmh_to_kt),
        gust_kt: o.as_ref().and_then(|o| o.gust_kmh).map(kmh_to_kt),
        wind_dir: o
            .as_ref()
            .and_then(|o| o.wind_dir_deg)
            .map(|d| compass(d).to_string()),
        pressure_in: o
            .as_ref()
            .and_then(|o| o.slp_pa.or(o.pressure_pa))
            .map(|pa| pa / 3386.389),
        alerts,
    }
}

fn c_to_f(c: f32) -> f32 {
    c * 9.0 / 5.0 + 32.0
}

fn kmh_to_kt(kmh: f32) -> f32 {
    kmh / 1.852
}

/// 16-point compass label for a wind direction in degrees.
fn compass(deg: f32) -> &'static str {
    const D: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    D[((deg / 22.5).round() as usize) % 16]
}

/// Conditions as one phrase: `74°F 62%rh SW 12kt`. Empty when the station reported nothing.
fn conditions(s: &SpotStatus) -> String {
    let mut parts = Vec::new();
    if let Some(t) = s.temp_f {
        parts.push(format!("{t:.0}°F"));
    }
    if let Some(rh) = s.rh {
        parts.push(format!("{rh:.0}%rh"));
    }
    if let Some(kt) = s.wind_kt {
        let dir = s
            .wind_dir
            .as_deref()
            .map(|d| format!("{d} "))
            .unwrap_or_default();
        let gust = match s.gust_kt {
            Some(g) => format!(" G{g:.0}"),
            None => String::new(),
        };
        parts.push(if kt < 1.0 {
            "calm".to_string()
        } else {
            format!("{dir}{kt:.0}kt{gust}")
        });
    }
    parts.join(" ")
}

fn alert_phrase(a: &AlertBrief) -> String {
    let mark = if a.escalation >= 2 { "‼" } else { "⚠" };
    let until = match &a.until {
        Some(t) => format!(" until {t}"),
        None => String::new(),
    };
    let away = if a.distance_km > 0.5 {
        format!(" ({:.0} km away)", a.distance_km)
    } else {
        String::new()
    };
    format!("{mark} {}{until}{away}", a.event)
}

/// Multi-line report, one block per spot.
pub fn human(report: &[SpotStatus]) -> String {
    let mut out = String::new();
    for s in report {
        let mut head = format!("{}:", s.name);
        let c = conditions(s);
        if c.is_empty() {
            head.push_str(" no observations");
        } else {
            head.push(' ');
            head.push_str(&c);
        }
        if let Some(st) = &s.station {
            head.push_str(&format!(" ({st})"));
        }
        out.push_str(&head);
        out.push('\n');
        for a in &s.alerts {
            out.push_str(&format!("  {}\n", alert_phrase(a)));
        }
    }
    if out.is_empty() {
        out.push_str("no spots configured — add a marker or pass LAT,LON\n");
    }
    out
}

/// One line for the home spot (or the first one) — what a status bar has room for.
pub fn line(report: &[SpotStatus]) -> String {
    let Some(s) = report.iter().find(|s| s.home).or_else(|| report.first()) else {
        return String::new();
    };
    let c = conditions(s);
    match s.alerts.first() {
        Some(a) if c.is_empty() => alert_phrase(a),
        Some(a) => format!("{c} · {}", alert_phrase(a)),
        None => c,
    }
}

/// Fetch and print. Builds its own runtime — this runs instead of the GUI, not beside it.
pub fn run(spots: &[Spot], mode: Mode) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let report = rt.block_on(async {
        let http = reqwest::Client::new();
        collect(&http, spots).await
    })?;
    match mode {
        Mode::Human => print!("{}", human(&report)),
        Mode::Line => println!("{}", line(&report)),
        Mode::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(name: &str, home: bool) -> Spot {
        Spot {
            name: name.to_string(),
            lat: 35.5,
            lon: -97.5,
            radius_mi: 20.0,
            home,
        }
    }

    fn ob() -> wxdata::obs::Observation {
        wxdata::obs::Observation {
            time: None,
            temp_c: Some(23.3),
            dewpoint_c: Some(16.7),
            rh: Some(62.0),
            wind_kmh: Some(22.2),
            gust_kmh: None,
            wind_dir_deg: Some(225.0),
            pressure_pa: None,
            slp_pa: None,
        }
    }

    fn warning() -> AlertBrief {
        AlertBrief {
            event: "Tornado Warning".into(),
            until: Some("14:30".into()),
            distance_km: 0.0,
            escalation: 2,
        }
    }

    #[test]
    fn human_report_has_a_block_per_spot() {
        let report = vec![
            status_for(
                &spot("Home", true),
                Some(("KOKC".into(), ob())),
                vec![warning()],
            ),
            status_for(&spot("Cabin", false), None, vec![]),
        ];
        let text = human(&report);
        assert_eq!(
            text,
            "Home: 74°F 62%rh SW 12kt (KOKC)\n  ‼ Tornado Warning until 14:30\nCabin: no observations\n"
        );
    }

    #[test]
    fn line_reports_home_even_when_it_is_not_first() {
        let report = vec![
            status_for(&spot("Cabin", false), None, vec![]),
            status_for(
                &spot("Home", true),
                Some(("KOKC".into(), ob())),
                vec![warning()],
            ),
        ];
        assert_eq!(
            line(&report),
            "74°F 62%rh SW 12kt · ‼ Tornado Warning until 14:30"
        );
    }

    #[test]
    fn line_without_alerts_is_just_conditions() {
        let report = vec![status_for(
            &spot("Home", true),
            Some(("KOKC".into(), ob())),
            vec![],
        )];
        assert_eq!(line(&report), "74°F 62%rh SW 12kt");
        assert!(line(&[]).is_empty());
    }

    #[test]
    fn json_keeps_the_field_names_the_ha_component_reads() {
        let report = vec![status_for(
            &spot("Home", true),
            Some(("KOKC".into(), ob())),
            vec![warning()],
        )];
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();
        assert_eq!(v[0]["name"], "Home");
        assert_eq!(v[0]["station"], "KOKC");
        assert_eq!(v[0]["home"], true);
        assert!((v[0]["temp_f"].as_f64().unwrap() - 73.94).abs() < 0.01);
        assert_eq!(v[0]["wind_dir"], "SW");
        assert_eq!(v[0]["alerts"][0]["event"], "Tornado Warning");
        assert_eq!(v[0]["alerts"][0]["escalation"], 2);
        // A station that reported nothing serializes as null, not as a zero.
        let empty = status_for(&spot("Cabin", false), None, vec![]);
        let v = serde_json::to_value(&empty).unwrap();
        assert!(v["temp_f"].is_null() && v["station"].is_null());
    }

    #[test]
    fn missing_fields_shrink_the_phrase() {
        let sparse = wxdata::obs::Observation {
            temp_c: Some(10.0),
            ..ob()
        };
        let mut sparse = sparse;
        sparse.rh = None;
        sparse.wind_kmh = None;
        let s = status_for(&spot("Home", true), Some(("KXYZ".into(), sparse)), vec![]);
        assert_eq!(conditions(&s), "50°F");
    }

    #[test]
    fn explicit_point_beats_markers() {
        let mut settings = crate::settings::Settings::default();
        settings.markers.push(crate::settings::Marker {
            name: "Home".into(),
            lat: 1.0,
            lon: 2.0,
            icon: None,
            alert_radius_mi: 20.0,
            home: true,
        });
        let s = spots(&settings, Some("35.3,-97.5"));
        assert_eq!(s.len(), 1);
        assert!((s[0].lat - 35.3).abs() < 1e-9 && (s[0].lon + 97.5).abs() < 1e-9);
        // Garbage falls through to the markers rather than reporting on nowhere.
        assert_eq!(spots(&settings, Some("not-a-point"))[0].name, "Home");
    }

    #[test]
    fn markers_report_home_first_and_empty_settings_use_the_default_site() {
        let mut settings = crate::settings::Settings::default();
        settings.markers = vec![
            crate::settings::Marker {
                name: "Cabin".into(),
                lat: 1.0,
                lon: 2.0,
                icon: None,
                alert_radius_mi: 30.0,
                home: false,
            },
            crate::settings::Marker {
                name: "Home".into(),
                lat: 3.0,
                lon: 4.0,
                icon: None,
                alert_radius_mi: 20.0,
                home: true,
            },
        ];
        let s = spots(&settings, None);
        assert_eq!(s[0].name, "Home");
        assert_eq!(s[1].radius_mi, 30.0);

        let bare = crate::settings::Settings::default();
        let s = spots(&bare, None);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, bare.default_site);
    }

    #[test]
    #[ignore = "network"]
    fn live_status_runs() {
        run(&[spot("Home", true)], Mode::Human).unwrap();
    }
}
