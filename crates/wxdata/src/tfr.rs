//! Temporary Flight Restrictions from the FAA.
//!
//! A TFR is airspace you may not fly through right now: a wildfire with tankers working it, a
//! stadium, a VIP movement, a rocket launch. For a drone operator or a chase pilot it is the
//! difference between a legal flight and a violation, and no other layer in the app carries it.
//!
//! This is FAA data, not aviationweather.gov's, and it comes in two steps with two formats. The
//! list ([`LIST_URL`]) is JSON and carries no geometry at all — only a NOTAM id per restriction.
//! The shape lives in a per-NOTAM XNOTAM XML document, which is why fetching is staged and
//! aggressively cached: a TFR's outline does not change once it is issued.

use crate::alerts::USER_AGENT;
use crate::overlay::{FeatureKind, GeoFeature};

/// The active TFR list. JSON, keyless, no geometry.
pub const LIST_URL: &str = "https://tfr.faa.gov/tfrapi/exportTfrList";

/// One row of the list — everything known before the shape is fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct TfrEntry {
    /// NOTAM id as issued, e.g. `6/7631`.
    pub notam_id: String,
    /// FAA's category: HAZARDS, SECURITY, VIP, SPACE OPERATIONS, …
    pub kind: String,
    pub description: String,
    pub state: String,
}

/// Colour per FAA TFR category. Security and VIP restrictions are the ones with teeth, so they
/// are the loudest; hazard areas (fire, spills) are the common case and stay calmer.
fn kind_color(kind: &str) -> [u8; 3] {
    match kind {
        "SECURITY" | "VIP" => [230, 60, 60],
        "SPACE OPERATIONS" => [200, 60, 200],
        "AIR SHOWS/SPORTS" | "UAS PUBLIC GATHERING" => [240, 170, 60],
        "HAZARDS" => [235, 120, 40],
        _ => [190, 130, 90],
    }
}

/// Parse the active-TFR list.
pub fn parse_list(json: &str) -> anyhow::Result<Vec<TfrEntry>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("TFR list is not an array"))?;
    Ok(arr
        .iter()
        .filter_map(|e| {
            let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let notam_id = s("notam_id");
            (!notam_id.is_empty()).then(|| TfrEntry {
                notam_id,
                kind: s("type"),
                description: s("description"),
                state: s("state"),
            })
        })
        .collect())
}

/// The detail document for a NOTAM id. The id's `/` becomes `_` in the filename.
pub fn detail_url(notam_id: &str) -> String {
    format!(
        "https://tfr.faa.gov/download/detail_{}.xml",
        notam_id.replace('/', "_")
    )
}

/// One XML element's text content, first occurrence only.
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim())
}

/// An FAA coordinate string (`41.2000473N`, `123.65833333W`) as signed degrees.
fn coord(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, hemi) = s.split_at(s.len().checked_sub(1)?);
    let v: f64 = num.parse().ok()?;
    match hemi {
        "N" | "E" => Some(v),
        "S" | "W" => Some(-v),
        _ => None,
    }
}

/// Every `<Avx>` vertex inside `xml`, in document order.
fn vertices(xml: &str) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    for chunk in xml.split("<Avx>").skip(1) {
        let Some(body) = chunk.split("</Avx>").next() else {
            continue;
        };
        if let (Some(lat), Some(lon)) = (
            tag(body, "geoLat").and_then(coord),
            tag(body, "geoLong").and_then(coord),
        ) {
            out.push([lon, lat]);
        }
    }
    out
}

/// Build the drawable feature for one TFR from its XNOTAM detail document.
///
/// The FAA describes the same restriction twice: once as it was written (a circle, an arc, a
/// corridor) and once as `<abdMergedArea>`, a densified ring of great-circle points. The second
/// is what to draw — no arc maths, and multi-part restrictions arrive already merged. A
/// restriction defined purely as a circle has no merged area, so that case is drawn from the
/// centre and radius instead.
pub fn parse_detail(xml: &str, entry: &TfrEntry) -> Option<GeoFeature> {
    let ring = match xml.find("<abdMergedArea>") {
        Some(i) => {
            let body = &xml[i..];
            let body = body.split("</abdMergedArea>").next().unwrap_or(body);
            vertices(body)
        }
        None => circle_ring(xml)?,
    };
    // Two points is a line, not an area; below that there is nothing to fill.
    if ring.len() < 3 {
        return None;
    }
    let rgb = kind_color(&entry.kind);
    let alt = match (
        tag(xml, "valDistVerLower").and_then(|v| v.parse::<f64>().ok()),
        tag(xml, "valDistVerUpper").and_then(|v| v.parse::<f64>().ok()),
    ) {
        (Some(lo), Some(hi)) => format!(
            "\n{} to {hi:.0} ft",
            if lo <= 0.0 {
                "Surface".to_string()
            } else {
                format!("{lo:.0} ft")
            }
        ),
        _ => String::new(),
    };
    let effective = match (tag(xml, "dateEffective"), tag(xml, "dateExpire")) {
        (Some(a), Some(b)) => format!("\nEffective {a} to {b}"),
        _ => String::new(),
    };
    Some(GeoFeature {
        rings: vec![ring],
        // Airspace you may not enter reads as a hatched-looking boundary, not a solid block of
        // colour over the weather: the radar underneath is the reason the app is open.
        fill: [rgb[0], rgb[1], rgb[2], 40],
        stroke: [rgb[0], rgb[1], rgb[2], 230],
        kind: FeatureKind::Tfr,
        title: format!("TFR {} — {}", entry.notam_id, entry.kind),
        detail: format!(
            "{}{alt}{effective}\n\nNOTAM {}",
            entry.description, entry.notam_id
        ),
        alert: None,
    })
}

/// A ring approximating a TFR given only as a centre and a radius.
fn circle_ring(xml: &str) -> Option<Vec<[f64; 2]>> {
    let i = xml.find("<Avx>")?;
    let body = xml[i..].split("</Avx>").next()?;
    let lat = tag(body, "geoLat").and_then(coord)?;
    let lon = tag(body, "geoLong").and_then(coord)?;
    let nm: f64 = tag(body, "valRadiusArc")?.parse().ok()?;
    let km = nm * 1.852;
    // 60 points is smooth at any zoom a TFR is legible at.
    Some(
        (0..=60)
            .map(|i| {
                let brg = i as f64 * 6.0;
                let r = km / 6371.0088;
                let (p0, l0) = (lat.to_radians(), lon.to_radians());
                let b = brg.to_radians();
                let p = (p0.sin() * r.cos() + p0.cos() * r.sin() * b.cos()).asin();
                let l = l0 + (b.sin() * r.sin() * p0.cos()).atan2(r.cos() - p0.sin() * p.sin());
                [l.to_degrees(), p.to_degrees()]
            })
            .collect(),
    )
}

/// Fetch the list, then the shapes for every restriction not already in `have`.
///
/// ponytail: shapes are fetched serially in a bounded batch rather than all 135 at once. A TFR's
/// outline never changes once issued, so the caller keeps them and this does nothing on almost
/// every refresh — the batch cap only shapes the very first load. Raise it, or fetch
/// concurrently, if that first load turns out to feel slow.
pub async fn fetch(
    client: &reqwest::Client,
    have: &[String],
    max_new: usize,
) -> anyhow::Result<(Vec<(String, GeoFeature)>, usize)> {
    let body = client
        .get(crate::net::fetch_url(LIST_URL))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let list = parse_list(&body)?;
    let wanted: Vec<TfrEntry> = list
        .into_iter()
        .filter(|e| !have.contains(&e.notam_id))
        .collect();
    let remaining = wanted.len().saturating_sub(max_new);
    let mut out = Vec::new();
    for entry in wanted.into_iter().take(max_new) {
        let url = detail_url(&entry.notam_id);
        // One restriction's detail failing is not the layer failing: skip it and keep the rest.
        let Ok(resp) = client
            .get(crate::net::fetch_url(&url))
            .header("User-Agent", USER_AGENT)
            .send()
            .await
        else {
            continue;
        };
        let Ok(xml) = resp.text().await else { continue };
        if let Some(f) = parse_detail(&xml, &entry) {
            out.push((entry.notam_id.clone(), f));
        } else {
            // Remember the miss too, or an unparseable TFR is refetched forever.
            out.push((
                entry.notam_id.clone(),
                GeoFeature {
                    rings: Vec::new(),
                    fill: [0; 4],
                    stroke: [0; 4],
                    kind: FeatureKind::Tfr,
                    title: String::new(),
                    detail: String::new(),
                    alert: None,
                },
            ));
        }
    }
    Ok((out, remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"[
      {"notam_id":"6/7631","type":"HAZARDS","facility":"ZSE","state":"CA",
       "description":"12NM N WILLOW CREEK, CA","creation_date":"08/25/2026"},
      {"notam_id":"","type":"VIP","facility":"ZDC","state":"DC",
       "description":"no id","creation_date":"08/25/2026"}
    ]"#;

    #[test]
    fn the_list_parses_and_skips_rows_without_an_id() {
        let l = parse_list(LIST).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].notam_id, "6/7631");
        assert_eq!(l[0].kind, "HAZARDS");
    }

    #[test]
    fn the_detail_url_swaps_the_slash() {
        assert_eq!(
            detail_url("6/7631"),
            "https://tfr.faa.gov/download/detail_6_7631.xml"
        );
    }

    /// West and south are negative; anything else is not a coordinate.
    #[test]
    fn coordinates_carry_their_hemisphere() {
        assert_eq!(coord("41.2000473N"), Some(41.2000473));
        assert_eq!(coord("123.65833333W"), Some(-123.65833333));
        assert_eq!(coord("12.0S"), Some(-12.0));
        assert_eq!(coord("12.0"), None);
        assert_eq!(coord(""), None);
    }

    #[test]
    fn the_merged_area_becomes_the_ring() {
        let xml = "<Not><abdMergedArea>\
            <Avx><codeType>GRC</codeType><geoLat>41.0N</geoLat><geoLong>123.0W</geoLong></Avx>\
            <Avx><codeType>GRC</codeType><geoLat>41.5N</geoLat><geoLong>123.0W</geoLong></Avx>\
            <Avx><codeType>GRC</codeType><geoLat>41.5N</geoLat><geoLong>122.5W</geoLong></Avx>\
            </abdMergedArea>\
            <valDistVerLower>0</valDistVerLower><valDistVerUpper>9000</valDistVerUpper></Not>";
        let e = TfrEntry {
            notam_id: "6/7631".into(),
            kind: "HAZARDS".into(),
            description: "willow creek".into(),
            state: "CA".into(),
        };
        let f = parse_detail(xml, &e).expect("a ring");
        assert_eq!(f.rings[0].len(), 3);
        assert_eq!(f.rings[0][0], [-123.0, 41.0]);
        assert!(f.title.contains("6/7631"));
        assert!(f.detail.contains("Surface to 9000 ft"), "{}", f.detail);
    }

    /// A circle-only restriction still gets drawn, from its centre and radius.
    #[test]
    fn a_circle_only_restriction_is_drawn_as_a_circle() {
        let xml = "<Abd><Avx><codeType>CIR</codeType><geoLat>41.1166N</geoLat>\
            <geoLong>123.6583W</geoLong><valRadiusArc>5.0</valRadiusArc></Avx></Abd>";
        let e = TfrEntry {
            notam_id: "6/1".into(),
            kind: "SECURITY".into(),
            description: "d".into(),
            state: "CA".into(),
        };
        let f = parse_detail(xml, &e).expect("a circle");
        assert_eq!(f.rings[0].len(), 61);
        // Every point sits ~5 NM (9.26 km) from the centre.
        for p in &f.rings[0] {
            let dx = (p[0] + 123.6583) * 111.32 * 41.1166_f64.to_radians().cos();
            let dy = (p[1] - 41.1166) * 111.32;
            let d = (dx * dx + dy * dy).sqrt();
            assert!((d - 9.26).abs() < 0.3, "{d} km from centre");
        }
    }

    #[test]
    fn a_document_with_no_geometry_is_none() {
        let e = TfrEntry {
            notam_id: "6/2".into(),
            kind: "HAZARDS".into(),
            description: "d".into(),
            state: "TX".into(),
        };
        assert!(parse_detail("<Not><txtRmk>nothing</txtRmk></Not>", &e).is_none());
    }
}
