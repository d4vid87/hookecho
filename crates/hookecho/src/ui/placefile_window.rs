//! Placefile Manager: add/remove GRLevelX placefile URLs and toggle each on/off.

use crate::settings::{PlacefileConfig, PluginConfig, Settings};

/// Load status for a configured placefile (built by the app from its loaded set).
pub struct PlacefileStatus {
    pub url: String,
    pub loaded: bool,
    pub items: usize,
    pub title: String,
    /// Why the last load failed, if it did — a plugin's stderr usually ends up here.
    pub error: Option<String>,
}

#[derive(Default)]
pub struct PlacefileWindow {
    pub open: bool,
    new_url: String,
    new_plugin: String,
    new_command: String,
}

impl PlacefileWindow {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        status: &[PlacefileStatus],
    ) {
        let mut open = self.open;
        crate::ui::fit_phone(ctx, egui::Window::new("Placefile Manager"))
            .open(&mut open)
            .default_size([520.0, 320.0])
            .show(ctx, |ui| {
                ui.label("GRLevelX placefiles (lines, polygons, text, icons at lat/lon).");
                ui.add_space(4.0);

                let mut remove: Option<usize> = None;
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for (i, cfg) in settings.placefiles.iter_mut().enumerate() {
                            let st = status.iter().find(|s| s.url == cfg.url);
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut cfg.enabled, "");
                                if ui.button("✖").on_hover_text("Remove").clicked() {
                                    remove = Some(i);
                                }
                                ui.vertical(|ui| {
                                    let title = st
                                        .filter(|s| !s.title.is_empty())
                                        .map(|s| s.title.as_str())
                                        .unwrap_or("(untitled)");
                                    ui.strong(title);
                                    ui.weak(&cfg.url);
                                    status_line(ui, st);
                                });
                            });
                            ui.separator();
                        }
                    });
                if let Some(i) = remove {
                    settings.placefiles.remove(i);
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("URL:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_url)
                            .desired_width(340.0)
                            .hint_text("http://…/placefile.txt"),
                    );
                    let valid = self.new_url.starts_with("http")
                        && !settings
                            .placefiles
                            .iter()
                            .any(|c| c.url == self.new_url.trim());
                    if ui.add_enabled(valid, egui::Button::new("Add")).clicked() {
                        settings.placefiles.push(PlacefileConfig {
                            url: self.new_url.trim().to_string(),
                            enabled: true,
                            opacity: 1.0,
                        });
                        self.new_url.clear();
                    }
                });

                ui.add_space(10.0);
                ui.separator();
                ui.strong("Plugins");
                ui.label(
                    "A plugin is any command that prints a placefile on stdout — the app hands it \
                     the view in environment variables and draws what comes back.",
                );
                ui.colored_label(
                    egui::Color32::from_rgb(230, 190, 120),
                    "These run as you, with your privileges. Only add commands you would run \
                     yourself.",
                );
                let mut drop_plugin: Option<usize> = None;
                for (i, p) in settings.plugins.iter_mut().enumerate() {
                    let st = status.iter().find(|s| s.url == format!("plugin:{}", p.name));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut p.enabled, "");
                        if ui.button("✖").on_hover_text("Remove").clicked() {
                            drop_plugin = Some(i);
                        }
                        ui.vertical(|ui| {
                            ui.strong(&p.name);
                            ui.weak(format!("{} {}", p.command, p.args.join(" ")));
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut p.refresh_secs)
                                        .range(5..=3600)
                                        .suffix(" s"),
                                )
                                .on_hover_text("How often to run it");
                                status_line(ui, st);
                            });
                        });
                    });
                    if let Some(e) = st.and_then(|s| s.error.as_deref()) {
                        ui.colored_label(egui::Color32::from_rgb(230, 130, 130), e);
                    }
                    ui.separator();
                }
                if let Some(i) = drop_plugin {
                    settings.plugins.remove(i);
                }
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_plugin)
                            .desired_width(100.0)
                            .hint_text("my-plugin"),
                    );
                    ui.label("Command:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_command)
                            .desired_width(240.0)
                            .hint_text("python3 /path/to/plugin.py"),
                    );
                    let name = self.new_plugin.trim().to_string();
                    let valid = !name.is_empty()
                        && !self.new_command.trim().is_empty()
                        && !settings.plugins.iter().any(|p| p.name == name);
                    if ui.add_enabled(valid, egui::Button::new("Add")).clicked() {
                        // Whitespace splitting, not a shell: no quoting rules to get wrong, and
                        // no shell metacharacters to be surprised by. A command that needs a
                        // pipeline can be `sh -c "…"`, chosen deliberately.
                        let mut words = self.new_command.split_whitespace();
                        let command = words.next().unwrap_or_default().to_string();
                        settings.plugins.push(PluginConfig {
                            name,
                            command,
                            args: words.map(str::to_string).collect(),
                            refresh_secs: 60,
                            enabled: true,
                        });
                        self.new_plugin.clear();
                        self.new_command.clear();
                    }
                });
            });
        self.open = open;
    }
}

/// "N items" / "loading…" / "failed", shared by the placefile rows and the plugin rows.
fn status_line(ui: &mut egui::Ui, st: Option<&PlacefileStatus>) {
    match st {
        Some(s) if s.error.is_some() => {
            ui.small(egui::RichText::new("failed").color(egui::Color32::from_rgb(230, 130, 130)));
        }
        Some(s) if s.loaded => {
            ui.small(format!("{} items", s.items));
        }
        Some(_) => {
            ui.small("loading…");
        }
        None => {}
    }
}
