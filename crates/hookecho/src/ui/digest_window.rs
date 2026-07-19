//! Plain-language storm digest window: a "what does this mean for me" briefing of the in-view
//! weather. The app fills `text` (templated instantly; Claude-enhanced if a key is set).

#[derive(Default)]
pub struct DigestWindow {
    pub open: bool,
    pub text: String,
    pub busy: bool,
    /// True once a Claude-enhanced version replaced the templated text.
    pub enhanced: bool,
}

pub enum DigestAction {
    Generate,
}

impl DigestWindow {
    pub fn show(&mut self, ctx: &egui::Context) -> Option<DigestAction> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut action = None;
        egui::Window::new("Storm Digest")
            .open(&mut open)
            .default_size([420.0, 240.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("↻ Generate")).clicked() {
                        action = Some(DigestAction::Generate);
                    }
                    if self.busy {
                        ui.spinner();
                        ui.weak("asking Claude…");
                    } else if self.enhanced {
                        ui.weak("· enhanced by Claude");
                    }
                });
                ui.separator();
                if self.text.is_empty() {
                    ui.weak("Click Generate for a plain-language briefing of the in-view weather.");
                } else {
                    ui.label(egui::RichText::new(&self.text).size(14.0));
                }
                ui.add_space(6.0);
                ui.weak("Set an Anthropic key in Settings ▸ Audio for friendlier prose; otherwise a built-in summary is used.");
            });
        self.open = open;
        action
    }
}
