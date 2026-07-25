//! The Layer Manager: per-placefile enable, paint order, and opacity.
//!
//! `settings.placefiles` order *is* the paint order (see `visible_placefile_items`), so the
//! ↑/↓ buttons here reorder the list directly. Field layers keep their fixed `DRAW_ORDER`;
//! `// ponytail: per-field opacity means re-baking the color LUTs — placefiles first.`

use crate::settings::Settings;

/// Show the window. Returns `true` if anything changed (the caller bumps the overlay generation
/// so the tessellated geometry rebuilds).
pub(crate) fn show(ctx: &egui::Context, open: &mut bool, settings: &mut Settings) -> bool {
    if !*open {
        return false;
    }
    let mut changed = false;
    let mut win_open = *open;
    crate::ui::fit_phone(ctx, egui::Window::new("Layer Manager"))
        .open(&mut win_open)
        .default_width(420.0)
        .show(ctx, |ui| {
            if settings.placefiles.is_empty() {
                ui.weak("No placefiles configured — add one in Tools ▸ Placefile Manager.");
                return;
            }
            ui.weak("Top of the list paints first (underneath).");
            ui.separator();
            let n = settings.placefiles.len();
            let mut swap: Option<(usize, usize)> = None;
            for i in 0..n {
                let cfg = &mut settings.placefiles[i];
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut cfg.enabled, "").changed();
                    // The URL's file name is the readable part; the full URL is the tooltip.
                    let name = cfg.url.rsplit('/').next().unwrap_or(&cfg.url).to_string();
                    ui.label(egui::RichText::new(name).strong()).on_hover_text(&cfg.url);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(i + 1 < n, egui::Button::new("▼")).on_hover_text("Paint later (on top)").clicked() {
                            swap = Some((i, i + 1));
                        }
                        if ui.add_enabled(i > 0, egui::Button::new("▲")).on_hover_text("Paint earlier (underneath)").clicked() {
                            swap = Some((i, i - 1));
                        }
                        changed |= ui
                            .add(egui::Slider::new(&mut cfg.opacity, 0.05..=1.0).show_value(false))
                            .on_hover_text(format!("Opacity {:.0}%", cfg.opacity * 100.0))
                            .changed();
                    });
                });
            }
            if let Some((a, b)) = swap {
                settings.placefiles.swap(a, b);
                changed = true;
            }
        });
    *open = win_open;
    changed
}
