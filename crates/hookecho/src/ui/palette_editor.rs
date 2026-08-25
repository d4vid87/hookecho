//! Color-table editor: edit a moment's palette stops with live preview, then Save (writes a
//! `.pal` into the local colortables dir and sets it as that moment's override). Also imports a
//! GRLevelX `.pal` and exports the working table to share.

use crate::colormap::{ColorTable, PalStop};
use crate::settings::Settings;
use wxdata::level2::Moment;

#[derive(Default)]
pub struct PaletteEditor {
    pub open: bool,
    moment_idx: usize,
    /// Working copy; rebuilt from the active table when the moment changes.
    table: Option<ColorTable>,
    loaded_for: Option<usize>,
    /// A `.pal` the user picked, waiting to become the working copy. The picker answers a frame
    /// or more later — on Android, after a whole activity round trip — so the file arrives here
    /// through the app rather than as a return value.
    pub pending_import: Option<std::path::PathBuf>,
}

/// Tag for the editor's own import, so a `.pal` picked here becomes the working copy instead of
/// overriding a moment's saved palette.
pub(crate) const EDITOR_TAG: &str = "\u{1}editor";

impl PaletteEditor {
    /// `active` is the currently-baked table for the chosen moment (edit starting point).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        active: &crate::colormap::Palettes,
        drawer: &mut crate::ui::drawer::Drawer,
    ) {
        if !self.open {
            return;
        }
        let moment = Moment::ALL[self.moment_idx];
        // (Re)load the working copy when the moment changes or on first open.
        if self.loaded_for != Some(self.moment_idx) {
            self.table = Some(active.table(moment).clone());
            self.loaded_for = Some(self.moment_idx);
        }
        // A `.pal` the user picked since the last frame becomes the working copy.
        if let Some(path) = self.pending_import.take() {
            if let Some(t) = read_pal(&path) {
                self.table = Some(t);
                self.loaded_for = Some(self.moment_idx);
            }
        }

        let mut open = self.open;
        let Some(window) = drawer.page_sized(
            ctx,
            "Color-Table Editor",
            &mut open,
            false,
            460.0,
            egui::Window::new("Color-Table Editor"),
        ) else {
            self.open = open;
            return;
        };
        window.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Moment");
                    egui::ComboBox::from_id_salt("pal_moment")
                        .selected_text(moment.short_name())
                        .show_ui(ui, |ui| {
                            for (i, m) in Moment::ALL.iter().enumerate() {
                                if ui
                                    .selectable_value(&mut self.moment_idx, i, m.short_name())
                                    .clicked()
                                {
                                    self.loaded_for = None; // force reload for the new moment
                                }
                            }
                        });
                });
                ui.separator();

                let Some(table) = self.table.as_mut() else {
                    return;
                };

                // Live gradient preview across the moment's value range.
                preview_bar(ui, table, moment.value_range());
                ui.add_space(6.0);

                let mut remove = None;
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        egui::Grid::new("stops")
                            .num_columns(5)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.strong("Value");
                                ui.strong("Color");
                                ui.strong("End");
                                ui.strong("Solid");
                                ui.end_row();
                                for (i, s) in table.stops.iter_mut().enumerate() {
                                    ui.add(
                                        egui::DragValue::new(&mut s.value)
                                            .speed(0.5)
                                            .max_decimals(2),
                                    );
                                    color_edit(ui, &mut s.rgba);
                                    // Optional second color (hard break).
                                    let mut has_end = s.end.is_some();
                                    if ui.checkbox(&mut has_end, "").changed() {
                                        s.end = if has_end { Some(s.rgba) } else { None };
                                    }
                                    if let Some(end) = s.end.as_mut() {
                                        color_edit(ui, end);
                                    } else {
                                        ui.label("");
                                    }
                                    ui.checkbox(&mut s.solid, "");
                                    if ui.button("✖").clicked() {
                                        remove = Some(i);
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                if let Some(i) = remove {
                    table.stops.remove(i);
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("➕ Add stop").clicked() {
                        let value = table.stops.last().map(|s| s.value + 5.0).unwrap_or(0.0);
                        table.stops.push(PalStop {
                            value,
                            rgba: [255, 255, 255, 255],
                            end: None,
                            solid: false,
                        });
                    }
                    if ui.button("Sort by value").clicked() {
                        table.stops.sort_by(|a, b| {
                            a.value
                                .partial_cmp(&b.value)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("💾 Save & apply").clicked() {
                        save_and_apply(table, moment, settings);
                    }
                    if ui.button("Import .pal…").clicked() {
                        crate::dialog::request_open(
                            crate::dialog::ImportKind::Palette,
                            EDITOR_TAG,
                        );
                    }
                    if ui.button("Export .pal…").clicked() {
                        export_pal(table);
                    }
                    if ui.button("Revert to default").clicked() {
                        *table = crate::colormap::default_table(moment).clone();
                    }
                });
            });
        self.open = open;
    }
}

/// A horizontal gradient sampled from the working table across `range` (data floor → ceiling).
fn preview_bar(ui: &mut egui::Ui, table: &ColorTable, range: (f32, f32)) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let (vmin, vmax) = range;
    let n = rect.width().max(1.0) as usize;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let v = vmin + t * (vmax - vmin);
        let col = table
            .sample(v)
            .map(|c| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
            .unwrap_or(egui::Color32::TRANSPARENT);
        let x = rect.left() + i as f32;
        p.vline(x, rect.y_range(), egui::Stroke::new(1.0, col));
    }
    p.rect_stroke(
        rect,
        3.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
}

fn color_edit(ui: &mut egui::Ui, rgba: &mut [u8; 4]) {
    let mut c = egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
    if ui.color_edit_button_srgba(&mut c).changed() {
        *rgba = [c.r(), c.g(), c.b(), c.a()];
    }
}

/// Write the working table into the colortables dir and set it as the moment's override.
fn save_and_apply(table: &ColorTable, moment: Moment, settings: &mut Settings) {
    let Some(dir) = Settings::colortables_dir() else {
        return;
    };
    let path = dir.join(format!("{}.pal", moment.short_name()));
    if let Err(e) = std::fs::write(&path, crate::colormap::to_pal_string(table)) {
        log::warn!("palette save failed: {e}");
        return;
    }
    // Setting the override triggers the app's dirty-diff palette reload.
    settings.palettes.insert(
        moment.short_name().to_string(),
        path.to_string_lossy().into_owned(),
    );
}

/// Parse a picked `.pal` into a table, or complain in the log and change nothing.
pub(crate) fn read_pal(path: &std::path::Path) -> Option<ColorTable> {
    match std::fs::read_to_string(path)
        .ok()
        .and_then(|t| crate::colormap::parse_pal(&t).ok())
    {
        Some(t) => Some(t),
        None => {
            log::warn!("could not parse {}", path.display());
            None
        }
    }
}

fn export_pal(table: &ColorTable) {
    let Some(path) = crate::dialog::save_path("palette.pal", "pal") else {
        return;
    };
    if let Err(e) = std::fs::write(&path, crate::colormap::to_pal_string(table)) {
        log::warn!("palette export failed: {e}");
    }
}
