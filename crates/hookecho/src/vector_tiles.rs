//! Vector (MVT) basemap: fetch OpenFreeMap `.pbf` tiles, tessellate to GPU triangles via the
//! shared overlay pipeline, and extract city/town labels for the egui text pass.
//!
//! The tile template comes from the OpenFreeMap TileJSON at runtime (its snapshot segment
//! rotates, so it can't be hardcoded). Tiles are gzip-compressed; we sniff `0x1f 0x8b` and
//! gunzip. All fetch + decode + tessellate work runs on the tokio pool; the UI thread only
//! makes GPU buffers from the finished `(vertices, indices, labels)` triples.

use crate::basemap_style;
use crate::render::mercator::Camera;
use crate::render::{OverlayVertex, PendingVectorTile, TileId, VisibleTile};
use crate::tiles::{load_tile_bytes, tile_cover};
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use mvt_reader::feature::Value;
use mvt_reader::Reader;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use tokio::runtime::Handle;

const TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";
const MAX_VECTOR_Z: u8 = 14;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; hookecho/0.0; +github.com/d4vid87/hookecho)";

// Fill layers in painter's-algorithm order (drawn after the background quad, before strokes).
const FILL_LAYERS: &[(&str, &str)] =
    &[("landcover", "class"), ("landuse", "class"), ("park", "class"), ("water", "class")];
// Stroke layers, drawn last (over the fills).
const STROKE_LAYERS: &[(&str, &str)] =
    &[("waterway", ""), ("transportation", "class"), ("boundary", "admin_level")];

/// A city/town label to draw with the egui painter (never appears in GPU/headless PNGs).
#[derive(Clone, Debug)]
pub struct PlaceLabel {
    pub world: [f32; 2],
    pub name: String,
    /// OpenMapTiles `rank` (lower = more important); used for collision priority.
    pub rank: i64,
    /// True for `city` class (always shown); towns only appear when zoomed in.
    pub city: bool,
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn color(rgba: [u8; 4]) -> [f32; 4] {
    [srgb_to_linear(rgba[0]), srgb_to_linear(rgba[1]), srgb_to_linear(rgba[2]), rgba[3] as f32 / 255.0]
}

/// Stringify a feature property (Strings pass through, numbers stringify) for style matching.
fn prop(props: &Option<HashMap<String, Value>>, key: &str) -> String {
    match props.as_ref().and_then(|p| p.get(key)) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Int(i)) | Some(Value::SInt(i)) => i.to_string(),
        Some(Value::UInt(u)) => u.to_string(),
        Some(Value::Double(d)) => (*d as i64).to_string(),
        Some(Value::Float(f)) => (*f as i64).to_string(),
        _ => String::new(),
    }
}

/// Tile-local `(lx, ly)` in `0..extent` -> normalized mercator world point for tile `(z,x,y)`.
fn tw(lx: f32, ly: f32, n: f64, tx: f64, ty: f64, extent: f64) -> lyon::math::Point {
    let wx = (tx + lx as f64 / extent) / n;
    let wy = (ty + ly as f64 / extent) / n;
    lyon::math::point(wx as f32, wy as f32)
}

fn add_ring(
    b: &mut lyon::path::path::Builder,
    ring: &geo_types::LineString<f32>,
    closed: bool,
    n: f64,
    tx: f64,
    ty: f64,
    extent: f64,
) {
    let mut it = ring.0.iter();
    if let Some(c0) = it.next() {
        b.begin(tw(c0.x, c0.y, n, tx, ty, extent));
        for c in it {
            b.line_to(tw(c.x, c.y, n, tx, ty, extent));
        }
        b.end(closed);
    }
}

fn append(verts: &mut Vec<OverlayVertex>, indices: &mut Vec<u32>, buf: VertexBuffers<OverlayVertex, u32>) {
    let base = verts.len() as u32;
    verts.extend(buf.vertices);
    indices.extend(buf.indices.into_iter().map(|i| i + base));
}

/// Gunzip if the bytes are a gzip stream, else pass through (OpenFreeMap serves `.pbf` gzipped).
fn maybe_gunzip(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        use std::io::Read;
        let mut out = Vec::new();
        if flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    bytes.to_vec()
}

/// Decode + style + tessellate one MVT tile. Pure (no GPU/network); returns overlay geometry
/// (with a background land quad baked in) plus its city/town labels.
pub fn build_tile(
    bytes: &[u8],
    id: TileId,
    dark: bool,
    tess_zoom: f64,
) -> (Vec<OverlayVertex>, Vec<u32>, Vec<PlaceLabel>) {
    let (z, tx, ty) = id;
    let n = (1u64 << z) as f64;
    let (txf, tyf) = (tx as f64, ty as f64);

    let mut verts: Vec<OverlayVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Background land quad covering the whole tile.
    let bg = color(basemap_style::style(dark).background);
    let (x0, y0) = (txf / n, tyf / n);
    let (x1, y1) = ((txf + 1.0) / n, (tyf + 1.0) / n);
    let base = verts.len() as u32;
    verts.extend_from_slice(&[
        OverlayVertex { world: [x0 as f32, y0 as f32], color: bg },
        OverlayVertex { world: [x1 as f32, y0 as f32], color: bg },
        OverlayVertex { world: [x1 as f32, y1 as f32], color: bg },
        OverlayVertex { world: [x0 as f32, y1 as f32], color: bg },
    ]);
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    let data = maybe_gunzip(bytes);
    let Ok(reader) = Reader::new(data) else {
        return (verts, indices, Vec::new());
    };
    let names = reader.get_layer_names().unwrap_or_default();
    let meta = reader.get_layer_metadata().unwrap_or_default();
    let extent_of = |i: usize| meta.get(i).map(|m| m.extent as f64).unwrap_or(4096.0);

    let mut fill_t = FillTessellator::new();
    let mut stroke_t = StrokeTessellator::new();
    let px_to_world = 1.0 / (256.0 * 2f64.powf(tess_zoom));

    // Fills.
    for (layer, key) in FILL_LAYERS {
        let Some(i) = names.iter().position(|nm| nm == layer) else { continue };
        let extent = extent_of(i);
        let feats = reader.get_features(i).unwrap_or_default();
        for f in &feats {
            let cls = prop(&f.properties, key);
            let Some(c) = basemap_style::fill(dark, layer, &cls) else { continue };
            let mut b = Path::builder();
            let mut any = false;
            match &f.geometry {
                geo_types::Geometry::Polygon(p) => {
                    add_ring(&mut b, p.exterior(), true, n, txf, tyf, extent);
                    for r in p.interiors() {
                        add_ring(&mut b, r, true, n, txf, tyf, extent);
                    }
                    any = true;
                }
                geo_types::Geometry::MultiPolygon(mp) => {
                    for p in &mp.0 {
                        add_ring(&mut b, p.exterior(), true, n, txf, tyf, extent);
                        for r in p.interiors() {
                            add_ring(&mut b, r, true, n, txf, tyf, extent);
                        }
                        any = true;
                    }
                }
                _ => {}
            }
            if !any {
                continue;
            }
            let path = b.build();
            let fill = color(c);
            let opts = FillOptions::default().with_fill_rule(FillRule::EvenOdd);
            let mut buf: VertexBuffers<OverlayVertex, u32> = VertexBuffers::new();
            let _ = fill_t.tessellate_path(
                &path,
                &opts,
                &mut BuffersBuilder::new(&mut buf, |v: FillVertex| OverlayVertex {
                    world: [v.position().x, v.position().y],
                    color: fill,
                }),
            );
            append(&mut verts, &mut indices, buf);
        }
    }

    // Strokes.
    for (layer, key) in STROKE_LAYERS {
        let Some(i) = names.iter().position(|nm| nm == layer) else { continue };
        let extent = extent_of(i);
        let feats = reader.get_features(i).unwrap_or_default();
        for f in &feats {
            if *layer == "boundary" && prop(&f.properties, "maritime") == "1" {
                continue;
            }
            let cls = prop(&f.properties, key);
            let Some((c, wpx)) = basemap_style::stroke(dark, layer, &cls) else { continue };
            let w = (wpx as f64 * px_to_world) as f32;
            let mut b = Path::builder();
            let mut any = false;
            match &f.geometry {
                geo_types::Geometry::LineString(ls) => {
                    add_ring(&mut b, ls, false, n, txf, tyf, extent);
                    any = true;
                }
                geo_types::Geometry::MultiLineString(mls) => {
                    for ls in &mls.0 {
                        add_ring(&mut b, ls, false, n, txf, tyf, extent);
                        any = true;
                    }
                }
                _ => {}
            }
            if !any {
                continue;
            }
            let path = b.build();
            let stroke = color(c);
            let opts = StrokeOptions::default()
                .with_line_width(w)
                .with_line_cap(lyon::path::LineCap::Round)
                .with_line_join(lyon::path::LineJoin::Round);
            let mut buf: VertexBuffers<OverlayVertex, u32> = VertexBuffers::new();
            let _ = stroke_t.tessellate_path(
                &path,
                &opts,
                &mut BuffersBuilder::new(&mut buf, |v: StrokeVertex| OverlayVertex {
                    world: [v.position().x, v.position().y],
                    color: stroke,
                }),
            );
            append(&mut verts, &mut indices, buf);
        }
    }

    let labels = extract_labels(&reader, &names, n, txf, tyf);
    (verts, indices, labels)
}

/// Pull city/town point labels from the `place` layer.
fn extract_labels(reader: &Reader, names: &[String], n: f64, txf: f64, tyf: f64) -> Vec<PlaceLabel> {
    let Some(i) = names.iter().position(|nm| nm == "place") else { return Vec::new() };
    let extent = reader.get_layer_metadata().ok().and_then(|m| m.get(i).map(|l| l.extent as f64)).unwrap_or(4096.0);
    let mut out = Vec::new();
    for f in reader.get_features(i).unwrap_or_default() {
        let cls = prop(&f.properties, "class");
        let city = cls == "city";
        if !city && cls != "town" {
            continue;
        }
        let name = {
            let en = prop(&f.properties, "name:en");
            if en.is_empty() { prop(&f.properties, "name") } else { en }
        };
        if name.is_empty() {
            continue;
        }
        // OpenMapTiles encodes place labels as single-point MultiPoints.
        let pt = match &f.geometry {
            geo_types::Geometry::Point(p) => Some((p.x(), p.y())),
            geo_types::Geometry::MultiPoint(mp) => mp.0.first().map(|p| (p.x(), p.y())),
            _ => None,
        };
        if let Some((px, py)) = pt {
            let p = tw(px, py, n, txf, tyf, extent);
            let rank = match f.properties.as_ref().and_then(|p| p.get("rank")) {
                Some(Value::Int(r)) | Some(Value::SInt(r)) => *r,
                Some(Value::UInt(r)) => *r as i64,
                _ => 100,
            };
            out.push(PlaceLabel { world: [p.x, p.y], name, rank, city });
        }
    }
    out
}

fn fill_template(template: &str, z: u8, x: u32, y: u32) -> String {
    template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
}

/// Fetch the OpenFreeMap tile URL template from TileJSON, disk-cached with a TTL.
/// `// ponytail: 12h TTL over the rotating snapshot; add 404-driven refetch if a snapshot is
/// pulled mid-session before the TTL expires.`
pub async fn fetch_tilejson(client: &reqwest::Client, cache_dir: Option<&std::path::Path>) -> Option<String> {
    if let Some(dir) = cache_dir {
        let p = dir.join("tilejson.txt");
        if let Ok(meta) = std::fs::metadata(&p) {
            if let Ok(modt) = meta.modified() {
                if modt.elapsed().map(|e| e.as_secs() < 12 * 3600).unwrap_or(false) {
                    if let Ok(s) = std::fs::read_to_string(&p) {
                        return Some(s.trim().to_string());
                    }
                }
            }
        }
    }
    let body = client.get(TILEJSON_URL).send().await.ok()?.error_for_status().ok()?.text().await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let t = v["tiles"].get(0)?.as_str()?.to_string();
    if let Some(dir) = cache_dir {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join("tilejson.txt"), &t);
    }
    Some(t)
}

/// Headless helper: fetch + tessellate all `visible` vector tiles, returning GPU-ready geometry
/// and the merged label list (no async drain loop).
pub async fn fetch_visible_vector(
    client: &reqwest::Client,
    template: &str,
    dark: bool,
    tess_zoom: f64,
    visible: &[VisibleTile],
) -> (Vec<PendingVectorTile>, Vec<PlaceLabel>) {
    let mut out = Vec::new();
    let mut labels = Vec::new();
    for v in visible {
        let (z, x, y) = v.id;
        let url = fill_template(template, z, x, y);
        match load_tile_bytes(client, &url, None).await {
            Ok(bytes) => {
                let (verts, indices, lbls) = build_tile(&bytes, v.id, dark, tess_zoom);
                labels.extend(lbls);
                out.push(PendingVectorTile { id: v.id, vertices: verts, indices });
            }
            Err(e) => log::warn!("vector tile {url}: {e}"),
        }
    }
    (out, labels)
}

struct FetchedVector {
    id: TileId,
    vertices: Vec<OverlayVertex>,
    indices: Vec<u32>,
    labels: Vec<PlaceLabel>,
}

/// Async vector-tile manager for the GUI (mirrors [`crate::tiles::TileManager`]).
pub struct VectorTileManager {
    rt: Handle,
    client: reqwest::Client,
    tx: Sender<FetchedVector>,
    rx: Receiver<FetchedVector>,
    requested: HashSet<TileId>,
    uploaded: HashSet<TileId>,
    labels: HashMap<TileId, Vec<PlaceLabel>>,
    dark: bool,
    tess_zoom: i32,
    cache_root: Option<PathBuf>,
    template: Option<String>,
    template_tx: Sender<Option<String>>,
    template_rx: Receiver<Option<String>>,
    template_requested: bool,
}

impl VectorTileManager {
    pub fn new(rt: Handle) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (template_tx, template_rx) = std::sync::mpsc::channel();
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("build reqwest client");
        let cache_root = crate::paths::cache_dir().map(|d| d.join("vector"));
        Self {
            rt,
            client,
            tx,
            rx,
            requested: HashSet::new(),
            uploaded: HashSet::new(),
            labels: HashMap::new(),
            dark: true,
            tess_zoom: 7,
            cache_root,
            template: None,
            template_tx,
            template_rx,
            template_requested: false,
        }
    }

    /// Switch dark/light. Returns true if changed (caller should clear the GPU vector cache).
    pub fn set_style(&mut self, dark: bool) -> bool {
        if self.dark == dark {
            return false;
        }
        self.dark = dark;
        self.requested.clear();
        self.uploaded.clear();
        self.labels.clear();
        true
    }

    /// Note the current camera zoom (drives stroke widths). Returns true (clear GPU cache) only
    /// when overzooming past the max tile level, where tile ids stop changing but widths must.
    pub fn note_zoom(&mut self, cam_zoom: f64) -> bool {
        let tz = cam_zoom.round() as i32;
        if tz == self.tess_zoom {
            return false;
        }
        let overzoom = self.tess_zoom > MAX_VECTOR_Z as i32 || tz > MAX_VECTOR_Z as i32;
        self.tess_zoom = tz;
        if overzoom {
            self.requested.clear();
            self.uploaded.clear();
            self.labels.clear();
            true
        } else {
            false
        }
    }

    pub fn visible(&self, cam: &Camera, viewport_px: (f32, f32)) -> Vec<VisibleTile> {
        tile_cover(cam, viewport_px, MAX_VECTOR_Z)
    }

    /// Kick off tilejson + tile fetches for anything visible and not yet requested.
    pub fn request_missing(&mut self, visible: &[VisibleTile]) {
        while let Ok(t) = self.template_rx.try_recv() {
            self.template = t;
        }
        let Some(template) = self.template.clone() else {
            if !self.template_requested {
                self.template_requested = true;
                let client = self.client.clone();
                let tx = self.template_tx.clone();
                let dir = self.cache_root.clone();
                self.rt.spawn(async move {
                    let t = fetch_tilejson(&client, dir.as_deref()).await;
                    let _ = tx.send(t);
                });
            }
            return;
        };
        let dark = self.dark;
        let tess_zoom = self.tess_zoom as f64;
        for v in visible {
            if !self.requested.insert(v.id) {
                continue;
            }
            let (z, x, y) = v.id;
            let url = fill_template(&template, z, x, y);
            let path = self.cache_root.as_ref().map(|d| d.join(format!("{z}/{x}/{y}.pbf")));
            let client = self.client.clone();
            let tx = self.tx.clone();
            let id = v.id;
            self.rt.spawn(async move {
                if let Ok(bytes) = load_tile_bytes(&client, &url, path.as_deref()).await {
                    let (vertices, indices, labels) = build_tile(&bytes, id, dark, tess_zoom);
                    let _ = tx.send(FetchedVector { id, vertices, indices, labels });
                }
            });
        }
    }

    /// Drain finished tessellations into upload-ready tiles (each returned once).
    pub fn drain_ready(&mut self) -> Vec<PendingVectorTile> {
        let mut ready = Vec::new();
        while let Ok(f) = self.rx.try_recv() {
            if self.uploaded.insert(f.id) {
                self.labels.insert(f.id, f.labels);
                ready.push(PendingVectorTile { id: f.id, vertices: f.vertices, indices: f.indices });
            }
        }
        ready
    }

    /// Labels for the given visible tile ids (for the egui text pass).
    pub fn labels_for<'a>(&'a self, ids: impl Iterator<Item = &'a TileId>) -> Vec<&'a PlaceLabel> {
        ids.filter_map(|id| self.labels.get(id)).flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_yield_background_quad_only() {
        // Not a valid tile: build_tile still emits the background land quad (2 triangles).
        let (verts, indices, labels) = build_tile(b"", (7, 30, 49), true, 7.0);
        assert_eq!(verts.len(), 4);
        assert_eq!(indices.len(), 6);
        assert!(labels.is_empty());
    }

    #[test]
    fn template_fill() {
        assert_eq!(
            fill_template("https://x/planet/SNAP/{z}/{x}/{y}.pbf", 7, 30, 49),
            "https://x/planet/SNAP/7/30/49.pbf"
        );
    }
}
