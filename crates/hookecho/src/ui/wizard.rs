//! First-run setup: four cards — home radar site, map and look, alerts and places, and the
//! hand-off. Shown once (gated on `Settings::setup_done`), re-runnable from the sidebar's App
//! section, the command palette, and Settings → General.
//!
//! It used to be ten pages, two of which were walls of text describing controls the app no longer
//! has. Teaching the chrome is the tour's job now ([`crate::ui::tour`]); this is only the
//! configuration you cannot guess for someone, and the last card offers the tour.

use crate::settings::{Settings, Theme};
use crate::tiles::BasemapStyle;
use crate::ui::marker_window::IconTextures;

/// Card titles, and the one source of truth for how many cards there are.
const TITLES: [&str; 4] = ["Welcome", "Map & look", "Alerts & places", "Ready"];

/// What the last card hands back.
pub struct Finish {
    /// The chosen home site, to load and fly to.
    pub site: String,
    pub take_tour: bool,
}

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

fn heading(ui: &mut egui::Ui, step: usize) {
    ui.strong(format!("{} ({}/{})", TITLES[step], step + 1, TITLES.len()));
    ui.add_space(6.0);
}

/// Show the wizard. Returns `Some` when finished; the caller marks `setup_done`, saves settings,
/// jumps the view to the site, and starts the tour if it was asked for. `basemap` is the active
/// pane's style (updated live as the user picks); `icon_tex` renders marker-icon thumbnails.
pub fn show(
    ctx: &egui::Context,
    wiz: &mut Wizard,
    settings: &mut Settings,
    basemap: &mut BasemapStyle,
    icon_tex: &IconTextures,
) -> Option<Finish> {
    if !wiz.open {
        return None;
    }
    let mut finished = None;
    let mut open = true;
    crate::ui::phone_surface(ctx, egui::Window::new("Welcome to Hook Echo-WX"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            // Full width on desktop; on a phone, whatever fits (phone_surface caps the window).
            ui.set_width(420.0_f32.min(ctx.content_rect().width() - 40.0));
            match wiz.step {
                0 => card_site(ui, wiz, settings),
                1 => card_map(ui, settings, basemap),
                2 => card_alerts(ui, settings, icon_tex),
                _ => {
                    if let Some(take_tour) = card_done(ui, settings, *basemap) {
                        finished = Some(Finish {
                            site: settings.default_site.clone(),
                            take_tour,
                        });
                    }
                }
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if wiz.step > 0 && ui.button("Back").clicked() {
                    wiz.step -= 1;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The last card's own two buttons are the finish; there is no Next past it.
                    if wiz.step + 1 < TITLES.len() && ui.button("Next").clicked() {
                        wiz.step += 1;
                    }
                });
            });
        });
    if finished.is_some() || !open {
        wiz.open = false;
    }
    finished
}

fn card_site(ui: &mut egui::Ui, wiz: &mut Wizard, settings: &mut Settings) {
    heading(ui, 0);
    ui.label(
        "Under a minute of setup, and everything here can be changed later in Settings.\n\nStart \
         with your home radar \u{2014} the one you'll open to.",
    );
    ui.add_space(6.0);
    ui.add(egui::TextEdit::singleline(&mut wiz.filter).hint_text("Search by ID, city, or state…"));
    let needle = wiz.filter.to_ascii_uppercase();
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for s in wxdata::sites::sites() {
                if !needle.is_empty()
                    && !s.id.to_ascii_uppercase().contains(&needle)
                    && !s.city.to_ascii_uppercase().contains(&needle)
                    && !s.state.to_ascii_uppercase().contains(&needle)
                {
                    continue;
                }
                let label = format!("{}  \u{2014}  {}, {}", s.id, s.city, s.state);
                if ui
                    .selectable_label(settings.default_site == s.id, label)
                    .clicked()
                {
                    settings.default_site = s.id.to_string();
                }
            }
        });
}

fn card_map(ui: &mut egui::Ui, settings: &mut Settings, basemap: &mut BasemapStyle) {
    heading(ui, 1);
    ui.small(
        "Plenty of basemaps work with no key at all. Free keys from mapbox.com and maptiler.com \
         unlock the premium ones; they stay on this machine.",
    );
    ui.add_space(6.0);
    // Same widget as the Settings > Basemaps tab: responsive width plus the Android Paste button
    // (a soft keyboard cannot reach the system clipboard on a NativeActivity).
    super::settings_window::key_field(ui, "Mapbox token", &mut settings.mapbox_key);
    ui.add_space(4.0);
    super::settings_window::key_field(ui, "MapTiler key", &mut settings.maptiler_key);
    ui.add_space(6.0);
    // Basemap picker, filtered by whichever keys are set this frame (typing a key unlocks styles).
    let mb = !settings.mapbox_key.is_empty();
    let mt = !settings.maptiler_key.is_empty();
    let cx = crate::tiles::valid_xyz_template(&settings.custom_tile_url);
    ui.horizontal(|ui| {
        ui.label("Basemap");
        egui::ComboBox::from_id_salt("wiz_basemap")
            .selected_text(basemap.label())
            .show_ui(ui, |ui| {
                for s in BasemapStyle::ALL {
                    if s.available(mb, mt, cx)
                        && ui.selectable_label(*basemap == s, s.label()).clicked()
                    {
                        *basemap = s;
                        settings.basemap = s.slug().to_string();
                    }
                }
            });
    });
    ui.add_space(8.0);
    ui.label("Theme");
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            for t in Theme::ALL {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(28.0, 14.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, 3.0, crate::theme::preview_bg(t));
                    p.circle_filled(rect.center(), 5.0, crate::theme::accent(t));
                    ui.selectable_value(&mut settings.theme, t, t.label());
                });
            }
        });
}

fn card_alerts(ui: &mut egui::Ui, settings: &mut Settings, icon_tex: &IconTextures) {
    heading(ui, 2);
    ui.checkbox(
        &mut settings.alert_sound,
        "Play a sound when a new warning appears",
    );
    ui.checkbox(
        &mut settings.lightning_alarm,
        "Alarm on nearby lightning (within ~15 km of a saved location)",
    );
    ui.horizontal(|ui| {
        ui.label("ntfy.sh topic:");
        ui.text_edit_singleline(&mut settings.ntfy_topic);
    });
    ui.small("Optional: push to your phone when a warning covers a saved location.");
    // Collapsed: a sound per alert kind is a preference, not a decision anyone has to make in
    // their first minute.
    egui::CollapsingHeader::new("Alert sounds")
        .default_open(false)
        .show(ui, |ui| {
            crate::ui::settings_window::sound_picker(ui, settings);
        });

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Saved locations");
    ui.small(
        "Places the alerts watch \u{2014} warnings, lightning and rain arrival. Later you can \
         search a place and save it, or drop one with the Drop marker tool.",
    );
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            crate::ui::marker_window::marker_grid(ui, &mut settings.markers, icon_tex);
        });
    if ui.button("\u{2795} Add location").clicked() {
        let n = settings.markers.len() + 1;
        // Seed at the home radar rather than at (0, 0) in the Gulf of Guinea, staggered so a
        // second one is visibly its own marker. Drag or edit from there.
        let (lat, lon) = wxdata::sites::site_by_id(&settings.default_site)
            .map(|s| (s.latitude as f64, s.longitude as f64))
            .unwrap_or((0.0, 0.0));
        let offset = 0.05 * n as f64;
        settings.markers.push(crate::settings::Marker {
            id: crate::settings::new_marker_id(),
            name: format!("Location {n}"),
            lat: lat + offset,
            lon: lon + offset,
            icon: None,
            alert_radius_mi: crate::settings::default_alert_radius_mi(),
            video_url: String::new(),
            home: false,
        });
    }
}

/// The hand-off. `Some(take_tour)` once the user picks one of the two ways out.
fn card_done(ui: &mut egui::Ui, settings: &Settings, basemap: BasemapStyle) -> Option<bool> {
    heading(ui, 3);
    ui.label(format!("Home radar: {}", settings.default_site));
    ui.label(format!("Theme: {}", settings.theme.label()));
    ui.label(format!("Basemap: {}", basemap.label()));
    ui.label(format!("Saved locations: {}", settings.markers.len()));
    ui.label(format!(
        "Alert sound: {}",
        if settings.alert_sound { "on" } else { "off" }
    ));
    ui.add_space(8.0);
    ui.small(
        "The tour is four stops on the live map \u{2014} the timeline, the products, where \
         everything else lives, and how to read a storm.",
    );
    ui.add_space(6.0);
    let mut out = None;
    ui.horizontal(|ui| {
        if ui.button("Take the 60-second tour").clicked() {
            out = Some(true);
        }
        if ui.button("Explore on my own").clicked() {
            out = Some(false);
        }
    });
    ui.add_space(4.0);
    ui.small(if cfg!(target_os = "android") {
        "Both are re-runnable later from Layers \u{2192} App, and from Settings \u{2192} General."
    } else {
        "Both are re-runnable later from the sidebar's App section, Ctrl+K, and Settings \
         \u{2192} General."
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_card_is_the_finish() {
        // `show` offers Next while `step + 1 < len`, so the last index has to be the summary —
        // otherwise the wizard has no way out but its ✕.
        assert_eq!(TITLES.len(), 4);
        assert_eq!(TITLES[TITLES.len() - 1], "Ready");
    }

    #[test]
    fn start_resets_to_the_first_card() {
        let mut w = Wizard {
            open: false,
            step: 3,
            filter: "KTLX".into(),
        };
        w.start();
        assert!(w.open && w.step == 0 && w.filter.is_empty());
    }
}
