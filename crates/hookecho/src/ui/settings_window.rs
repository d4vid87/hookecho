//! Settings window: General, Palettes, Units, Basemaps, Alerts, Hotkeys, Sync, Storage.

use crate::app::PaletteEntry;
use crate::colormap::Palettes;
use crate::hotkeys::{self, BindableAction, Binding};
use crate::settings::{Settings, Theme, TimeDisplay, VelocityUnit};
use wxdata::level2::Moment;

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    General,
    Palettes,
    Units,
    Basemaps,
    Alerts,
    Hotkeys,
    Sync,
    #[cfg(not(target_arch = "wasm32"))]
    Storage,
}

/// What the app knows about the sync session, handed in so this window stays state-free.
pub struct SyncView<'a> {
    pub signed_in: bool,
    pub status: &'a str,
    /// The Google URL to visit, while a sign-in is in flight.
    pub login_url: Option<&'a str>,
    /// Unix seconds of the last successful sync (0 = never).
    pub last_sync: i64,
}

/// What the user asked for in the Sync tab.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SyncAction {
    SignIn,
    SignOut,
    SyncNow,
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    tab: Tab,
    prev_open: bool,
    /// Cached `.pal` file stems in the color-tables folder; rescanned on window/tab open.
    pal_stems: Vec<String>,
    scanned: bool,
    /// Hotkeys tab: the action whose key we're waiting on, the filter box, and the last conflict
    /// we resolved by stealing (shown inline so the theft isn't silent).
    rebinding: Option<BindableAction>,
    hotkey_query: String,
    stolen: Option<String>,
    /// Cache sizes, measured once on a background thread when the tab opens — a walk of a full
    /// tile cache is tens of thousands of `stat` calls and has no business on the UI thread.
    #[cfg(not(target_arch = "wasm32"))]
    storage: Option<std::sync::mpsc::Receiver<Vec<crate::storage::Entry>>>,
    #[cfg(not(target_arch = "wasm32"))]
    storage_rows: Option<Vec<crate::storage::Entry>>,
    /// Set by the General tab's two buttons; the app drains them after `show`.
    pub run_wizard: bool,
    pub run_tour: bool,
    /// True while a keypress is being captured — the app stands its global hotkey table down so
    /// binding `A` doesn't also toggle the alert panel.
    pub capturing: bool,
}

impl SettingsWindow {
    /// `palettes` is read-only here (for parse-error badges); edits go through `settings` and
    /// the app reloads tables via the settings dirty-diff.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        palettes: &Palettes,
        sync: SyncView,
        entries: &[PaletteEntry],
    ) -> Option<SyncAction> {
        let mut action = None;
        if self.open && !self.prev_open {
            self.scanned = false; // rescan the folder each time the window opens
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.storage_rows = None; // and re-measure the caches, which move between visits
            }
        }
        self.prev_open = self.open;

        let mut open = self.open;
        crate::ui::phone_surface(ctx, egui::Window::new("Settings"))
            .open(&mut open)
            .default_size([460.0, 340.0])
            .show(ctx, |ui| {
                // Keep the whole window inside a phone screen; tabs wrap instead of clipping.
                if cfg!(target_os = "android") {
                    ui.set_max_width(ui.ctx().content_rect().width() - 28.0);
                }
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (Tab::General, "General"),
                        (Tab::Palettes, "Palettes"),
                        (Tab::Units, "Units"),
                        (Tab::Basemaps, "Basemaps"),
                        (Tab::Alerts, "Alerts"),
                        (Tab::Hotkeys, "Hotkeys"),
                        (Tab::Sync, "Sync"),
                        #[cfg(not(target_arch = "wasm32"))]
                        (Tab::Storage, "Storage"),
                    ] {
                        // Chips on the phone: a `selectable_value` is a text-height target, and
                        // seven of them wrapped across a 360pt screen is a game of darts.
                        if cfg!(target_os = "android") {
                            if crate::ui::m3::chip(ui, label, self.tab == tab).clicked() {
                                self.tab = tab;
                            }
                        } else {
                            ui.selectable_value(&mut self.tab, tab, label);
                        }
                    }
                });
                ui.separator();
                match self.tab {
                    Tab::General => {
                        general_tab(ui, settings, &mut self.run_wizard, &mut self.run_tour)
                    }
                    Tab::Palettes => self.palettes_tab(ui, settings, palettes),
                    Tab::Units => units_tab(ui, settings),
                    Tab::Basemaps => basemaps_tab(ui, settings),
                    Tab::Alerts => alerts_tab(ui, settings),
                    Tab::Hotkeys => self.hotkeys_tab(ui, settings, entries),
                    Tab::Sync => action = sync_tab(ui, settings, &sync),
                    #[cfg(not(target_arch = "wasm32"))]
                    Tab::Storage => self.storage_tab(ui, settings),
                }
            });
        self.capturing = self.rebinding.is_some() && open;
        if !open {
            self.rebinding = None;
        }
        self.open = open;
        action
    }

    /// What the app has written to disk, and the buttons that take it back.
    #[cfg(not(target_arch = "wasm32"))]
    fn storage_tab(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        // Kick off one measurement per tab visit; the walk lands over a channel a frame or two
        // later, and the rows stay put until Refresh or a Clear asks for a fresh one.
        if self.storage.is_none() && self.storage_rows.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::storage::report());
            });
            self.storage = Some(rx);
        }
        if let Some(rx) = &self.storage {
            if let Ok(rows) = rx.try_recv() {
                self.storage_rows = Some(rows);
                self.storage = None;
            }
        }

        let Some(rows) = &self.storage_rows else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Measuring…");
            });
            ui.ctx().request_repaint();
            return;
        };

        let total: u64 = rows.iter().map(|r| r.bytes).sum();
        let mut recheck = false;
        ui.horizontal(|ui| {
            ui.strong(format!("{} in caches", crate::storage::human(total)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                recheck |= ui.button("Refresh").clicked();
            });
        });
        ui.weak("Everything here is re-downloadable. Clearing costs the next fetch, nothing else.");
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in rows {
                ui.horizontal(|ui| {
                    ui.label(row.label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Only the capped directories get a Clear: the loose-files row is a
                        // mixture of things with different owners (the alert snapshot is read at
                        // startup), and deleting it wholesale is not a button's decision.
                        if row.cap.is_some() {
                            if ui.button("Clear").clicked() {
                                if let Err(e) = crate::storage::clear(&row.path) {
                                    log::warn!("clearing {}: {e}", row.path.display());
                                }
                                recheck = true;
                            }
                            if ui.button("Open").clicked() {
                                let _ = crate::platform::open_url(&row.path.to_string_lossy());
                            }
                        }
                        match row.cap {
                            Some(cap) => ui.weak(format!(
                                "{} of {}",
                                crate::storage::human(row.bytes),
                                crate::storage::human(cap)
                            )),
                            None => ui.weak(crate::storage::human(row.bytes)),
                        };
                    });
                });
            }
        });
        if recheck {
            self.storage_rows = None;
        }

        // The caps themselves. The sweep runs at startup (deleting mid-session would race the
        // fetch tasks writing into the same directories), so a change lands on the next launch.
        ui.separator();
        ui.label(egui::RichText::new("Limits").small().strong());
        for (label, mb, hint) in [
            (
                "Radar volumes",
                &mut settings.volume_cache_mb,
                "0 uses the platform default: 2 GB on the desktop, 300 MB on Android.",
            ),
            (
                "Map tiles (each cache)",
                &mut settings.tile_disk_cache_mb,
                "Applies to the raster and vector tile caches separately. 0 = platform default.",
            ),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(
                    egui::DragValue::new(mb)
                        .range(0..=64_000)
                        .speed(50.0)
                        .suffix(" MB"),
                )
                .on_hover_text(hint);
                if *mb == 0 {
                    ui.weak("default");
                }
            });
        }
        ui.weak("New limits apply at the next start.");
    }

    /// Rebindable keyboard shortcuts. Rows come from the binding table itself, so anything the
    /// command registry can do is bindable; `Palette` rows borrow the registry's own label.
    fn hotkeys_tab(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut Settings,
        entries: &[PaletteEntry],
    ) {
        let name = |a: BindableAction| -> String {
            match a {
                BindableAction::Palette(p) => entries
                    .iter()
                    .find(|e| e.action == p)
                    .map(|e| e.label.clone())
                    .unwrap_or_else(|| format!("{p:?}")),
                other => hotkeys::label(other).unwrap_or("").to_string(),
            }
        };

        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit_singleline(&mut self.hotkey_query);
            if ui.button("Reset to defaults").clicked() {
                settings.keybinds.clear();
                self.rebinding = None;
                self.stolen = None;
            }
        });
        ui.label(
            egui::RichText::new(
                "Click a shortcut, then press the key you want. Escape cancels — it always closes \
                 whatever is in front of you, so it can't be bound.",
            )
            .weak()
            .small(),
        );
        ui.separator();

        // First edit copies the whole shipped table into settings, so a later change to the
        // defaults doesn't silently rewrite keys the user already learned.
        if settings.keybinds.is_empty() {
            settings.keybinds = hotkeys::defaults();
        }

        // Capture: the first real keypress while a row is armed becomes its binding.
        if let Some(target) = self.rebinding {
            if let Some((mods, key)) = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if *key != egui::Key::Escape => Some((*modifiers, *key)),
                    _ => None,
                })
            }) {
                let shortcut = egui::KeyboardShortcut::new(mods, key);
                self.stolen =
                    hotkeys::conflict(&settings.keybinds, shortcut, target).map(|b| name(b.action));
                settings.keybinds.retain(|b| b.shortcut != shortcut);
                if let Some(row) = settings.keybinds.iter_mut().find(|b| b.action == target) {
                    row.shortcut = shortcut;
                } else {
                    settings.keybinds.push(Binding {
                        shortcut,
                        action: target,
                    });
                }
                self.rebinding = None;
            } else if ui
                .ctx()
                .input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.any_click())
            {
                self.rebinding = None;
            }
        }

        if let Some(lost) = &self.stolen {
            ui.colored_label(
                egui::Color32::from_rgb(230, 170, 60),
                format!("“{lost}” lost that key and is now unbound."),
            );
        }

        let rows: Vec<(usize, String)> = settings
            .keybinds
            .iter()
            .enumerate()
            .map(|(i, b)| (i, name(b.action)))
            .filter(|(_, label)| {
                crate::ui::layers_panel::fuzzy(&self.hotkey_query, label).is_some()
            })
            .collect();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, label) in rows {
                let armed = self.rebinding == Some(settings.keybinds[i].action);
                let text = if armed {
                    "press a key…".to_string()
                } else {
                    ui.ctx().format_shortcut(&settings.keybinds[i].shortcut)
                };
                ui.horizontal(|ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.selectable_label(armed, text).clicked() {
                            self.rebinding = if armed {
                                None
                            } else {
                                self.stolen = None;
                                Some(settings.keybinds[i].action)
                            };
                        }
                    });
                });
            }
        });
    }

    fn rescan(&mut self) {
        self.pal_stems.clear();
        if let Some(dir) = Settings::colortables_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    // File names, not stems: `.pal` and `.pal3` both load, and two tables can
                    // share a stem.
                    if p.extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("pal") || x.eq_ignore_ascii_case("pal3"))
                    {
                        if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                            self.pal_stems.push(name.to_string());
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

        egui::Grid::new("palette_grid")
            .num_columns(3)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                for moment in Moment::ALL {
                    let key = moment.short_name();
                    ui.label(key);

                    let current = settings.palettes.get(key).cloned();
                    // A `builtin:` value names a compiled-in alternate, not a file, so it has no
                    // stem — show the alternate's own name.
                    let builtin = current
                        .as_deref()
                        .and_then(|v| v.strip_prefix(crate::colormap::BUILTIN_PREFIX))
                        .map(str::to_string);
                    let current_stem = current
                        .as_deref()
                        .filter(|_| builtin.is_none())
                        .and_then(|p| std::path::Path::new(p).file_name().and_then(|s| s.to_str()))
                        .map(str::to_string);
                    let selected_text = builtin
                        .clone()
                        .or_else(|| current_stem.clone())
                        .unwrap_or_else(|| "Default".to_string());

                    egui::ComboBox::from_id_salt(("pal_combo", key))
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_none(), "Default").clicked() {
                                settings.palettes.remove(key);
                            }
                            for name in crate::colormap::alt_names(moment) {
                                let is_sel = builtin.as_deref() == Some(name);
                                if ui.selectable_label(is_sel, name).clicked() {
                                    settings.palettes.insert(
                                        key.to_string(),
                                        format!("{}{name}", crate::colormap::BUILTIN_PREFIX),
                                    );
                                }
                            }
                            for stem in &self.pal_stems {
                                let is_sel = current_stem.as_deref() == Some(stem.as_str());
                                if ui.selectable_label(is_sel, stem).clicked() {
                                    if let Some(d) = &dir {
                                        let path = d.join(stem);
                                        settings.palettes.insert(
                                            key.to_string(),
                                            path.to_string_lossy().into_owned(),
                                        );
                                    }
                                }
                            }
                        });

                    if ui.button("Browse…").clicked() {
                        // Tagged with the moment key: the picker answers later (always, on
                        // Android), and by then nothing else remembers which row asked.
                        crate::dialog::request_open(crate::dialog::ImportKind::Palette, key);
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

        ui.label("Temperature");
        ui.horizontal(|ui| {
            for u in crate::settings::TempUnit::ALL {
                ui.selectable_value(&mut settings.temp_unit, u, u.label());
            }
        })
        .response
        .on_hover_text("Surface station plots (observations arrive in Celsius)");
        ui.end_row();

        ui.label("Time display");
        ui.horizontal(|ui| {
            for d in TimeDisplay::ALL {
                ui.selectable_value(&mut settings.time_display, d, d.label());
            }
        })
        .response
        .on_hover_text("Site local reads the clock the radar is standing in; UTC is the Zulu time on the wire");
        ui.end_row();
    });
    ui.weak("Reflectivity stays dBZ; internal data is unchanged (display-only).");
}

fn basemaps_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.label("Provider API keys unlock additional raster basemap styles.");
    ui.add_space(6.0);
    key_field(ui, "Mapbox access token", &mut settings.mapbox_key);
    ui.add_space(8.0);
    key_field(ui, "MapTiler API key", &mut settings.maptiler_key);
    ui.add_space(6.0);
    ui.weak("Keys are stored locally in settings.json and sent only to the provider's tile API.");
    ui.add_space(12.0);
    ui.separator();
    ui.label("Live station cards");
    ui.weak("Optional. Airport METARs need no key; these add personal weather stations.");
    ui.add_space(6.0);
    key_field(ui, "WeatherFlow Tempest token", &mut settings.tempest_token);
    ui.add_space(8.0);
    key_field(ui, "Weather Underground API key", &mut settings.wu_key);
    ui.add_space(12.0);
    ui.separator();
    ui.label("Crowd reports");
    ui.weak(
        "Optional. A free mPING key (mping.ou.edu) adds crowd-sourced precipitation-type \
         reports \u{2014} the only source that says whether it is landing as rain or snow.",
    );
    ui.add_space(6.0);
    key_field(ui, "mPING API key", &mut settings.mping_key);
    ui.add_space(12.0);
    ui.separator();
    ui.label("Webcams");
    ui.weak(
        "Optional. The FAA's ~2,600 cameras need no key but stop at the US border; a free Windy \
         key adds their global network. Windy returns the 50 most popular cameras in view.",
    );
    ui.add_space(6.0);
    key_field(ui, "Windy API key", &mut settings.windy_key);
    ui.add_space(12.0);
    ui.separator();
    ui.label("Air quality");
    ui.weak("Optional. A free key from docs.airnowapi.org turns on the AirNow AQI layer.");
    ui.add_space(6.0);
    key_field(ui, "AirNow API key", &mut settings.airnow_key);
    ui.add_space(8.0);
    ui.label("Field mill URL (JSON, kV/m)");
    ui.text_edit_singleline(&mut settings.field_mill_url)
        .on_hover_text(
            "A ground field mill publishing {\"time\": …, \"kv_per_m\": …}. Left empty, the cards \
             chart NOAA's ionospheric PPEF model in mV/m instead — a different quantity entirely.",
        );
}

/// A masked API-key entry: a label above a field that fills the row, then a Clear (✕) button, and
/// on Android a Paste button that fills the field straight from the system clipboard (typing long
/// keys on a soft keyboard is impractical, and reading the clipboard directly sidesteps IME
/// quirks). Laid out responsively so it fits any width, phone included.
pub(crate) fn key_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.horizontal(|ui| {
        // Reserve space for the trailing buttons; the field takes the rest.
        let paste_w = if cfg!(target_os = "android") {
            62.0
        } else {
            0.0
        };
        let field_w = (ui.available_width() - paste_w - 34.0).max(80.0);
        ui.add(
            egui::TextEdit::singleline(value)
                .password(true)
                .desired_width(field_w),
        );
        #[cfg(target_os = "android")]
        if ui
            .button("Paste")
            .on_hover_text("Paste from clipboard")
            .clicked()
        {
            if let Some(t) = crate::platform::clipboard_text() {
                *value = t.trim().to_string();
            }
        }
        // Phosphor X, not "✕": the Android fallback face has no U+2715 and drew a tofu box.
        if !value.is_empty()
            && ui
                .small_button(egui_phosphor::regular::X)
                .on_hover_text("Clear")
                .clicked()
        {
            value.clear();
        }
    });
}

/// Sign in with Google and keep every machine's settings the same. The data goes to the hidden
/// per-app folder in the user's own Drive — there is no Hook Echo account and no server.
fn sync_tab(ui: &mut egui::Ui, settings: &mut Settings, sync: &SyncView) -> Option<SyncAction> {
    let mut action = None;
    ui.label("Sign in with Google to keep your settings, saved locations, placefiles and API keys the same on every machine.");
    ui.add_space(6.0);
    if sync.signed_in {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("✓ Signed in").strong());
            if ui.button("Sign out").clicked() {
                action = Some(SyncAction::SignOut);
            }
        });
        ui.checkbox(
            &mut settings.sync_enabled,
            "Sync automatically (every 5 minutes)",
        );
        if ui.button("Sync now").clicked() {
            action = Some(SyncAction::SyncNow);
        }
        if sync.last_sync > 0 {
            let ago = (crate::share::now() - sync.last_sync).max(0);
            ui.weak(match ago {
                0..=59 => "last synced just now".to_string(),
                60..=3599 => format!("last synced {} min ago", ago / 60),
                _ => format!("last synced {} h ago", ago / 3600),
            });
        }
    } else if let Some(url) = sync.login_url {
        // The browser has the user right now. Show the link anyway: on a phone (or a desktop with
        // no opener) launching it can fail, and then this is the only way through.
        ui.label("Waiting for you to finish in the browser…");
        ui.horizontal(|ui| {
            if ui.button("Open sign-in page").clicked() {
                if let Err(e) = crate::platform::open_url(url) {
                    log::warn!("open_url failed: {e}");
                }
            }
            if ui.button("Copy link").clicked() {
                ui.ctx().copy_text(url.to_string());
            }
        });
    } else {
        ui.label("Your own Google OAuth client (one-time setup — see docs/sync.md):");
        key_field(ui, "Client ID", &mut settings.sync_client_id);
        ui.add_space(4.0);
        key_field(ui, "Client secret", &mut settings.sync_client_secret);
        ui.add_space(6.0);
        if ui.button("Sign in with Google").clicked() {
            action = Some(SyncAction::SignIn);
        }
    }
    if !sync.status.is_empty() {
        ui.add_space(6.0);
        ui.weak(sync.status);
    }
    ui.add_space(6.0);
    ui.weak(
        "Screen scale, device name and background alerts stay local to each machine; everything \
         else follows the sync. The grant covers only this app's own Drive folder.",
    );
    action
}

fn general_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    run_wizard: &mut bool,
    run_tour: &mut bool,
) {
    egui::Grid::new("general_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
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
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(46.0, 18.0), egui::Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 3.0, crate::theme::preview_bg(settings.theme));
                p.circle_filled(rect.center(), 6.0, crate::theme::accent(settings.theme));
            });
            ui.end_row();

            ui.label("UI scale");
            // Phones start denser: 0.5 × a 4.0 density factor ≈ a desktop-density canvas.
            let lo = if cfg!(target_os = "android") {
                0.5
            } else {
                0.7
            };
            ui.add(egui::Slider::new(&mut settings.ui_scale, lo..=1.6).step_by(0.05));
            ui.end_row();
        });
    ui.weak("UI scale also responds to Ctrl+= / Ctrl+- / Ctrl+0.");

    let valid = wxdata::sites::site_by_id(&settings.default_site).is_some();
    if !valid && !settings.default_site.is_empty() {
        ui.colored_label(egui::Color32::YELLOW, "⚠ unknown site id");
    }

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Getting started");
    ui.horizontal(|ui| {
        if ui
            .button("Setup wizard\u{2026}")
            .on_hover_text("Re-run first-time setup")
            .clicked()
        {
            *run_wizard = true;
        }
        if ui
            .button("Take the tour\u{2026}")
            .on_hover_text("A 60-second walk through the app's controls")
            .clicked()
        {
            *run_tour = true;
        }
    });

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Background");
    ui.checkbox(
        &mut settings.close_to_tray,
        "Keep running in background when window closes",
    )
    .on_hover_text(
        "Closing the window minimizes instead of quitting, so alert polling + push keep going",
    );

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Workspaces");
    if settings.workspaces.is_empty() {
        ui.weak("None yet \u{2014} arrange your panes, then run \"Save workspace\" from Ctrl+K.");
    } else {
        ui.weak("Restore one from Ctrl+K. Renaming here is what the command is called.");
    }
    let mut remove = None;
    for (i, ws) in settings.workspaces.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut ws.name).desired_width(220.0));
            ui.weak(format!(
                "{} pane{}",
                ws.panes.len(),
                if ws.panes.len() == 1 { "" } else { "s" }
            ));
            if ui.button("Delete").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        settings.workspaces.remove(i);
    }

    ui.add_space(8.0);
    ui.separator();
    ui.strong("AI");
    ui.horizontal(|ui| {
        ui.label("Anthropic key:");
        ui.add(
            egui::TextEdit::singleline(&mut settings.anthropic_key)
                .password(true)
                .hint_text("sk-ant-…")
                .desired_width(240.0),
        );
    });
    ui.weak("Optional. Storm Digest (Ctrl+K) works offline; a key lets Claude write friendlier prose. Held locally only.");
}

/// Alert-sound controls: master toggle, volume, and a per-event sound picker with previews.
/// Shared by the Settings ▸ Audio tab and the first-run wizard.
pub fn sound_picker(ui: &mut egui::Ui, settings: &mut Settings) {
    use crate::settings::AlertSound;

    ui.checkbox(&mut settings.mute_alerts, "Mute all alert audio")
        .on_hover_text("Silences chimes and spoken warnings without changing the choices below");
    ui.checkbox(&mut settings.alert_sound, "Play a sound on alerts")
        .on_hover_text("Master switch for the warning / TDS / lightning alert sounds");
    ui.checkbox(&mut settings.scan_chime, "Chime on every new scan")
        .on_hover_text(
            "A tap on the shoulder when a new volume lands on the live pane you're watching",
        );
    ui.horizontal(|ui| {
        ui.label("Volume");
        ui.add(egui::Slider::new(&mut settings.alert_volume, 0.0..=1.0).step_by(0.05));
    });
    ui.add_space(4.0);

    // One row per alert kind: sound combo (+ Custom… file picker) and a ▶ preview.
    type SoundRow = (&'static str, fn(&mut Settings) -> &mut AlertSound);
    let rows: [SoundRow; 6] = [
        ("New scan", |s| &mut s.scan_sound),
        ("Warning", |s| &mut s.warn_sound),
        ("Emergency", |s| &mut s.emergency_sound),
        ("TDS", |s| &mut s.tds_sound),
        ("Rotation", |s| &mut s.rotation_sound),
        ("Lightning", |s| &mut s.lightning_sound),
    ];
    let volume = settings.alert_volume;
    egui::Grid::new("sound_grid")
        .num_columns(3)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
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
                            crate::dialog::request_open(
                                crate::dialog::ImportKind::AlertSound,
                                label,
                            );
                        }
                    });
                let preview = sound.clone();
                if ui
                    .button(egui_phosphor::regular::PLAY)
                    .on_hover_text("Preview")
                    .clicked()
                {
                    crate::audio::play(&preview, volume);
                }
                ui.end_row();
            }
        });
}

/// Everything that fires when weather happens: sounds, push, proximity alarms.
fn alerts_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    sound_picker(ui, settings);

    ui.add_space(8.0);
    ui.separator();
    ui.strong("When to interrupt");
    ui.add_enabled_ui(!cfg!(target_os = "android"), |ui| {
        ui.checkbox(&mut settings.desktop_notify, "Post alerts to the desktop")
            .on_hover_text(
                "Use the system notification centre, so an alert arrives with the window \
                 behind something else",
            );
    });
    ui.checkbox(&mut settings.alert_follow_gps, "Alert where I am, too")
        .on_hover_text(
            "While a GPS fix is coming in, your own position joins the saved locations the \
             lightning and rotation alerts watch. Nothing is saved or shared.",
        );
    ui.horizontal(|ui| {
        ui.checkbox(&mut settings.quiet_hours, "Quiet hours");
        ui.add_enabled_ui(settings.quiet_hours, |ui| {
            ui.add(egui::DragValue::new(&mut settings.quiet_start_hour).range(0..=23));
            ui.label("to");
            ui.add(egui::DragValue::new(&mut settings.quiet_end_hour).range(0..=23));
            ui.weak("local");
        });
    });
    ui.weak(
        "Holds sounds and pushes between those hours. Tornado Emergency, PDS and destructive \
         warnings still come through — that tier is what quiet hours is for.",
    );
    ui.horizontal(|ui| {
        ui.label("Push and sound only for:");
        for (tier, label) in [
            (0u8, "Every warning"),
            (1, "Considerable and up"),
            (2, "Escalated only"),
        ] {
            ui.selectable_value(&mut settings.alert_min_escalation, tier, label);
        }
    });
    ui.weak("Quieter warnings still banner and still show in the alert list.");

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Push notifications (ntfy.sh)");
    ui.horizontal(|ui| {
        ui.label("Topic:");
        ui.add(egui::TextEdit::singleline(&mut settings.ntfy_topic).hint_text("your-secret-topic"));
    });
    ui.weak("When a warning covers a saved location marker, a push is sent to ntfy.sh/<topic>.");
    ui.weak("Subscribe to the same topic in the ntfy app on your phone. Leave blank to disable.");
    ui.add_enabled_ui(!cfg!(target_os = "android"), |ui| {
        ui.checkbox(&mut settings.ntfy_snapshot, "Attach a picture of the radar")
            .on_hover_text(
                "Pushes the view you're looking at alongside the warning. Desktop only — the \
                 phone's background alert service has nothing to render from.",
            );
    });

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Chat webhooks");
    ui.horizontal(|ui| {
        ui.label("Discord:");
        ui.add(
            egui::TextEdit::singleline(&mut settings.discord_webhook)
                .hint_text("https://discord.com/api/webhooks/…"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Slack:");
        ui.add(
            egui::TextEdit::singleline(&mut settings.slack_webhook)
                .hint_text("https://hooks.slack.com/services/…"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Matrix server:");
        ui.add(
            egui::TextEdit::singleline(&mut settings.matrix_homeserver)
                .hint_text("https://matrix.org"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Matrix room:");
        ui.add(egui::TextEdit::singleline(&mut settings.matrix_room).hint_text("!room:matrix.org"));
    });
    ui.horizontal(|ui| {
        ui.label("Matrix token:");
        ui.add(
            egui::TextEdit::singleline(&mut settings.matrix_token)
                .password(true)
                .hint_text("access token"),
        );
    });
    ui.weak("Every alert that goes to ntfy also posts here. Blank fields are off.");
    ui.weak("These URLs and the token are secrets — they stay in your settings file.");

    if cfg!(target_os = "android") {
        ui.add_space(8.0);
        ui.separator();
        ui.strong("Background alerts");
        if ui
            .checkbox(
                &mut settings.background_alerts,
                "Watch my saved locations while the app is closed",
            )
            .on_hover_text(
                "Runs a small service that checks api.weather.gov for your marker locations and \
                 posts a notification. Tapping it flies the map to that location. Costs a \
                 permanent notification and some battery.",
            )
            .changed()
        {
            crate::platform::set_background_alerts(settings.background_alerts);
        }
        ui.weak(
            "Some phones kill background services aggressively — if alerts stop arriving, \
                 exempt Hook Echo-WX from battery optimisation in system settings.",
        );
    }

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Weather radio relays");
    ui.weak(
        "NOAA broadcasts NWR on VHF and streams nothing itself, so these are listener-run relays \
         — find the MP3 URL for your county and paste it here. Play them from the drawer.",
    );
    let mut remove = None;
    for (i, s) in settings.nwr_streams.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut s.name)
                    .hint_text("KEC55 Norman")
                    .desired_width(130.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut s.url)
                    .hint_text("https://…/stream.mp3")
                    .desired_width(220.0),
            );
            if ui.button("✖").on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        settings.nwr_streams.remove(i);
    }
    if ui.button("Add relay").clicked() {
        settings.nwr_streams.push(crate::settings::NwrStream {
            name: String::new(),
            url: String::new(),
        });
    }

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Spoken warnings");
    ui.checkbox(&mut settings.speak_warnings, "Read new warnings aloud")
        .on_hover_text(
            "Speaks the event, area and expiry through the system speech engine \u{2014} for when \
             your eyes are on the road",
        );
    ui.weak("Linux needs spd-say or espeak installed; Android uses its built-in voice.");

    ui.add_space(8.0);
    ui.separator();
    ui.strong("Proximity alarms");
    ui.checkbox(
        &mut settings.rain_alerts,
        "Rain heading for a saved location",
    )
    .on_hover_text(
        "Watches upwind of your saved places and your chase position, and says roughly how \
             many minutes out the rain is",
    );
    ui.checkbox(&mut settings.lightning_alarm, "Lightning within ~15 km of a saved location")
        .on_hover_text("Chime + push when CG lightning strikes near a marker. Requires the Lightning layer (National) to be on.");
}
