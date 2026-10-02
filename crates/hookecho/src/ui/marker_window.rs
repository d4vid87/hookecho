//! Quiet Dock for saved locations. Markers can also be dropped directly on the map.

use crate::settings::{Marker, Settings};
use egui::{Color32, RichText, Stroke, TextureHandle};
use std::collections::HashMap;

/// Texture cache keyed by uploaded marker icon filename.
pub type IconTextures = HashMap<String, Option<TextureHandle>>;
pub const ICON_D: f32 = 24.0;

const BG: Color32 = Color32::from_rgb(16, 36, 54);
const CARD: Color32 = Color32::from_rgb(25, 52, 74);
const BLUE: Color32 = Color32::from_rgb(40, 107, 195);
const BORDER: Color32 = Color32::from_rgb(66, 99, 122);
const MUTED: Color32 = Color32::from_rgb(163, 191, 209);

const ICONS: [(&str, &str, &str); 5] = [
    ("home", "Home", egui_phosphor::regular::HOUSE),
    ("work", "Work", egui_phosphor::regular::BRIEFCASE),
    ("car", "Car", egui_phosphor::regular::CAR),
    ("place", "Place", egui_phosphor::regular::MAP_PIN),
    ("favorite", "Favorite", egui_phosphor::regular::STAR),
];

pub fn builtin_icon(icon: Option<&str>, home: bool) -> Option<&'static str> {
    let key = icon.and_then(|s| s.strip_prefix("builtin:"));
    ICONS.iter().find(|(id, _, _)| Some(*id) == key).map(|(_, _, glyph)| *glyph)
        .or_else(|| (icon.is_none() && home).then_some(egui_phosphor::regular::HOUSE))
}

#[derive(Default)]
pub struct MarkerWindow {
    pub open: bool,
    pub query: String,
    pub searching: bool,
    pub status: Option<String>,
    pub removed: Option<usize>,
    pub focus: Option<usize>,
    pub pending_name: Option<String>,
    pub pending_icon: Option<String>,
    selected: Option<usize>,
    adding: bool,
    new_name: String,
    new_address: String,
    new_icon: String,
}

impl MarkerWindow {
    /// Return the address to geocode; the app adds and centers the resolved marker.
    #[must_use]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        icon_tex: &IconTextures,
        drawer: &mut crate::ui::drawer::Drawer,
        metric: bool,
    ) -> Option<String> {
        let mut open = self.open;
        let mut go = None;
        let mut make_home = None;
        self.removed = None;
        self.focus = None;
        let Some(window) = drawer.page_sized(
            ctx, "Location Markers", &mut open, false, 420.0,
            egui::Window::new("Location Markers"),
        ) else {
            self.open = open;
            return None;
        };
        window.show(ctx, |ui| {
            ui.set_width(ui.available_width());
            ui.scope(|ui| {
                ui.visuals_mut().widgets.noninteractive.bg_fill = BG;
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(29, 58, 80);
                ui.painter().rect_filled(ui.max_rect(), 10.0, BG);
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new("YOUR MAP / LOCATIONS").monospace().size(10.0).color(MUTED));
                    ui.label(RichText::new("Your places").size(24.0).strong().color(Color32::WHITE));
                    ui.label(RichText::new("Save the places you check most.").size(12.0).color(MUTED));
                    ui.add_space(15.0);
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 89.0).max(90.0);
                        let field = ui.add(egui::TextEdit::singleline(&mut self.query)
                            .hint_text("City, address, or place").desired_width(width));
                        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let clicked = ui.add_enabled(!self.searching, primary_button(if self.searching { "Searching…" } else { "Search" })).clicked();
                        if (clicked || entered) && !self.searching && !self.query.trim().is_empty() {
                            self.pending_name = None;
                            self.pending_icon = None;
                            go = Some(self.query.trim().to_string());
                        }
                    });
                    if let Some(status) = &self.status {
                        ui.label(RichText::new(status).size(11.0).color(MUTED));
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Saved places").size(12.0).strong().color(Color32::WHITE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{} markers", settings.markers.len())).size(11.0).color(MUTED));
                        });
                    });
                    ui.add_space(5.0);
                    if settings.markers.is_empty() {
                        egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, BORDER))
                            .corner_radius(10.0).inner_margin(egui::Margin::same(14))
                            .show(ui, |ui| { ui.label("No places saved yet. Search above or add a place below."); });
                    }
                    for (i, marker) in settings.markers.iter().enumerate() {
                        let selected = self.selected == Some(i);
                        let icon = builtin_icon(marker.icon.as_deref(), marker.home)
                            .unwrap_or(egui_phosphor::regular::MAP_PIN);
                        let fill = if selected { Color32::from_rgb(33, 76, 114) } else { CARD };
                        let card = egui::Frame::new().fill(fill)
                            .stroke(Stroke::new(1.0, if selected { Color32::from_rgb(145, 207, 255) } else { BORDER }))
                            .corner_radius(10.0).inner_margin(egui::Margin::symmetric(11, 9))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    egui::Frame::new().fill(Color32::from_rgb(36, 77, 117))
                                        .corner_radius(8.0).inner_margin(egui::Margin::symmetric(8, 6))
                                        .show(ui, |ui| {
                                            if let Some(tex) = marker.icon.as_ref().and_then(|n| icon_tex.get(n)).and_then(|t| t.as_ref()) {
                                                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(17.0, 17.0)));
                                            } else {
                                                ui.label(RichText::new(icon).size(17.0).color(Color32::from_rgb(158, 215, 255)));
                                            }
                                        });
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&marker.name).strong().color(Color32::WHITE));
                                        ui.label(RichText::new(format!("{:.4}, {:.4}", marker.lat, marker.lon)).size(11.0).color(MUTED));
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(egui_phosphor::regular::CARET_RIGHT).color(MUTED));
                                    });
                                });
                            });
                        if ui.interact(card.response.rect, ui.id().with(("marker_card", i)), egui::Sense::click()).clicked() {
                            self.selected = Some(i);
                            self.focus = Some(i);
                        }
                        ui.add_space(5.0);
                    }
                    ui.add_space(5.0);
                    if ui.add_sized([ui.available_width(), 35.0], primary_button(
                        if self.adding { "Adding a place" } else { "+ Add a place" }
                    )).clicked() {
                        self.adding = !self.adding;
                        self.selected = None;
                        self.new_name.clear();
                        self.new_address.clear();
                        self.new_icon = "place".into();
                    }
                    if self.adding {
                        ui.add_space(10.0);
                        egui::Frame::new().fill(BG).stroke(Stroke::new(1.0, BORDER))
                            .corner_radius(10.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new("Name this place").size(11.0).color(MUTED));
                                ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("e.g. Family cabin").desired_width(ui.available_width()));
                                ui.add_space(6.0);
                                ui.label(RichText::new("Address or place").size(11.0).color(MUTED));
                                ui.add(egui::TextEdit::singleline(&mut self.new_address).hint_text("City, address, or place").desired_width(ui.available_width()));
                                ui.add_space(8.0);
                                icon_choices(ui, &mut self.new_icon);
                                ui.add_space(8.0);
                                if ui.add_sized([ui.available_width(), 32.0], primary_button("Save marker")).clicked() {
                                    if self.new_address.trim().is_empty() {
                                        self.status = Some("Enter an address or place first.".into());
                                    } else if !self.searching {
                                        self.pending_name = (!self.new_name.trim().is_empty()).then(|| self.new_name.trim().to_string());
                                        self.pending_icon = Some(format!("builtin:{}", self.new_icon));
                                        go = Some(self.new_address.trim().to_string());
                                        self.adding = false;
                                    }
                                }
                            });
                    } else if let Some(i) = self.selected.filter(|i| *i < settings.markers.len()) {
                        ui.add_space(10.0);
                        let marker = &mut settings.markers[i];
                        if marker_editor(ui, marker, i, icon_tex, metric, &mut self.removed) {
                            make_home = Some(i);
                        }
                    } else {
                        ui.add_space(8.0);
                        ui.label(RichText::new("Choose a place to center the map, or add one with its own icon.").size(11.0).color(MUTED));
                    }
                });
            });
        });
        if let Some(i) = make_home {
            for (j, marker) in settings.markers.iter_mut().enumerate() {
                marker.home = i == j;
            }
        }
        if let Some(i) = self.removed {
            settings.markers.remove(i);
            self.selected = None;
        }
        self.open = open;
        go
    }
}

fn primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
        .fill(BLUE).stroke(Stroke::new(1.0, Color32::from_rgb(112, 178, 251)))
        .corner_radius(8.0)
}

fn icon_choices(ui: &mut egui::Ui, selected: &mut String) -> bool {
    let mut changed = false;
    ui.label(RichText::new("Marker icon").size(11.0).color(MUTED));
    ui.horizontal_wrapped(|ui| {
        for (id, label, glyph) in ICONS {
            let active = selected == id;
            if ui.add(egui::Button::new(format!("{glyph} {label}"))
                .fill(if active { BLUE } else { CARD })
                .stroke(Stroke::new(1.0, if active { Color32::from_rgb(154, 212, 255) } else { BORDER }))
                .corner_radius(8.0)).clicked() {
                *selected = id.into();
                changed = true;
            }
        }
    });
    changed
}

fn marker_editor(ui: &mut egui::Ui, marker: &mut Marker, i: usize, icon_tex: &IconTextures, metric: bool, remove: &mut Option<usize>) -> bool {
    let mut make_home = false;
    egui::Frame::new().fill(BG).stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("PLACE DETAILS").monospace().size(10.0).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut marker.name).desired_width(ui.available_width()));
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.label("Latitude");
                ui.add(egui::DragValue::new(&mut marker.lat).range(-90.0..=90.0).speed(0.01).max_decimals(4));
                ui.label("Longitude");
                ui.add(egui::DragValue::new(&mut marker.lon).range(-180.0..=180.0).speed(0.01).max_decimals(4));
            });
            let mut shown = if metric { marker.alert_radius_mi * crate::geo::KM_PER_MILE } else { marker.alert_radius_mi };
            let (max, suffix) = if metric { (320.0, " km") } else { (200.0, " mi") };
            ui.horizontal(|ui| {
                ui.label("Watch radius");
                if ui.add(egui::DragValue::new(&mut shown).range(0.0..=max).speed(1.0).max_decimals(0).suffix(suffix)).changed() {
                    marker.alert_radius_mi = if metric { shown / crate::geo::KM_PER_MILE } else { shown };
                }
            });
            if ui.radio(marker.home, "Use as Home for alerts").clicked() {
                make_home = true;
            }
            ui.label(RichText::new("Video stream URL").size(11.0).color(MUTED));
            ui.add(egui::TextEdit::singleline(&mut marker.video_url).hint_text("Optional HLS or MJPEG URL").desired_width(ui.available_width()));
            ui.add_space(7.0);
            let mut choice = marker.icon.as_deref().and_then(|v| v.strip_prefix("builtin:"))
                .unwrap_or(if marker.icon.is_some() { "" } else if marker.home { "home" } else { "place" }).to_string();
            if icon_choices(ui, &mut choice) {
                marker.icon = Some(format!("builtin:{choice}"));
            }
            if let Some(tex) = marker.icon.as_ref().and_then(|n| icon_tex.get(n)).and_then(|t| t.as_ref()) {
                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(20.0, 20.0)));
            }
            if !cfg!(target_arch = "wasm32") && ui.button("Upload custom icon…").clicked() {
                crate::dialog::request_open(crate::dialog::ImportKind::MarkerIcon, i.to_string());
            }
            ui.add_space(6.0);
            if ui.button(format!("{} Remove place", egui_phosphor::regular::TRASH)).clicked() {
                *remove = Some(i);
            }
        });
    make_home
}

/// Copy a picked PNG into the marker-icons dir and return the stored filename.
pub(crate) fn store_icon(src: &std::path::Path) -> Option<String> {
    let name = src.file_name()?.to_string_lossy().into_owned();
    let dir = Settings::marker_icons_dir()?;
    if let Err(e) = std::fs::copy(src, dir.join(&name)) {
        log::warn!("marker icon copy failed ({name}): {e}");
        return None;
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::builtin_icon;
    #[test]
    fn built_in_icons_and_home_fallback() {
        assert_eq!(builtin_icon(Some("builtin:car"), false), Some(egui_phosphor::regular::CAR));
        assert_eq!(builtin_icon(None, true), Some(egui_phosphor::regular::HOUSE));
        assert_eq!(builtin_icon(Some("photo.png"), false), None);
    }
}
