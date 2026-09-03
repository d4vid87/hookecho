//! Web-mercator projection and a slippy-map camera.
//!
//! World coordinates are normalized mercator in `[0,1]`: `(0,0)` at 180°W / ~85.05°N,
//! `(1,1)` at 180°E / ~85.05°S. This matches the XYZ tile scheme so tile `(z,x,y)`
//! covers the world rect `[x/2^z, (x+1)/2^z] × [y/2^z, (y+1)/2^z]`.

use std::f64::consts::PI;

/// Max latitude representable in web mercator (where y would go to infinity).
pub const MAX_LAT: f64 = 85.05112878;

/// Longitude/latitude (degrees) -> normalized mercator world coord.
pub fn lonlat_to_world(lon: f64, lat: f64) -> (f64, f64) {
    let x = (lon + 180.0) / 360.0;
    let lat = lat.clamp(-MAX_LAT, MAX_LAT);
    let sin = (lat * PI / 180.0).sin();
    let y = 0.5 - (((1.0 + sin) / (1.0 - sin)).ln()) / (4.0 * PI);
    (x, y)
}

/// Normalized mercator world coord -> longitude/latitude (degrees).
/// CPU mirror of the radar shader's inverse projection; used in tests and map picking.
#[allow(dead_code)]
pub fn world_to_lonlat(x: f64, y: f64) -> (f64, f64) {
    let lon = x * 360.0 - 180.0;
    let n = PI * (1.0 - 2.0 * y);
    let lat = n.sinh().atan() * 180.0 / PI;
    (lon, lat)
}

/// Slippy-map camera: a world-space center and a fractional zoom.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// World-space center in `[0,1]²`.
    pub center: (f64, f64),
    /// Fractional zoom; `2^zoom` tiles span the world per axis.
    pub zoom: f64,
}

impl Camera {
    pub fn at_lonlat(lon: f64, lat: f64, zoom: f64) -> Self {
        Self {
            center: lonlat_to_world(lon, lat),
            zoom,
        }
    }

    /// World units covered by one screen pixel.
    pub fn world_per_pixel(&self) -> f64 {
        1.0 / (256.0 * 2f64.powf(self.zoom))
    }

    /// Pan by a screen-pixel delta (drag). Positive dx/dy move content right/down.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32, viewport_px: (f32, f32)) {
        let wpp = self.world_per_pixel();
        self.center.0 -= dx as f64 * wpp;
        self.center.1 -= dy as f64 * wpp;
        self.settle(viewport_px);
    }

    /// Put the camera back somewhere legal after a pan or a zoom.
    ///
    /// The two axes are not symmetric, because the world is not. Longitude wraps: pan east past
    /// the antimeridian and you arrive from the west, so `center.0` is taken modulo one world.
    /// Latitude does not — mercator stops at about 85 degrees and there is nothing beyond it — so
    /// `center.1` is clamped, and clamped by *half a viewport* rather than to the world's edge.
    /// Clamping the center to `0..=1` let the top or bottom half of the screen fill with nothing
    /// at low zoom, which is where "the map slid off the screen" came from.
    ///
    /// Zoomed far enough out that the whole world is shorter than the viewport, there is no
    /// legal range left and the world is simply centered.
    fn settle(&mut self, viewport_px: (f32, f32)) {
        self.center.0 = self.center.0.rem_euclid(1.0);
        let half = viewport_px.1 as f64 / 2.0 * self.world_per_pixel();
        self.center.1 = if half >= 0.5 {
            0.5
        } else {
            self.center.1.clamp(half, 1.0 - half)
        };
    }

    /// Zoom toward a screen point (cursor) so that world point stays under the cursor.
    pub fn zoom_at(&mut self, delta: f64, cursor_px: (f32, f32), viewport_px: (f32, f32)) {
        let before = self.screen_to_world(cursor_px, viewport_px);
        // 20 rather than the tile providers' 18: `tile_cover` clamps the tile z on its own, so the
        // last two levels overzoom the resident tiles instead of turning a spread-pinch into a
        // silent no-op — which is what it felt like at the old ceiling.
        self.zoom = (self.zoom + delta).clamp(2.0, 20.0);
        let after = self.screen_to_world(cursor_px, viewport_px);
        self.center.0 += before.0 - after.0;
        self.center.1 += before.1 - after.1;
        // The same settling a pan gets. Zooming used to clamp latitude and leave longitude alone,
        // so a zoom near the antimeridian walked `center.0` outside `0..1` and left it there until
        // the next drag happened to wrap it.
        self.settle(viewport_px);
    }

    /// Screen pixel (origin top-left) -> world coord.
    pub fn screen_to_world(&self, px: (f32, f32), viewport_px: (f32, f32)) -> (f64, f64) {
        let wpp = self.world_per_pixel();
        let wx = self.center.0 + (px.0 as f64 - viewport_px.0 as f64 / 2.0) * wpp;
        let wy = self.center.1 + (px.1 as f64 - viewport_px.1 as f64 / 2.0) * wpp;
        (wx, wy)
    }

    /// World coord -> screen pixel (origin top-left); inverse of [`Self::screen_to_world`].
    pub fn world_to_screen(&self, world: (f64, f64), viewport_px: (f32, f32)) -> (f32, f32) {
        let ppw = 256.0 * 2f64.powf(self.zoom); // pixels per world unit
        let px = (world.0 - self.center.0) * ppw + viewport_px.0 as f64 / 2.0;
        let py = (world.1 - self.center.1) * ppw + viewport_px.1 as f64 / 2.0;
        (px as f32, py as f32)
    }

    /// The `(center, scale)` uniform mapping a world point to clip space:
    /// `clip = (world - center) * scale`, with `scale.y` negated for screen-down Y.
    pub fn world_to_clip_uniform(&self, viewport_px: (f32, f32)) -> ([f32; 2], [f32; 2]) {
        let ppw = 256.0 * 2f64.powf(self.zoom); // pixels per world unit
        let sx = ppw * 2.0 / viewport_px.0 as f64;
        let sy = ppw * 2.0 / viewport_px.1 as f64;
        (
            [self.center.0 as f32, self.center.1 as f32],
            [sx as f32, -sy as f32],
        )
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_map_cannot_be_pushed_off_the_top_of_the_screen() {
        // Zoom 3, 800 px tall: half a viewport is 400 px, and one world is 256 * 2^3 = 2048 px,
        // so the legal band for the center is 0.195..=0.805.
        let vp = (800.0, 800.0);
        let mut c = Camera {
            center: (0.5, 0.5),
            zoom: 3.0,
        };
        c.pan_pixels(0.0, 100_000.0, vp);
        let half = 400.0 / (256.0 * 8.0);
        assert!(
            (c.center.1 - half).abs() < 1e-9,
            "dragged to the top edge and stopped there, got {}",
            c.center.1
        );
        c.pan_pixels(0.0, -100_000.0, vp);
        assert!((c.center.1 - (1.0 - half)).abs() < 1e-9);
    }

    #[test]
    fn zoomed_out_past_the_whole_world_the_world_is_centered() {
        // At zoom 2 a world is 1024 px and the viewport is 2000: there is no legal band left, so
        // there is nothing to choose but the middle.
        let mut c = Camera {
            center: (0.5, 0.1),
            zoom: 2.0,
        };
        c.pan_pixels(0.0, 300.0, (2000.0, 2000.0));
        assert_eq!(c.center.1, 0.5);
    }

    #[test]
    fn zooming_near_the_antimeridian_wraps_like_panning_does() {
        // Longitude is a cylinder and both gestures have to agree about that. Zooming only ever
        // clamped latitude, so a zoom out here left `center.0` above 1.0 until the next drag.
        let vp = (800.0, 800.0);
        let mut c = Camera {
            center: (0.999, 0.5),
            zoom: 8.0,
        };
        c.zoom_at(-1.0, (799.0, 400.0), vp);
        assert!(
            (0.0..1.0).contains(&c.center.0),
            "center.0 left the world: {}",
            c.center.0
        );
    }
    use super::*;

    #[test]
    fn lonlat_world_roundtrip() {
        for &(lon, lat) in &[(-97.28, 35.33), (0.0, 0.0), (139.7, 35.68), (-122.4, 47.6)] {
            let (x, y) = lonlat_to_world(lon, lat);
            let (lon2, lat2) = world_to_lonlat(x, y);
            assert!((lon - lon2).abs() < 1e-9, "lon {lon} -> {lon2}");
            assert!((lat - lat2).abs() < 1e-6, "lat {lat} -> {lat2}");
        }
    }

    #[test]
    fn world_origin_and_center() {
        let (x, y) = lonlat_to_world(-180.0, MAX_LAT);
        assert!(
            x.abs() < 1e-9 && y.abs() < 1e-6,
            "top-left is (0,0), got ({x},{y})"
        );
        let (x, y) = lonlat_to_world(0.0, 0.0);
        assert!(
            (x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9,
            "equator/prime meridian is center"
        );
    }

    #[test]
    fn zoom_at_keeps_cursor_fixed() {
        let mut cam = Camera::at_lonlat(-97.28, 35.33, 7.0);
        let vp = (800.0, 600.0);
        let cursor = (600.0, 200.0);
        let world_before = cam.screen_to_world(cursor, vp);
        cam.zoom_at(1.0, cursor, vp);
        let world_after = cam.screen_to_world(cursor, vp);
        assert!((world_before.0 - world_after.0).abs() < 1e-9);
        assert!((world_before.1 - world_after.1).abs() < 1e-9);
    }
}
