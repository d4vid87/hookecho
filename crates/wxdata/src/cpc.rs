//! NOAA CPC temperature and precipitation outlook polygons.
//!
//! These are public NWS ArcGIS GeoJSON feeds, independent of RadarOmega's private backend.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const BASE: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/outlooks";
const DISCUSSION_BASE: &str = "https://www.cpc.ncep.noaa.gov/products/predictions";

#[derive(Debug, Clone)]
pub struct Discussion {
    pub period: Period,
    pub text: String,
    pub source_url: &'static str,
}

pub fn discussion_url(period: Period) -> &'static str {
    match period {
        Period::Day6To10 | Period::Day8To14 => "https://www.cpc.ncep.noaa.gov/products/predictions/610day/fxus06.html",
        Period::Week3To4 => "https://www.cpc.ncep.noaa.gov/products/predictions/WK34/texts/week34fcst.txt",
        Period::Month1 => "https://www.cpc.ncep.noaa.gov/products/predictions/90day/fxus07.html",
        Period::Month3 => "https://www.cpc.ncep.noaa.gov/products/predictions/90day/fxus05.html",
    }
}

fn plain_text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for ch in markup.chars() {
        match ch {
            '<' => { in_tag = true; tag.clear(); }
            '>' if in_tag => {
                if tag.trim().to_ascii_lowercase().starts_with("br") { out.push('\n'); }
                in_tag = false;
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    let out = out.replace("&nbsp;", " ").replace("&amp;", "&")
        .replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"")
        .replace("&#39;", "'").replace("&deg;", "°");
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_owned()
}

pub fn parse_discussion(body: &str, period: Period) -> anyhow::Result<Discussion> {
    anyhow::ensure!(body.len() <= 1024 * 1024, "CPC discussion exceeds 1 MB");
    let selected = if period == Period::Week3To4 {
        body
    } else {
        let lower = body.to_ascii_lowercase();
        let start = lower.find("<pre").ok_or_else(|| anyhow::anyhow!("CPC discussion body missing"))?;
        let body_start = lower[start..].find('>').ok_or_else(|| anyhow::anyhow!("CPC discussion body malformed"))? + start + 1;
        let end = lower[body_start..].find("</pre>").ok_or_else(|| anyhow::anyhow!("CPC discussion body incomplete"))? + body_start;
        &body[body_start..end]
    };
    let text = plain_text(selected);
    anyhow::ensure!(text.starts_with("Prognostic Discussion") && text.len() > 100, "CPC discussion text unavailable");
    Ok(Discussion { period, text, source_url: discussion_url(period) })
}

pub async fn fetch_discussion(client: &reqwest::Client, period: Period) -> anyhow::Result<Discussion> {
    let url = discussion_url(period);
    debug_assert!(url.starts_with(DISCUSSION_BASE));
    let body = client.get(crate::net::fetch_url(url)).timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT).send().await?.error_for_status()?.text().await?;
    parse_discussion(&body, period)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variable { Temperature, Precipitation }

impl Variable {
    pub fn label(self) -> &'static str { match self { Self::Temperature => "Temperature", Self::Precipitation => "Precipitation" } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period { Day6To10, Day8To14, Week3To4, Month1, Month3 }

impl Period {
    pub const ALL: [Self; 5] = [Self::Day6To10, Self::Day8To14, Self::Week3To4, Self::Month1, Self::Month3];
    pub fn label(self) -> &'static str { match self { Self::Day6To10 => "6–10 day", Self::Day8To14 => "8–14 day", Self::Week3To4 => "3–4 week", Self::Month1 => "1 month", Self::Month3 => "3 month" } }
}

fn service(period: Period, variable: Variable) -> Option<(&'static str, u8)> {
    Some(match (period, variable) {
        (Period::Day6To10, Variable::Temperature) => ("cpc_6_10_day_outlk", 0),
        (Period::Day6To10, Variable::Precipitation) => ("cpc_6_10_day_outlk", 1),
        (Period::Day8To14, Variable::Temperature) => ("cpc_8_14_day_outlk", 0),
        (Period::Day8To14, Variable::Precipitation) => ("cpc_8_14_day_outlk", 1),
        (Period::Month1, Variable::Temperature) => ("cpc_mthly_temp_outlk", 0),
        (Period::Month1, Variable::Precipitation) => ("cpc_mthly_precip_outlk", 0),
        (Period::Month3, Variable::Temperature) => ("cpc_sea_temp_outlk", 0),
        (Period::Month3, Variable::Precipitation) => ("cpc_sea_precip_outlk", 0),
        (Period::Week3To4, _) => return None,
    })
}

fn category_color(category: &str, variable: Variable) -> [u8; 3] {
    match (category.to_ascii_lowercase().as_str(), variable) {
        ("above", Variable::Temperature) => [230, 103, 72],
        ("below", Variable::Temperature) => [76, 134, 220],
        ("above", Variable::Precipitation) => [61, 159, 108],
        ("below", Variable::Precipitation) => [183, 145, 78],
        _ => [155, 163, 172],
    }
}

fn date(ms: Option<i64>) -> Option<String> {
    use chrono::TimeZone;
    chrono::Utc.timestamp_millis_opt(ms?).single().map(|d| d.format("%Y-%m-%d").to_string())
}

pub fn parse(json: &str, period: Period, variable: Variable) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let cat = props.get("cat").and_then(|v| v.as_str()).unwrap_or("");
        if cat.is_empty() { return; }
        let rgb = category_color(cat, variable);
        let probability = props.get("prob").and_then(|v| v.as_i64());
        let issued = date(props.get("fcst_date").and_then(|v| v.as_i64()));
        let start = date(props.get("start_date").and_then(|v| v.as_i64()));
        let end = date(props.get("end_date").and_then(|v| v.as_i64()));
        let valid = props.get("valid_seas").and_then(|v| v.as_str()).map(str::to_owned)
            .or_else(|| match (start, end) { (Some(a), Some(b)) => Some(format!("{a} to {b}")), _ => None });
        let title = format!("CPC {} {}: {}", period.label(), variable.label(), cat);
        let detail = format!("NOAA CPC {} {} outlook\nCategory: {}\nProbability: {}\nIssued: {}\nValid: {}\nSource: NOAA/NWS Climate Prediction Center",
            period.label(), variable.label(), cat,
            probability.map(|p| format!("{p}%")).unwrap_or_else(|| "not reported".into()),
            issued.unwrap_or_else(|| "not reported".into()), valid.unwrap_or_else(|| "not reported".into()));
        for rings in polygons_of(geom) {
            out.push(GeoFeature { rings, fill: [rgb[0], rgb[1], rgb[2], 55], stroke: [rgb[0], rgb[1], rgb[2], 215],
                kind: FeatureKind::Outlook, title: title.clone(), detail: detail.clone(), alert: None });
        }
    })?;
    Ok(out)
}

fn kml_cell(description: &str, key: &str) -> Option<String> {
    let field = format!("<td>{key}</td>");
    let after = description.split_once(&field)?.1;
    let value = after.split_once("<td>")?.1.split_once("</td>")?.0.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn kml_date(value: Option<String>) -> String {
    value.and_then(|raw| chrono::NaiveDate::parse_from_str(&raw, "%m/%d/%Y").ok())
        .map(|day| day.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "not reported".to_owned())
}

/// CPC publishes its Week 3–4 outlooks as KML rather than on the NWS ArcGIS service.
/// The placemark description carries the category, probability and forecast dates.
pub fn parse_week34(kml: &str, variable: Variable) -> anyhow::Result<Vec<GeoFeature>> {
    anyhow::ensure!(kml.len() <= 8 * 1024 * 1024, "CPC Week 3–4 KML exceeds 8 MB");
    let mut out = Vec::new();
    for placemark in crate::gis::xml_elements(kml, "Placemark") {
        let Some(description) = crate::gis::xml_elements(placemark, "description").first().map(|d| crate::gis::xml_text(d)) else { continue };
        let Some(category) = kml_cell(&description, "Category") else { continue };
        let probability = kml_cell(&description, "Probability").unwrap_or_else(|| "not reported".to_owned());
        let issued = kml_date(kml_cell(&description, "Fcst Date"));
        let start = kml_date(kml_cell(&description, "Start Date"));
        let end = kml_date(kml_cell(&description, "End Date"));
        let rgb = category_color(&category, variable);
        let title = format!("CPC 3–4 week {}: {}", variable.label(), category);
        let detail = format!("NOAA CPC 3–4 week {} outlook\nCategory: {}\nProbability: {}%\nIssued: {}\nValid: {} to {}\nSource: NOAA/NWS Climate Prediction Center", variable.label(), category, probability, issued, start, end);
        for polygon in crate::gis::xml_elements(placemark, "Polygon") {
            let rings: Vec<_> = crate::gis::xml_elements(polygon, "LinearRing")
                .into_iter().map(crate::gis::xml_coordinates).filter(|ring| ring.len() >= 3).collect();
            if !rings.is_empty() {
                out.push(GeoFeature { rings, fill: [rgb[0], rgb[1], rgb[2], 55], stroke: [rgb[0], rgb[1], rgb[2], 215],
                    kind: FeatureKind::Outlook, title: title.clone(), detail: detail.clone(), alert: None });
            }
        }
    }
    anyhow::ensure!(!out.is_empty(), "CPC Week 3–4 KML has no outlook polygons");
    Ok(out)
}

pub async fn fetch(client: &reqwest::Client, period: Period, variable: Variable) -> anyhow::Result<Vec<GeoFeature>> {
    if period == Period::Week3To4 {
        let product = match variable { Variable::Temperature => "temp", Variable::Precipitation => "prcp" };
        let url = format!("https://ftp.cpc.ncep.noaa.gov/GIS/us_tempprcpfcst/wk34{product}_latest.kml");
        let body = client.get(crate::net::fetch_url(&url)).timeout(crate::net::FEED_TIMEOUT)
            .header("User-Agent", USER_AGENT).send().await?.error_for_status()?.text().await?;
        return parse_week34(&body, variable);
    }
    let (service, layer) = service(period, variable).ok_or_else(|| anyhow::anyhow!("CPC period has no ArcGIS service"))?;
    let fields = match period {
        Period::Day6To10 | Period::Day8To14 => "cat,prob,fcst_date,start_date,end_date",
        Period::Month1 | Period::Month3 => "cat,prob,fcst_date,valid_seas",
        Period::Week3To4 => unreachable!("handled above"),
    };
    let url = format!("{BASE}/{service}/MapServer/{layer}/query?where=1%3D1&outFields={fields}&geometryPrecision=3&maxAllowableOffset=0.03&outSR=4326&f=geojson");
    let body = client.get(crate::net::fetch_url(&url)).timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", USER_AGENT).send().await?.error_for_status()?.text().await?;
    parse(&body, period, variable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_public_cpc_fields_and_avoids_false_equivalence() {
        let sample = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"cat":"Above","prob":50,"fcst_date":1791849600000,"start_date":1792368000000,"end_date":1792713600000},"geometry":{"type":"Polygon","coordinates":[[[-100,35],[-99,35],[-99,36],[-100,35]]]}}]}"#;
        let f = parse(sample, Period::Day6To10, Variable::Temperature).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].title, "CPC 6–10 day Temperature: Above");
        assert!(f[0].detail.contains("Probability: 50%"));
        assert!(f[0].detail.contains("Valid: 2026-10-19 to 2026-10-23"));
        assert_eq!(f[0].fill[..3], [230, 103, 72]);
        assert_ne!(category_color("Above", Variable::Temperature), category_color("Above", Variable::Precipitation));
    }
    #[test]
    fn parses_week34_kml_placemark_metadata_and_geometry() {
        let kml = r#"<kml><Placemark><description>&lt;td&gt;Fcst Date&lt;/td&gt;&lt;td&gt;10/9/2026&lt;/td&gt;&lt;td&gt;Start Date&lt;/td&gt;&lt;td&gt;10/24/2026&lt;/td&gt;&lt;td&gt;End Date&lt;/td&gt;&lt;td&gt;11/6/2026&lt;/td&gt;&lt;td&gt;Probability&lt;/td&gt;&lt;td&gt;40&lt;/td&gt;&lt;td&gt;Category&lt;/td&gt;&lt;td&gt;Above&lt;/td&gt;</description><MultiGeometry><Polygon><outerBoundaryIs><LinearRing><coordinates>-100,35,0 -99,35,0 -99,36,0 -100,35,0</coordinates></LinearRing></outerBoundaryIs></Polygon></MultiGeometry></Placemark></kml>"#;
        let features = parse_week34(kml, Variable::Temperature).unwrap();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].title, "CPC 3–4 week Temperature: Above");
        assert!(features[0].detail.contains("Probability: 40%"));
        assert!(features[0].detail.contains("Valid: 2026-10-24 to 2026-11-06"));
        assert_eq!(features[0].rings[0].len(), 4);
    }
    #[tokio::test]
    #[ignore = "queries live NOAA CPC products"]
    async fn public_week34_products_have_inspectable_polygons() {
        let client = reqwest::Client::new();
        for variable in [Variable::Temperature, Variable::Precipitation] {
            let features = fetch(&client, Period::Week3To4, variable).await.unwrap();
            assert!(features.len() > 10);
            assert!(features.iter().any(|feature| feature.detail.contains("Issued: 20")));
            assert!(features.iter().all(|feature| !feature.rings.is_empty()));
        }
    }
    #[test]
    fn discussion_parser_extracts_only_the_forecast_product() {
        let html = "<html><h1>Navigation</h1><PRE>Prognostic Discussion for Monthly Outlook <br>NWS Climate Prediction Center<br>Issued today. Forecast details follow.</PRE><footer>Unrelated page footer</footer></html>";
        let discussion = parse_discussion(html, Period::Month1).unwrap();
        assert!(discussion.text.contains("NWS Climate Prediction Center\nIssued today"));
        assert!(!discussion.text.contains("Unrelated page footer"));
        let text = "Prognostic Discussion for Week 3-4 Temperature and Precipitation Outlooks<br>NWS Climate Prediction Center<br>Forecast details follow through the valid period.";
        assert!(parse_discussion(text, Period::Week3To4).unwrap().text.contains("\nNWS Climate"));
    }
    #[tokio::test]
    #[ignore = "queries live NOAA CPC discussion products"]
    async fn public_discussions_cover_every_period() {
        let client = reqwest::Client::new();
        for period in Period::ALL {
            let discussion = fetch_discussion(&client, period).await.unwrap();
            assert!(discussion.text.len() > 1000);
        }
    }
}
