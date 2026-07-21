//! CAPPI window: a constant-altitude horizontal reflectivity slice re-sliced from the cached
//! volume tilts, with an altitude slider. Window-only (no on-map layer) in this first version.

use crate::colormap::ColorTable;
use wxdata::volume3d::Cappi;

/// Turn a CAPPI slice into an egui image (row 0 = north), colored via `table`.
pub fn to_image(c: &Cappi, table: &ColorTable) -> egui::ColorImage {
    let mut buf = vec![0u8; c.n * c.n * 4];
    for i in 0..c.n * c.n {
        let rgba = c.dbz[i].and_then(|v| table.sample(v)).unwrap_or([18, 18, 18, 255]);
        buf[i * 4..i * 4 + 4].copy_from_slice(&rgba);
    }
    egui::ColorImage::from_rgba_unmultiplied([c.n, c.n], &buf)
}

/// Show the CAPPI window. `alt_km` is edited by the slider; `length` is the box width (km).
/// Returns `false` when the window should close.
pub fn show(ctx: &egui::Context, tex: &egui::TextureHandle, alt_km: &mut f32, length: f32) -> bool {
    let mut open = true;
    crate::ui::fit_phone(ctx, egui::Window::new("CAPPI slice"))
        .open(&mut open)
        .default_size([420.0, 480.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Altitude");
                ui.add(egui::Slider::new(alt_km, 0.5..=15.0).suffix(" km"));
            });
            ui.label(format!("{length:.0} km across · reflectivity · north up"));
            ui.separator();
            let w = ui.available_size().x.clamp(200.0, 420.0);
            let img = egui::Image::new(tex)
                .fit_to_exact_size(egui::vec2(w, w))
                .texture_options(egui::TextureOptions::NEAREST);
            ui.add(img);
            ui.weak("Constant-altitude PPI; gaps = no beam coverage at this height.");
        });
    open
}
