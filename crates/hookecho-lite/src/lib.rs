//! The lite radar viewer: decode Level 3 super-res reflectivity/velocity and paint it into a 2D
//! canvas, on a CPU, in a wasm bundle small enough for a machine that cannot run the real app.
//!
//! The main app needs WebGPU or WebGL2 and megabytes of wasm. This has neither requirement: the
//! only thing it asks of the browser is `putImageData`. Everything asynchronous — fetching,
//! listing S3, the loop timer, the basemap tiles, the warning overlay — lives in plain JS in
//! `web/lite/app.js`; this crate is synchronous CPU work and nothing else.
//!
//! The view is fixed (a site, a zoom preset), which is what makes it cheap: the pixel → radar-bin
//! mapping is computed once in [`Viewer::set_view`] and every frame after that is a lookup per
//! pixel through a 256-entry color table.

use wasm_bindgen::prelude::*;
use wasm_bindgen::Clamped;
use web_sys::{CanvasRenderingContext2d, ImageData};

mod palette;

/// Azimuth slots, i.e. the resolution of the radial lookup. Super-res products report 720 radials
/// of 0.5°, so this is one slot per radial with no interpolation to do.
const SLOTS: usize = 720;

/// Range-bin size of the super-res digital products (N0B / N0G), in km.
const BIN_KM: f32 = 0.25;

/// One decoded scan: data levels laid out slot-major, plus the color table baked for its own
/// thresholds (they are per-product constants in practice, but they come off the wire, so the LUT
/// follows the frame rather than the product).
struct Frame {
    nbins: usize,
    levels: Vec<u8>,
    lut: [u8; 1024],
}

/// The product being displayed. Both are super-res digital radial arrays with the same tenths
/// threshold layout; they differ in color table and in whether level 1 means anything.
#[derive(Clone, Copy, PartialEq)]
enum Product {
    Reflectivity,
    Velocity,
}

#[wasm_bindgen]
pub struct Viewer {
    product: Product,
    ref_table: palette::Table,
    vel_table: palette::Table,
    frames: Vec<Frame>,
    // Per-canvas-pixel lookup, rebuilt only when the view changes.
    width: usize,
    height: usize,
    slot: Vec<u16>,
    /// Range bin index for the pixel, or `u16::MAX` when the pixel is beyond any product's range.
    bin: Vec<u16>,
    /// Reused RGBA scratch so a redraw allocates nothing.
    rgba: Vec<u8>,
}

#[wasm_bindgen]
impl Viewer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Viewer {
        Viewer {
            product: Product::Reflectivity,
            ref_table: palette::reflectivity(),
            vel_table: palette::velocity(),
            frames: Vec::new(),
            width: 0,
            height: 0,
            slot: Vec::new(),
            bin: Vec::new(),
            rgba: Vec::new(),
        }
    }

    /// Point the viewer at a radar and a canvas size, in the same Web Mercator frame the basemap
    /// tiles use: `zoom` is the slippy-map tile zoom, so a canvas pixel here is a tile pixel
    /// there and the two line up without either side knowing about the other.
    ///
    /// This is the expensive call — one inverse projection per pixel — and it is why the viewer
    /// has no pan or zoom: a fixed view pays this once and then every frame is a table lookup.
    pub fn set_view(&mut self, lat: f64, lon: f64, width: usize, height: usize, zoom: f64) {
        self.width = width;
        self.height = height;
        self.slot = vec![0; width * height];
        self.bin = vec![u16::MAX; width * height];
        self.rgba = vec![0; width * height * 4];

        let world = 256.0 * 2f64.powf(zoom);
        let (cx, cy) = merc(lat, lon, world);
        // Equirectangular km per degree at the radar. Good to a fraction of a percent inside a
        // 300 km radius, which is the whole product.
        let km_per_lat = 110.574;
        let km_per_lon = 111.320 * lat.to_radians().cos();

        // Pixel centers, not corners, so the radar lands exactly on the middle pixel.
        for py in 0..height {
            let wy = cy + py as f64 + 0.5 - height as f64 / 2.0;
            let plat = inv_merc_lat(wy, world);
            let dy = (plat - lat) * km_per_lat;
            for px in 0..width {
                let wx = cx + px as f64 + 0.5 - width as f64 / 2.0;
                let plon = wx / world * 360.0 - 180.0;
                let dx = (plon - lon) * km_per_lon;
                let r = (dx * dx + dy * dy).sqrt();
                let b = (r / BIN_KM as f64).round();
                let i = py * width + px;
                if b >= u16::MAX as f64 {
                    continue;
                }
                // Compass azimuth: 0 = north, increasing clockwise, same as the product's.
                let az = dx.atan2(dy).to_degrees().rem_euclid(360.0);
                self.slot[i] = ((az / 360.0 * SLOTS as f64) as usize % SLOTS) as u16;
                self.bin[i] = b as u16;
            }
        }
    }

    /// `"vel"` selects velocity, anything else reflectivity. Changing product invalidates the
    /// frames (they are one product's data levels), so the caller refetches.
    pub fn set_product(&mut self, name: &str) {
        let want = if name == "vel" {
            Product::Velocity
        } else {
            Product::Reflectivity
        };
        if want != self.product {
            self.product = want;
            self.frames.clear();
        }
    }

    /// Decode one Level 3 file and append it as the newest frame. Returns false if the bytes are
    /// not a digital radial product — a caller with a bad key should skip it, not blow up.
    pub fn add_frame(&mut self, bytes: &[u8]) -> bool {
        let Ok(p) = nexrad_level3::decode(bytes) else {
            return false;
        };
        let Some(ra) = p.radial.as_ref() else {
            return false;
        };
        if ra.radials.is_empty() || ra.nbins == 0 {
            return false;
        }
        let nbins = ra.nbins as usize;
        let mut levels = vec![0u8; SLOTS * nbins];
        for radial in &ra.radials {
            // A radial covers the wedge [start, start + delta). Fill every slot whose *center*
            // falls inside it: radials start on arbitrary fractions of a degree, and rounding the
            // edges instead of the centers leaves unpainted spokes between them.
            let to_slot = |deg: f32| (deg / 360.0 * SLOTS as f32 - 0.5).ceil() as i32;
            let first = to_slot(radial.start_deg);
            let last = to_slot(radial.start_deg + radial.delta_deg.max(f32::EPSILON));
            for k in 0..(last - first).max(1) {
                let slot = (first + k).rem_euclid(SLOTS as i32) as usize;
                let n = radial.levels.len().min(nbins);
                levels[slot * nbins..slot * nbins + n].copy_from_slice(&radial.levels[..n]);
            }
        }
        let (table, folded) = match self.product {
            Product::Reflectivity => (&self.ref_table, false),
            Product::Velocity => (&self.vel_table, true),
        };
        self.frames.push(Frame {
            nbins,
            levels,
            lut: table.bake(&p.thresholds, folded),
        });
        true
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn clear_frames(&mut self) {
        self.frames.clear();
    }

    /// Drop the oldest frames until at most `keep` remain, so a page left open overnight holds a
    /// loop rather than a night's worth of scans.
    pub fn trim_frames(&mut self, keep: usize) {
        if self.frames.len() > keep {
            self.frames.drain(..self.frames.len() - keep);
        }
    }

    /// Paint frame `idx` over the whole canvas. Out-of-range indices and an unset view are a
    /// no-op — the JS side drives this from a timer and must not have to synchronize with it.
    pub fn render(&mut self, ctx: &CanvasRenderingContext2d, idx: usize) -> Result<(), JsValue> {
        let Some(frame) = self.frames.get(idx) else {
            return Ok(());
        };
        if self.width == 0 || self.height == 0 {
            return Ok(());
        }
        let nbins = frame.nbins;
        for i in 0..self.width * self.height {
            let out = i * 4;
            let bin = self.bin[i] as usize;
            let level = if bin < nbins {
                frame.levels[self.slot[i] as usize * nbins + bin]
            } else {
                0
            };
            let c = level as usize * 4;
            self.rgba[out..out + 4].copy_from_slice(&frame.lut[c..c + 4]);
        }
        let img = ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(&self.rgba),
            self.width as u32,
            self.height as u32,
        )?;
        ctx.put_image_data(&img, 0.0, 0.0)
    }
}

impl Default for Viewer {
    fn default() -> Self {
        Self::new()
    }
}

/// Web Mercator: lat/lon to pixel coordinates on a `world`-pixel-wide square world.
fn merc(lat: f64, lon: f64, world: f64) -> (f64, f64) {
    let x = (lon + 180.0) / 360.0 * world;
    let s = lat.to_radians().sin().clamp(-0.9999, 0.9999);
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * world;
    (x, y)
}

fn inv_merc_lat(y: f64, world: f64) -> f64 {
    let n = std::f64::consts::PI * (1.0 - 2.0 * y / world);
    n.sinh().atan().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_map_to_the_expected_azimuth_and_range() {
        let mut v = Viewer::new();
        // 201×201 at zoom 9 around KTLX: ~249 m per pixel at this latitude.
        v.set_view(35.333, -97.278, 201, 201, 9.0);
        let center = 100 * 201 + 100;
        assert_eq!(v.bin[center], 0, "the radar sits at the center pixel");

        // 50 px straight up is due north; straight right is due east.
        let north = 50 * 201 + 100;
        let east = 100 * 201 + 150;
        // One slot of slack: the projection round-trip leaves a pixel a hair off the axis.
        assert!(v.slot[north] <= 1 || v.slot[north] >= 719, "north is azimuth 0");
        assert!(
            v.slot[east].abs_diff((SLOTS / 4) as u16) <= 1,
            "east is 90 degrees, got slot {}",
            v.slot[east]
        );
        // Mercator scale at 35.3 N, zoom 9: 156543 * cos(lat) / 2^9 ≈ 249 m/px, so 50 px is
        // ~12.5 km — fifty quarter-kilometer bins.
        let km = v.bin[east] as f32 * BIN_KM;
        assert!((km - 12.5).abs() < 0.5, "50 px east is ~12.5 km, got {km}");
    }

    #[test]
    fn reflectivity_lut_colors_only_real_data_levels() {
        let mut thr = [0i16; 16];
        thr[0] = -320; // -32 dBZ
        thr[1] = 5; // 0.5 dB steps
        let lut = palette::reflectivity().bake(&thr, false);
        assert_eq!(&lut[0..4], &[0, 0, 0, 0], "level 0 is transparent");
        assert_eq!(&lut[4..8], &[0, 0, 0, 0], "reflectivity never range-folds");
        // Level 86 is 10 dBZ, the table's lowest stop; below it stays transparent.
        assert_eq!(lut[85 * 4 + 3], 0, "below the display floor");
        assert!(lut[86 * 4 + 3] > 0, "the floor stop is painted");
    }

    #[test]
    fn velocity_lut_paints_the_range_folded_level() {
        let mut thr = [0i16; 16];
        thr[0] = -635;
        thr[1] = 5;
        let lut = palette::velocity().bake(&thr, true);
        assert!(lut[7] > 0, "level 1 carries the RF color");
    }
}
