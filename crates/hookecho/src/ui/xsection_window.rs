//! Vertical cross-section window: a distance×height reflectivity panel reconstructed from the
//! volume's stacked tilts, colored with the reflectivity palette.

use crate::colormap::ColorTable;
use wxdata::xsection::CrossSection;

/// Turn a cross-section into an egui image (row 0 = top of panel), colored via `table`.
pub fn to_image(xs: &CrossSection, table: &ColorTable) -> egui::ColorImage {
    let mut buf = vec![0u8; xs.cols * xs.rows * 4];
    for r in 0..xs.rows {
        for c in 0..xs.cols {
            let rgba = xs.at(c, r).and_then(|v| table.sample(v)).unwrap_or([18, 18, 18, 255]);
            buf[(r * xs.cols + c) * 4..(r * xs.cols + c) * 4 + 4].copy_from_slice(&rgba);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([xs.cols, xs.rows], &buf)
}

/// Show the cross-section window. Returns `false` when it should close.
pub fn show(ctx: &egui::Context, xs: &CrossSection, tex: &egui::TextureHandle) -> bool {
    let mut open = true;
    egui::Window::new("Cross-section")
        .open(&mut open)
        .default_size([560.0, 300.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "Length {:.0} km · top {:.0} km · reflectivity",
                xs.length_km, xs.max_height_km
            ));
            ui.separator();
            // Draw the panel stretched to a readable size (distance wide, height tall).
            let avail = ui.available_size();
            let w = avail.x.max(200.0);
            let h = (w * 0.4).clamp(120.0, 260.0);
            let img = egui::Image::new(tex)
                .fit_to_exact_size(egui::vec2(w, h))
                .texture_options(egui::TextureOptions::LINEAR);
            let resp = ui.add(img);
            // Axis captions along the drawn rect.
            let rect = resp.rect;
            let cap = |ui: &egui::Ui, pos, anchor, txt: &str| {
                ui.painter().text(pos, anchor, txt, egui::FontId::proportional(10.0), egui::Color32::from_gray(200));
            };
            cap(ui, rect.left_top() + egui::vec2(2.0, 2.0), egui::Align2::LEFT_TOP, &format!("{:.0} km", xs.max_height_km));
            cap(ui, rect.left_bottom() + egui::vec2(2.0, -2.0), egui::Align2::LEFT_BOTTOM, "0 km");
            cap(ui, rect.left_bottom() + egui::vec2(2.0, -14.0), egui::Align2::LEFT_BOTTOM, "A");
            cap(ui, rect.right_bottom() + egui::vec2(-2.0, -14.0), egui::Align2::RIGHT_BOTTOM, "B");
            ui.weak("A→B left to right; height increases upward. Gaps = no beam coverage.");
        });
    open
}
