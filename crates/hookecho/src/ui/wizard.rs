//! First-run setup wizard: walks through radar site, map/API keys, theme, sounds, saved
//! locations, and alerts. Shown once (gated on `Settings::setup_done`), re-runnable from Help.

use crate::settings::{Settings, Theme};
use crate::tiles::BasemapStyle;
use crate::ui::marker_window::IconTextures;

const LAST_STEP: usize = 7;

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

/// Show the wizard. Returns `Some(site)` when finished (the chosen home site to load); the caller
/// marks `setup_done`, saves settings, and jumps the view there. `basemap` is the active pane's
/// style (updated live as the user picks); `icon_tex` renders marker-icon thumbnails.
pub fn show(
    ctx: &egui::Context,
    wiz: &mut Wizard,
    settings: &mut Settings,
    basemap: &mut BasemapStyle,
    icon_tex: &IconTextures,
) -> Option<String> {
    if !wiz.open {
        return None;
    }
    let mut finished = None;
    let mut open = true;
    crate::ui::fit_phone(ctx, egui::Window::new("Welcome to Hook Echo-WX"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            // Full width on desktop; on a phone, whatever fits (fit_phone caps the window).
            ui.set_width(420.0_f32.min(ctx.content_rect().width() - 40.0));
            match wiz.step {
                0 => page_welcome(ui),
                1 => page_site(ui, wiz, settings),
                2 => page_map(ui, settings, basemap),
                3 => page_theme(ui, settings),
                4 => {
                    ui.strong("Sounds (5/8)");
                    ui.small("Pick a sound per alert kind, set the volume, and preview with ▶.");
                    ui.add_space(6.0);
                    crate::ui::settings_window::sound_picker(ui, settings);
                }
                5 => {
                    ui.strong("Saved locations (6/8)");
                    ui.small("Add places you care about — proximity alerts and quick jumps use them. You can also drop markers later via Tools ▸ Drop marker.");
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        crate::ui::marker_window::marker_grid(ui, &mut settings.markers, icon_tex);
                    });
                    if ui.button("➕ Add location").clicked() {
                        let n = settings.markers.len() + 1;
                        settings.markers.push(crate::settings::Marker {
                            name: format!("Location {n}"),
                            lat: 0.0,
                            lon: 0.0,
                            icon: None,
                        });
                    }
                }
                6 => page_alerts(ui, settings),
                _ => page_done(ui, settings, *basemap),
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if wiz.step > 0 && ui.button("Back").clicked() {
                    wiz.step -= 1;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if wiz.step < LAST_STEP {
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

fn page_welcome(ui: &mut egui::Ui) {
    ui.strong("Welcome (1/8)");
    ui.add_space(6.0);
    ui.label("This quick setup configures your radar, map, theme, sounds, and saved locations. It takes under a minute — everything here can be changed later in Settings.");
    ui.add_space(4.0);
    ui.small("Press Next to begin.");
}

fn page_site(ui: &mut egui::Ui, wiz: &mut Wizard, settings: &mut Settings) {
    ui.strong("Home radar site (2/8)");
    ui.add_space(6.0);
    ui.add(egui::TextEdit::singleline(&mut wiz.filter).hint_text("Search by ID, city, or state…"));
    let needle = wiz.filter.to_ascii_uppercase();
    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
        for s in wxdata::sites::sites() {
            if !needle.is_empty()
                && !s.id.to_ascii_uppercase().contains(&needle)
                && !s.city.to_ascii_uppercase().contains(&needle)
                && !s.state.to_ascii_uppercase().contains(&needle)
            {
                continue;
            }
            let label = format!("{}  —  {}, {}", s.id, s.city, s.state);
            if ui.selectable_label(settings.default_site == s.id, label).clicked() {
                settings.default_site = s.id.to_string();
            }
        }
    });
}

fn page_map(ui: &mut egui::Ui, settings: &mut Settings, basemap: &mut BasemapStyle) {
    ui.strong("Map & API keys (3/8)");
    ui.small("Optional keys unlock premium basemaps. Free keys: mapbox.com and maptiler.com. Plenty of basemaps work with no key at all.");
    ui.add_space(6.0);
    egui::Grid::new("wiz_keys").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Mapbox token");
        ui.add(egui::TextEdit::singleline(&mut settings.mapbox_key).password(true).desired_width(240.0));
        ui.end_row();
        ui.label("MapTiler key");
        ui.add(egui::TextEdit::singleline(&mut settings.maptiler_key).password(true).desired_width(240.0));
        ui.end_row();
    });
    ui.add_space(6.0);
    // Basemap picker, filtered by whichever keys are set this frame (typing a key unlocks styles).
    let mb = !settings.mapbox_key.is_empty();
    let mt = !settings.maptiler_key.is_empty();
    ui.horizontal(|ui| {
        ui.label("Basemap");
        egui::ComboBox::from_id_salt("wiz_basemap")
            .selected_text(basemap.label())
            .show_ui(ui, |ui| {
                for s in BasemapStyle::ALL {
                    if s.available(mb, mt) && ui.selectable_label(*basemap == s, s.label()).clicked() {
                        *basemap = s;
                        settings.basemap = s.slug().to_string();
                    }
                }
            });
    });
}

fn page_theme(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.strong("Look and feel (4/8)");
    ui.add_space(6.0);
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        for t in Theme::ALL {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 14.0), egui::Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, 3.0, crate::theme::preview_bg(t));
                p.circle_filled(rect.center(), 5.0, crate::theme::accent(t));
                ui.selectable_value(&mut settings.theme, t, t.label());
            });
        }
    });
}

fn page_alerts(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.strong("Alerts (7/8)");
    ui.add_space(6.0);
    ui.checkbox(&mut settings.alert_sound, "Play a sound when a new warning appears");
    ui.checkbox(&mut settings.lightning_alarm, "Alarm on nearby lightning (within ~15 km of a saved location)");
    ui.horizontal(|ui| {
        ui.label("ntfy.sh topic:");
        ui.text_edit_singleline(&mut settings.ntfy_topic);
    });
    ui.small("Optional: push notifications to your phone when a warning covers a saved location. Leave empty to skip.");
}

fn page_done(ui: &mut egui::Ui, settings: &Settings, basemap: BasemapStyle) {
    ui.strong("All set (8/8)");
    ui.add_space(6.0);
    ui.label(format!("Home radar: {}", settings.default_site));
    ui.label(format!("Theme: {}", settings.theme.label()));
    ui.label(format!("Basemap: {}", basemap.label()));
    ui.label(format!("Saved locations: {}", settings.markers.len()));
    ui.label(format!("Alert sound: {}", if settings.alert_sound { "on" } else { "off" }));
    ui.add_space(4.0);
    ui.small("Press Finish to jump to your home radar. Re-run this anytime from Help ▸ Setup wizard.");
}
