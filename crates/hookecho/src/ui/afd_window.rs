//! Area Forecast Discussion window: the WFO's own forecast reasoning, as monospace text.

use wxdata::afd::Afd;

/// Show the AFD window. Returns `true` when the user asked for a refresh.
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    afd: Option<&Afd>,
    busy: bool,
    error: Option<&str>,
) -> bool {
    let mut refresh = false;
    crate::ui::fit_phone(ctx, egui::Window::new("Forecast Discussion"))
        .open(open)
        .default_size([560.0, 520.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                match afd {
                    Some(a) => {
                        ui.strong(format!("AFD {}", a.office));
                        ui.weak(&a.issued);
                    }
                    None => {
                        ui.strong("AFD");
                    }
                }
                if busy {
                    ui.spinner();
                }
                if ui.button("⟳ Refresh").clicked() {
                    refresh = true;
                }
            });
            if let Some(e) = error {
                ui.colored_label(egui::Color32::from_rgb(230, 90, 90), e);
            }
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                match afd {
                    Some(a) => {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&a.text).monospace().size(12.0))
                                .wrap(),
                        );
                    }
                    None if busy => {
                        ui.weak("Fetching the discussion…");
                    }
                    None => {
                        ui.weak("No discussion loaded.");
                    }
                }
            });
        });
    refresh
}
