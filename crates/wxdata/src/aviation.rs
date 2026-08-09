//! Aviation hazard polygons — SIGMETs and AIRMETs (G-AIRMETs are a follow-up) from
//! aviationweather.gov, decoded into [`GeoFeature`]s colored by hazard.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const AIRSIGMET_URL: &str = "https://aviationweather.gov/api/data/airsigmet";
const PIREP_URL: &str = "https://aviationweather.gov/api/data/pirep";

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
        let alt_line = match (
            alt("altitudeLow1").or_else(|| alt("altitudeLow2")),
            alt("altitudeHi1").or_else(|| alt("altitudeHi2")),
        ) {
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
        .get(crate::net::fetch_url(AIRSIGMET_URL))
        .query(&[("format", "geojson")])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body)
}

/// A pilot report: what someone actually flew through, at a known altitude.
///
/// The hazard summary is what makes it worth a marker — a turbulence or icing report is a
/// real-world observation of something no model or radar product measures directly.
#[derive(Debug, Clone, PartialEq)]
pub struct Pirep {
    pub lat: f64,
    pub lon: f64,
    /// Flight level in hundreds of feet, as reported.
    pub alt_ft: Option<i64>,
    pub ac_type: String,
    /// Short hazard summary ("Severe turbulence", "Moderate icing"), empty when it is a
    /// routine sky/wind report.
    pub hazard: String,
    pub urgent: bool,
    pub raw: String,
    pub time: chrono::DateTime<chrono::Utc>,
}

/// Turbulence/icing intensity words, worst first, as the API spells them.
fn intensity(word: &str) -> Option<&'static str> {
    let w = word.to_ascii_uppercase();
    // "NEGclr" and "NEG" mean the pilot looked and found nothing — not a hazard.
    if w.starts_with("NEG") || w.is_empty() {
        return None;
    }
    Some(match () {
        _ if w.contains("EXTRM") || w.contains("EXTREME") => "Extreme",
        _ if w.contains("SEV") => "Severe",
        _ if w.contains("MOD") => "Moderate",
        _ if w.contains("LGT") || w.contains("LIGHT") => "Light",
        _ => "Reported",
    })
}

/// Parse a `pirep?format=json` payload.
pub fn parse_pireps(json: &str) -> anyhow::Result<Vec<Pirep>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("not an array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for r in arr {
        let (Some(lat), Some(lon)) = (
            r.get("lat").and_then(|v| v.as_f64()),
            r.get("lon").and_then(|v| v.as_f64()),
        ) else {
            continue;
        };
        let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        // Worst reported turbulence, then icing; the two fields per report are two layers.
        let turb = intensity(&s("tbInt1")).or_else(|| intensity(&s("tbInt2")));
        let ice = intensity(&s("icgInt1")).or_else(|| intensity(&s("icgInt2")));
        let hazard = match (turb, ice) {
            (Some(t), Some(i)) => format!("{t} turbulence, {i} icing"),
            (Some(t), None) => format!("{t} turbulence"),
            (None, Some(i)) => format!("{i} icing"),
            (None, None) => String::new(),
        };
        // `obsTime` is Unix seconds, not milliseconds.
        let time = r
            .get("obsTime")
            .and_then(|v| v.as_i64())
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .unwrap_or_else(chrono::Utc::now);
        out.push(Pirep {
            lat,
            lon,
            alt_ft: r.get("fltLvl").and_then(|v| v.as_i64()).map(|fl| fl * 100),
            ac_type: s("acType"),
            urgent: s("pirepType").eq_ignore_ascii_case("URGENT PIREP"),
            hazard,
            raw: s("rawOb"),
            time,
        });
    }
    Ok(out)
}

/// Fetch recent pilot reports within a lat/lon bbox `(lat0, lon0, lat1, lon1)`.
pub async fn fetch_pireps(
    client: &reqwest::Client,
    lat0: f64,
    lon0: f64,
    lat1: f64,
    lon1: f64,
) -> anyhow::Result<Vec<Pirep>> {
    let bbox = format!("{lat0},{lon0},{lat1},{lon1}");
    let body = client
        .get(crate::net::fetch_url(PIREP_URL))
        .query(&[
            ("bbox", bbox.as_str()),
            ("format", "json"),
            // Two hours: older than that and a turbulence report is history, not a heads-up.
            ("age", "2"),
        ])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut p = parse_pireps(&body)?;
    p.truncate(300);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIREPS: &str = r#"[
      {"obsTime":1785606780,"acType":"TEX2","lat":31.0873,"lon":-101.3677,"fltLvl":220,
       "icgInt1":"","icgInt2":"","tbInt1":"NEG","tbInt2":"NEG","pirepType":"PIREP",
       "rawOb":"SJT UA /OV SJT240050/TM 1753/FL220/TP TEX2/TB NEG CAT"},
      {"obsTime":1785606720,"acType":"B739","lat":41.0,"lon":-85.0,"fltLvl":240,
       "icgInt1":"MOD","icgInt2":"","tbInt1":"SEV","tbInt2":"","pirepType":"Urgent PIREP",
       "rawOb":"FWA UA /OV FWA250015/TM 1749/FL240/TP B739/TB SEV"},
      {"acType":"C172","fltLvl":50,"rawOb":"no position"}]"#;

    #[test]
    fn pireps_summarize_their_hazards() {
        let p = parse_pireps(PIREPS).unwrap();
        assert_eq!(p.len(), 2, "the report with no position is skipped");
        // "Negative" turbulence is a pilot reporting smooth air, not a hazard.
        assert_eq!(p[0].hazard, "");
        assert_eq!(p[0].alt_ft, Some(22_000));
        assert!(!p[0].urgent);
        assert_eq!(p[1].hazard, "Severe turbulence, Moderate icing");
        assert!(p[1].urgent, "urgent PIREPs are flagged");
        assert_eq!(p[1].ac_type, "B739");
        assert_eq!(p[0].time.to_rfc3339(), "2026-08-01T17:53:00+00:00");
    }

    #[test]
    fn intensity_ignores_negative_reports() {
        assert_eq!(intensity("NEG"), None);
        assert_eq!(intensity("NEGclr"), None);
        assert_eq!(intensity(""), None);
        assert_eq!(intensity("SEV"), Some("Severe"));
        assert_eq!(intensity("OCNL LGT"), Some("Light"));
    }

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
