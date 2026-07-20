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

/// Which Day-1 outlook to fetch: the categorical risk, or a hazard probability grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutlookKind {
    #[default]
    Categorical,
    Tornado,
    Wind,
    Hail,
}

impl OutlookKind {
    pub const ALL: [OutlookKind; 4] =
        [OutlookKind::Categorical, OutlookKind::Tornado, OutlookKind::Wind, OutlookKind::Hail];

    /// SPC filename slug (`day1otlk_<slug>.lyr.geojson`).
    pub fn slug(self) -> &'static str {
        match self {
            OutlookKind::Categorical => "cat",
            OutlookKind::Tornado => "torn",
            OutlookKind::Wind => "wind",
            OutlookKind::Hail => "hail",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OutlookKind::Categorical => "Categorical",
            OutlookKind::Tornado => "Tornado",
            OutlookKind::Wind => "Wind",
            OutlookKind::Hail => "Hail",
        }
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
    parse_outlook_kind(json, day, OutlookKind::Categorical)
}

/// Parse an SPC outlook GeoJSON payload for a given hazard kind. Categorical uses the risk
/// `LABEL2`; probabilistic layers carry a numeric `LABEL` (e.g. "0.05") plus a `SIGN` significant
/// hatch polygon.
pub fn parse_outlook_kind(json: &str, day: u8, kind: OutlookKind) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        if kind == OutlookKind::Categorical {
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
            push_polys(&mut out, geom, [rgb[0], rgb[1], rgb[2], 70], [rgb[0], rgb[1], rgb[2], 230], title, detail);
        } else {
            let label = str_of("LABEL");
            if label.is_empty() {
                return;
            }
            let hazard = kind.label();
            // SIGN = 10%+ significant hazard hatch; probability labels are fractions like "0.05".
            if label.eq_ignore_ascii_case("SIGN") {
                let title = format!("Day {day} {hazard}: SIG");
                let detail = format!("SPC Day {day} {hazard} Outlook\nSignificant (10%+)\nValid: {}", str_of("VALID"));
                // ponytail: SIG hatching approximated by translucent black — lyon has no hatch pattern.
                push_polys(&mut out, geom, [0, 0, 0, 60], [0, 0, 0, 200], title, detail);
            } else {
                let pct = label.parse::<f32>().map(|f| (f * 100.0).round() as i32).unwrap_or(0);
                let rgb = hex_rgb(str_of("fill")).unwrap_or_else(|| risk_color(label));
                let title = format!("Day {day} {hazard}: {pct}%");
                let detail = format!("SPC Day {day} {hazard} Probability\n{pct}%\nValid: {}", str_of("VALID"));
                push_polys(&mut out, geom, [rgb[0], rgb[1], rgb[2], 70], [rgb[0], rgb[1], rgb[2], 230], title, detail);
            }
        }
    })?;
    Ok(out)
}

/// Push one `GeoFeature` per polygon part of `geom` with the given styling/text.
fn push_polys(out: &mut Vec<GeoFeature>, geom: &geojson::GeometryValue, fill: [u8; 4], stroke: [u8; 4], title: String, detail: String) {
    for poly in polygons_of(geom) {
        out.push(GeoFeature {
            rings: poly,
            fill,
            stroke,
            kind: FeatureKind::Outlook,
            title: title.clone(),
            detail: detail.clone(),
            alert: None,
        });
    }
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
    fetch_outlook_kind(client, day, OutlookKind::Categorical).await
}

/// Fetch an outlook for `day` and hazard `kind` (probabilistic layers are Day-1 only).
pub async fn fetch_outlook_kind(client: &reqwest::Client, day: u8, kind: OutlookKind) -> anyhow::Result<Vec<GeoFeature>> {
    let url = format!("{OUTLOOK_BASE}/day{day}otlk_{}.lyr.geojson", kind.slug());
    let body = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_outlook_kind(&body, day, kind)
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

/// Kind of a local storm report (drives the marker color/label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Tornado,
    Wind,
    Hail,
    Flood,
    /// Anything else the LSR feed carries (funnel cloud, waterspout, dust, …).
    Other,
}

impl ReportKind {
    pub fn label(self) -> &'static str {
        match self {
            ReportKind::Tornado => "Tornado",
            ReportKind::Wind => "Wind",
            ReportKind::Hail => "Hail",
            ReportKind::Flood => "Flood",
            ReportKind::Other => "Storm",
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

// The SPC `today.csv` daily-log fetcher lived here; superseded by the live IEM LSR feed in
// [`crate::lsr`] (same [`StormReport`] type, minutes-fresh, archive-capable).

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_probabilistic_outlook_with_sig() {
        let json = r##"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-100,35],[-98,35],[-98,37],[-100,35]]]},
             "properties":{"LABEL":"0.05","fill":"#8B4726","VALID":"today"}},
            {"type":"Feature",
             "geometry":{"type":"Polygon","coordinates":[[[-99,35],[-98,35],[-98,36],[-99,35]]]},
             "properties":{"LABEL":"SIGN","VALID":"today"}}]}"##;
        let feats = parse_outlook_kind(json, 1, OutlookKind::Tornado).unwrap();
        assert_eq!(feats.len(), 2);
        assert_eq!(feats[0].title, "Day 1 Tornado: 5%");
        assert_eq!(feats[0].stroke, [139, 71, 38, 230]);
        assert_eq!(feats[1].title, "Day 1 Tornado: SIG");
        assert_eq!(feats[1].fill, [0, 0, 0, 60]);
    }

    #[test]
    fn hex_parse() {
        assert_eq!(hex_rgb("#ff8800"), Some([255, 136, 0]));
        assert_eq!(hex_rgb("bad"), None);
    }
}
