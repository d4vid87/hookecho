//! GRLevelX placefile parser.
//!
//! Placefiles are a plain-text overlay format (lines/polygons/text/icons at lat,lon) used by
//! the spotter/warning community. We support the common drawing statements: `Color`,
//! `Threshold`, `Line`, `Polygon`, `Text`, `Icon`/`Place`, `TimeRange`, plus `Title` and
//! `RefreshSeconds`. `Object` (relative/pixel coords), `Triangles`, `Image`, and icon sheets
//! are parsed-and-skipped for now.
//!
//! `// ponytail: Object/Triangles/Image + IconFile PNG sheets deferred — the 90% case is
//! colored lines/polygons + text labels; add the rest when a real placefile needs them.`

use chrono::{DateTime, Utc};

/// A parsed placefile: metadata plus a flat list of drawable items.
#[derive(Debug, Clone, Default)]
pub struct Placefile {
    pub title: String,
    /// Seconds between refetches (0 = static).
    pub refresh_secs: u32,
    pub items: Vec<PlaceItem>,
}

/// One drawable, with the display gates that were in effect when it was declared.
#[derive(Debug, Clone)]
pub struct PlaceItem {
    /// View-range gate in nautical miles: shown only when the map range ≤ this (0 = always).
    pub threshold_nmi: f32,
    /// Optional valid-time window; outside it the item is hidden.
    pub time: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub kind: PlaceKind,
}

/// The geometry/label variants a placefile can draw.
#[derive(Debug, Clone)]
pub enum PlaceKind {
    /// Polyline in `[lon, lat]` with an RGBA color and pixel width.
    Line { color: [u8; 4], width: f32, pts: Vec<[f64; 2]> },
    /// Filled polygon; `rings[0]` outer, others holes, each `[lon, lat]`.
    Polygon { color: [u8; 4], rings: Vec<Vec<[f64; 2]>> },
    /// A text label at `[lon, lat]` with hover text.
    Text { color: [u8; 4], pos: [f64; 2], text: String, hover: String },
    /// A point marker at `[lon, lat]` (icon sheets not yet rendered) with hover text.
    Icon { color: [u8; 4], pos: [f64; 2], hover: String },
}

/// Fetch and parse a placefile from `url`.
pub async fn fetch(http: &reqwest::Client, url: &str) -> anyhow::Result<Placefile> {
    let text = http.get(url).send().await?.error_for_status()?.text().await?;
    Ok(parse(&text))
}

/// Strip a trailing `;` comment (not inside quotes) and trim.
fn strip_comment(line: &str) -> &str {
    let mut in_q = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_q = !in_q,
            ';' if !in_q => return line[..i].trim(),
            _ => {}
        }
    }
    line.trim()
}

/// The first double-quoted substring, if any.
fn quoted(s: &str) -> Option<String> {
    let a = s.find('"')?;
    let b = s[a + 1..].find('"')?;
    Some(s[a + 1..a + 1 + b].to_string())
}

/// Parse `R G B [A]` (space or comma separated) into RGBA, default alpha 255.
fn parse_color(args: &str) -> [u8; 4] {
    let n: Vec<u8> = args
        .split([',', ' '])
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.trim().parse::<f32>().ok())
        .map(|v| v.clamp(0.0, 255.0) as u8)
        .collect();
    [
        n.first().copied().unwrap_or(255),
        n.get(1).copied().unwrap_or(255),
        n.get(2).copied().unwrap_or(255),
        n.get(3).copied().unwrap_or(255),
    ]
}

/// Parse a `lat, lon` coordinate line into `[lon, lat]` (placefile order is lat first).
fn parse_coord(line: &str) -> Option<[f64; 2]> {
    let mut it = line.split(',').map(str::trim);
    let lat: f64 = it.next()?.parse().ok()?;
    let lon: f64 = it.next()?.parse().ok()?;
    Some([lon, lat])
}

fn parse_time_range(args: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let mut it = args.split_whitespace();
    let a = it.next()?;
    let b = it.next()?;
    let pa = DateTime::parse_from_rfc3339(a).ok()?.with_timezone(&Utc);
    let pb = DateTime::parse_from_rfc3339(b).ok()?.with_timezone(&Utc);
    Some((pa, pb))
}

/// Parse placefile `text` into a [`Placefile`]. Malformed lines are skipped, not fatal.
pub fn parse(text: &str) -> Placefile {
    let mut pf = Placefile::default();
    let mut color = [255u8, 255, 255, 255];
    let mut threshold = 999.0f32;
    let mut pending_time: Option<(DateTime<Utc>, DateTime<Utc>)> = None;

    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = strip_comment(lines[i]);
        i += 1;
        if line.is_empty() {
            continue;
        }
        // `Keyword: rest` or a bare block keyword ending in `:`.
        let (kw, rest) = match line.split_once(':') {
            Some((k, r)) => (k.trim(), r.trim()),
            None => (line, ""),
        };
        match kw.to_ascii_lowercase().as_str() {
            "title" => pf.title = rest.to_string(),
            "refreshseconds" => pf.refresh_secs = rest.parse().unwrap_or(0),
            "refresh" => pf.refresh_secs = rest.parse::<u32>().unwrap_or(0).saturating_mul(60),
            "color" => color = parse_color(rest),
            "threshold" => {
                threshold = rest.split([',', ' ']).find(|t| !t.is_empty())
                    .and_then(|t| t.parse().ok()).unwrap_or(threshold);
            }
            "timerange" => pending_time = parse_time_range(rest),
            "line" => {
                // `Line: width, flags [, "hover"]` then coords until `End:`.
                let width = rest.split(',').next().and_then(|t| t.trim().parse().ok()).unwrap_or(1.0);
                let mut pts = Vec::new();
                while i < lines.len() {
                    let l = strip_comment(lines[i]);
                    i += 1;
                    if l.eq_ignore_ascii_case("end") || l.eq_ignore_ascii_case("end:") {
                        break;
                    }
                    if let Some(c) = parse_coord(l) {
                        pts.push(c);
                    }
                }
                if pts.len() >= 2 {
                    pf.items.push(PlaceItem {
                        threshold_nmi: threshold,
                        time: pending_time.take(),
                        kind: PlaceKind::Line { color, width, pts },
                    });
                } else {
                    pending_time = None;
                }
            }
            "polygon" => {
                // Coords until `End:`; a blank coord line starts a new ring (hole/contour).
                let mut rings: Vec<Vec<[f64; 2]>> = vec![Vec::new()];
                while i < lines.len() {
                    let l = strip_comment(lines[i]);
                    i += 1;
                    if l.eq_ignore_ascii_case("end") || l.eq_ignore_ascii_case("end:") {
                        break;
                    }
                    if l.is_empty() {
                        if !rings.last().unwrap().is_empty() {
                            rings.push(Vec::new());
                        }
                        continue;
                    }
                    if let Some(c) = parse_coord(l) {
                        rings.last_mut().unwrap().push(c);
                    }
                }
                rings.retain(|r| r.len() >= 3);
                if !rings.is_empty() {
                    pf.items.push(PlaceItem {
                        threshold_nmi: threshold,
                        time: pending_time.take(),
                        kind: PlaceKind::Polygon { color, rings },
                    });
                } else {
                    pending_time = None;
                }
            }
            "text" => {
                // `Text: lat, lon, fontNumber, "string" [, "hover"]`.
                if let Some(pos) = parse_coord(rest) {
                    let text = quoted(rest).unwrap_or_default();
                    let hover = rest.matches('"').count().ge(&4)
                        .then(|| rest.rsplit('"').nth(1).unwrap_or("").to_string())
                        .unwrap_or_default();
                    if !text.is_empty() {
                        pf.items.push(PlaceItem {
                            threshold_nmi: threshold,
                            time: pending_time.take(),
                            kind: PlaceKind::Text { color, pos, text, hover },
                        });
                    }
                }
            }
            "icon" | "place" => {
                if let Some(pos) = parse_coord(rest) {
                    let hover = quoted(rest).unwrap_or_default();
                    pf.items.push(PlaceItem {
                        threshold_nmi: threshold,
                        time: pending_time.take(),
                        kind: PlaceKind::Icon { color, pos, hover },
                    });
                }
            }
            // Skip block bodies we don't render yet.
            "object" | "triangles" => {
                while i < lines.len() {
                    let l = strip_comment(lines[i]);
                    i += 1;
                    if l.eq_ignore_ascii_case("end") || l.eq_ignore_ascii_case("end:") {
                        break;
                    }
                }
                pending_time = None;
            }
            _ => {} // Font, IconFile, Image, etc. — ignored.
        }
    }
    pf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_statements() {
        let src = r#"
; a sample placefile
Title: Test Overlay
RefreshSeconds: 30
Threshold: 100
Color: 255 0 0
Line: 2, 0, "a line"
 35.0, -97.0
 36.0, -96.0
End:
Color: 0, 255, 0, 128
Polygon:
 35.0, -97.5
 35.0, -96.5
 34.5, -96.5
End:
Text: 35.5, -97.2, 1, "OKC", "Oklahoma City"
Icon: 34.0, -98.0, 0, 1, 5, "marker"
"#;
        let pf = parse(src);
        assert_eq!(pf.title, "Test Overlay");
        assert_eq!(pf.refresh_secs, 30);
        assert_eq!(pf.items.len(), 4);

        match &pf.items[0].kind {
            PlaceKind::Line { color, pts, .. } => {
                assert_eq!(*color, [255, 0, 0, 255]);
                assert_eq!(pts.len(), 2);
                assert_eq!(pts[0], [-97.0, 35.0]); // [lon, lat]
            }
            k => panic!("expected line, got {k:?}"),
        }
        assert_eq!(pf.items[0].threshold_nmi, 100.0);

        match &pf.items[1].kind {
            PlaceKind::Polygon { color, rings } => {
                assert_eq!(*color, [0, 255, 0, 128]);
                assert_eq!(rings.len(), 1);
                assert_eq!(rings[0].len(), 3);
            }
            k => panic!("expected polygon, got {k:?}"),
        }

        match &pf.items[2].kind {
            PlaceKind::Text { text, hover, .. } => {
                assert_eq!(text, "OKC");
                assert_eq!(hover, "Oklahoma City");
            }
            k => panic!("expected text, got {k:?}"),
        }
        assert!(matches!(pf.items[3].kind, PlaceKind::Icon { .. }));
    }

    #[test]
    fn skips_object_blocks() {
        let src = "Object: 35, -97\n Line: 1,0\n 0,0\n 5,5\n End:\nEnd:\nText: 35, -97, 1, \"after\"";
        let pf = parse(src);
        // Object block skipped entirely; only the trailing Text survives.
        assert_eq!(pf.items.len(), 1);
        assert!(matches!(pf.items[0].kind, PlaceKind::Text { .. }));
    }
}
