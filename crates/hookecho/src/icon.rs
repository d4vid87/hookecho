//! The app logo: the HookEcho mark, bundled as one PNG and resampled to whatever size a caller
//! wants (window icon, tray, About page, `.ico` frames, hicolor theme export).
//!
//! It used to be drawn procedurally — six blobs and a distance field — which scaled to any size
//! for free but was never going to be the brand. The artwork replaces it. The bundled copy is a
//! 256 px palette PNG (~24 KB): 256 is the largest size anything asks for, and the quantise keeps
//! it cheap enough to ride in the wasm as well as the binary. Regenerate it from the master with
//! `scripts/brand/icons.sh`.

use std::sync::OnceLock;

/// The 256 px master, decoded once per process.
///
/// ponytail: decoded eagerly on first use and kept for the process — it is ~256 KB of RGBA, and
/// the alternative (decode per call) would re-run the PNG decoder for every tray repaint.
fn master() -> &'static image::RgbaImage {
    static MASTER: OnceLock<image::RgbaImage> = OnceLock::new();
    MASTER.get_or_init(|| {
        image::load_from_memory(include_bytes!("../data/logo.png"))
            .expect("bundled logo.png decodes")
            .to_rgba8()
    })
}

/// Render the logo as `size × size` RGBA8 (row-major, straight alpha).
pub fn rgba(size: usize) -> Vec<u8> {
    let size = size.max(1) as u32;
    let m = master();
    if m.width() == size && m.height() == size {
        return m.as_raw().clone();
    }
    // Lanczos3 down, which is what the small launcher sizes are; it is also fine on the rare
    // upscale past 256 (`--headless-icon 512`), just soft.
    image::imageops::resize(m, size, size, image::imageops::FilterType::Lanczos3).into_raw()
}

/// The 64px window icon.
pub fn icon_data() -> egui::IconData {
    let size = 64;
    egui::IconData {
        rgba: rgba(size),
        width: size as u32,
        height: size as u32,
    }
}

/// The logo as a texture, drawn once and kept for the session.
///
/// The handle lives in egui's own temp store keyed by size, which is already the right lifetime:
/// it dies with the context.
pub fn texture(ctx: &egui::Context, size: usize) -> egui::TextureHandle {
    let id = egui::Id::new(("app_logo_tex", size));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([size, size], &rgba(size));
    let t = ctx.load_texture("app_logo", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_mark_at_any_size() {
        for s in [16usize, 64, 256] {
            let px = rgba(s);
            assert_eq!(px.len(), s * s * 4, "{s}px buffer is RGBA8");
            // The artwork is a badge on transparency: the very corner is outside it.
            assert_eq!(px[3], 0, "{s}px top-left corner is transparent");
            // And something in the middle is opaque and colored, i.e. the decode produced the
            // mark rather than an empty buffer.
            let mid = ((s / 2) * s + s / 2) * 4;
            assert_eq!(px[mid + 3], 255, "{s}px centre is opaque");
        }
    }
}
