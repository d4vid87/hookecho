//! NOAA SPC forecaster reasoning for the selected fire-weather outlook.

use wxdata::fire_weather::Discussion;

#[derive(Default)]
pub struct FireWeatherDiscussionWindow {
    pub open: bool,
    pub busy: bool,
    pub day: u8,
    pub discussion: Option<Discussion>,
    pub error: Option<String>,
}

impl FireWeatherDiscussionWindow {
    /// Returns true when the user requests the latest NOAA text again.
    pub fn show(&mut self, ctx: &egui::Context, drawer: &mut crate::ui::drawer::Drawer) -> bool {
        let Some(window) = drawer.page_sized(
            ctx, "Fire Weather Forecast Discussion", &mut self.open, false, 620.0,
            egui::Window::new("Fire Weather Forecast Discussion"),
        ) else { return false };
        let mut refresh = false;
        window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("NOAA SPC · Day {}", self.day));
                if self.busy { crate::ui::loading(ui, "Fetching discussion…"); }
                if ui.button("⟳ Refresh").clicked() { refresh = true; }
                if let Some(discussion) = &self.discussion {
                    ui.hyperlink_to("NOAA source", discussion.source_url);
                    crate::ui::reader_button(ui, "SPC fire-weather outlook", &format!("Day {}", discussion.day), &discussion.text);
                }
            });
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
