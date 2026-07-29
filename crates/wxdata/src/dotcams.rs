//! Highway cameras from state DOTs — the video half of the camera catalog.
//!
//! There is no national DOT camera API. Every state runs its own, and most of the big ones (the
//! 511 sites built on the same commercial platform) want a per-user API key before they return a
//! single camera, which is exactly the friction the FAA feed lets us avoid. So this module carries
//! the agencies that publish openly, and says plainly which those are rather than implying
//! nationwide coverage:
//!
//! * **Caltrans** — twelve district files, keyless, each camera carrying a lat/lon, a still image
//!   and, for most of them, a live HLS playlist. That last part is the point: these are the public
//!   cameras that actually stream video rather than posting a still every few minutes.
//!
//! Anything else is a camera the user adds by URL in Settings, which the player handles the same
//! way. New open agencies drop in as another entry in [`SOURCES`].

use crate::alerts::USER_AGENT;

/// One highway camera.
#[derive(Debug, Clone, PartialEq)]
pub struct DotCam {
    /// Stable id: agency plus the agency's own index.
    pub id: String,
    /// Operating agency, for the card's owner chip ("Caltrans D3").
    pub agency: String,
    /// What the camera looks at ("Hwy 5 at Pocket").
    pub name: String,
    /// Nearest town, where the agency names one.
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    /// Live video (HLS playlist), when the agency streams this camera.
    pub stream_url: Option<String>,
    /// Latest still, which nearly every camera has even when it does not stream.
    pub image_url: Option<String>,
}

/// An open agency feed: a display name and the URL to pull.
pub struct Source {
    pub agency: &'static str,
    pub url: &'static str,
}

/// Every agency feed we can fetch without a key. Verified live 2026-07-29.
pub const SOURCES: &[Source] = &[
    caltrans(1),
    caltrans(2),
    caltrans(3),
    caltrans(4),
    caltrans(5),
    caltrans(6),
    caltrans(7),
    caltrans(8),
    caltrans(9),
    caltrans(10),
    caltrans(11),
    caltrans(12),
];

/// Caltrans publishes one file per district; the path uses a bare number and the filename a
/// zero-padded one.
const fn caltrans(district: u8) -> Source {
    // `const fn` can't format, so the twelve URLs are spelled out by index.
    const AGENCIES: [&str; 13] = [
        "",
        "Caltrans D1",
        "Caltrans D2",
        "Caltrans D3",
        "Caltrans D4",
        "Caltrans D5",
        "Caltrans D6",
        "Caltrans D7",
        "Caltrans D8",
        "Caltrans D9",
        "Caltrans D10",
        "Caltrans D11",
        "Caltrans D12",
    ];
    const URLS: [&str; 13] = [
        "",
        "https://cwwp2.dot.ca.gov/data/d1/cctv/cctvStatusD01.json",
        "https://cwwp2.dot.ca.gov/data/d2/cctv/cctvStatusD02.json",
        "https://cwwp2.dot.ca.gov/data/d3/cctv/cctvStatusD03.json",
        "https://cwwp2.dot.ca.gov/data/d4/cctv/cctvStatusD04.json",
        "https://cwwp2.dot.ca.gov/data/d5/cctv/cctvStatusD05.json",
        "https://cwwp2.dot.ca.gov/data/d6/cctv/cctvStatusD06.json",
        "https://cwwp2.dot.ca.gov/data/d7/cctv/cctvStatusD07.json",
        "https://cwwp2.dot.ca.gov/data/d8/cctv/cctvStatusD08.json",
        "https://cwwp2.dot.ca.gov/data/d9/cctv/cctvStatusD09.json",
        "https://cwwp2.dot.ca.gov/data/d10/cctv/cctvStatusD10.json",
        "https://cwwp2.dot.ca.gov/data/d11/cctv/cctvStatusD11.json",
        "https://cwwp2.dot.ca.gov/data/d12/cctv/cctvStatusD12.json",
    ];
    Source {
        agency: AGENCIES[district as usize],
        url: URLS[district as usize],
    }
}

/// Parse a Caltrans district file. Out-of-service cameras are dropped — their images are stale and
/// their streams 404.
pub fn parse_caltrans(json: &str, agency: &str) -> Vec<DotCam> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in arr {
        let Some(c) = entry.get("cctv") else { continue };
        if c.get("inService").and_then(|s| s.as_str()) == Some("false") {
            continue;
        }
        let loc = c.get("location");
        // Caltrans quotes its numbers, so every coordinate arrives as a string.
        let num = |k: &str| {
            loc.and_then(|l| l.get(k))
                .and_then(|x| x.as_str())
                .and_then(|s| s.trim().parse::<f64>().ok())
        };
        let (Some(lat), Some(lon)) = (num("latitude"), num("longitude")) else {
            continue;
        };
        if lat == 0.0 && lon == 0.0 {
            continue;
        }
        let text = |v: Option<&serde_json::Value>, k: &str| {
            v.and_then(|v| v.get(k))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let img = c.get("imageData");
        let url = |v: Option<&serde_json::Value>, k: &str| {
            let s = text(v, k);
            (s.starts_with("http")).then_some(s)
        };
        out.push(DotCam {
            id: format!("{agency}:{}", text(Some(c), "index")),
            agency: agency.to_string(),
            name: text(loc, "locationName"),
            place: text(loc, "nearbyPlace"),
            lat,
            lon,
            stream_url: url(img, "streamingVideoURL"),
            image_url: url(img.and_then(|i| i.get("static")), "currentImageURL"),
        });
    }
    out
}

/// Fetch one agency feed.
pub async fn fetch_source(client: &reqwest::Client, s: &Source) -> anyhow::Result<Vec<DotCam>> {
    let body = client
        .get(s.url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_caltrans(&body, s.agency))
}

/// Fetch every agency whose coverage could reach a lon/lat box, and keep the cameras inside it.
///
/// The district files are megabytes each and change slowly, so the caller is expected to hold the
/// result rather than poll it; there is no per-view query to push the filtering onto.
pub async fn fetch_bbox(
    client: &reqwest::Client,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
) -> anyhow::Result<Vec<DotCam>> {
    // Every source we carry is Californian; skip the whole download for a view nowhere near it.
    if !boxes_overlap(
        (min_lon, min_lat, max_lon, max_lat),
        (-125.0, 32.0, -113.5, 42.5),
    ) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for s in SOURCES {
        match fetch_source(client, s).await {
            Ok(cams) => out.extend(cams.into_iter().filter(|c| {
                c.lon >= min_lon && c.lon <= max_lon && c.lat >= min_lat && c.lat <= max_lat
            })),
            // One district being down must not blank out the other eleven.
            Err(e) => log::warn!("{} cameras: {e}", s.agency),
        }
    }
    Ok(out)
}

fn boxes_overlap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 <= b.2 && a.2 >= b.0 && a.1 <= b.3 && a.3 >= b.1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a live district-3 file, plus one camera out of service and one with no fix.
    const D3: &str = r#"{"data":[
      {"cctv":{"index":"1","inService":"true",
        "location":{"locationName":"Hwy 5 at Pocket","nearbyPlace":"Sacramento",
          "longitude":"-121.510528","latitude":"38.481128","county":"Sacramento","route":"I-5"},
        "imageData":{"streamingVideoURL":"https://wzmedia.dot.ca.gov/D3/5_Pocket.stream/playlist.m3u8",
          "static":{"currentImageURL":"https://cwwp2.dot.ca.gov/data/d3/cctv/image/a/a.jpg"}}}},
      {"cctv":{"index":"2","inService":"false",
        "location":{"locationName":"Dark","longitude":"-121.0","latitude":"38.0"},
        "imageData":{"static":{"currentImageURL":"https://x/b.jpg"}}}},
      {"cctv":{"index":"3","inService":"true",
        "location":{"locationName":"No fix","longitude":"","latitude":""},
        "imageData":{"streamingVideoURL":"","static":{"currentImageURL":""}}}}
    ]}"#;

    #[test]
    fn parses_cameras_and_drops_the_unusable() {
        let cams = parse_caltrans(D3, "Caltrans D3");
        assert_eq!(cams.len(), 1, "out-of-service and fix-less cameras must drop");
        let c = &cams[0];
        assert_eq!(c.id, "Caltrans D3:1");
        assert_eq!(c.place, "Sacramento");
        assert!((c.lat - 38.481128).abs() < 1e-6);
        assert!(c.stream_url.as_ref().unwrap().ends_with(".m3u8"));
        assert!(c.image_url.is_some());
        assert!(parse_caltrans("not json", "x").is_empty());
    }

    #[test]
    fn empty_stream_url_is_none_not_an_empty_string() {
        let json = r#"{"data":[{"cctv":{"index":"9","inService":"true",
          "location":{"locationName":"Still only","longitude":"-120.0","latitude":"37.0"},
          "imageData":{"streamingVideoURL":"","static":{"currentImageURL":"https://x/c.jpg"}}}}]}"#;
        let c = &parse_caltrans(json, "Caltrans D10")[0];
        assert_eq!(c.stream_url, None, "a still-only camera has no stream");
        assert!(c.image_url.is_some());
    }

    #[test]
    fn every_source_url_is_spelled_right() {
        assert_eq!(SOURCES.len(), 12);
        for s in SOURCES {
            assert!(s.url.starts_with("https://"), "{}", s.agency);
            assert!(!s.agency.is_empty());
        }
    }

    /// The district files are hand-maintained by the agency; check they still parse when a camera
    /// looks wrong. Run with `--ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_caltrans_districts_parse() {
        let client = reqwest::Client::new();
        for s in SOURCES {
            let cams = fetch_source(&client, s).await.unwrap();
            let streaming = cams.iter().filter(|c| c.stream_url.is_some()).count();
            println!("{}: {} cameras, {streaming} streaming", s.agency, cams.len());
            assert!(!cams.is_empty(), "{} returned nothing", s.agency);
        }
    }
}
