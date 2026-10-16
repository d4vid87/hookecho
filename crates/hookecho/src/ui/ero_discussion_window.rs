//! NOAA WPC forecaster reasoning for the selected excessive rainfall outlook.

use wxdata::ero::Discussion;

#[derive(Default)]
pub struct EroDiscussionWindow {
    pub open: bool,
    pub busy: bool,
    pub day: u8,
    pub discussion: Option<Discussion>,
    pub error: Option<String>,
}

impl EroDiscussionWindow {
    /// Returns true when the user requests the latest NOAA text again.
    pub fn show(&mut self, ctx: &egui::Context, drawer: &mut crate::ui::drawer::Drawer) -> bool {
        let Some(window) = drawer.page_sized(
            ctx, "Rainfall Outlook Discussion", &mut self.open, false, 620.0,
            egui::Window::new("Rainfall Outlook Discussion"),
        ) else { return false };
        let mut refresh = false;
        window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("NOAA WPC · Day {}", self.day));
                if self.busy { crate::ui::loading(ui, "Fetching discussion…"); }
                if ui.button("⟳ Refresh").clicked() { refresh = true; }
                if let Some(discussion) = &self.discussion {
                    ui.hyperlink_to("NOAA source", discussion.source_url);
                    crate::ui::reader_button(ui, "WPC rainfall outlook", &format!("Day {}", discussion.day), &discussion.text);
                }
            });
            if self.day >= 4 {
                ui.weak("Days 4–5 use WPC's Extended Forecast Discussion, which covers more than excessive rainfall.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::from_rgb(230, 90, 90), error);
            }
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                match &self.discussion {
                    Some(discussion) => { ui.add(egui::Label::new(egui::RichText::new(&discussion.text).monospace().size(12.0)).wrap()); }
                    None if self.busy => { ui.weak("Fetching NOAA's forecast reasoning…"); }
                    None => { ui.weak("No discussion loaded."); }
                }
            });
        });
        refresh
    }
}
