//! Caption, color bar and city labels painted into a finished render, on the CPU.
//!
//! A shared image has to say what it is looking at and when — a radar picture with no site, no
//! time and no scale is a pretty texture. The app draws all three with egui over the map; this
//! does the same thing for the off-screen renders the server serves, without a window.
//!
//! Why the CPU and not a second `egui_wgpu` pass over the same target: the renders are produced on
//! whatever adapter the box has, including lavapipe, and a text pass through the GPU is the part
//! most likely to differ between them. Laying glyphs out with epaint and blitting the atlas by
//! hand is a few dozen lines, has no adapter in it at all, and is byte-identical everywhere.
//!
//! Native only — nothing here is reachable from the wasm build, which draws its chrome the normal
//! egui way.

use crate::colormap::ColorTable;
use egui::epaint::text::{FontId, Fonts, TextOptions};
use egui::Color32;

/// Distance from the image edge to anything drawn here.
const INSET: f32 = 14.0;
/// Caption size in pixels, at the reference width below. Everything scales off this.
const CAPTION_PX: f32 = 15.0;
/// The width the sizes above are tuned for; a bigger render gets proportionally bigger text.
const REFERENCE_W: f32 = 1000.0;

/// The color bar to draw, if the render has a moment to explain.
pub struct Bar {
    pub table: ColorTable,
    /// e.g. `"dBZ"`.
    pub unit: &'static str,
}

/// Everything painted on top of a finished render.
#[derive(Default)]
pub struct Stamp {
    /// One line, bottom left: `KTLX · REF 0.5° · 2026-08-29 20:32Z · hookecho.io`.
    pub caption: String,
    pub bar: Option<Bar>,
    /// City labels, already projected to pixels by the caller — this module knows nothing about
    /// cameras, and the caller is the only place the projection exists.
    pub labels: Vec<(f32, f32, String)>,
}

/// The buffer being painted into: an RGBA8 image and its dimensions, kept together so the
/// drawing helpers take one argument for "where" instead of three.
struct Canvas<'a> {
    rgba: &'a mut [u8],
    w: u32,
    h: u32,
}

/// Paint `stamp` into an RGBA8 buffer of `w`x`h` pixels, in place.
pub fn draw(rgba: &mut [u8], w: u32, h: u32, stamp: &Stamp) {
    if rgba.len() < (w as usize * h as usize * 4) {
        return;
    }
    let canvas = &mut Canvas { rgba, w, h };
    let mut fonts = Fonts::new(
        TextOptions {
            max_texture_side: 2048,
            ..Default::default()
        },
        crate::fonts::base(),
    );
    let scale = (w as f32 / REFERENCE_W).clamp(0.7, 2.0);

    // Labels first: the caption and the bar are the frame, and nothing may cover them.
    let label_px = 12.0 * scale;
    let mut taken: Vec<[f32; 4]> = Vec::new();
    for (x, y, name) in &stamp.labels {
        let size = text_size(&mut fonts, name, label_px);
        let (x0, y0) = (x - size.0 / 2.0, y - size.1 / 2.0);
        let box_ = [x0, y0, x0 + size.0, y0 + size.1];
        // Greedy: first label in wins its space, later ones that would collide are dropped. A
        // proper label solver is a different project; ten names on a radar picture is the job.
        if taken.iter().any(|t| overlaps(t, &box_)) {
            continue;
        }
        taken.push(box_);
        text(
            canvas,
            &mut fonts,
            name,
            label_px,
            x0,
            y0,
            Color32::from_rgb(235, 235, 240),
        );
    }

    let caption_px = CAPTION_PX * scale;
    let inset = INSET * scale;
    if !stamp.caption.is_empty() {
        let size = text_size(&mut fonts, &stamp.caption, caption_px);
        text(
            canvas,
            &mut fonts,
            &stamp.caption,
            caption_px,
            inset,
            h as f32 - inset - size.1,
            Color32::from_rgb(240, 240, 245),
        );
    }

    if let Some(bar) = &stamp.bar {
        draw_bar(canvas, &mut fonts, bar, scale);
    }
}

/// The moment's color scale as a horizontal ramp above the bottom-right corner, with its ends
/// labelled. Sampled from the same [`ColorTable`] the shader's LUT is baked from, so the picture
/// and its key cannot drift apart.
fn draw_bar(c: &mut Canvas<'_>, fonts: &mut Fonts, bar: &Bar, scale: f32) {
    let (w, h) = (c.w, c.h);
    let (Some(first), Some(last)) = (bar.table.stops.first(), bar.table.stops.last()) else {
        return;
    };
    let (vmin, vmax) = (first.value, last.value);
    let span = (vmax - vmin).max(f32::EPSILON);

    let inset = INSET * scale;
    let bar_h = 10.0 * scale;
    let bar_w = (w as f32 * 0.32).min(360.0 * scale);
    let x1 = w as f32 - inset;
    let x0 = x1 - bar_w;
    let y1 = h as f32 - inset;
    let y0 = y1 - bar_h;

    for px in 0..bar_w as u32 {
        let value = vmin + (px as f32 / bar_w) * span;
        let Some(color) = bar.table.sample(value) else {
            continue;
        };
        for py in y0 as u32..y1 as u32 {
            c.blend(x0 as u32 + px, py, [color[0], color[1], color[2]], 255);
        }
    }

    let tick_px = 11.0 * scale;
    let low = format!("{vmin:.0}");
    let high = format!("{vmax:.0} {}", bar.unit);
    let low_size = text_size(fonts, &low, tick_px);
    let high_size = text_size(fonts, &high, tick_px);
    let ty = y0 - low_size.1 - 2.0 * scale;
    let dim = Color32::from_rgb(215, 215, 220);
    text(c, fonts, &low, tick_px, x0, ty, dim);
    text(c, fonts, &high, tick_px, x1 - high_size.0, ty, dim);
}

fn overlaps(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn text_size(fonts: &mut Fonts, s: &str, px: f32) -> (f32, f32) {
    let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
        s.to_owned(),
        FontId::proportional(px),
        Color32::WHITE,
    );
    (galley.rect.width(), galley.rect.height())
}

/// Blit one line of text with its top-left at (`x`, `y`).
///
/// Drawn twice: once in black one pixel down-right, then in `color`. The halo is what makes a
/// caption readable over a bright echo and over an empty dark map both.
fn text(c: &mut Canvas<'_>, fonts: &mut Fonts, s: &str, px: f32, x: f32, y: f32, color: Color32) {
    let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
        s.to_owned(),
        FontId::proportional(px),
        color,
    );
    // The atlas has to be read after layout: laying the text out is what allocates its glyphs.
    let atlas = fonts.image();
    let [aw, _ah] = atlas.size;
    for (dx, dy, ink) in [(1.0, 1.0, Color32::BLACK), (0.0, 0.0, color)] {
        for row in &galley.rows {
            for glyph in &row.row.glyphs {
                if glyph.uv_rect.is_nothing() {
                    continue;
                }
                let left = row.pos.x + glyph.pos.x + glyph.uv_rect.offset.x + x + dx;
                let top = row.pos.y + glyph.pos.y + glyph.uv_rect.offset.y + y + dy;
                let gw = (glyph.uv_rect.max[0] - glyph.uv_rect.min[0]) as u32;
                let gh = (glyph.uv_rect.max[1] - glyph.uv_rect.min[1]) as u32;
                for gy in 0..gh {
                    for gx in 0..gw {
                        let tx = glyph.uv_rect.min[0] as usize + gx as usize;
                        let ty = glyph.uv_rect.min[1] as usize + gy as usize;
                        let Some(texel) = atlas.pixels.get(ty * aw + tx) else {
                            continue;
                        };
                        let alpha = texel.a();
                        if alpha == 0 {
                            continue;
                        }
                        let (px_x, px_y) = (left + gx as f32, top + gy as f32);
                        if px_x < 0.0 || px_y < 0.0 {
                            continue;
                        }
                        c.blend(
                            px_x as u32,
                            px_y as u32,
                            [ink.r(), ink.g(), ink.b()],
                            alpha,
                        );
                    }
                }
            }
        }
    }
}

/// Source-over one pixel. Out-of-bounds writes are dropped rather than wrapped — a label near the
/// edge is clipped, never smeared onto the opposite side.
impl Canvas<'_> {
    fn blend(&mut self, x: u32, y: u32, color: [u8; 3], alpha: u8) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        let a = alpha as u32;
        for (k, &c) in color.iter().enumerate() {
            let dst = self.rgba[i + k] as u32;
            self.rgba[i + k] = ((c as u32 * a + dst * (255 - a)) / 255) as u8;
        }
        self.rgba[i + 3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text lands inside the buffer, changes pixels where it is drawn, and leaves the rest alone.
    /// This is the test that catches an epaint atlas API change — the only thing here that can
    /// silently start producing nothing.
    #[test]
    fn a_caption_marks_the_bottom_left_and_nothing_else() {
        let (w, h) = (400u32, 200u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        draw(
            &mut rgba,
            w,
            h,
            &Stamp {
                caption: "KTLX · REF · hookecho.io".to_string(),
                ..Default::default()
            },
        );
        let lit = |x0: u32, y0: u32, x1: u32, y1: u32| {
            let mut n = 0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    if rgba[i] > 0 || rgba[i + 1] > 0 || rgba[i + 2] > 0 {
                        n += 1;
                    }
                }
            }
            n
        };
        assert!(lit(0, h - 40, w / 2, h) > 50, "no caption was drawn");
        assert_eq!(lit(0, 0, w, h / 2), 0, "something was drawn off-caption");
    }

    /// Blending stays in bounds: a glyph hanging off the edge is clipped, not wrapped.
    #[test]
    fn out_of_bounds_pixels_are_dropped() {
        let mut rgba = vec![0u8; 4 * 4 * 4];
        let mut canvas = Canvas {
            rgba: &mut rgba,
            w: 4,
            h: 4,
        };
        canvas.blend(9, 1, [255, 255, 255], 255);
        canvas.blend(1, 9, [255, 255, 255], 255);
        assert!(rgba.iter().all(|&b| b == 0));
    }
}
