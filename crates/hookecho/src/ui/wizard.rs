//! First-run setup wizard: home radar site, theme, and alert preferences in three short
//! steps. Shown once (gated on `Settings::setup_done`), re-runnable from the Help menu.

use crate::settings::{Settings, Theme};

#[derive(Default)]
pub struct Wizard {
    pub open: bool,
    step: usize,
    filter: String,
}

impl Wizard {
    pub fn start(&mut self) {
        self.open = true;
        self.step = 0;
        self.filter.clear();
    }
}

/// Show the wizard. Returns `Some(site)` when finished (the chosen home site to load);
/// the caller marks `setup_done`, saves settings, and jumps the view there.
pub fn show(ctx: &egui::Context, wiz: &mut Wizard, settings: &mut Settings) -> Option<String> {
    if !wiz.open {
        return None;
    }
    let mut finished = None;
    let mut open = true;
    egui::Window::new("Welcome to Hook Echo-WX")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(360.0);
            match wiz.step {
                0 => {
                    ui.label("Let's get set up (1/3): pick your home radar site.");
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut wiz.filter)
                            .hint_text("Search by ID, city, or state…"),
                    );
                    let needle = wiz.filter.to_ascii_uppercase();
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for s in wxdata::sites::sites() {
                            if !needle.is_empty()
                                && !s.id.to_ascii_uppercase().contains(&needle)
                                && !s.city.to_ascii_uppercase().contains(&needle)
                                && !s.state.to_ascii_uppercase().contains(&needle)
                            {
                                continue;
                            }
                            let label = format!("{}  —  {}, {}", s.id, s.city, s.state);
                            if ui
                                .selectable_label(settings.default_site == s.id, label)
                                .clicked()
                            {
                                settings.default_site = s.id.to_string();
                            }
                        }
                    });
                }
                1 => {
                    ui.label("Look and feel (2/3): pick a theme. Change anytime in Settings.");
                    ui.add_space(6.0);
                    for t in Theme::ALL {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(28.0, 14.0), egui::Sense::hover());
                            let p = ui.painter();
                            p.rect_filled(rect, 3.0, crate::theme::preview_bg(t));
                            p.circle_filled(rect.center(), 5.0, crate::theme::accent(t));
                            ui.selectable_value(&mut settings.theme, t, t.label());
                        });
                    }
                }
                _ => {
                    ui.label("Alerts (3/3): how should warnings reach you?");
                    ui.add_space(6.0);
                    ui.checkbox(&mut settings.alert_sound, "Chime when a new warning appears");
                    ui.horizontal(|ui| {
                        ui.label("ntfy.sh topic:");
                        ui.text_edit_singleline(&mut settings.ntfy_topic);
                    });
                    ui.small("Optional: push notifications to your phone when a warning covers a saved location. Leave empty to skip.");
                }
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if wiz.step > 0 && ui.button("Back").clicked() {
                    wiz.step -= 1;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if wiz.step < 2 {
                        if ui.button("Next").clicked() {
                            wiz.step += 1;
                        }
                    } else if ui.button("Finish").clicked() {
                        finished = Some(settings.default_site.clone());
                    }
                });
            });
        });
    if finished.is_some() || !open {
        wiz.open = false;
    }
    finished
}
