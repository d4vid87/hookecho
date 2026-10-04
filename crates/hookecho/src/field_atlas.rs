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
    pub warning: bool,
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
    let event = f.alert.as_ref().map_or(f.title.as_str(), |a| a.event.as_str());
    let (rgb, fill, dash) = match f.kind {
        FeatureKind::Watch | FeatureKind::Advisory | FeatureKind::Statement
            if matches!(event, "Air Quality Alert" | "Air Quality Watch") =>
        {
            ([255, 255, 255], 6, Some((1.5, 5.0)))
        }
        FeatureKind::Warning if emergency => ([255, 118, 213], 18, None),
        FeatureKind::Warning if event == "Flash Flood Warning" => ([57, 255, 20], 12, None),
        FeatureKind::Warning
            if f.alert
                .as_ref()
                .map_or(f.title.as_str(), |a| a.event.as_str())
                == "Flood Warning" =>
        {
            ([0, 160, 90], 12, None)
        }
        FeatureKind::Warning if event == "Severe Thunderstorm Warning" =>
            ([255, 225, 40], 12, None),
        FeatureKind::Warning
            if matches!(event, "Red Flag Warning" | "Extreme Heat Warning" | "Excessive Heat Warning") =>
        {
            ([f.stroke[0], f.stroke[1], f.stroke[2]], 12, None)
        }
        FeatureKind::Warning => ([255, 117, 93], 12, None),
        FeatureKind::Watch | FeatureKind::WatchBox if event.starts_with("Tornado Watch") => {
            ([230, 40, 40], 8, Some((5.0, 5.0)))
        }
        FeatureKind::Watch if matches!(event, "Fire Weather Watch" | "Extreme Heat Watch") => {
            ([f.stroke[0], f.stroke[1], f.stroke[2]], 8, Some((5.0, 5.0)))
        }
        FeatureKind::Watch | FeatureKind::WatchBox => ([255, 226, 108], 8, Some((5.0, 5.0))),
        FeatureKind::Advisory if event == "Heat Advisory" => {
            ([f.stroke[0], f.stroke[1], f.stroke[2]], 6, Some((1.5, 5.0)))
        }
        FeatureKind::Statement
            if f.alert
                .as_ref()
                .map_or(f.title.as_str(), |a| a.event.as_str())
                == "Special Weather Statement" =>
        {
            ([255, 228, 181], 6, Some((1.5, 5.0)))
        }
        FeatureKind::Advisory | FeatureKind::Statement => ([114, 186, 255], 6, Some((1.5, 5.0))),
        FeatureKind::MesoDiscussion => ([58, 96, 245], 8, None),
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
        warning: f.kind == FeatureKind::Warning,
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

/// Warnings use a white halo and bold colored core; other overlays keep their dark casing.
pub fn boundary(painter: &Painter, points: &[Pos2], clip: Rect, style: Style, scale: f32) {
    let mut shapes = Vec::new();
    boundary_shapes(&mut shapes, points, clip, style, scale);
    painter.extend(shapes);
}

fn boundary_shapes(shapes: &mut Vec<Shape>, points: &[Pos2], clip: Rect, style: Style, scale: f32) {
    for pair in points.windows(2) {
        let Some(segment) = clip_segment(pair[0], pair[1], clip.expand(8.0 * scale)) else {
            continue;
        };
        let mut draw = |width, color| {
            let stroke = Stroke::new(width * scale, color);
            if let Some((dash, gap)) = style.dash {
                shapes.extend(Shape::dashed_line(
                    &segment,
                    stroke,
                    dash * scale,
                    gap * scale,
                ));
            } else {
                shapes.push(Shape::line_segment(segment, stroke));
            }
        };
        if style.warning {
            draw(9.0, INK);
            draw(7.0, Color32::WHITE);
            if style.emergency {
                draw(5.0, style.color());
                draw(2.5, Color32::WHITE);
                draw(1.2, style.color());
            } else {
                draw(3.2, style.color());
            }
        } else {
            draw(6.0, INK);
            draw(1.8, style.color());
        }
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

#[cfg(test)]
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

/// Regional labels share the existing bulletin/detail readers. Original polygons are never
/// simplified or filtered for hit testing; only the painted outline is cached.
#[derive(Default)]
pub struct Cache {
    generation: Option<u64>,
    prepared: Vec<Prepared>,
    panes: std::collections::HashMap<usize, Pane>,
    open: Option<Cluster>,
}
struct Prepared {
    index: usize,
    style: Style,
    key: String,
    rings: Vec<WorldRing>,
}
struct WorldRing {
    points: Vec<(f64, f64)>,
    bounds: [f64; 4],
    exterior: Vec<bool>,
}
#[derive(Clone)]
struct Cluster {
    name: &'static str,
    anchor: Pos2,
    items: Vec<usize>,
}
#[derive(PartialEq)]
struct ViewKey {
    center: (f64, f64),
    zoom: f64,
    clip: Rect,
    scale: f32,
    ppp: f32,
}
struct Pane {
    key: ViewKey,
    mesh: std::sync::Arc<egui::Mesh>,
    labels: Vec<(usize, Style, Pos2)>,
    clusters: Vec<Cluster>,
}
const CLUSTER_ZOOM: f64 = 7.0;

fn region(lon: f64, lat: f64) -> &'static str {
    if lat > 51.0 && lon < -130.0 {
        "Alaska"
    } else if lon < -130.0 {
        "Pacific"
    } else if lon < -106.0 {
        "West"
    } else if lon < -94.0 {
        "Plains"
    } else if lon < -82.0 {
        "Central"
    } else if lon < -60.0 {
        "East"
    } else {
        "Atlantic"
    }
}

fn edge_key(a: (f64, f64), b: (f64, f64)) -> ([(u64, u64); 2], i32) {
    let a = (a.0.to_bits(), a.1.to_bits());
    let b = (b.0.to_bits(), b.1.to_bits());
    if a < b {
        ([a, b], 1)
    } else {
        ([b, a], -1)
    }
}

impl Cache {
    fn prepare(&mut self, generation: u64, features: &[GeoFeature]) {
        if self.generation == Some(generation) {
            return;
        }
        self.generation = Some(generation);
        self.panes.clear();
        // Preserve an open reader by bulletin identity, never by stale polygon indices.
        let reopen = self.open.take().map(|c| {
            let indices: std::collections::HashSet<_> = c.items.iter().copied().collect();
            let keys: std::collections::HashSet<_> = self
                .prepared
                .iter()
                .filter(|p| indices.contains(&p.index))
                .map(|p| p.key.clone())
                .collect();
            (c, keys)
        });
        self.prepared = features
            .iter()
            .enumerate()
            .filter_map(|(index, f)| {
                let s = style(f)?;
                let key = f
                    .alert
                    .as_ref()
                    .map(|a| a.id.clone())
                    .unwrap_or_else(|| format!("{:?}:{}:{}", f.kind, f.title, f.detail));
                let rings = f
                    .rings
                    .iter()
                    .filter(|r| r.len() >= 3)
                    .map(|ring| {
                        let mut points: Vec<_> = ring
                            .iter()
                            .map(|p| crate::render::mercator::lonlat_to_world(p[0], p[1]))
                            .collect();
                        let mut bounds = [
                            f64::INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::NEG_INFINITY,
                        ];
                        for &(x, y) in &points {
                            bounds[0] = bounds[0].min(x);
                            bounds[1] = bounds[1].min(y);
                            bounds[2] = bounds[2].max(x);
                            bounds[3] = bounds[3].max(y);
                        }
                        // Normalize winding so identical duplicate parts do not erase their
                        // outline, while opposite sides of a shared county edge cancel.
                        let area: f64 = points
                            .iter()
                            .zip(points.iter().cycle().skip(1))
                            .take(points.len())
                            .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
                            .sum();
                        if area < 0.0 {
                            points.reverse();
                        }
                        WorldRing {
                            exterior: vec![true; points.len()],
                            points,
                            bounds,
                        }
                    })
                    .collect();
                Some(Prepared {
                    index,
                    style: s,
                    key,
                    rings,
                })
            })
            .collect();
        // Only exact shared edges of the SAME bulletin are suppressed at wide zoom.
        // Different alerts, nonmatching edges, fills and source hit-test geometry stay intact.
        let mut groups = std::collections::HashMap::new();
        let mut counts = std::collections::HashMap::new();
        for p in &self.prepared {
            let next = groups.len();
            let group = *groups.entry(p.key.clone()).or_insert(next);
            for r in &p.rings {
                for i in 0..r.points.len() {
                    let (edge, sign) = edge_key(r.points[i], r.points[(i + 1) % r.points.len()]);
                    *counts.entry((group, edge)).or_insert(0_i32) += sign;
                }
            }
        }
        for p in &mut self.prepared {
            let group = groups[&p.key];
            for r in &mut p.rings {
                for i in 0..r.points.len() {
                    let (edge, _) = edge_key(r.points[i], r.points[(i + 1) % r.points.len()]);
                    r.exterior[i] = counts[&(group, edge)] != 0;
                }
            }
        }
        self.prepared
            .sort_by_key(|p| (p.style.emergency, features[p.index].kind.z()));
        if let Some((mut c, mut keys)) = reopen {
            c.items = self
                .prepared
                .iter()
                .rev()
                .filter_map(|p| keys.remove(&p.key).then_some(p.index))
                .collect();
            self.open = (!c.items.is_empty()).then_some(c);
        }
    }

    fn build(&self, ctx: &egui::Context, key: ViewKey, features: &[GeoFeature]) -> Pane {
        use crate::render::mercator::{world_to_lonlat, Camera};
        let cam = Camera {
            center: key.center,
            zoom: key.zoom,
        };
        let vp = (key.clip.width(), key.clip.height());
        let lo = cam.screen_to_world((-10.0, -10.0), vp);
        let hi = cam.screen_to_world((vp.0 + 10.0, vp.1 + 10.0), vp);
        let project = |p| {
            let (x, y) = cam.world_to_screen(p, vp);
            key.clip.min + egui::vec2(x, y)
        };
        let mut shapes = Vec::new();
        let mut labels = Vec::new();
        let mut label_seen = std::collections::HashSet::new();
        let mut groups: std::collections::BTreeMap<
            &str,
            (Cluster, std::collections::HashSet<&str>),
        > = Default::default();
        for p in &self.prepared {
            for ring in &p.rings {
                let [x0, y0, x1, y1] = ring.bounds;
                if x1 < lo.0 || y1 < lo.1 || x0 > hi.0 || y0 > hi.1 {
                    continue;
                }
                if key.zoom < CLUSTER_ZOOM && !p.style.emergency {
                    let stroke = Stroke::new(1.1 * key.scale, p.style.color());
                    for i in 0..ring.points.len() {
                        if !ring.exterior[i] {
                            continue;
                        }
                        let a = project(ring.points[i]);
                        let b = project(ring.points[(i + 1) % ring.points.len()]);
                        if let Some(edge) = clip_segment(a, b, key.clip.expand(8.0)) {
                            if p.style.warning {
                                boundary_shapes(&mut shapes, &edge, key.clip, p.style, key.scale * 0.5);
                            } else if let Some((dash, gap)) = p.style.dash {
                                shapes.extend(Shape::dashed_line(&edge, stroke, dash, gap));
                            } else {
                                shapes.push(Shape::line_segment(edge, stroke));
                            }
                        }
                    }
                } else {
                    // Subpixel simplification affects paint only, never source inspection.
                    let mut pts = Vec::with_capacity(ring.points.len());
                    for &w in &ring.points {
                        let s = project(w);
                        if pts
                            .last()
                            .is_none_or(|last: &Pos2| last.distance_sq(s) >= 0.64)
                        {
                            pts.push(s);
                        }
                    }
                    if pts.len() >= 3 {
                        pts.push(pts[0]);
                        boundary_shapes(&mut shapes, &pts, key.clip, p.style, key.scale);
                    }
                }
                let bounds = Rect::from_two_pos(project((x0, y0)), project((x1, y1)));
                let visible = bounds.intersect(key.clip);
                let anchor = visible.center();
                let f = &features[p.index];
                if key.zoom < CLUSTER_ZOOM && f.kind != FeatureKind::TropicalCone {
                    let (lon, lat) = world_to_lonlat((x0 + x1) * 0.5, (y0 + y1) * 0.5);
                    let name = region(lon, lat);
                    let (group, seen) = groups.entry(name).or_insert_with(|| {
                        (
                            Cluster {
                                name,
                                anchor: Pos2::ZERO,
                                items: Vec::new(),
                            },
                            Default::default(),
                        )
                    });
                    if seen.insert(&p.key) {
                        group.items.push(p.index);
                        group.anchor += anchor.to_vec2();
                    }
                }
                if (key.zoom >= CLUSTER_ZOOM || p.style.emergency)
                    && bounds.width() >= 25.0
                    && label_seen.insert(&p.key)
                {
                    labels.push((p.index, p.style, anchor));
                }
            }
        }
        let mut clusters: Vec<_> = groups
            .into_values()
            .map(|(mut c, _)| {
                c.anchor = Pos2::ZERO + c.anchor.to_vec2() / c.items.len() as f32;
                c.items.sort_by_key(|&i| {
                    std::cmp::Reverse((
                        style(&features[i]).is_some_and(|s| s.emergency),
                        features[i].kind.z(),
                    ))
                });
                c
            })
            .collect();
        // Greedy placement keeps chips separated even when nearby regions have active weather.
        let mut occupied: Vec<Rect> = Vec::new();
        for c in &mut clusters {
            let xpad = 72.0_f32.min(key.clip.width() * 0.4);
            let top = 115.0_f32.min(key.clip.height() * 0.35);
            let bottom = 85.0_f32.min(key.clip.height() * 0.3);
            c.anchor.x = c
                .anchor
                .x
                .clamp(key.clip.left() + xpad, key.clip.right() - xpad);
            c.anchor.y = c
                .anchor
                .y
                .clamp(key.clip.top() + top, key.clip.bottom() - bottom);
            for _ in 0..14 {
                let r = Rect::from_center_size(c.anchor, egui::vec2(130.0, 34.0));
                if !occupied.iter().any(|o| o.intersects(r)) {
                    occupied.push(r);
                    break;
                }
                c.anchor.y += 38.0;
                if c.anchor.y > key.clip.bottom() - 55.0 {
                    c.anchor.y = key.clip.top() + 115.0;
                    c.anchor.x += 138.0;
                }
            }
        }
        let clipped = shapes
            .into_iter()
            .map(|shape| egui::epaint::ClippedShape {
                clip_rect: key.clip,
                shape,
            })
            .collect();
        let mut mesh = egui::Mesh::default();
        for p in ctx.tessellate(clipped, key.ppp) {
            if let egui::epaint::Primitive::Mesh(m) = p.primitive {
                mesh.append(m);
            }
        }
        Pane {
            key,
            mesh: std::sync::Arc::new(mesh),
            labels,
            clusters,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ui: &egui::Ui,
        pane: usize,
        generation: u64,
        features: &[GeoFeature],
        clip: Rect,
        cam: crate::render::mercator::Camera,
        scale: f32,
    ) {
        self.prepare(generation, features);
        let key = ViewKey {
            center: cam.center,
            zoom: cam.zoom,
            clip,
            scale,
            ppp: ui.ctx().pixels_per_point(),
        };
        if self.panes.get(&pane).is_none_or(|p| p.key != key) {
            let built = self.build(ui.ctx(), key, features);
            self.panes.insert(pane, built);
        }
        let cached = &self.panes[&pane];
        let painter = ui.painter().with_clip_rect(clip);
        painter.add(Shape::Mesh(cached.mesh.clone()));
        let mut taken = Vec::new();
        for &(i, s, p) in cached.labels.iter().rev() {
            label(&painter, clip, p, &features[i].title, s.color(), &mut taken);
        }
        for c in &cached.clusters {
            let clicked = egui::Area::new(egui::Id::new(("regional_cluster", pane, c.name)))
                .order(egui::Order::Middle)
                .fixed_pos(c.anchor - egui::vec2(58.0, 15.0))
                .show(ui.ctx(), |ui| {
                    ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!("{} · {}", c.name, c.items.len()))
                                .size(12.0)
                                .color(Color32::from_rgb(220, 239, 255)),
                        )
                        .fill(Color32::from_rgb(31, 57, 76))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(104, 154, 190)))
                        .corner_radius(16)
                        .min_size(egui::vec2(116.0, 30.0)),
                    )
                    .on_hover_text("Regional bulletins · all hazard boundaries remain visible")
                    .clicked()
                })
                .inner;
            if clicked {
                self.open = Some(c.clone());
            }
        }
    }

    pub fn show_list(
        &mut self,
        ctx: &egui::Context,
        generation: u64,
        features: &[GeoFeature],
    ) -> Option<usize> {
        if self.generation != Some(generation) {
            self.open = None;
        }
        let group = self.open.as_ref()?;
        let mut open = true;
        let mut chosen = None;
        egui::Window::new(format!("{} · regional bulletins", group.name))
            .id(egui::Id::new("regional_bulletins"))
            .open(&mut open)
            .collapsible(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.weak(format!(
                    "{} bulletins in view · all hazard types",
                    group.items.len()
                ));
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for &i in &group.items {
                            let Some(f) = features.get(i) else {
                                continue;
                            };
                            let col = style(f).map_or(Color32::WHITE, |s| s.color());
                            let text = if let Some(a) = &f.alert {
                                format!("{}\n{}", f.title, a.area)
                            } else {
                                f.title.clone()
                            };
                            if ui
                                .add_sized(
                                    [ui.available_width(), 48.0],
                                    egui::Button::new(egui::RichText::new(text).color(col)).wrap(),
                                )
                                .clicked()
                            {
                                chosen = Some(i);
                            }
                        }
                    });
            });
        if !open || chosen.is_some() {
            self.open = None;
        }
        chosen
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
    fn regional_cache_deduplicates_bulletins_and_invalidates_on_view_or_feed_change() {
        use crate::render::mercator::Camera;
        let ctx = egui::Context::default();
        let clip = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0));
        let mut a = feature(FeatureKind::Statement);
        a.rings = vec![vec![
            [-99.0, 35.0],
            [-98.0, 35.0],
            [-98.0, 36.0],
            [-99.0, 36.0],
        ]];
        let mut features = vec![a.clone(), a.clone()];
        let mut md = a.clone();
        md.kind = FeatureKind::MesoDiscussion;
        md.title = "MD 1248".into();
        features.push(md);
        let mut cache = Cache::default();
        let cam = Camera::at_lonlat(-98.0, 35.0, 5.0);
        let draw = |cache: &mut Cache, generation, cam, pane| {
            let _ = ctx.run_ui(Default::default(), |ui| {
                cache.draw(ui, pane, generation, &features, clip, cam, 1.0)
            });
        };
        draw(&mut cache, 1, cam, 0);
        assert_eq!(cache.prepared.len(), 3, "polygon parts stay intact");
        assert_eq!(cache.panes[&0].clusters.len(), 1);
        assert_eq!(
            cache.panes[&0].clusters[0].items.len(),
            2,
            "same bulletin counted once"
        );
        let mesh = cache.panes[&0].mesh.clone();
        assert!(!mesh.vertices.is_empty());
        let start = std::time::Instant::now();
        for _ in 0..20 {
            draw(&mut cache, 1, cam, 0);
        }
        eprintln!(
            "20 warm outline frames: {:?}; shared mesh reused",
            start.elapsed()
        );
        assert!(std::sync::Arc::ptr_eq(&mesh, &cache.panes[&0].mesh));
        draw(&mut cache, 1, Camera::at_lonlat(-98.0, 35.0, 8.0), 1);
        assert!(
            cache.panes[&1].clusters.is_empty(),
            "local view shows individual labels"
        );
        assert!(!cache.panes[&1].labels.is_empty());
        draw(&mut cache, 1, Camera::at_lonlat(50.0, 0.0, 8.0), 0);
        assert!(
            cache.panes[&0].mesh.vertices.is_empty(),
            "offscreen rings culled"
        );
        cache.open = Some(
            cache.panes[&1]
                .clusters
                .first()
                .cloned()
                .unwrap_or(Cluster {
                    name: "Plains",
                    anchor: Pos2::ZERO,
                    items: vec![0],
                }),
        );
        draw(&mut cache, 2, cam, 0);
        assert!(
            cache.open.is_some(),
            "open reader survives a feed refresh by bulletin identity"
        );
        assert_eq!(cache.panes.len(), 1, "old pane generation evicted");
        assert!(!std::sync::Arc::ptr_eq(&mesh, &cache.panes[&0].mesh));
        cache.prepare(3, &[]);
        assert!(
            cache.open.is_none(),
            "removed bulletins cannot leave stale indices"
        );
    }

    #[test]
    fn nationwide_outline_work_is_reused_during_playback() {
        use crate::render::mercator::{lonlat_to_world, Camera};
        let ctx = egui::Context::default();
        let clip = Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let cam = Camera::at_lonlat(-97.0, 38.0, 4.5);
        let features: Vec<_> = (0..1000)
            .map(|i| {
                let mut f = feature(FeatureKind::Statement);
                let lon = -122.0 + (i % 50) as f64;
                let lat = 26.0 + (i / 50) as f64;
                f.title = format!("Statement {}", i / 10);
                f.rings = vec![(0..40)
                    .map(|j| {
                        let a = j as f64 * std::f64::consts::TAU / 40.0;
                        [lon + a.cos() * 0.6, lat + a.sin() * 0.6]
                    })
                    .collect()];
                f
            })
            .collect();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            let _ = ctx.run_ui(Default::default(), |ui| {
                draw(ui.painter(), &features, clip, cam.zoom, 1.0, |lon, lat| {
                    let (x, y) = cam.world_to_screen(lonlat_to_world(lon, lat), (1200.0, 800.0));
                    egui::pos2(x, y)
                })
            });
        }
        let old = start.elapsed();
        let mut cache = Cache::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            cache.draw(ui, 0, 1, &features, clip, cam, 1.0)
        });
        let mesh = cache.panes[&0].mesh.clone();
        let start = std::time::Instant::now();
        for _ in 0..5 {
            let _ = ctx.run_ui(Default::default(), |ui| {
                cache.draw(ui, 0, 1, &features, clip, cam, 1.0)
            });
        }
        eprintln!("1,000 polygon parts, five CPU paint passes: original {:?}, cached {:?} (not full browser frame time)",old,start.elapsed());
        assert!(std::sync::Arc::ptr_eq(&mesh, &cache.panes[&0].mesh));
        assert_eq!(cache.prepared.len(), 1000);
        assert!(!cache.panes[&0].clusters.is_empty());
    }

    #[test]
    fn shared_county_edges_disappear_only_within_the_same_bulletin() {
        let a = feature(FeatureKind::Watch);
        let mut b = a.clone();
        for p in &mut b.rings[0] {
            p[0] += 1.0;
        }
        let mut cache = Cache::default();
        cache.prepare(1, &[a.clone(), b.clone()]);
        assert_eq!(
            cache
                .prepared
                .iter()
                .flat_map(|p| &p.rings)
                .flat_map(|r| &r.exterior)
                .filter(|&&x| !x)
                .count(),
            2
        );
        b.title = "Different watch".into();
        cache.prepare(2, &[a.clone(), b]);
        assert!(cache
            .prepared
            .iter()
            .flat_map(|p| &p.rings)
            .flat_map(|r| &r.exterior)
            .all(|&x| x));
        cache.prepare(3, &[a.clone(), a]);
        assert!(
            cache
                .prepared
                .iter()
                .flat_map(|p| &p.rings)
                .flat_map(|r| &r.exterior)
                .all(|&x| x),
            "duplicate polygons retain their outline"
        );
    }

    #[test]
    fn styles_keep_hazard_roles_distinct_and_do_not_invent_emergencies() {
        let warning = style(&feature(FeatureKind::Warning)).unwrap();
        let watch = style(&feature(FeatureKind::Watch)).unwrap();
        let md = style(&feature(FeatureKind::MesoDiscussion)).unwrap();
        assert!(warning.dash.is_none() && !warning.emergency);
        assert!(watch.dash.is_some());
        for kind in [FeatureKind::Watch, FeatureKind::WatchBox] {
            let mut tornado = feature(kind);
            tornado.title = "Tornado Watch 0638".into();
            assert_eq!(style(&tornado).unwrap().rgb, [230, 40, 40]);
            tornado.title = "Severe Thunderstorm Watch 0639".into();
            assert_eq!(style(&tornado).unwrap().rgb, [255, 226, 108]);
        }
        assert_eq!(md.rgb, [58, 96, 245]);
        assert!(md.dash.is_none());
        assert!(watch.fill < warning.fill);
        assert!(style(&feature(FeatureKind::Outlook)).is_none());
        let mut flood = feature(FeatureKind::Warning);
        flood.title = "Flood Warning".into();
        assert_eq!(style(&flood).unwrap().rgb, [0, 160, 90]);
        flood.title = "Flash Flood Warning".into();
        assert_eq!(style(&flood).unwrap().rgb, [57, 255, 20]);
        flood.title = "Severe Thunderstorm Warning".into();
        assert_eq!(style(&flood).unwrap().rgb, [255, 225, 40]);
        let mut statement = feature(FeatureKind::Statement);
        statement.title = "Special Weather Statement".into();
        assert_eq!(style(&statement).unwrap().rgb, [255, 228, 181]);
        statement.alert = Some(
            serde_json::from_value(serde_json::json!({
                "id":"sws", "event":"Special Weather Statement", "headline":"Test",
                "area":"Test only", "description":"", "instruction":""
            }))
            .unwrap(),
        );
        statement.title = "A different display title".into();
        assert_eq!(style(&statement).unwrap().rgb, [255, 228, 181]);
        statement.alert.as_mut().unwrap().event = "Marine Weather Statement".into();
        assert_eq!(style(&statement).unwrap().rgb, [114, 186, 255]);
        assert_eq!(
            style(&feature(FeatureKind::Advisory)).unwrap().rgb,
            [114, 186, 255]
        );
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
        for (kind, event) in [(FeatureKind::Statement, "Air Quality Alert"), (FeatureKind::Watch, "Air Quality Watch")] {
            let mut air = feature(kind);
            air.title = event.into();
            assert_eq!(style(&air).unwrap().rgb, [255, 255, 255]);
            air.alert = Some(serde_json::from_value(serde_json::json!({
                "id":"air", "event":event, "headline":"Test",
                "area":"Test only", "description":"", "instruction":""
            })).unwrap());
            air.title = "Custom label".into();
            assert_eq!(style(&air).unwrap().rgb, [255, 255, 255]);
        }
        let mut tropical = feature(FeatureKind::TropicalCone);
        for title in ["Storm 34 kt wind", "Surge: 3 ft"] {
            tropical.title = title.into();
            assert!(style(&tropical).is_none());
        }
        tropical.title = "Storm cone".into();
        assert!(style(&tropical).is_some());
    }
    #[test]
    fn warning_edges_have_a_white_halo_and_hazard_colored_core() {
        let clip = Rect::from_min_max(Pos2::ZERO, egui::pos2(100.0, 100.0));
        let points = [egui::pos2(10.0, 10.0), egui::pos2(90.0, 10.0)];
        let mut warning = feature(FeatureKind::Warning);
        for event in ["Severe Thunderstorm Warning", "Tornado Warning", "Flood Warning"] {
            warning.title = event.into();
            for scale in [0.5, 1.0, 2.0] {
                let mut shapes = Vec::new();
                let style = style(&warning).unwrap();
                boundary_shapes(&mut shapes, &points, clip, style, scale);
                let strokes: Vec<_> = shapes.iter().filter_map(|shape| match shape {
                    Shape::LineSegment { stroke, .. } => Some(*stroke),
                    _ => None,
                }).collect();
                assert_eq!(strokes.len(), 3);
                assert_eq!(strokes[1], Stroke::new(7.0 * scale, Color32::WHITE));
                assert_eq!(strokes[2], Stroke::new(3.2 * scale, style.color()));
                assert!(strokes[0].width > strokes[1].width);
            }
        }
        assert!(!style(&feature(FeatureKind::Watch)).unwrap().warning);
        assert!(!style(&feature(FeatureKind::Statement)).unwrap().warning);
    }

    #[test]
    fn fire_and_heat_map_edges_and_labels_keep_the_alert_color() {
        for (kind, event) in [
            (FeatureKind::Watch, "Fire Weather Watch"),
            (FeatureKind::Watch, "Extreme Heat Watch"),
            (FeatureKind::Warning, "Red Flag Warning"),
            (FeatureKind::Warning, "Extreme Heat Warning"),
            (FeatureKind::Warning, "Excessive Heat Warning"),
            (FeatureKind::Advisory, "Heat Advisory"),
        ] {
            let mut alert = feature(kind);
            alert.title = event.into();
            let rgb = match event {
                "Extreme Heat Watch" => [166, 94, 42],
                "Heat Advisory" => [190, 117, 66],
                _ => [205, 133, 63],
            };
            alert.stroke = [rgb[0], rgb[1], rgb[2], 235];
            assert_eq!(style(&alert).unwrap().rgb, rgb, "{event}");
        }
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
