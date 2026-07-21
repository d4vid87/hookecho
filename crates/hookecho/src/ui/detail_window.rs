//! Detail window shown when a map overlay feature (warning, watch, outlook, MD) is clicked.

/// The currently-open detail popup.
pub struct Detail {
    pub title: String,
    pub body: String,
    pub color: [u8; 4],
}

/// Show the detail window. Returns `false` when it should close.
pub fn show(ctx: &egui::Context, detail: &Detail) -> bool {
    let mut open = true;
    crate::ui::fit_phone(ctx, egui::Window::new("Feature Details"))
        .open(&mut open)
        .default_size([380.0, 320.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let c = detail.color;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    egui::Color32::from_rgb(c[0], c[1], c[2]),
                );
                ui.heading(&detail.title);
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Monospace keeps L3 attribute-table columns aligned.
                ui.add(egui::Label::new(egui::RichText::new(&detail.body).monospace()).wrap());
            });
        });
    open
}
