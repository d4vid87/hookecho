//! Field Atlas map styling. Source geometry, visibility and hit-testing stay untouched.
use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke};
use wxdata::overlay::{FeatureKind, GeoFeature};

pub const INK: Color32 = Color32::from_rgb(7, 14, 21);
pub const TROPICAL: Color32 = Color32::from_rgb(119, 221, 255);
pub const ROTATION: Color32 = Color32::from_rgb(141, 242, 214);

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub rgb: [u8; 3],
    pub fill: u8,
    pub dash: Option<(f32, f32)>,
    pub emergency: bool,
}
impl Style {
    pub fn color(self) -> Color32 {
        Color32::from_rgb(self.rgb[0], self.rgb[1], self.rgb[2])
    }
}

pub fn style(f: &GeoFeature) -> Option<Style> {
    // PDS/observed/destructive remains a warning, not an invented emergency designation.
    let emergency = f.kind == FeatureKind::Warning
        && f.alert.as_ref().is_some_and(|a| {
            [&a.headline, &a.description].iter().any(|text| {
                let text = text.to_ascii_uppercase();
                text.contains("TORNADO EMERGENCY") || text.contains("FLASH FLOOD EMERGENCY")
            })
        });
    let (rgb, fill, dash) = match f.kind {
        FeatureKind::Warning if emergency => ([255, 118, 213], 18, None),
        FeatureKind::Warning
            if f.alert.as_ref().map_or(f.title.as_str(), |a| a.event.as_str()) == "Flood Warning" =>
        {
            ([0, 160, 90], 12, None)
        }
        FeatureKind::Warning => ([255, 117, 93], 12, None),
        FeatureKind::Watch | FeatureKind::WatchBox => ([255, 226, 108], 8, Some((5.0, 5.0))),
        FeatureKind::Advisory | FeatureKind::Statement => ([114, 186, 255], 6, Some((1.5, 5.0))),
        FeatureKind::MesoDiscussion => ([180, 154, 255], 8, Some((3.0, 6.0))),
        // Wind radii and surge share this kind; keep their source intensity colors and fills.
        FeatureKind::TropicalCone if f.title.ends_with(" cone") => {
            ([119, 221, 255], 10, Some((5.0, 7.0)))
        }
        _ => return None,
    };
    Some(Style {
        rgb,
        fill,
        dash,
        emergency,
    })
}

/// Clip before generating dashes. A zoomed-in state boundary can be millions of pixels long.
fn clip_segment(a: Pos2, b: Pos2, clip: Rect) -> Option<[Pos2; 2]> {
    let d = b - a;
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for (p, q) in [
        (-d.x, a.x - clip.left()),
        (d.x, clip.right() - a.x),
        (-d.y, a.y - clip.top()),
        (d.y, clip.bottom() - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                lo = lo.max(t);
            } else {
                hi = hi.min(t);
            }
            if lo > hi {
                return None;
            }
        }
    }
    Some([a + d * lo, a + d * hi])
}

/// Screen-space outline with a dark casing; emergencies receive two separated color strokes.
pub fn boundary(painter: &Painter, points: &[Pos2], clip: Rect, style: Style, scale: f32) {
    for pair in points.windows(2) {
        let Some(segment) = clip_segment(pair[0], pair[1], clip.expand(8.0 * scale)) else {
            continue;
        };
        let draw = |width, color| {
            let stroke = Stroke::new(width * scale, color);
            if let Some((dash, gap)) = style.dash {
                painter.add(Shape::dashed_line(
                    &segment,
                    stroke,
                    dash * scale,
                    gap * scale,
                ));
            } else {
                painter.line_segment(segment, stroke);
            }
        };
        draw(6.0, INK);
        if style.emergency {
            draw(5.0, style.color());
            draw(3.0, INK);
        }
        draw(1.8, style.color());
    }
}

/// Small square technical label, with corner ticks and collision rejection.
pub fn label(
    painter: &Painter,
    clip: Rect,
    anchor: Pos2,
    text: &str,
    color: Color32,
    taken: &mut Vec<Rect>,
) {
    let text: String = if text.chars().count() > 42 {
        text.chars().take(39).chain("…".chars()).collect()
    } else {
        text.to_owned()
    };
    let galley = painter.layout_no_wrap(text, FontId::monospace(11.0), color);
    let size = galley.size() + egui::vec2(12.0, 8.0);
    for offset in [
        egui::vec2(8.0, -size.y - 7.0),
        egui::vec2(8.0, 8.0),
        egui::vec2(-size.x - 8.0, 8.0),
    ] {
        let rect = Rect::from_min_size(anchor + offset, size);
        if !clip.contains_rect(rect.expand(4.0))
            || taken.iter().any(|r| r.expand(3.0).intersects(rect))
        {
            continue;
        }
        taken.push(rect);
        painter.line_segment([anchor, rect.center()], Stroke::new(1.0, color));
        painter.rect_filled(rect, 2.0, INK);
        painter.rect_stroke(rect, 2.0, Stroke::new(0.8, color), egui::StrokeKind::Inside);
        for (p, dx, dy) in [
            (rect.left_top(), -1.0, -1.0),
            (rect.right_bottom(), 1.0, 1.0),
        ] {
            painter.line_segment(
                [
                    p + egui::vec2(dx * 4.0, 0.0),
                    p + egui::vec2(dx * 4.0, dy * 4.0),
                ],
                Stroke::new(1.0, color),
            );
            painter.line_segment(
                [
                    p + egui::vec2(0.0, dy * 4.0),
                    p + egui::vec2(dx * 4.0, dy * 4.0),
                ],
                Stroke::new(1.0, color),
            );
        }
        painter.galley(rect.min + egui::vec2(6.0, 4.0), galley, color);
        break;
    }
}

pub fn draw(
    painter: &Painter,
    features: &[GeoFeature],
    clip: Rect,
    zoom: f64,
    scale: f32,
    to_screen: impl Fn(f64, f64) -> Pos2,
) {
    let painter = painter.with_clip_rect(clip);
    let mut styled: Vec<_> = features
        .iter()
        .filter_map(|f| style(f).map(|s| (f, s)))
        .collect();
    styled.sort_by_key(|(f, s)| (s.emergency, f.kind.z()));
    // Draw all boundaries first; labels then get priority in reverse severity order.
    let mut candidates = Vec::new();
    for (f, s) in styled {
        let mut anchor = None;
        for ring in &f.rings {
            if ring.len() < 3 {
                continue;
            }
            let mut pts: Vec<_> = ring.iter().map(|p| to_screen(p[0], p[1])).collect();
            let bounds = pts
                .iter()
                .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p)));
            if !bounds.intersects(clip) {
                continue;
            }
            if anchor.is_none() && bounds.width() >= 45.0 && bounds.height() >= 35.0 {
                anchor = pts
                    .iter()
                    .copied()
                    .filter(|p| clip.shrink(12.0).contains(*p))
                    .min_by(|a, b| a.y.total_cmp(&b.y));
            }
            pts.push(pts[0]);
            boundary(&painter, &pts, clip, s, scale);
        }
        if let Some(p) = anchor {
            candidates.push((f, s, p));
        }
    }
    if zoom >= 5.0 {
        let mut taken = Vec::new();
        for (f, s, p) in candidates.into_iter().rev() {
            let title = if s.emergency {
                format!("EMERGENCY · {}", f.title)
            } else {
                f.title
                    .replace("Mesoscale Discussion", "MD")
                    .replace("Mesoscale discussion", "MD")
            };
            label(&painter, clip, p, &title, s.color(), &mut taken);
        }
    }
}

pub fn rotation_marker(painter: &Painter, p: Pos2) {
    painter.circle_filled(p, 11.0, INK);
    painter.circle_stroke(p, 10.0, Stroke::new(1.8, ROTATION));
    painter.circle_stroke(p, 5.0, Stroke::new(1.0, ROTATION));
    for d in [
        egui::vec2(1.0, 0.0),
        egui::vec2(-1.0, 0.0),
        egui::vec2(0.0, 1.0),
        egui::vec2(0.0, -1.0),
    ] {
        painter.line_segment([p + d * 13.0, p + d * 17.0], Stroke::new(1.2, INK));
        painter.line_segment([p + d * 13.0, p + d * 17.0], Stroke::new(0.8, ROTATION));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feature(kind: FeatureKind) -> GeoFeature {
        GeoFeature {
            rings: vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]],
            fill: [255; 4],
            stroke: [255; 4],
            kind,
            title: "Sample".into(),
            detail: String::new(),
            alert: None,
        }
    }
    #[test]
    fn styles_keep_hazard_roles_distinct_and_do_not_invent_emergencies() {
        let warning = style(&feature(FeatureKind::Warning)).unwrap();
        let watch = style(&feature(FeatureKind::Watch)).unwrap();
        let md = style(&feature(FeatureKind::MesoDiscussion)).unwrap();
        assert!(warning.dash.is_none() && !warning.emergency);
        assert!(watch.dash.is_some());
        assert_ne!(watch.dash, md.dash);
        assert!(watch.fill < warning.fill);
        assert!(style(&feature(FeatureKind::Outlook)).is_none());
        let mut flood = feature(FeatureKind::Warning);
        flood.title = "Flood Warning".into();
        assert_eq!(style(&flood).unwrap().rgb, [0, 160, 90]);
        flood.title = "Severe Thunderstorm Warning".into();
        assert_eq!(style(&flood).unwrap().rgb, warning.rgb);
        let mut alert = feature(FeatureKind::Warning);
        alert.alert=Some(serde_json::from_value(serde_json::json!({
            "id":"test", "event":"Tornado Warning", "headline":"Particularly Dangerous Situation",
            "area":"Test only", "description":"", "instruction":""
        })).unwrap());
        assert!(
            !style(&alert).unwrap().emergency,
            "PDS is not an emergency designation"
        );
        alert.alert.as_mut().unwrap().headline = "Tornado Emergency".into();
        assert!(style(&alert).unwrap().emergency);
        alert.alert.as_mut().unwrap().headline = "Flash Flood Emergency".into();
        assert!(style(&alert).unwrap().emergency);
        alert.kind = FeatureKind::Watch;
        assert!(
            !style(&alert).unwrap().emergency,
            "watch text cannot promote a watch to emergency"
        );
        let mut tropical = feature(FeatureKind::TropicalCone);
        for title in ["Storm 34 kt wind", "Surge: 3 ft"] {
            tropical.title = title.into();
            assert!(style(&tropical).is_none());
        }
        tropical.title = "Storm cone".into();
        assert!(style(&tropical).is_some());
    }
    #[test]
    fn clipping_bounds_work_at_high_zoom_and_preserve_crossing_edges() {
        let r = Rect::from_min_max(Pos2::ZERO, egui::pos2(100.0, 100.0));
        assert_eq!(
            clip_segment(egui::pos2(-1e6, 50.0), egui::pos2(1e6, 50.0), r),
            Some([egui::pos2(0.0, 50.0), egui::pos2(100.0, 50.0)])
        );
        assert!(clip_segment(egui::pos2(-100.0, -10.0), egui::pos2(200.0, -10.0), r).is_none());
        let ctx = egui::Context::default();
        let out = ctx.run_ui(Default::default(), |ui| {
            let features = [
                feature(FeatureKind::Warning),
                feature(FeatureKind::Watch),
                feature(FeatureKind::MesoDiscussion),
            ];
            draw(ui.painter(), &features, r, 8.0, 1.0, |x, y| {
                egui::pos2(x as f32 * 80.0 + 10.0, y as f32 * 80.0 + 10.0)
            });
        });
        assert!(!out.shapes.is_empty());
    }
}
