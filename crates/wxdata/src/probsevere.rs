//! NOAA/CIMSS ProbSevere v3 — per-storm machine-learning probabilities of severe hazards.
//!
//! Feed: <https://mrms.ncep.noaa.gov/data/ProbSevere/PROBSEVERE/> (public, ~2-min cadence).
//! A GeoJSON FeatureCollection of storm-object polygons whose properties carry integer percent
//! probabilities: `ProbSevere`, `ProbTor`, `ProbHail`, `ProbWind`. Decoded into the shared
//! [`GeoFeature`] so it draws + hit-tests like the alert/outlook layers; the storm's dominant
//! probability drives its color and the map badge (via `title`).

use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const PROBSEVERE_DIR: &str = "https://mrms.ncep.noaa.gov/data/ProbSevere/PROBSEVERE/";

/// Fetch the latest ProbSevere FeatureCollection.
pub async fn fetch_probsevere(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    // The directory index lists timestamped files; the last one is newest.
    let index = client
        .get(PROBSEVERE_DIR)
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let file = latest_file(&index)
        .ok_or_else(|| anyhow::anyhow!("no ProbSevere file in directory index"))?;
    let body = client
        .get(format!("{PROBSEVERE_DIR}{file}"))
        .header("User-Agent", crate::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_probsevere(&body)
}

/// Pick the newest `MRMS_PROBSEVERE_*.json` from an Apache directory listing (lexical max = newest,
/// since names are zero-padded `YYYYMMDD_HHMMSS`).
fn latest_file(index: &str) -> Option<String> {
    index
        .split(|c| c == '"' || c == '<' || c == '>')
        .filter(|s| s.starts_with("MRMS_PROBSEVERE_") && s.ends_with(".json"))
        .max()
        .map(str::to_string)
}

/// Color ramp for a dominant probability percent: 0 green → 50 yellow → 100 red.
fn prob_color(pct: u8) -> [u8; 3] {
    let t = (pct as f32 / 100.0).clamp(0.0, 1.0);
    if t < 0.5 {
        let k = t / 0.5;
        [(60.0 + k * 195.0) as u8, 200, 60]
    } else {
        let k = (t - 0.5) / 0.5;
        [255, (200.0 - k * 160.0) as u8, (60.0 - k * 20.0) as u8]
    }
}

/// Parse a ProbSevere FeatureCollection into colored, badged [`GeoFeature`]s.
pub fn parse_probsevere(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let num = |k: &str| props.get(k).and_then(|v| v.as_str()).and_then(|s| s.parse::<u8>().ok());
        let severe = num("ProbSevere").unwrap_or(0);
        let tor = num("ProbTor").unwrap_or(0);
        let hail = num("ProbHail").unwrap_or(0);
        let wind = num("ProbWind").unwrap_or(0);
        let dominant = severe.max(tor).max(hail).max(wind);
        let id = props.get("ID").and_then(|v| v.as_str()).unwrap_or("");
        let rgb = prob_color(dominant);
        // Badge shows the dominant probability; the storm's leading hazard tags it.
        let lead = [("Tor", tor), ("Hail", hail), ("Wind", wind), ("Svr", severe)]
            .into_iter()
            .max_by_key(|&(_, p)| p)
            .map(|(l, _)| l)
            .unwrap_or("Svr");
        let title = format!("{lead} {dominant}%");
        let detail = format!(
            "ProbSevere storm {id}\nSevere: {severe}%\nTornado: {tor}%\nHail: {hail}%\nWind: {wind}%",
        );
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: [rgb[0], rgb[1], rgb[2], 22],
                stroke: [rgb[0], rgb[1], rgb[2], 235],
                kind: FeatureKind::ProbSevere,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probsevere_features() {
        let json = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[-97.5,35.0],[-97.4,35.0],[-97.4,35.1],[-97.5,35.1],[-97.5,35.0]]]},
             "properties":{"ID":"12345","ProbSevere":"78","ProbTor":"12","ProbHail":"64","ProbWind":"40"}}
        ]}"#;
        let f = parse_probsevere(json).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, FeatureKind::ProbSevere);
        // Dominant = ProbSevere 78 → badge "Svr 78%".
        assert_eq!(f[0].title, "Svr 78%");
        assert!(f[0].detail.contains("Tornado: 12%"));
        assert_eq!(f[0].rings[0].len(), 5);
    }

    #[test]
    fn latest_file_picks_newest() {
        let idx = r#"<a href="MRMS_PROBSEVERE_20260719_000042.json">x</a>
                     <a href="MRMS_PROBSEVERE_20260719_000442.json">y</a>
                     <a href="MRMS_PROBSEVERE_20260718_235839.json">z</a>"#;
        assert_eq!(latest_file(idx).as_deref(), Some("MRMS_PROBSEVERE_20260719_000442.json"));
    }
}
