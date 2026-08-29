//! Vertical cross-section window: a distance×height reflectivity panel reconstructed from the
//! volume's stacked tilts, colored with the reflectivity palette.

use crate::colormap::ColorTable;
use wxdata::xsection::CrossSection;

/// Turn a cross-section into an egui image (row 0 = top of panel), colored via `table`.
pub fn to_image(xs: &CrossSection, table: &ColorTable) -> egui::ColorImage {
    let mut buf = vec![0u8; xs.cols * xs.rows * 4];
    for r in 0..xs.rows {
        for c in 0..xs.cols {
            let rgba = xs
                .at(c, r)
                .and_then(|v| table.sample(v))
                .unwrap_or([18, 18, 18, 255]);
            buf[(r * xs.cols + c) * 4..(r * xs.cols + c) * 4 + 4].copy_from_slice(&rgba);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([xs.cols, xs.rows], &buf)
}

/// Show the cross-section window. Returns `false` when it should close.
pub fn show(
    ctx: &egui::Context,
    xs: &CrossSection,
    tex: &egui::TextureHandle,
    moment: &mut wxdata::level2::Moment,
    drawer: &mut crate::ui::drawer::Drawer,
) -> bool {
    use wxdata::level2::Moment;
    let mut open = true;
    let mut changed = false;
    let Some(window) = drawer.page_sized(
        ctx,
        "Cross-section",
        &mut open,
        false,
        560.0,
        egui::Window::new("Cross-section"),
    ) else {
        return open;
    };
    window.show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(format!(
                "Length {:.0} km · top {:.0} km",
                xs.length_km, xs.max_height_km
            ));
            ui.separator();
            // Velocity shows how a couplet leans with height; CC shows how deep the debris
            // ball really is. Both were unreachable while this was hard-wired to REF.
            for m in [
                Moment::Reflectivity,
                Moment::Velocity,
                Moment::CorrelationCoefficient,
            ] {
                let info = crate::products::info(m);
                if ui
                    .selectable_label(*moment == m, info.short)
                    .on_hover_text(info.name)
                    .clicked()
                    && *moment != m
                {
                    *moment = m;
                    changed = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                crate::ui::csv_buttons(
                    ui,
                    "xsection.csv",
                    "The sampled grid, top row first",
                    || xs.to_csv(),
                );
            });
        });
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
            ui.painter().text(
                pos,
                anchor,
                txt,
                egui::FontId::proportional(10.0),
                egui::Color32::from_gray(200),
            );
        };
        cap(
            ui,
            rect.left_top() + egui::vec2(2.0, 2.0),
            egui::Align2::LEFT_TOP,
            &format!("{:.0} km", xs.max_height_km),
        );
        cap(
            ui,
            rect.left_bottom() + egui::vec2(2.0, -2.0),
            egui::Align2::LEFT_BOTTOM,
            "0 km",
        );
        cap(
            ui,
            rect.left_bottom() + egui::vec2(2.0, -14.0),
            egui::Align2::LEFT_BOTTOM,
            "A",
        );
        cap(
            ui,
            rect.right_bottom() + egui::vec2(-2.0, -14.0),
            egui::Align2::RIGHT_BOTTOM,
            "B",
        );
        ui.weak("A→B left to right; height increases upward. Gaps = no beam coverage.");
    });
    if changed {
        // Force a rebuild on the next frame with the new moment.
        ctx.request_repaint();
    }
    open
}
