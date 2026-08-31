//! SPC convective products: Day 1–3 outlooks, the Day 4–8 severe probabilities, and Mesoscale
//! Discussions.
//!
//! Outlooks come as static GeoJSON that already carries per-feature `fill`/`stroke` colors
//! and a risk `LABEL2`; MDs come from the NWS map service as GeoJSON. Both decode into the
//! shared [`GeoFeature`] type.

use crate::alerts::USER_AGENT;
use crate::overlay::{for_each_feature, polygons_of, FeatureKind, GeoFeature};

const OUTLOOK_BASE: &str = "https://www.spc.noaa.gov/products/outlook";
/// Days 4-8 live under the experimental products path and are one probabilistic layer per day
/// (no hazard split, no categorical risk — that far out SPC forecasts a single severe
/// probability, or says predictability is too low and means it).
const OUTLOOK_EXPER_BASE: &str = "https://www.spc.noaa.gov/products/exper/day4-8";
const MD_URL: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/spc_mesoscale_discussion/MapServer/0/query?where=1%3D1&outFields=*&f=geojson";

/// Watch polygons (tornado and severe thunderstorm) from the same map service the MDs come from,
/// so this needs no proxy-allowlist change. Layer 1 is the county-resolution watch/warning layer;
/// filtering to the two watch products server-side keeps the payload to the boxes in effect.
const WATCH_URL: &str = "https://mapservices.weather.noaa.gov/eventdriven/rest/services/WWA/watch_warn_adv/MapServer/1/query?where=prod_type+IN+%28%27Tornado+Watch%27%2C%27Severe+Thunderstorm+Watch%27%29&outFields=*&f=geojson";

/// Fill color for a categorical risk label, when the GeoJSON doesn't supply one.
pub(crate) fn risk_color(label: &str) -> [u8; 3] {
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
    pub const ALL: [OutlookKind; 4] = [
        OutlookKind::Categorical,
        OutlookKind::Tornado,
        OutlookKind::Wind,
        OutlookKind::Hail,
    ];

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
pub fn parse_outlook_kind(
    json: &str,
    day: u8,
    kind: OutlookKind,
) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        if kind == OutlookKind::Categorical {
            let label = {
                let l2 = str_of("LABEL2");
                if l2.is_empty() {
                    str_of("LABEL").to_string()
                } else {
                    l2.to_string()
                }
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
            push_polys(
                &mut out,
                geom,
                [rgb[0], rgb[1], rgb[2], 70],
                [rgb[0], rgb[1], rgb[2], 230],
                title,
                detail,
            );
        } else {
            let label = str_of("LABEL");
            if label.is_empty() {
                return;
            }
            let hazard = kind.label();
            // SIGN = 10%+ significant hazard hatch; probability labels are fractions like "0.05".
            if label.eq_ignore_ascii_case("SIGN") {
                let title = format!("Day {day} {hazard}: SIG");
                let detail = format!(
                    "SPC Day {day} {hazard} Outlook\nSignificant (10%+)\nValid: {}",
                    str_of("VALID")
                );
                // ponytail: SIG hatching approximated by translucent black — lyon has no hatch pattern.
                push_polys(&mut out, geom, [0, 0, 0, 60], [0, 0, 0, 200], title, detail);
            } else {
                let pct = label
                    .parse::<f32>()
                    .map(|f| (f * 100.0).round() as i32)
                    .unwrap_or(0);
                let rgb = hex_rgb(str_of("fill")).unwrap_or_else(|| risk_color(label));
                let title = format!("Day {day} {hazard}: {pct}%");
                let detail = format!(
                    "SPC Day {day} {hazard} Probability\n{pct}%\nValid: {}",
                    str_of("VALID")
                );
                push_polys(
                    &mut out,
                    geom,
                    [rgb[0], rgb[1], rgb[2], 70],
                    [rgb[0], rgb[1], rgb[2], 230],
                    title,
                    detail,
                );
            }
        }
    })?;
    Ok(out)
}

/// Push one `GeoFeature` per polygon part of `geom` with the given styling/text.
fn push_polys(
    out: &mut Vec<GeoFeature>,
    geom: &geojson::GeometryValue,
    fill: [u8; 4],
    stroke: [u8; 4],
    title: String,
    detail: String,
) {
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

/// The plain-text bulletin URL for an MCD whose `popupinfo` is `page`.
///
/// SPC serves every discussion twice: `md2032.html` for a browser and `md2032.txt` for the raw
/// product. The map service only ever hands out the HTML one, so the readout has to be derived.
fn md_text_url(page: &str) -> Option<String> {
    let page = page.trim();
    let rest = page
        .strip_prefix("https://")
        .or_else(|| page.strip_prefix("http://"))?;
    // Upgraded to https on the way out: the service still advertises these links as plain http.
    Some(format!("https://{}.txt", rest.strip_suffix(".html")?))
}

/// Strip the WMO routing header off a raw SPC product, leaving the discussion itself.
///
/// The first four lines are `ZCZC`, the AWIPS/WMO ids and the UGC zone line — machine routing
/// nobody clicked a polygon to read. Anything unrecognized is returned whole rather than
/// truncated on a guess.
fn strip_wmo_header(text: &str) -> &str {
    match text.find("\nMesoscale Discussion") {
        Some(i) => text[i + 1..].trim_end(),
        None => text.trim_end(),
    }
}

/// Parse an SPC Mesoscale Discussion GeoJSON payload into features paired with the URL of each
/// one's bulletin text, which the payload itself does not carry.
pub fn parse_md(json: &str) -> anyhow::Result<Vec<(GeoFeature, Option<String>)>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let name = str_of("name");
        let title = if name.is_empty() {
            "Mesoscale Discussion".to_string()
        } else {
            format!("Mesoscale Discussion {name}")
        };
        let page = str_of("popupinfo");
        let detail = format!("{}\n\n{page}", str_of("folderpath"));
        let text_url = md_text_url(page);
        for poly in polygons_of(geom) {
            out.push((
                GeoFeature {
                    rings: poly,
                    fill: [255, 120, 0, 30],
                    stroke: [255, 140, 0, 235],
                    kind: FeatureKind::MesoDiscussion,
                    title: title.clone(),
                    detail: detail.clone(),
                    alert: None,
                },
                text_url.clone(),
            ));
        }
    })?;
    Ok(out)
}

/// Parse the watch layer's GeoJSON into features.
///
/// Tornado watches are red and severe thunderstorm watches yellow, the colors SPC itself uses,
/// and both are drawn faint: a watch covers whole states for hours and must not compete with the
/// warning polygons inside it.
pub fn parse_watches(json: &str) -> anyhow::Result<Vec<GeoFeature>> {
    let mut out = Vec::new();
    for_each_feature(json, |geom, props| {
        let str_of = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let prod = str_of("prod_type");
        let tornado = prod.starts_with("Tornado");
        // The service returns one row per county in a watch, so a single watch arrives as dozens
        // of features. They keep their own polygons — merging them would need a union — but they
        // share a title, and the number is what identifies the watch to a chaser.
        let number = str_of("event");
        let title = if number.is_empty() {
            prod.to_string()
        } else {
            format!("{prod} {number}")
        };
        let mut detail = String::new();
        for (label, key) in [("Until", "ends"), ("Issued", "issuance"), ("Office", "wfo")] {
            let v = str_of(key);
            if !v.is_empty() {
                detail.push_str(&format!("{label}: {v}\n"));
            }
        }
        detail.push_str(str_of("url"));
        for poly in polygons_of(geom) {
            out.push(GeoFeature {
                rings: poly,
                fill: if tornado {
                    [230, 40, 40, 20]
                } else {
                    [230, 200, 30, 18]
                },
                stroke: if tornado {
                    [230, 40, 40, 235]
                } else {
                    [230, 200, 30, 235]
                },
                kind: FeatureKind::WatchBox,
                title: title.clone(),
                detail: detail.clone(),
                alert: None,
            });
        }
    })?;
    Ok(out)
}

/// Fetch the watches in effect.
pub async fn fetch_watches(client: &reqwest::Client) -> anyhow::Result<Vec<GeoFeature>> {
    let body = client
        .get(crate::net::fetch_url(WATCH_URL))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_watches(&body)
}

/// Fetch the categorical outlook for `day` (1–3), or the severe probability for a Day 4–8.
pub async fn fetch_outlook(client: &reqwest::Client, day: u8) -> anyhow::Result<Vec<GeoFeature>> {
    fetch_outlook_kind(client, day, OutlookKind::Categorical).await
}

/// Fetch an outlook for `day` and hazard `kind`. Probabilistic hazard layers are Day-1 only;
/// Days 4–8 ignore `kind` and fetch the single experimental probability layer.
pub async fn fetch_outlook_kind(
    client: &reqwest::Client,
    day: u8,
    kind: OutlookKind,
) -> anyhow::Result<Vec<GeoFeature>> {
    let url = if day >= 4 {
        format!("{OUTLOOK_EXPER_BASE}/day{day}prob.lyr.geojson")
    } else {
        format!("{OUTLOOK_BASE}/day{day}otlk_{}.lyr.geojson", kind.slug())
    };
    let body = client
        .get(crate::net::fetch_url(&url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_outlook_kind(&body, day, kind)
}

/// Fetch one MCD's bulletin text. `None` on any failure: a discussion whose text will not load
/// still has a polygon worth drawing, and the URL stays in the detail either way.
async fn fetch_md_text(client: &reqwest::Client, url: &str) -> Option<String> {
    let body = client
        .get(crate::net::fetch_url(url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .await
        .ok()?;
    Some(strip_wmo_header(&body).to_string())
}

/// Fetch active Mesoscale Discussions, each with its full bulletin text.
///
/// The GeoJSON carries a link and nothing else, so clicking a discussion used to show a URL. The
/// text is a second request per discussion, done here rather than on click so the popup opens
/// with the readout already in it.
pub async fn fetch_mesoscale_discussions(
    client: &reqwest::Client,
) -> anyhow::Result<Vec<GeoFeature>> {
    let body = client
        .get(crate::net::fetch_url(MD_URL))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    // ponytail: sequential, and cached per URL because one MultiPolygon MD yields several
    // features sharing a bulletin. A handful are active at once even on an outbreak day; reach
    // for join_all only if that stops being true.
    let mut texts: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for (mut feature, url) in parse_md(&body)? {
        if let Some(url) = url {
            if !texts.contains_key(&url) {
                if let Some(text) = fetch_md_text(client, &url).await {
                    texts.insert(url.clone(), text);
                }
            }
            if let Some(text) = texts.get(&url) {
                feature.detail = format!("{text}\n\n{}", feature.detail);
            }
        }
        out.push(feature);
    }
    Ok(out)
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

    /// Live check against SPC, run by hand: `cargo test -p wxdata live_mcd -- --ignored`.
    /// Quiet when no discussion is active — there is nothing to fetch on a calm day.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_mcd_text_arrives_with_the_polygon() {
        let client = reqwest::Client::new();
        let features = fetch_mesoscale_discussions(&client).await.unwrap();
        for f in &features {
            assert!(
                f.detail.starts_with("Mesoscale Discussion"),
                "no readout, only a link: {}",
                f.detail
            );
        }
    }

    #[test]
    fn a_discussion_link_becomes_its_bulletin_url() {
        // What the map service actually hands out, plain http and all.
        assert_eq!(
            md_text_url("http://www.spc.noaa.gov/products/md/md2032.html").as_deref(),
            Some("https://www.spc.noaa.gov/products/md/md2032.txt")
        );
        assert_eq!(md_text_url(""), None);
        assert_eq!(
            md_text_url("www.spc.noaa.gov/products/md/md2032.html"),
            None
        );
        assert_eq!(
            md_text_url("https://www.spc.noaa.gov/products/md/md2032"),
            None
        );
    }

    #[test]
    fn the_readout_starts_at_the_discussion_not_the_routing_header() {
        let raw = "ZCZC SPCSWOMCD ALL\nACUS11 KWNS 190154 \nSPC MCD 190154 \nSDZ000-190330-\n\n\
                   Mesoscale Discussion 2032\nNWS Storm Prediction Center Norman OK\n\nSUMMARY...\
                   hail\n";
        let out = strip_wmo_header(raw);
        assert!(out.starts_with("Mesoscale Discussion 2032"), "{out}");
        assert!(out.ends_with("hail"), "{out}");
        // A product that doesn't look like an MCD comes back whole rather than half-eaten.
        assert_eq!(strip_wmo_header("something else\n"), "something else");
    }

    #[test]
    fn parsing_a_discussion_yields_the_url_the_text_fetch_needs() {
        let json = r##"{"type":"FeatureCollection","features":[
            {"type":"Feature",
             "properties":{"name":"MD 2032","folderpath":"MD 2032 Active Till 0330 UTC",
                           "popupinfo":"http://www.spc.noaa.gov/products/md/md2032.html"},
             "geometry":{"type":"Polygon","coordinates":[[[-98.0,35.0],[-97.0,35.0],[-97.0,36.0],[-98.0,35.0]]]}}
        ]}"##;
        let out = parse_md(json).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.title, "Mesoscale Discussion MD 2032");
        assert_eq!(out[0].0.kind, FeatureKind::MesoDiscussion);
        assert_eq!(
            out[0].1.as_deref(),
            Some("https://www.spc.noaa.gov/products/md/md2032.txt")
        );
    }

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

    #[test]
    fn watches_parse_with_spc_colors_and_a_watch_number() {
        // Trimmed from a live response: one row per county, product name and number in props.
        let json = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature",
           "geometry":{"type":"Polygon","coordinates":[[[-100.0,48.0],[-101.0,48.0],[-101.0,49.0],[-100.0,48.0]]]},
           "properties":{"prod_type":"Tornado Watch","event":"0638","wfo":"KOUN",
                         "ends":"2026-08-30T23:00:00-05:00","url":"https://api.weather.gov/alerts/x"}},
          {"type":"Feature",
           "geometry":{"type":"Polygon","coordinates":[[[-90.0,38.0],[-91.0,38.0],[-91.0,39.0],[-90.0,38.0]]]},
           "properties":{"prod_type":"Severe Thunderstorm Watch","event":"0637","wfo":"KBIS"}}]}"#;
        let f = parse_watches(json).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].title, "Tornado Watch 0638");
        assert_eq!(f[0].kind, FeatureKind::WatchBox);
        assert_eq!(f[0].stroke, [230, 40, 40, 235]);
        assert!(f[0].detail.contains("Until: 2026-08-30T23:00:00-05:00"));
        assert_eq!(f[1].title, "Severe Thunderstorm Watch 0637");
        assert_eq!(f[1].stroke, [230, 200, 30, 235]);
        // A watch is a backdrop for the warnings inside it, so both fills stay nearly clear.
        assert!(f.iter().all(|x| x.fill[3] < 30));
    }
}
