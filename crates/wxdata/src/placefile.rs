//! GRLevelX placefile parser.
//!
//! Placefiles are a plain-text overlay format (lines/polygons/text/icons at lat,lon) used by
//! the spotter/warning community. We support the common drawing statements: `Color`,
//! `Threshold`, `Line`, `Polygon`, `Text`, `Icon`/`Place`, `IconFile`, `TimeRange`, `Object`
//! (screen-anchored shapes) and `Triangles` (per-vertex-colored mesh), plus `Title` and
//! `RefreshSeconds`.
//!
//! `Image:` is still parsed-and-skipped. `// ponytail: Image needs a georeferenced textured quad
//! and its corner/UV syntax varies between real files — add it when one in the wild needs it,
//! with that file as the fixture.`

use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// A parsed placefile: metadata plus a flat list of drawable items.
#[derive(Debug, Clone, Default)]
pub struct Placefile {
    pub title: String,
    /// Seconds between refetches (0 = static).
    pub refresh_secs: u32,
    pub items: Vec<PlaceItem>,
    /// Declared icon sheets by file number, referenced by [`PlaceKind::Icon::sheet`].
    pub icon_files: HashMap<u32, IconSheet>,
}

/// An `IconFile:` declaration — one PNG holding a row/column grid of same-size icons.
#[derive(Debug, Clone, PartialEq)]
pub struct IconSheet {
    /// Absolute URL of the sheet image (relative paths are resolved against the placefile's URL).
    pub url: String,
    pub icon_w: u32,
    pub icon_h: u32,
    /// Hot spot inside one icon, in pixels — the point that lands on the coordinate.
    pub hot_x: u32,
    pub hot_y: u32,
}

/// One drawable, with the display gates that were in effect when it was declared.
#[derive(Debug, Clone)]
pub struct PlaceItem {
    /// View-range gate in nautical miles: shown only when the map range ≤ this (0 = always).
    pub threshold_nmi: f32,
    /// Optional valid-time window; outside it the item is hidden.
    pub time: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// `Object:` anchor in `[lon, lat]`. When set, every coordinate in `kind` is a *pixel offset*
    /// from this point (x right, y up) rather than a position — that's how placefiles draw
    /// fixed-size symbols, which stay the same size on screen as you zoom.
    pub anchor: Option<[f64; 2]>,
    pub kind: PlaceKind,
}

/// The geometry/label variants a placefile can draw.
#[derive(Debug, Clone)]
pub enum PlaceKind {
    /// Polyline in `[lon, lat]` with an RGBA color and pixel width.
    Line {
        color: [u8; 4],
        width: f32,
        pts: Vec<[f64; 2]>,
    },
    /// Filled polygon; `rings[0]` outer, others holes, each `[lon, lat]`.
    Polygon {
        color: [u8; 4],
        rings: Vec<Vec<[f64; 2]>>,
    },
    /// A text label at `[lon, lat]` with hover text.
    Text {
        color: [u8; 4],
        pos: [f64; 2],
        text: String,
        hover: String,
    },
    /// A per-vertex-colored triangle mesh: every three entries are one triangle, each a
    /// `([lon, lat], rgba)`. Goes straight into the overlay buffers — no tessellation needed,
    /// the file already did it.
    Triangles { verts: Vec<([f64; 2], [u8; 4])> },
    /// A point marker at `[lon, lat]` with hover text. `sheet` is `(file number, icon number)`
    /// into [`Placefile::icon_files`] when the line referenced a sheet; without one (or before
    /// the image loads) the renderer falls back to a plain marker. `angle` rotates the icon
    /// clockwise in degrees.
    Icon {
        color: [u8; 4],
        pos: [f64; 2],
        angle: f32,
        sheet: Option<(u32, u32)>,
        hover: String,
    },
}

/// Fetch and parse a placefile from `url`.
pub async fn fetch(http: &reqwest::Client, url: &str) -> anyhow::Result<Placefile> {
    let text = http
        .get(crate::net::fetch_url(url))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut pf = parse(&text);
    // Sheet paths are usually relative to the placefile itself.
    let base = url.rsplit_once('/').map(|(b, _)| b).unwrap_or("");
    for sheet in pf.icon_files.values_mut() {
        if !sheet.url.starts_with("http") && !base.is_empty() {
            sheet.url = format!("{base}/{}", sheet.url.trim_start_matches("./"));
        }
    }
    Ok(pf)
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

/// Flip every coordinate in an item from `[lon, lat]` back to `[x, y]`.
///
/// Object bodies reuse the ordinary statement parsers, which read placefile coordinate order
/// (`lat, lon`) and store `[lon, lat]`. Inside an object the same pair is `x, y` pixels, so the
/// swap has to be undone once here instead of threading a flag through every statement.
fn swap_coords(kind: &mut PlaceKind) {
    let swap = |p: &mut [f64; 2]| p.swap(0, 1);
    match kind {
        PlaceKind::Line { pts, .. } => pts.iter_mut().for_each(swap),
        PlaceKind::Polygon { rings, .. } => rings.iter_mut().flatten().for_each(swap),
        PlaceKind::Triangles { verts } => verts.iter_mut().for_each(|(p, _)| swap(p)),
        PlaceKind::Text { pos, .. } | PlaceKind::Icon { pos, .. } => swap(pos),
    }
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
                threshold = rest
                    .split([',', ' '])
                    .find(|t| !t.is_empty())
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(threshold);
            }
            "timerange" => pending_time = parse_time_range(rest),
            "line" => {
                // `Line: width, flags [, "hover"]` then coords until `End:`.
                let width = rest
                    .split(',')
                    .next()
                    .and_then(|t| t.trim().parse().ok())
                    .unwrap_or(1.0);
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
                        anchor: None,
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
                        anchor: None,
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
                    let hover = if rest.matches('"').count().ge(&4) {
                        rest.rsplit('"').nth(1).unwrap_or("").to_string()
                    } else {
                        Default::default()
                    };
                    if !text.is_empty() {
                        pf.items.push(PlaceItem {
                            threshold_nmi: threshold,
                            time: pending_time.take(),
                            anchor: None,
                            kind: PlaceKind::Text {
                                color,
                                pos,
                                text,
                                hover,
                            },
                        });
                    }
                }
            }
            "iconfile" => {
                // `IconFile: fileNumber, iconWidth, iconHeight, hotX, hotY, fileName`.
                let f: Vec<&str> = rest.split(',').map(str::trim).collect();
                if let (Some(num), true) =
                    (f.first().and_then(|t| t.parse::<u32>().ok()), f.len() >= 6)
                {
                    let n = |i: usize| f[i].parse::<u32>().unwrap_or(0);
                    let url = f[5..].join(",").trim().trim_matches('"').to_string();
                    if n(1) > 0 && n(2) > 0 && !url.is_empty() {
                        pf.icon_files.insert(
                            num,
                            IconSheet {
                                url,
                                icon_w: n(1),
                                icon_h: n(2),
                                hot_x: n(3),
                                hot_y: n(4),
                            },
                        );
                    }
                }
            }
            "icon" | "place" => {
                // `Icon: lat, lon, angle, fileNumber, iconNumber [, "hover"]`. Older files stop
                // after the coordinate, so everything past it is optional.
                if let Some(pos) = parse_coord(rest) {
                    let f: Vec<&str> = rest.split(',').map(str::trim).collect();
                    let num = |i: usize| f.get(i).and_then(|t| t.parse::<f32>().ok());
                    let angle = num(2).unwrap_or(0.0);
                    let sheet = match (num(3), num(4)) {
                        (Some(file), Some(icon)) if file >= 0.0 && icon >= 0.0 => {
                            Some((file as u32, icon as u32))
                        }
                        _ => None,
                    };
                    let hover = quoted(rest).unwrap_or_default();
                    pf.items.push(PlaceItem {
                        threshold_nmi: threshold,
                        time: pending_time.take(),
                        anchor: None,
                        kind: PlaceKind::Icon {
                            color,
                            pos,
                            angle,
                            sheet,
                            hover,
                        },
                    });
                }
            }
            "object" => {
                // `Object: lat, lon` … nested statements … `End:`. The body is ordinary placefile
                // syntax, so it is parsed by recursion rather than by a second copy of this loop;
                // the only difference is that its coordinates are pixel offsets, which is what
                // `anchor` records for the renderer.
                let anchor = parse_coord(rest);
                let mut depth = 1usize;
                let mut body = String::new();
                while i < lines.len() {
                    let l = strip_comment(lines[i]);
                    i += 1;
                    let head = l.split(':').next().unwrap_or("").trim();
                    if head.eq_ignore_ascii_case("end") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if ["object", "line", "polygon", "triangles"]
                        .contains(&head.to_ascii_lowercase().as_str())
                    {
                        depth += 1;
                    }
                    body.push_str(l);
                    body.push('\n');
                }
                if let Some(anchor) = anchor {
                    let inner = parse(&body);
                    for mut item in inner.items {
                        // `parse_coord` reads `lat, lon`; inside an object the pair is `x, y`,
                        // so undo the swap rather than teaching every statement about objects.
                        swap_coords(&mut item.kind);
                        item.anchor = Some(anchor);
                        if item.time.is_none() {
                            item.time = pending_time;
                        }
                        pf.items.push(item);
                    }
                }
                pending_time = None;
            }
            "triangles" => {
                // Vertices until `End:`, three to a triangle, each optionally carrying its own
                // color: `lat, lon [, r, g, b [, a]]`.
                let mut verts = Vec::new();
                while i < lines.len() {
                    let l = strip_comment(lines[i]);
                    i += 1;
                    if l.eq_ignore_ascii_case("end") || l.eq_ignore_ascii_case("end:") {
                        break;
                    }
                    let Some(pos) = parse_coord(l) else { continue };
                    let n = l.split(',').count();
                    let vc = if n >= 5 {
                        let rest: String = l.splitn(3, ',').nth(2).unwrap_or("").to_string();
                        parse_color(&rest)
                    } else {
                        color
                    };
                    verts.push((pos, vc));
                }
                // A trailing partial triangle is a malformed file, not a hint.
                verts.truncate(verts.len() - verts.len() % 3);
                if !verts.is_empty() {
                    pf.items.push(PlaceItem {
                        threshold_nmi: threshold,
                        time: pending_time.take(),
                        anchor: None,
                        kind: PlaceKind::Triangles { verts },
                    });
                } else {
                    pending_time = None;
                }
            }
            _ => {} // Font, Image, etc. — ignored.
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
    fn parses_objects_as_pixel_offsets() {
        // A hollow square drawn around a point — the classic Object use: a symbol that stays the
        // same size on screen however far you zoom in.
        let src = r#"
Object: 35.5, -97.5
 Threshold: 999
 Color: 255 255 0
 Line: 2, 0
  -10, -10
  10, -10
  10, 10
  -10, 10
  -10, -10
 End:
 Text: 0, 14, 1, "hail"
End:
"#;
        let pf = parse(src);
        assert_eq!(pf.items.len(), 2, "the object's body, flattened");
        for it in &pf.items {
            assert_eq!(it.anchor, Some([-97.5, 35.5]));
        }
        match &pf.items[0].kind {
            // Stored as [x, y] pixels, not [lon, lat]: the first vertex is 10 left, 10 down.
            PlaceKind::Line { pts, .. } => assert_eq!(pts[0], [-10.0, -10.0]),
            k => panic!("expected line, got {k:?}"),
        }
        match &pf.items[1].kind {
            PlaceKind::Text { pos, text, .. } => {
                assert_eq!(*pos, [0.0, 14.0], "14 pixels above the anchor");
                assert_eq!(text, "hail");
            }
            k => panic!("expected text, got {k:?}"),
        }
    }

    #[test]
    fn parses_triangle_meshes() {
        let src = r#"
Color: 0 0 255
Triangles:
 35.0, -97.0, 255, 0, 0
 35.0, -96.0, 0, 255, 0, 128
 34.0, -96.0
 34.0, -97.0
End:
"#;
        let pf = parse(src);
        match &pf.items[0].kind {
            PlaceKind::Triangles { verts } => {
                // Four vertices in the file, but only whole triangles survive.
                assert_eq!(verts.len(), 3);
                assert_eq!(verts[0], ([-97.0, 35.0], [255, 0, 0, 255]));
                assert_eq!(verts[1].1, [0, 255, 0, 128], "per-vertex alpha");
                assert_eq!(
                    verts[2].1,
                    [0, 0, 255, 255],
                    "falls back to the current Color"
                );
            }
            k => panic!("expected triangles, got {k:?}"),
        }
    }

    #[test]
    fn parses_icon_sheets() {
        let src = r#"
IconFile: 1, 32, 32, 16, 31, "https://example.com/spotters.png"
Icon: 35.0, -97.0, 135, 1, 3, "spotter facing SE"
Icon: 34.0, -98.0, 0, 0, 0
"#;
        let pf = parse(src);
        let sheet = pf.icon_files.get(&1).expect("sheet 1 declared");
        assert_eq!(sheet.icon_w, 32);
        assert_eq!(sheet.hot_y, 31);
        assert_eq!(sheet.url, "https://example.com/spotters.png");
        match &pf.items[0].kind {
            PlaceKind::Icon {
                angle,
                sheet,
                hover,
                ..
            } => {
                assert_eq!(*angle, 135.0);
                assert_eq!(*sheet, Some((1, 3)));
                assert_eq!(hover, "spotter facing SE");
            }
            k => panic!("expected icon, got {k:?}"),
        }
        // A no-hover icon still parses, and file 0 is a legitimate reference.
        assert!(matches!(
            pf.items[1].kind,
            PlaceKind::Icon {
                sheet: Some((0, 0)),
                ..
            }
        ));
    }

    #[test]
    fn object_blocks_end_where_they_should() {
        // The nested `End:` closes the Line, the second closes the Object — miscount either and
        // the statement after the block is swallowed or reparented.
        let src =
            "Object: 35, -97\n Line: 1,0\n 0,0\n 5,5\n End:\nEnd:\nText: 35, -97, 1, \"after\"";
        let pf = parse(src);
        assert_eq!(pf.items.len(), 2);
        assert_eq!(pf.items[0].anchor, Some([-97.0, 35.0]));
        assert!(matches!(pf.items[0].kind, PlaceKind::Line { .. }));
        assert_eq!(
            pf.items[1].anchor, None,
            "the trailing Text is not in the object"
        );
        assert!(matches!(pf.items[1].kind, PlaceKind::Text { .. }));
    }
}
