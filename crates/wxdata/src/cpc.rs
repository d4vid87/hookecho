//! NOAA CPC temperature and precipitation outlook polygons.
//!
//! These are public NWS ArcGIS GeoJSON feeds, independent of RadarOmega's private backend.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const BASE: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/outlooks";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variable { Temperature, Precipitation }

impl Variable {
    pub fn label(self) -> &'static str { match self { Self::Temperature => "Temperature", Self::Precipitation => "Precipitation" } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period { Day6To10, Day8To14, Month1, Month3 }

impl Period {
    pub const ALL: [Self; 4] = [Self::Day6To10, Self::Day8To14, Self::Month1, Self::Month3];
    pub fn label(self) -> &'static str { match self { Self::Day6To10 => "6–10 day", Self::Day8To14 => "8–14 day", Self::Month1 => "1 month", Self::Month3 => "3 month" } }
}

fn service(period: Period, variable: Variable) -> (&'static str, u8) {
    match (period, variable) {
        (Period::Day6To10, Variable::Temperature) => ("cpc_6_10_day_outlk", 0),
        (Period::Day6To10, Variable::Precipitation) => ("cpc_6_10_day_outlk", 1),
        (Period::Day8To14, Variable::Temperature) => ("cpc_8_14_day_outlk", 0),
        (Period::Day8To14, Variable::Precipitation) => ("cpc_8_14_day_outlk", 1),
        (Period::Month1, Variable::Temperature) => ("cpc_mthly_temp_outlk", 0),
        (Period::Month1, Variable::Precipitation) => ("cpc_mthly_precip_outlk", 0),
        (Period::Month3, Variable::Temperature) => ("cpc_sea_temp_outlk", 0),
        (Period::Month3, Variable::Precipitation) => ("cpc_sea_precip_outlk", 0),
    }
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

pub async fn fetch(client: &reqwest::Client, period: Period, variable: Variable) -> anyhow::Result<Vec<GeoFeature>> {
    let (service, layer) = service(period, variable);
    let fields = match period {
        Period::Day6To10 | Period::Day8To14 => "cat,prob,fcst_date,start_date,end_date",
        Period::Month1 | Period::Month3 => "cat,prob,fcst_date,valid_seas",
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
}
