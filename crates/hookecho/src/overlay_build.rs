//! CPU tessellation of overlay features into GPU-ready triangles.
//!
//! Fills and outlines of the lon/lat [`GeoFeature`] polygons are projected to world space
//! and tessellated with lyon. Stroke width is set in world units for the current zoom, so
//! the app rebuilds this when the zoom bucket changes (feature counts are small).

use crate::render::mercator::lonlat_to_world;
use crate::render::OverlayVertex;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use wxdata::overlay::GeoFeature;
use wxdata::placefile::{PlaceItem, PlaceKind};

/// Tessellated overlay geometry ready for a single vertex+index draw.
#[derive(Default)]
pub struct OverlayGeom {
    pub vertices: Vec<OverlayVertex>,
    pub indices: Vec<u32>,
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
    [
        srgb_to_linear(rgba[0]),
        srgb_to_linear(rgba[1]),
        srgb_to_linear(rgba[2]),
        rgba[3] as f32 / 255.0,
    ]
}

/// Build tessellated fills + outlines for `features` at `zoom`.
pub fn build(features: &[GeoFeature], zoom: f64) -> OverlayGeom {
    let mut geom = OverlayGeom::default();
    let mut fill_tess = FillTessellator::new();
    let mut stroke_tess = StrokeTessellator::new();
    // ~1.6 px outline in world units at this zoom.
    let stroke_w = (1.6 / (256.0 * 2f64.powf(zoom))) as f32;
    let fill_opts = FillOptions::default().with_tolerance(stroke_w * 0.5);
    let stroke_opts = StrokeOptions::default()
        .with_line_width(stroke_w)
        .with_tolerance(stroke_w * 0.5);

    for f in features {
        let path = feature_path(f);
        let fill = color(f.fill);
        let stroke = color(f.stroke);

        let mut buf: VertexBuffers<OverlayVertex, u32> = VertexBuffers::new();
        let _ = fill_tess.tessellate_path(
            &path,
            &fill_opts,
            &mut BuffersBuilder::new(&mut buf, |v: FillVertex| OverlayVertex {
                world: [v.position().x, v.position().y],
                color: fill,
            }),
        );
        let _ = stroke_tess.tessellate_path(
            &path,
            &stroke_opts,
            &mut BuffersBuilder::new(&mut buf, |v: StrokeVertex| OverlayVertex {
                world: [v.position().x, v.position().y],
                color: stroke,
            }),
        );
        append(&mut geom, buf);
    }
    geom
}

/// Append tessellated placefile line/polygon geometry to `geom` (text/icons are drawn by the
/// egui painter). Items must be pre-filtered by threshold/time. Line widths honor `zoom`.
pub fn append_placefiles(geom: &mut OverlayGeom, items: &[&PlaceItem], zoom: f64) {
    let mut fill_tess = FillTessellator::new();
    let mut stroke_tess = StrokeTessellator::new();
    let px = |w: f32| (w as f64 / (256.0 * 2f64.powf(zoom))) as f32;

    for item in items {
        match &item.kind {
            PlaceKind::Line { color: col, width, pts } => {
                let stroke = color(*col);
                let mut b = Path::builder();
                let mut it = pts.iter().map(|&[lon, lat]| {
                    let (wx, wy) = lonlat_to_world(lon, lat);
                    lyon::math::point(wx as f32, wy as f32)
                });
                if let Some(first) = it.next() {
                    b.begin(first);
                    for p in it {
                        b.line_to(p);
                    }
                    b.end(false);
                }
                let path = b.build();
                let opts = StrokeOptions::default()
                    .with_line_width(px(*width).max(px(1.0)))
                    .with_line_cap(lyon::path::LineCap::Round)
                    .with_line_join(lyon::path::LineJoin::Round);
                let mut buf: VertexBuffers<OverlayVertex, u32> = VertexBuffers::new();
                let _ = stroke_tess.tessellate_path(
                    &path,
                    &opts,
                    &mut BuffersBuilder::new(&mut buf, |v: StrokeVertex| OverlayVertex {
                        world: [v.position().x, v.position().y],
                        color: stroke,
                    }),
                );
                append(geom, buf);
            }
            PlaceKind::Polygon { color: col, rings } => {
                let fill = color(*col);
                let mut b = Path::builder();
                for ring in rings {
                    let mut it = ring.iter().map(|&[lon, lat]| {
                        let (wx, wy) = lonlat_to_world(lon, lat);
                        lyon::math::point(wx as f32, wy as f32)
                    });
                    if let Some(first) = it.next() {
                        b.begin(first);
                        for p in it {
                            b.line_to(p);
                        }
                        b.end(true);
                    }
                }
                let path = b.build();
                let opts = FillOptions::default();
                let mut buf: VertexBuffers<OverlayVertex, u32> = VertexBuffers::new();
                let _ = fill_tess.tessellate_path(
                    &path,
                    &opts,
                    &mut BuffersBuilder::new(&mut buf, |v: FillVertex| OverlayVertex {
                        world: [v.position().x, v.position().y],
                        color: fill,
                    }),
                );
                append(geom, buf);
            }
            PlaceKind::Text { .. } | PlaceKind::Icon { .. } => {} // painter pass
        }
    }
}

/// Build a closed lyon path (outer ring + holes) from a feature, in world coordinates.
fn feature_path(f: &GeoFeature) -> Path {
    let mut b = Path::builder();
    for ring in &f.rings {
        let mut pts = ring.iter().map(|&[lon, lat]| {
            let (wx, wy) = lonlat_to_world(lon, lat);
            lyon::math::point(wx as f32, wy as f32)
        });
        if let Some(first) = pts.next() {
            b.begin(first);
            for p in pts {
                b.line_to(p);
            }
            b.end(true);
        }
    }
    b.build()
}

fn append(geom: &mut OverlayGeom, buf: VertexBuffers<OverlayVertex, u32>) {
    let base = geom.vertices.len() as u32;
    geom.vertices.extend(buf.vertices);
    geom.indices.extend(buf.indices.into_iter().map(|i| i + base));
}
