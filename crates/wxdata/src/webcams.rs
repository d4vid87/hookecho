//! Webcams from two networks, sharing one shape: FAA WeatherCams and Windy.
//!
//! The FAA runs ~2,600 cameras, thick in Alaska and thin but real across the lower 48, and serves
//! them keyless — you get a bbox of sites, then the newest frame per camera. Its API is the one
//! the FAA's own map calls, and it 401s without a `Referer` from its own origin. That's an origin
//! check on a public dataset, not authentication, so we send one.
//!
//! [`fetch_windy_bbox`] covers everywhere the FAA doesn't, which is most of the planet, at the
//! cost of an API key the user supplies. Both produce [`CamSite`]s, so the marker drawing,
//! hit-testing and detail window downstream never learn which network a camera came from. Two
//! differences leak through and are handled at the call site rather than hidden:
//!
//! - Windy caps `limit` at 50 per request, so a Windy bbox is the 50 most popular cameras in
//!   view, not all of them. The FAA returns everything in the box.
//! - Windy's free-tier image URLs are token-secured and the tokens expire after **10 minutes**,
//!   so those URLs must never be persisted or cached across a refresh.

use crate::alerts::USER_AGENT;

const API: &str = "https://weathercams.faa.gov/api";
/// The site's own origin, which its API insists on seeing.
const REFERER: &str = "https://weathercams.faa.gov/";

const WINDY_API: &str = "https://api.windy.com/webcams/api/v3/webcams";
/// Windy's own maximum; asking for more is rejected rather than clamped.
const WINDY_LIMIT: usize = 50;

/// One camera at a site: which way it points, and how to name it in a list.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub id: u64,
    pub name: String,
    /// Compass direction the lens faces ("NorthWest"), as the FAA labels it.
    pub direction: String,
    pub bearing: Option<f32>,
    pub out_of_order: bool,
    /// Ready-to-use still image. Windy returns one inline, so there is no second round trip; FAA
    /// cameras leave this `None` and go through [`latest_image`].
    ///
    /// On Windy's free tier this URL carries a token that dies after ten minutes. Never persist
    /// it, and drop any decoded texture alongside the site list on refresh.
    pub image_url: Option<String>,
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
    /// The camera's page on its own network. Windy's terms require every image to link back to
    /// this; FAA cameras leave it `None`.
    pub link: Option<String>,
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
                            image_url: None,
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
            link: None,
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

/// Parse a `/webcams/api/v3/webcams` payload into the shared [`CamSite`] shape.
///
/// Tolerant in the same way [`parse_sites`] is — every field is probed rather than deserialised
/// into a struct, and a record missing a position or an image is skipped instead of failing the
/// batch. That is deliberate: this parser is written against Windy's published V3 schema, and the
/// nesting under `images` is the part most likely to differ in practice, so a surprise there
/// should cost one camera rather than the layer.
pub fn parse_windy(json: &str) -> Vec<CamSite> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.get("webcams").and_then(|w| w.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for w in arr {
        let loc = w.get("location");
        let (Some(lat), Some(lon)) = (
            loc.and_then(|l| num(l, "latitude")),
            loc.and_then(|l| num(l, "longitude")),
        ) else {
            continue;
        };
        // V3 renamed the identifier from a string `id` to a numeric `webcamId`.
        let id = w.get("webcamId").and_then(|v| v.as_u64()).unwrap_or(0);
        let title = text(w, "title");
        // `images.current.preview` is the documented still. Fall back through the other sizes so
        // a camera that only publishes a thumbnail still gets a marker.
        let image_url = w.get("images").and_then(|im| {
            let cur = im.get("current");
            ["preview", "thumbnail", "icon"]
                .iter()
                .find_map(|k| cur.and_then(|c| c.get(*k)).and_then(|s| s.as_str()))
                .map(|s| s.to_string())
        });
        out.push(CamSite {
            id,
            name: if title.is_empty() {
                format!("Webcam {id}")
            } else {
                title.clone()
            },
            // Windy has no airport identifiers; the location's own labels are the nearest thing.
            ident: String::new(),
            icao: String::new(),
            state: loc.map(|l| text(l, "country")).unwrap_or_default(),
            lat,
            lon,
            cameras: vec![Camera {
                id,
                name: title,
                // Windy does not report which way a camera faces.
                direction: String::new(),
                bearing: None,
                // `status` is "active" for a live camera; anything else is not worth opening.
                out_of_order: w
                    .get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| !s.eq_ignore_ascii_case("active")),
                image_url,
            }],
            // Required by Windy's terms: every image links back to its own page.
            link: w
                .get("urls")
                .and_then(|u| u.get("detail"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
        });
    }
    out
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

/// Fetch Windy webcams inside a lon/lat box. Needs the user's own API key.
///
/// Two of Windy's rules shape this call. `bbox` is ordered **north, east, south, west** — not the
/// west/south/east/north this codebase uses everywhere else — and `limit` cannot exceed 50, so
/// this asks for the 50 most *popular* cameras in view rather than an arbitrary 50. A continental
/// bbox therefore returns a sample, not a census; the layer's description says so.
pub async fn fetch_windy_bbox(
    client: &reqwest::Client,
    key: &str,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
) -> anyhow::Result<Vec<CamSite>> {
    anyhow::ensure!(!key.is_empty(), "no Windy API key");
    let bbox = format!("{max_lat},{max_lon},{min_lat},{min_lon}");
    let body = client
        .get(WINDY_API)
        .query(&[
            ("bbox", bbox.as_str()),
            ("include", "images,location,urls"),
            ("limit", &WINDY_LIMIT.to_string()),
            ("sortKey", "popularity"),
            ("sortDirection", "desc"),
        ])
        .header("User-Agent", USER_AGENT)
        .header("x-windy-api-key", key)
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_windy(&body))
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

    /// Shaped from Windy's published V3 schema: the envelope is `{total, webcams}`, the id is a
    /// number, and `urls.detail` is the link-back their terms require.
    const WINDY: &str = r#"{"total":3,"webcams":[
      {"webcamId":1358084658,"title":"Zermatt: Matterhorn","status":"active",
       "location":{"city":"Zermatt","country":"Switzerland","latitude":46.0207,"longitude":7.7491},
       "images":{"current":{"preview":"https://images.windy.com/webcams/preview.jpg",
                            "thumbnail":"https://images.windy.com/webcams/thumb.jpg"}},
       "urls":{"detail":"https://www.windy.com/webcams/1358084658"}},
      {"webcamId":2,"title":"Only a thumbnail","status":"inactive",
       "location":{"latitude":40.0,"longitude":-3.0},
       "images":{"current":{"thumbnail":"https://images.windy.com/webcams/t2.jpg"}},
       "urls":{"detail":"https://www.windy.com/webcams/2"}},
      {"webcamId":3,"title":"No position","status":"active","images":{"current":{}}}
    ]}"#;

    #[test]
    fn parses_windy_webcams() {
        let sites = parse_windy(WINDY);
        assert_eq!(sites.len(), 2, "a camera with no position must drop");
        let s = &sites[0];
        assert_eq!(s.id, 1358084658);
        assert_eq!(s.name, "Zermatt: Matterhorn");
        assert_eq!(s.state, "Switzerland");
        assert!((s.lat - 46.0207).abs() < 1e-6 && (s.lon - 7.7491).abs() < 1e-6);
        // The link-back is not optional — Windy's terms require it beside every image.
        assert_eq!(
            s.link.as_deref(),
            Some("https://www.windy.com/webcams/1358084658")
        );
        assert!(s.cameras[0].image_url.as_deref().unwrap().ends_with("preview.jpg"));
        assert!(!s.cameras[0].out_of_order);

        // Falls back down the size list, and a non-"active" status reads as out of order.
        assert!(sites[1].cameras[0].image_url.as_deref().unwrap().ends_with("t2.jpg"));
        assert!(sites[1].cameras[0].out_of_order);

        // Garbage and an empty envelope are empty layers, not errors.
        assert!(parse_windy("not json").is_empty());
        assert!(parse_windy(r#"{"total":0,"webcams":[]}"#).is_empty());
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
