//! Settings window. General/Palettes/Units are live (U1/U3); the rest are later milestones.

use crate::colormap::Palettes;
use crate::settings::{Settings, Theme, VelocityUnit};
use wxdata::level2::Moment;

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    General,
    Palettes,
    Units,
    Basemaps,
    Audio,
    Text,
    Hotkeys,
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    tab: Tab,
    prev_open: bool,
    /// Cached `.pal` file stems in the color-tables folder; rescanned on window/tab open.
    pal_stems: Vec<String>,
    scanned: bool,
}

impl SettingsWindow {
    /// `palettes` is read-only here (for parse-error badges); edits go through `settings` and
    /// the app reloads tables via the settings dirty-diff.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings, palettes: &Palettes) {
        if self.open && !self.prev_open {
            self.scanned = false; // rescan the folder each time the window opens
        }
        self.prev_open = self.open;

        let mut open = self.open;
        egui::Window::new("Settings")
            .open(&mut open)
            .default_size([460.0, 340.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::General, "General"),
                        (Tab::Palettes, "Palettes"),
                        (Tab::Units, "Units"),
                        (Tab::Basemaps, "Basemaps"),
                        (Tab::Audio, "Audio"),
                        (Tab::Text, "Text"),
                        (Tab::Hotkeys, "Hotkeys"),
                    ] {
                        ui.selectable_value(&mut self.tab, tab, label);
                    }
                });
                ui.separator();
                match self.tab {
                    Tab::General => general_tab(ui, settings),
                    Tab::Palettes => self.palettes_tab(ui, settings, palettes),
                    Tab::Units => units_tab(ui, settings),
                    Tab::Basemaps => basemaps_tab(ui, settings),
                    Tab::Audio => audio_tab(ui, settings),
                    Tab::Text => placeholder(ui, "Fonts & text — U8"),
                    Tab::Hotkeys => placeholder(ui, "Configurable hotkeys — U8"),
                }
            });
        self.open = open;
    }

    fn rescan(&mut self) {
        self.pal_stems.clear();
        if let Some(dir) = Settings::colortables_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pal")) {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            self.pal_stems.push(stem.to_string());
                        }
                    }
                }
            }
        }
        self.pal_stems.sort();
        self.scanned = true;
    }

    fn palettes_tab(&mut self, ui: &mut egui::Ui, settings: &mut Settings, palettes: &Palettes) {
        if !self.scanned {
            self.rescan();
        }
        let dir = Settings::colortables_dir();
        ui.horizontal(|ui| {
            ui.label("Color tables (GRLevelX .pal)");
            if ui.button("⟳ Rescan folder").clicked() {
                self.scanned = false;
            }
        });
        if let Some(d) = &dir {
            ui.weak(format!("folder: {}", d.display()));
        }
        ui.add_space(4.0);

        egui::Grid::new("palette_grid").num_columns(3).spacing([10.0, 8.0]).show(ui, |ui| {
            for moment in Moment::ALL {
                let key = moment.short_name();
                ui.label(key);

                let current = settings.palettes.get(key).cloned();
                let current_stem = current
                    .as_deref()
                    .and_then(|p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str()))
                    .map(str::to_string);
                let selected_text = current_stem.clone().unwrap_or_else(|| "Default".to_string());

                egui::ComboBox::from_id_salt(("pal_combo", key)).selected_text(selected_text).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "Default").clicked() {
                        settings.palettes.remove(key);
                    }
                    for stem in &self.pal_stems {
                        let is_sel = current_stem.as_deref() == Some(stem.as_str());
                        if ui.selectable_label(is_sel, stem).clicked() {
                            if let Some(d) = &dir {
                                let path = d.join(format!("{stem}.pal"));
                                settings.palettes.insert(key.to_string(), path.to_string_lossy().into_owned());
                            }
                        }
                    }
                });

                if ui.button("Browse…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("GRLevelX palette", &["pal"])
                        .pick_file()
                    {
                        settings.palettes.insert(key.to_string(), path.to_string_lossy().into_owned());
                    }
                }
                ui.end_row();

                if let Some(err) = &palettes.errors[moment.index()] {
                    ui.label("");
                    ui.colored_label(egui::Color32::from_rgb(230, 170, 80), format!("⚠ {err}"));
                    ui.label("");
                    ui.end_row();
                }
            }
        });
    }
}

fn units_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    egui::Grid::new("units_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label("Velocity / spectrum width");
        ui.horizontal(|ui| {
            for u in VelocityUnit::ALL {
                ui.selectable_value(&mut settings.velocity_unit, u, u.label());
            }
        });
        ui.end_row();
    });
    ui.weak("Reflectivity stays dBZ; internal data is unchanged (display-only).");
}

fn basemaps_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.label("Provider API keys unlock additional raster basemap styles.");
    ui.add_space(4.0);
    egui::Grid::new("basemap_keys").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label("Mapbox access token");
        ui.add(egui::TextEdit::singleline(&mut settings.mapbox_key).password(true).desired_width(240.0));
        ui.end_row();
        ui.label("MapTiler API key");
        ui.add(egui::TextEdit::singleline(&mut settings.maptiler_key).password(true).desired_width(240.0));
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.weak("Keys are stored locally in settings.json and sent only to the provider's tile API.");
}

fn general_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    egui::Grid::new("general_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label("Default site");
        let mut site = settings.default_site.clone();
        if ui.text_edit_singleline(&mut site).changed() {
            settings.default_site = site.to_ascii_uppercase();
        }
        ui.end_row();

        ui.label("Poll interval (s)");
        ui.add(egui::DragValue::new(&mut settings.poll_interval_secs).range(10..=600));
        ui.end_row();

        ui.label("Theme");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("theme")
                .selected_text(settings.theme.label())
                .show_ui(ui, |ui| {
                    for t in Theme::ALL {
                        ui.selectable_value(&mut settings.theme, t, t.label());
                    }
                });
            // Live swatch: accent over the theme background, so the choice previews at a glance.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 18.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 3.0, crate::theme::preview_bg(settings.theme));
            p.circle_filled(rect.center(), 6.0, crate::theme::accent(settings.theme));
        });
        ui.end_row();

        ui.label("UI scale");
        ui.add(egui::Slider::new(&mut settings.ui_scale, 0.7..=1.6).step_by(0.05));
        ui.end_row();
    });
    ui.weak("UI scale also responds to Ctrl+= / Ctrl+- / Ctrl+0.");

    let valid = wxdata::sites::site_by_id(&settings.default_site).is_some();
    if !valid && !settings.default_site.is_empty() {
        ui.colored_label(egui::Color32::YELLOW, "⚠ unknown site id");
    }
}

/// Alert-sound controls: master toggle, volume, and a per-event sound picker with previews.
/// Shared by the Settings ▸ Audio tab and the first-run wizard.
pub fn sound_picker(ui: &mut egui::Ui, settings: &mut Settings) {
    use crate::settings::AlertSound;

    ui.checkbox(&mut settings.alert_sound, "Play a sound on alerts")
        .on_hover_text("Master switch for the warning / TDS / lightning alert sounds");
    ui.horizontal(|ui| {
        ui.label("Volume");
        ui.add(egui::Slider::new(&mut settings.alert_volume, 0.0..=1.0).step_by(0.05));
    });
    ui.add_space(4.0);

    // One row per alert kind: sound combo (+ Custom… file picker) and a ▶ preview.
    type SoundRow = (&'static str, fn(&mut Settings) -> &mut AlertSound);
    let rows: [SoundRow; 4] = [
        ("Warning", |s| &mut s.warn_sound),
        ("Emergency", |s| &mut s.emergency_sound),
        ("TDS", |s| &mut s.tds_sound),
        ("Lightning", |s| &mut s.lightning_sound),
    ];
    let volume = settings.alert_volume;
    egui::Grid::new("sound_grid").num_columns(3).spacing([10.0, 6.0]).show(ui, |ui| {
        for (label, field) in rows {
            ui.label(label);
            let sound = field(settings);
            egui::ComboBox::from_id_salt(label)
                .selected_text(sound.label())
                .show_ui(ui, |ui| {
                    for b in AlertSound::BUILTINS {
                        let sel = sound.label() == b.label();
                        if ui.selectable_label(sel, b.label()).clicked() {
                            *sound = b;
                        }
                    }
                    let is_custom = matches!(sound, AlertSound::Custom(_));
                    if ui.selectable_label(is_custom, "Custom…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("audio", &["wav", "mp3", "ogg", "flac"])
                            .pick_file()
                        {
                            *sound = AlertSound::Custom(path.to_string_lossy().into_owned());
                        }
                    }
                });
            let preview = sound.clone();
            if ui.button("▶").on_hover_text("Preview").clicked() {
                crate::audio::play(&preview, volume);
            }
            ui.end_row();
        }
    });
}

fn audio_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    sound_picker(ui, settings);

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Push notifications (ntfy.sh)");
    ui.horizontal(|ui| {
        ui.label("Topic:");
        ui.add(egui::TextEdit::singleline(&mut settings.ntfy_topic).hint_text("your-secret-topic"));
    });
    ui.weak("When a warning covers a saved location marker, a push is sent to ntfy.sh/<topic>.");
    ui.weak("Subscribe to the same topic in the ntfy app on your phone. Leave blank to disable.");

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Proximity alarms");
    ui.checkbox(&mut settings.lightning_alarm, "Lightning within ~15 km of a saved location")
        .on_hover_text("Chime + push when CG lightning strikes near a marker. Requires the Lightning layer (National) to be on.");

    ui.add_space(8.0);
    ui.separator();
    ui.checkbox(&mut settings.close_to_tray, "Keep running in background when window closes")
        .on_hover_text("Closing the window minimizes instead of quitting, so alert polling + push keep going");

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Storm digest (Claude)");
    ui.horizontal(|ui| {
        ui.label("Anthropic key:");
        ui.add(egui::TextEdit::singleline(&mut settings.anthropic_key).password(true).hint_text("sk-ant-…").desired_width(240.0));
    });
    ui.weak("Optional. Tools ▸ Storm Digest works offline; a key lets Claude write friendlier prose. Held locally only.");
}

fn placeholder(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.weak(text);
    });
}
