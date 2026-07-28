//! FAA WeatherCams: airport camera sites and their latest still images.
//!
//! The FAA runs ~2,600 cameras, thick in Alaska and thin but real across the lower 48, and serves
//! them keyless — you get a bbox of sites, then the newest frame per camera. That's the whole
//! appeal over Windy's webcam network, which needs an API key per user before a single image
//! loads. Windy remains the documented upgrade path if coverage ever matters more than setup:
//! swap [`fetch_bbox`] for their `/api/webcams/v2/list` and keep everything downstream.
//!
//! The API is the one the FAA's own map calls, and it 401s without a `Referer` from its own
//! origin. That's an origin check on a public dataset, not authentication, so we send one.

use crate::alerts::USER_AGENT;

const API: &str = "https://weathercams.faa.gov/api";
/// The site's own origin, which its API insists on seeing.
const REFERER: &str = "https://weathercams.faa.gov/";

/// One camera at a site: which way it points, and how to name it in a list.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub id: u64,
    pub name: String,
    /// Compass direction the lens faces ("NorthWest"), as the FAA labels it.
    pub direction: String,
    pub bearing: Option<f32>,
    pub out_of_order: bool,
}

/// A camera site — an airport or a remote strip — and the cameras standing on it.
#[derive(Debug, Clone, PartialEq)]
pub struct CamSite {
    pub id: u64,
    pub name: String,
    /// Airport identifier, e.g. "ENA".
    pub ident: String,
    pub icao: String,
    pub state: String,
    pub lat: f64,
    pub lon: f64,
    pub cameras: Vec<Camera>,
}

/// Parse the `/sites` payload. Tolerant: skips sites with no usable position, and sites that are
/// inactive or torn down for maintenance (their images would be stale or missing).
pub fn parse_sites(json: &str) -> Vec<CamSite> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.get("payload").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in arr {
        let (Some(lat), Some(lon)) = (num(s, "latitude"), num(s, "longitude")) else {
            continue;
        };
        if !flag(s, "siteActive", true) || flag(s, "siteInMaintenance", false) {
            continue;
        }
        let cameras = s
            .get("cameras")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| {
                        Some(Camera {
                            id: c.get("cameraId")?.as_u64()?,
                            name: text(c, "cameraName"),
                            direction: text(c, "cameraDirection"),
                            bearing: num(c, "cameraBearing").map(|b| b as f32),
                            out_of_order: flag(c, "cameraOutOfOrder", false)
                                || flag(c, "cameraInMaintenance", false),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if cameras.is_empty() {
            continue;
        }
        out.push(CamSite {
            id: s.get("siteId").and_then(|v| v.as_u64()).unwrap_or(0),
            name: text(s, "siteName"),
            ident: text(s, "siteIdentifier"),
            icao: text(s, "icao"),
            state: text(s, "state"),
            lat,
            lon,
            cameras,
        });
    }
    out
}

/// Pull the URL of the newest image out of a `/cameras/{id}/images/last/1` payload.
pub fn parse_latest_image(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let first = v.get("payload")?.as_array()?.first()?;
    let uri = first.get("imageUri")?.as_str()?;
    (!uri.is_empty()).then(|| uri.to_string())
}

fn text(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn num(v: &serde_json::Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| x.as_f64())
}

fn flag(v: &serde_json::Value, k: &str, default: bool) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(default)
}

/// Fetch the camera sites inside a lon/lat box. The server does the filtering, so a continental
/// view costs the same one request a county view does.
pub async fn fetch_bbox(
    client: &reqwest::Client,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
) -> anyhow::Result<Vec<CamSite>> {
    // The API wants "south,west|north,east" and rejects a zoom outside its own table; 10 is the
    // value its map uses for a regional view and it does not affect which sites come back.
    let bounds = format!("{min_lat},{min_lon}|{max_lat},{max_lon}");
    let body = client
        .get(format!("{API}/sites"))
        .query(&[("zoom", "10"), ("bounds", bounds.as_str())])
        .header("User-Agent", USER_AGENT)
        .header("Referer", REFERER)
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_sites(&body))
}

/// Fetch the URL of a camera's most recent still. `None` when the camera has posted nothing.
pub async fn latest_image(client: &reqwest::Client, camera_id: u64) -> anyhow::Result<Option<String>> {
    let body = client
        .get(format!("{API}/cameras/{camera_id}/images/last/1"))
        .header("User-Agent", USER_AGENT)
        .header("Referer", REFERER)
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_latest_image(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a live `/api/sites` response (Kenai, PAEN) plus two sites that must be
    /// dropped: one inactive, one with no cameras.
    const SITES: &str = r#"{"success":true,"count":3,"payload":[
      {"siteId":447,"siteName":"Kenai","siteIdentifier":"ENA","icao":"PAEN",
       "latitude":60.57007,"longitude":-151.24005,"state":"AK",
       "siteActive":true,"siteInMaintenance":false,
       "cameras":[
         {"cameraId":11613,"cameraName":"Camera 3","cameraDirection":"NorthWest","cameraBearing":295,"cameraOutOfOrder":false,"cameraInMaintenance":false},
         {"cameraId":11611,"cameraName":"Camera 1","cameraDirection":"North","cameraBearing":15,"cameraOutOfOrder":true,"cameraInMaintenance":false}
       ]},
      {"siteId":9,"siteName":"Decommissioned","latitude":61.0,"longitude":-150.0,
       "siteActive":false,"cameras":[{"cameraId":1,"cameraName":"Camera 1"}]},
      {"siteId":10,"siteName":"No cameras","latitude":62.0,"longitude":-149.0,
       "siteActive":true,"cameras":[]}
    ]}"#;

    #[test]
    fn parses_sites_and_drops_the_unusable_ones() {
        let sites = parse_sites(SITES);
        assert_eq!(sites.len(), 1, "inactive and camera-less sites must drop");
        let s = &sites[0];
        assert_eq!(s.ident, "ENA");
        assert_eq!(s.icao, "PAEN");
        assert_eq!(s.state, "AK");
        assert_eq!(s.cameras.len(), 2);
        assert_eq!(s.cameras[0].bearing, Some(295.0));
        // Out-of-order cameras are kept, flagged — the site still deserves a marker.
        assert!(s.cameras[1].out_of_order);
    }

    #[test]
    fn parses_latest_image_uri() {
        let json = r#"{"success":true,"count":1,"payload":[
          {"cameraId":11611,"imageUri":"https://images.wcams-static.faa.gov/webimages/447/28/11611-1785258226240.jpg",
           "imageDatetime":"2026-07-28T17:04:09.520Z"}]}"#;
        assert!(parse_latest_image(json).unwrap().ends_with(".jpg"));
        // An empty payload is a camera with nothing posted, not an error.
        assert_eq!(parse_latest_image(r#"{"payload":[]}"#), None);
        assert_eq!(parse_latest_image("not json"), None);
    }
}
