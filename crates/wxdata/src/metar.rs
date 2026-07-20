//! Surface observations (METAR) from aviationweather.gov, for station-plot overlays.
//!
//! A bounding-box query returns the current METARs; each becomes a [`SurfaceOb`] the app draws as
//! a station circle (flight-category colored) with a wind barb and temperature / dewpoint. Barb
//! geometry is generated in a unit frame by [`barb_segments`] and rotated to the wind direction at
//! draw time.

use crate::alerts::USER_AGENT;

const METAR_URL: &str = "https://aviationweather.gov/api/data/metar";

/// One surface observation (station plot).
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceOb {
    pub icao: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub temp_c: Option<f32>,
    pub dewp_c: Option<f32>,
    /// Wind direction (° from north); `None` for variable ("VRB") or calm.
    pub wdir_deg: Option<f32>,
    pub wspd_kt: f32,
    /// Flight category: VFR / MVFR / IFR / LIFR (empty if unreported).
    pub flt_cat: String,
    pub raw: String,
}

/// Parse an aviationweather.gov METAR JSON array. Tolerant: skips entries missing lat/lon.
pub fn parse(json: &str) -> Vec<SurfaceOb> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let Some(arr) = v.as_array() else { return Vec::new() };
    let mut out = Vec::new();
    for m in arr {
        let (Some(lat), Some(lon)) = (num(m, "lat"), num(m, "lon")) else { continue };
        // wdir arrives as an int, or the string "VRB" for variable wind.
        let wdir_deg = m.get("wdir").and_then(|w| w.as_f64()).map(|d| d as f32);
        out.push(SurfaceOb {
            icao: str_of(m, "icaoId"),
            name: str_of(m, "name"),
            lat,
            lon,
            temp_c: num(m, "temp").map(|t| t as f32),
            dewp_c: num(m, "dewp").map(|t| t as f32),
            wdir_deg,
            wspd_kt: num(m, "wspd").map(|s| s as f32).unwrap_or(0.0),
            flt_cat: str_of(m, "fltCat"),
            raw: str_of(m, "rawOb"),
        });
    }
    out
}

fn str_of(m: &serde_json::Value, k: &str) -> String {
    m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn num(m: &serde_json::Value, k: &str) -> Option<f64> {
    m.get(k).and_then(|v| v.as_f64())
}

/// Fetch the METARs within a lat/lon bounding box `(lat0, lon0, lat1, lon1)`. Capped at 200
/// stations. `USER_AGENT` identifies the app to NOAA.
pub async fn fetch_bbox(
    client: &reqwest::Client,
    lat0: f64,
    lon0: f64,
    lat1: f64,
    lon1: f64,
) -> anyhow::Result<Vec<SurfaceOb>> {
    let bbox = format!("{lat0},{lon0},{lat1},{lon1}");
    let body = client
        .get(METAR_URL)
        .query(&[("bbox", bbox.as_str()), ("format", "json")])
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut obs = parse(&body);
    obs.truncate(200);
    Ok(obs)
}

/// Generate wind-barb segments in a unit frame: origin at the station, shaft along +Y. US
/// convention — pennant/flag = 50 kt, full barb = 10 kt, half barb = 5 kt, rounded to nearest 5.
/// Calm (< 2.5 kt) returns no segments (the caller draws a hollow calm ring instead).
pub fn barb_segments(wspd_kt: f32) -> Vec<([f32; 2], [f32; 2])> {
    if wspd_kt < 2.5 {
        return Vec::new();
    }
    let mut segs = Vec::new();
    // Shaft from the station toward the wind source (drawn +Y here; rotated to wdir by the caller).
    const LEN: f32 = 1.0;
    segs.push(([0.0, 0.0], [0.0, LEN]));

    let mut speed = ((wspd_kt / 5.0).round() * 5.0) as i32;
    let flags = speed / 50;
    speed -= flags * 50;
    let fulls = speed / 10;
    speed -= fulls * 10;
    let halves = speed / 5;

    // Barbs hang off the tip end, marching back toward the station.
    let mut y = LEN;
    const STEP: f32 = 0.16;
    const BARB: f32 = 0.45; // half-length in −X
    for _ in 0..flags {
        // Pennant: a filled triangle, rendered as two outline segments (tip → base, base → shaft).
        let tip = [-BARB, y];
        let base = [0.0, y - STEP];
        segs.push(([0.0, y], tip));
        segs.push((tip, base));
        y -= STEP * 1.3;
    }
    for _ in 0..fulls {
        segs.push(([0.0, y], [-BARB, y + STEP * 0.5]));
        y -= STEP;
    }
    for _ in 0..halves {
        segs.push(([0.0, y], [-BARB * 0.5, y + STEP * 0.25]));
        y -= STEP;
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vrb_wind_and_missing_temp() {
        let json = r#"[
            {"icaoId":"KRQO","lat":35.47,"lon":-98.0,"temp":34,"dewp":19,"wdir":170,"wspd":11,"fltCat":"VFR","rawOb":"METAR KRQO ..."},
            {"icaoId":"KVRB","lat":36.0,"lon":-97.5,"wdir":"VRB","wspd":3,"rawOb":"METAR KVRB ... VRB03KT"}
        ]"#;
        let obs = parse(json);
        assert_eq!(obs.len(), 2);
        assert_eq!(obs[0].icao, "KRQO");
        assert_eq!(obs[0].temp_c, Some(34.0));
        assert_eq!(obs[0].wdir_deg, Some(170.0));
        // VRB wind → no numeric direction; missing temp → None.
        assert_eq!(obs[1].wdir_deg, None);
        assert_eq!(obs[1].temp_c, None);
    }

    #[test]
    fn barb_counts() {
        // Calm → nothing.
        assert!(barb_segments(1.0).is_empty());
        // 5 kt → shaft + one half barb.
        assert_eq!(barb_segments(5.0).len(), 2);
        // 20 kt → shaft + two full barbs.
        assert_eq!(barb_segments(20.0).len(), 3);
        // 65 kt → shaft + one flag (2 segs) + one full + one half.
        assert_eq!(barb_segments(65.0).len(), 5);
    }
}
