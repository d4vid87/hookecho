//! Procedural app logo: a hook-echo reflectivity signature on a dark radar scope.
//!
//! Drawn in code (no bundled image asset) so it scales to any size and stays in the binary.
//! `rgba(size)` returns straight RGBA8; `icon_data()` wraps the 64px version for the window.

/// Render the logo as `size × size` RGBA8 (row-major, straight alpha).
pub fn rgba(size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size * size * 4];
    let s = size as f32;
    let c = s * 0.5;
    let r_scope = s * 0.47;

    // Hook-echo core: a short arc of blob centers curling from center to lower-left, so the
    // filled union reads as a supercell reflectivity max with a hook appendage. Points are in
    // scope-radius units from center; distance-field union of soft disks colors by intensity.
    let arc: [(f32, f32, f32); 6] = [
        (0.10, -0.05, 0.30), // core (upper-right of the hook)
        (0.02, 0.08, 0.26),
        (-0.06, 0.20, 0.22),
        (-0.16, 0.28, 0.18), // curling down-left
        (-0.28, 0.26, 0.15),
        (-0.34, 0.14, 0.12), // hook tip curling back up
    ];

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - c;
            let dy = py - c;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = (y * size + x) * 4;

            if dist > r_scope + 1.0 {
                continue; // transparent outside the disc
            }

            // Base scope: dark navy disc, brighter accent rim.
            let mut col = [10u8, 18, 34, 255];
            if dist > r_scope - s * 0.03 {
                col = [77, 163, 255, 255]; // accent rim (#4da3ff)
            } else {
                // Faint green range rings + crosshair.
                let ring1 = (dist - r_scope * 0.33).abs() < s * 0.008;
                let ring2 = (dist - r_scope * 0.66).abs() < s * 0.008;
                let cross = dx.abs() < s * 0.006 || dy.abs() < s * 0.006;
                if ring1 || ring2 || cross {
                    col = [30, 70, 50, 255];
                }
            }

            // Hook-echo blobs: nearest blob's soft coverage picks a green→yellow→red color.
            let mut best = f32::MAX;
            for (bx, by, br) in arc {
                let cx = c + bx * r_scope;
                let cy = c + by * r_scope;
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() / (br * r_scope);
                if d < best {
                    best = d;
                }
            }
            if best < 1.0 {
                col = dbz_color(best);
            }

            buf[idx..idx + 4].copy_from_slice(&col);
        }
    }
    buf
}

/// Reflectivity ramp for the hook: `t` is normalized distance from a blob center (0 = core).
fn dbz_color(t: f32) -> [u8; 4] {
    // 0.0 red core → 0.4 orange → 0.7 yellow → 1.0 green edge.
    let (r, g, b) = if t < 0.4 {
        let k = t / 0.4;
        (230, (40.0 + k * 120.0) as u8, 20)
    } else if t < 0.7 {
        let k = (t - 0.4) / 0.3;
        ((230.0 - k * 20.0) as u8, (160.0 + k * 80.0) as u8, 20)
    } else {
        let k = (t - 0.7) / 0.3;
        ((210.0 - k * 160.0) as u8, 230, (20.0 + k * 40.0) as u8)
    };
    [r, g, b, 255]
}

/// The 64px window icon.
pub fn icon_data() -> egui::IconData {
    let size = 64;
    egui::IconData { rgba: rgba(size), width: size as u32, height: size as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_red_core_and_clear_corners() {
        let s = 64;
        let px = rgba(s);
        // Corner pixel is outside the disc → fully transparent.
        assert_eq!(px[3], 0, "top-left corner transparent");
        // Somewhere a strongly-red (hook core) pixel exists.
        let red = px
            .chunks_exact(4)
            .any(|p| p[3] == 255 && p[0] > 180 && p[1] < 120 && p[2] < 80);
        assert!(red, "logo has a red reflectivity core");
    }
}
