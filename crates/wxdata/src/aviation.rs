//! Aviation hazard polygons — SIGMETs and AIRMETs (G-AIRMETs are a follow-up) from
//! aviationweather.gov, decoded into [`GeoFeature`]s colored by hazard.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const AIRSIGMET_URL: &str = "https://aviationweather.gov/api/data/airsigmet";

/// Base RGB per hazard string (CONVECTIVE / TURB / ICE / IFR / MTN OBSCN / ASH).
fn hazard_color(hazard: &str) -> [u8; 3] {
    match hazard {
        "CONVECTIVE" => [255, 100, 30],
        "TURB" => [240, 190, 50],
        "ICE" | "ICING" => [90, 200, 240],
        "IFR" => [140, 140, 210],
        "MTN OBSCN" | "MT_OBSC" => [150, 125, 95],
        "ASH" => [200, 60, 200],
        _ => [170, 170, 170],
    }
}

/// Parse an `airsigmet?format=geojson` payload into overlay features.
pub fn parse(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let s = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let hazard = s("hazard");
        let kind = s("airSigmetType"); // SIGMET | AIRMET
        if hazard.is_empty() && kind.is_empty() {
            return;
        }
        let rgb = hazard_color(hazard);
        let title = format!("{kind} {hazard}").trim().to_string();
        let alt = |k: &str| props.get(k).and_then(|v| v.as_i64());
        let alt_line = match (alt("altitudeLow1").or_else(|| alt("altitudeLow2")), alt("altitudeHi1").or_else(|| alt("altitudeHi2"))) {
            (Some(lo), Some(hi)) => format!("FL{:03}–FL{:03}", lo / 100, hi / 100),
            (None, Some(hi)) => format!("to FL{:03}", hi / 100),
            _ => String::new(),
        };
        let detail = format!(
            "{title}\n\n{}\nValid: {} → {}\n\n{}",
            alt_line,
            s("validTimeFrom"),
            s("validTimeTo"),
            s("rawAirSigmet"),
        );
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 25],
                stroke: [rgb[0], rgb[1], rgb[2], 200],
                kind: FeatureKind::Sigmet,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// Fetch all current SIGMETs/AIRMETs as overlay features.
pub async fn fetch_airsigmet(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    let body = client
        .get(AIRSIGMET_URL)
        .query(&[("format", "geojson")])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_convective_sigmet() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "properties":{"airSigmetType":"SIGMET","hazard":"CONVECTIVE","severity":5,
               "altitudeHi1":45000,"validTimeFrom":"2026-07-20T03:55:00Z","validTimeTo":"2026-07-20T05:55:00Z",
               "rawAirSigmet":"SIGMET 14W VALID..."},
             "geometry":{"type":"Polygon","coordinates":[[[-98,35],[-97,35],[-97,36],[-98,35]]]}},
            {"type":"Feature",
             "properties":{"airSigmetType":"AIRMET","hazard":"TURB"},
             "geometry":{"type":"Polygon","coordinates":[[[-90,40],[-89,40],[-89,41],[-90,40]]]}}]}"#;
        let f = parse(json).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].kind, FeatureKind::Sigmet);
        assert_eq!(f[0].title, "SIGMET CONVECTIVE");
        assert_eq!(f[0].stroke, [255, 100, 30, 200]);
        assert!(f[0].detail.contains("to FL450"));
        assert_eq!(f[1].title, "AIRMET TURB");
    }
}
