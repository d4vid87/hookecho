//! SPC convective products: Day 1–3 categorical outlooks and Mesoscale Discussions.
//!
//! Outlooks come as static GeoJSON that already carries per-feature `fill`/`stroke` colors
//! and a risk `LABEL2`; MDs come from the NWS map service as GeoJSON. Both decode into the
//! shared [`GeoFeature`] type.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const OUTLOOK_BASE: &str = "https://www.spc.noaa.gov/products/outlook";
const MD_URL: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/spc_mesoscale_discussion/MapServer/0/query?where=1%3D1&outFields=*&f=geojson";

/// Fill color for a categorical risk label, when the GeoJSON doesn't supply one.
fn risk_color(label: &str) -> [u8; 3] {
    match label.to_ascii_uppercase().as_str() {
        "TSTM" => [192, 224, 163],
        "MRGL" => [127, 197, 127],
        "SLGT" => [246, 246, 131],
        "ENH" => [230, 152, 90],
        "MDT" => [214, 107, 107],
        "HIGH" => [204, 102, 204],
        _ => [150, 150, 150],
    }
}

/// Parse a `#rrggbb` hex color.
fn hex_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ])
}

/// Parse an SPC categorical-outlook GeoJSON payload.
pub fn parse_outlook(json: &str, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let label = {
            let l2 = str_of("LABEL2");
            if l2.is_empty() { str_of("LABEL").to_string() } else { l2.to_string() }
        };
        if label.is_empty() {
            return;
        }
        let rgb = hex_rgb(str_of("fill")).unwrap_or_else(|| risk_color(&label));
        let title = format!("Day {day}: {label}");
        let detail = format!(
            "SPC Day {day} Convective Outlook\nCategory: {label}\nValid: {}",
            str_of("VALID"),
        );
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 70],
                stroke: [rgb[0], rgb[1], rgb[2], 230],
                kind: FeatureKind::Outlook,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// Parse an SPC Mesoscale Discussion GeoJSON payload.
pub fn parse_md(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let name = str_of("name");
        let title = if name.is_empty() {
            "Mesoscale Discussion".to_string()
        } else {
            format!("Mesoscale Discussion {name}")
        };
        let detail = format!("{title}\n\n{}\n{}", str_of("popupinfo"), str_of("folderpath"));
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [255, 120, 0, 30],
                stroke: [255, 140, 0, 235],
                kind: FeatureKind::MesoDiscussion,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// Fetch the categorical outlook for `day` (1–3).
pub async fn fetch_outlook(client: &reqwest::Client, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    let url = format!("{OUTLOOK_BASE}/day{day}otlk_cat.lyr.geojson");
    let body = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_outlook(&body, day)
}

/// Fetch active Mesoscale Discussions.
pub async fn fetch_mesoscale_discussions(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    let body = client
        .get(MD_URL)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_md(&body)
}

const STORM_REPORTS_URL: &str = "https://www.spc.noaa.gov/climo/reports/today.csv";

/// Kind of a local storm report (drives the marker color/label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Tornado,
    Wind,
    Hail,
}

impl ReportKind {
    pub fn label(self) -> &'static str {
        match self {
            ReportKind::Tornado => "Tornado",
            ReportKind::Wind => "Wind",
            ReportKind::Hail => "Hail",
        }
    }
}

/// One SPC local storm report (today's preliminary log).
#[derive(Debug, Clone)]
pub struct StormReport {
    pub kind: ReportKind,
    pub lat: f64,
    pub lon: f64,
    /// UTC-ish report time as issued (HHMM string).
    pub time: String,
    /// F-scale, wind speed, or hail size column, as issued.
    pub magnitude: String,
    pub location: String,
    pub county: String,
    pub state: String,
    pub comments: String,
}

/// Fetch today's SPC storm reports (tornado + wind + hail).
pub async fn fetch_storm_reports(client: &reqwest::Client) -> anyhow::Result<Vec<StormReport>> {
    let body = client
        .get(STORM_REPORTS_URL)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_storm_reports(&body))
}

/// Parse the concatenated `today.csv` (three sections: tornado, wind, hail). Each section opens
/// with a `Time,<Magnitude>,...` header row whose 2nd column names the kind. Malformed rows are
/// skipped. `Comments` (the last column) may contain commas, so only split into 8 fields.
pub fn parse_storm_reports(csv: &str) -> Vec<StormReport> {
    let mut kind = ReportKind::Tornado;
    let mut out = Vec::new();
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Time,") {
            // Section header: the magnitude column names the kind.
            kind = if rest.starts_with("F_Scale") {
                ReportKind::Tornado
            } else if rest.starts_with("Speed") {
                ReportKind::Wind
            } else {
                ReportKind::Hail
            };
            continue;
        }
        let f: Vec<&str> = line.splitn(8, ',').collect();
        if f.len() < 7 {
            continue;
        }
        let (Ok(lat), Ok(lon)) = (f[5].trim().parse::<f64>(), f[6].trim().parse::<f64>()) else {
            continue;
        };
        out.push(StormReport {
            kind,
            lat,
            lon,
            time: f[0].trim().to_string(),
            magnitude: f[1].trim().to_string(),
            location: f[2].trim().to_string(),
            county: f[3].trim().to_string(),
            state: f[4].trim().to_string(),
            comments: f.get(7).map(|s| s.trim().to_string()).unwrap_or_default(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_storm_report_sections() {
        let csv = "Time,F_Scale,Location,County,State,Lat,Lon,Comments\n\
                   1823,UNK,9 NNW Town,Lauderdale,AL,35.00,-87.77,A tree, and a fence, fell (HUN)\n\
                   Time,Speed,Location,County,State,Lat,Lon,Comments\n\
                   1900,60,Somewhere,Dallas,TX,32.5,-96.8,gust\n\
                   Time,Size,Location,County,State,Lat,Lon,Comments\n\
                   1930,175,Elsewhere,Cook,IL,41.9,-87.6,hail\n";
        let r = parse_storm_reports(csv);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].kind, ReportKind::Tornado);
        assert_eq!(r[0].comments, "A tree, and a fence, fell (HUN)", "commas in comments kept");
        assert_eq!(r[1].kind, ReportKind::Wind);
        assert_eq!(r[2].kind, ReportKind::Hail);
        assert!((r[2].lon - -87.6).abs() < 1e-9);
    }

    #[test]
    fn parses_outlook_with_own_color() {
        let json = r##"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-100,35],[-98,35],[-98,37],[-100,35]]]},
             "properties":{"LABEL2":"ENH","fill":"#E6985A","VALID":"today"}}]}"##;
        let feats = parse_outlook(json, 1).unwrap();
        assert_eq!(feats.len(), 1);
        assert_eq!(feats[0].kind, FeatureKind::Outlook);
        assert_eq!(feats[0].stroke, [230, 152, 90, 230]);
        assert!(feats[0].title.contains("ENH"));
    }

    #[test]
    fn hex_parse() {
        assert_eq!(hex_rgb("#ff8800"), Some([255, 136, 0]));
        assert_eq!(hex_rgb("bad"), None);
    }
}
