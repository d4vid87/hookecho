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
    /// Orthographic sphere instead of the default Mercator plane.
    pub globe: bool,
}

impl Camera {
    pub fn at_lonlat(lon: f64, lat: f64, zoom: f64) -> Self {
        Self {
            center: lonlat_to_world(lon, lat),
            zoom,
            globe: false,
        }
    }

    /// World units covered by one screen pixel.
    pub fn world_per_pixel(&self) -> f64 {
        1.0 / (256.0 * 2f64.powf(self.zoom))
    }

    /// Pan by a screen-pixel delta (drag). Positive dx/dy move content right/down.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32, viewport_px: (f32, f32)) {
        if self.globe {
            let (_, lat) = world_to_lonlat(self.center.0, self.center.1);
            let radius = self.globe_radius();
            let lon = self.center.0 * 360.0 - 180.0 - dx as f64 / radius / lat.to_radians().cos().max(0.05) * 180.0 / PI;
            self.center = lonlat_to_world(lon, (lat + dy as f64 / radius * 180.0 / PI).clamp(-MAX_LAT, MAX_LAT));
            self.settle(viewport_px);
            return;
        }
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
        if self.globe { self.center.1 = self.center.1.clamp(0.0, 1.0); return; }
        let half = viewport_px.1 as f64 / 2.0 * self.world_per_pixel();
        self.center.1 = if half >= 0.5 {
            0.5
        } else {
            self.center.1.clamp(half, 1.0 - half)
        };
    }

    /// Zoom toward a screen point (cursor) so that world point stays under the cursor.
    pub fn zoom_at(&mut self, delta: f64, cursor_px: (f32, f32), viewport_px: (f32, f32)) {
        if self.globe {
            // Iterate the geographic anchor because the spherical projection is nonlinear.
            let anchor = self.pick_world(cursor_px, viewport_px);
            self.zoom = (self.zoom + delta).clamp(-2.0, 20.0);
            let distance = (cursor_px.0 as f64 - viewport_px.0 as f64 / 2.0).hypot(cursor_px.1 as f64 - viewport_px.1 as f64 / 2.0);
            // Once the cursor lies in sky, no rotation can keep a surface point
            // underneath it. Keep the globe centered rather than spinning it.
            if distance > self.globe_radius() { return; }
            if let Some(world) = anchor {
                for _ in 0..8 {
                    let at = self.world_to_screen(world, viewport_px);
                    if !at.0.is_finite() { break; }
                    self.pan_pixels(cursor_px.0 - at.0, cursor_px.1 - at.1, viewport_px);
                }
            }
            return;
        }
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
        if self.globe { return self.pick_world(px, viewport_px).unwrap_or((f64::NAN, f64::NAN)); }
        let wpp = self.world_per_pixel();
        let wx = self.center.0 + (px.0 as f64 - viewport_px.0 as f64 / 2.0) * wpp;
        let wy = self.center.1 + (px.1 as f64 - viewport_px.1 as f64 / 2.0) * wpp;
        (wx, wy)
    }

    /// World coord -> screen pixel (origin top-left); inverse of [`Self::screen_to_world`].
    pub fn world_to_screen(&self, world: (f64, f64), viewport_px: (f32, f32)) -> (f32, f32) {
        if self.globe {
            let (lon, lat) = world_to_lonlat(world.0, world.1);
            let (lon0, lat0) = world_to_lonlat(self.center.0, self.center.1);
            let (lat, lat0, dl) = (lat.to_radians(), lat0.to_radians(), (lon - lon0).to_radians());
            let depth = lat0.sin() * lat.sin() + lat0.cos() * lat.cos() * dl.cos();
            if depth < 0.0 { return (-1.0e9, -1.0e9); }
            let radius = self.globe_radius();
            return ((viewport_px.0 as f64 / 2.0 + radius * lat.cos() * dl.sin()) as f32,
                (viewport_px.1 as f64 / 2.0 - radius * (lat0.cos() * lat.sin() - lat0.sin() * lat.cos() * dl.cos())) as f32);
        }
        let ppw = 256.0 * 2f64.powf(self.zoom); // pixels per world unit
        let px = (world.0 - self.center.0) * ppw + viewport_px.0 as f64 / 2.0;
        let py = (world.1 - self.center.1) * ppw + viewport_px.1 as f64 / 2.0;
        (px as f32, py as f32)
    }

    pub fn globe_radius(&self) -> f64 {
        let (_, lat) = world_to_lonlat(self.center.0, self.center.1);
        256.0 * 2f64.powf(self.zoom) / (lat.to_radians().cos().max(0.05) * 2.0 * PI)
    }

    pub fn world_visible(&self, world: (f64, f64)) -> bool {
        if !self.globe { return true; }
        let (lon, lat) = world_to_lonlat(world.0, world.1);
        let (lon0, lat0) = world_to_lonlat(self.center.0, self.center.1);
        let (lat, lat0, dl) = (lat.to_radians(), lat0.to_radians(), (lon - lon0).to_radians());
        lat0.sin() * lat.sin() + lat0.cos() * lat.cos() * dl.cos() >= 0.0
    }

    /// Clip a short geographic edge to the visible hemisphere before projecting.
    pub fn project_segment(&self, mut a: (f64, f64), mut b: (f64, f64), vp: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
        if self.globe { b.0 = a.0 + (b.0 - a.0 + 0.5).rem_euclid(1.0) - 0.5; }
        let (va, vb) = (self.world_visible(a), self.world_visible(b));
        if !va && !vb { return None; }
        if va != vb {
            let (mut front, mut back) = if va { (a, b) } else { (b, a) };
            for _ in 0..32 {
                let mid = ((front.0 + back.0) / 2.0, (front.1 + back.1) / 2.0);
                if self.world_visible(mid) { front = mid; } else { back = mid; }
            }
            if va { b = front; } else { a = front; }
        }
        Some((self.world_to_screen(a, vp), self.world_to_screen(b, vp)))
    }

    /// Reject sky clicks instead of assigning a fabricated geographic location.
    pub fn pick_world(&self, px: (f32, f32), vp: (f32, f32)) -> Option<(f64, f64)> {
        if !self.globe { return Some(self.screen_to_world(px, vp)); }
        let radius = self.globe_radius();
        let x = (px.0 as f64 - vp.0 as f64 / 2.0) / radius;
        let y = (vp.1 as f64 / 2.0 - px.1 as f64) / radius;
        let rho = x.hypot(y);
        if rho > 1.0 { return None; }
        if rho < 1e-12 { return Some(self.center); }
        let (lon0, lat0) = world_to_lonlat(self.center.0, self.center.1);
        let (lon0, lat0) = (lon0.to_radians(), lat0.to_radians());
        let z = (1.0 - rho * rho).sqrt();
        let lat = (z * lat0.sin() + y * lat0.cos()).clamp(-1.0, 1.0).asin();
        let lon = lon0 + x.atan2(z * lat0.cos() - y * lat0.sin());
        if lat.abs().to_degrees() > MAX_LAT { return None; }
        let mut w = lonlat_to_world(lon.to_degrees(), lat.to_degrees());
        w.0 = w.0.rem_euclid(1.0);
        Some(w)
    }

    /// Conservative world bounds for fetching tiles/feeds, separate from picking.
    pub fn visible_bounds(&self, vp: (f32, f32)) -> ((f64, f64), (f64, f64)) {
        if !self.globe {
            return (self.screen_to_world((0.0, 0.0), vp), self.screen_to_world(vp, vp));
        }
        let radius = self.globe_radius();
        let corner = ((vp.0 as f64 / 2.0).hypot(vp.1 as f64 / 2.0) / radius).min(1.0);
        let angle = corner.asin();
        let (lon, lat) = world_to_lonlat(self.center.0, self.center.1);
        let lat = lat.to_radians();
        let north = (lat + angle).min(MAX_LAT.to_radians());
        let south = (lat - angle).max(-MAX_LAT.to_radians());
        let dl = if angle >= PI / 2.0 - lat.abs() { PI }
            else { (angle.sin() / lat.cos()).clamp(-1.0, 1.0).asin() };
        (lonlat_to_world(lon - dl.to_degrees(), north.to_degrees()),
         lonlat_to_world(lon + dl.to_degrees(), south.to_degrees()))
    }

    /// sin/cos center latitude and sphere radius in clip units; zeros mean Mercator.
    pub fn globe_uniform(&self, vp: (f32, f32)) -> [f32; 4] {
        if !self.globe { return [0.0; 4]; }
        let (_, lat) = world_to_lonlat(self.center.0, self.center.1);
        let lat = lat.to_radians();
        let r = self.globe_radius();
        [lat.sin() as f32, lat.cos() as f32, (2.0 * r / vp.0 as f64) as f32, (2.0 * r / vp.1 as f64) as f32]
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
            globe: false,
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
            globe: false,
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
            globe: false,
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
    fn globe_picking_roundtrips_and_rejects_sky_and_far_hemisphere() {
        let vp = (800.0, 600.0);
        for (lon, lat) in [(0.0, 0.0), (179.0, 35.0), (-97.0, 70.0)] {
            let mut camera = Camera::at_lonlat(lon, lat, 2.0);
            camera.globe = true;
            for (dx, dy) in [(0.0, 0.0), (15.0, 10.0), (-20.0, -5.0)] {
                let world = lonlat_to_world(lon + dx, lat + dy);
                let px = camera.world_to_screen(world, vp);
                let picked = camera.pick_world(px, vp).unwrap();
                assert!(((picked.0 - world.0 + 0.5).rem_euclid(1.0) - 0.5).abs() < 1e-6);
                assert!((picked.1 - world.1).abs() < 1e-6);
            }
            assert!(camera.pick_world((0.0, 0.0), vp).is_none());
            let far = camera.world_to_screen(lonlat_to_world(lon + 180.0, -lat), vp);
            assert!(far.0 < -1000.0);
            let (min, max) = camera.visible_bounds(vp);
            assert!(min.0 < max.0 && min.1 < max.1);
        }
    }

    #[test]
    fn globe_zooming_out_to_sky_does_not_spin_the_camera() {
        let mut camera = Camera::at_lonlat(0.0, 0.0, 2.0);
        camera.globe = true;
        let center = camera.center;
        assert!(camera.pick_world((540.0,300.0),(800.0,600.0)).is_some());
        camera.zoom_at(-2.0,(540.0,300.0),(800.0,600.0));
        assert_eq!(camera.center,center);
        assert!(camera.pick_world((540.0,300.0),(800.0,600.0)).is_none());
    }

    #[test]
    fn switching_projection_preserves_the_local_map_scale() {
        for lat in [0.0, 35.0, 80.0] {
            let mut camera = Camera::at_lonlat(-97.0, lat, 8.0);
            let point = lonlat_to_world(-96.999, lat + 0.001);
            let flat = camera.world_to_screen(point, (800.0,600.0));
            camera.globe = true;
            let globe = camera.world_to_screen(point, (800.0,600.0));
            assert!((flat.0 - globe.0).abs() < 0.01);
            assert!((flat.1 - globe.1).abs() < 0.01);
        }
    }

    #[test]
    fn globe_clips_vector_edges_at_the_horizon() {
        let mut camera = Camera::at_lonlat(0.0, 0.0, 2.0);
        camera.globe = true;
        let vp = (800.0, 600.0);
        let segment = camera.project_segment(lonlat_to_world(45.0, 0.0), lonlat_to_world(120.0, 0.0), vp).unwrap();
        assert!((segment.1.0 as f64 - vp.0 as f64 / 2.0 - camera.globe_radius()).abs() < 0.001);
        assert!(camera.project_segment(lonlat_to_world(120.0, 0.0), lonlat_to_world(150.0, 0.0), vp).is_none());
    }

    #[test]
    fn globe_zoom_preserves_geographic_cursor_anchor() {
        let mut camera = Camera::at_lonlat(179.0, 35.0, 7.0);
        camera.globe = true;
        let vp = (800.0, 600.0);
        let cursor = (550.0, 200.0);
        let before = camera.pick_world(cursor, vp).unwrap();
        camera.zoom_at(1.0, cursor, vp);
        let after = camera.pick_world(cursor, vp).unwrap();
        assert!(((before.0 - after.0 + 0.5).rem_euclid(1.0) - 0.5).abs() < 1e-6, "{before:?} -> {after:?}");
        assert!((before.1 - after.1).abs() < 1e-6);
    }

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
