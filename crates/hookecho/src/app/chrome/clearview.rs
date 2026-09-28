//! Quick Launch reuses the existing action registry and focused panels.
use super::*;
use crate::ui::a11y::Named as _;
pub(super) const CORNER_WIDTH: f32 = 330.0;
pub(super) const CORNER_RIGHT: f32 = 22.0;
pub(super) const CORNER_BOTTOM: f32 = 24.0;
pub(super) const HOME_KEY: &str = "quick_launch_home";

fn launch_home_open(panel_open: bool, drawer_open: bool, home: bool) -> bool {
    !panel_open || drawer_open || !home
}

pub(super) fn launch_cards(ui: &mut egui::Ui) -> Option<PanelSection> {
    use egui_phosphor::regular as ph;
    let mut selected = None;
    let cards = [
        ("Radar", "Site & products", ph::BROADCAST, PanelSection::Radar, [44, 78, 133], [169, 207, 255]),
        ("Layers", "Weather on your map", ph::STACK, PanelSection::Overlays, [35, 80, 82], [157, 231, 219]),
        ("Alerts", "Warnings & watches", ph::WARNING, PanelSection::Alerts, [91, 65, 48], [255, 207, 161]),
        ("Tools", "Measure & inspect", ph::WRENCH, PanelSection::Tools, [68, 55, 96], [218, 195, 255]),
    ];
    let width = (ui.available_width() - 10.0) / 2.0;
    for pair in cards.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for &(label, subtitle, icon, section, fill, tint) in pair {
                let mut text = egui::text::LayoutJob::default();
                for (line, size, color) in [
                    (format!("{icon}\n"), 25.0, egui::Color32::from_rgb(tint[0], tint[1], tint[2])),
                    (format!("{label}\n"), 17.0, egui::Color32::WHITE),
                    (subtitle.to_owned(), 11.0, egui::Color32::from_rgb(218, 229, 239)),
                ] {
                    text.append(&line, 0.0, egui::TextFormat {
                        font_id: egui::FontId::proportional(size), color, ..Default::default()
                    });
                }
                if ui.add_sized([width, 112.0], egui::Button::new(text)
                    .fill(egui::Color32::from_rgb(fill[0], fill[1], fill[2]))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(tint[0], tint[1], tint[2]).gamma_multiply(0.5)))
                    .corner_radius(11.0))
                    .named(label).on_hover_text(subtitle).clicked() {
                    selected = Some(section);
                }
            }
        });
        ui.add_space(8.0);
    }
    selected
}

impl HookEchoApp {
    pub(super) fn clearview_controls(&mut self, ctx: &egui::Context) {
        use egui_phosphor::regular as ph;
        let logo = crate::icon::texture(ctx, 64);
        let home_id = egui::Id::new(HOME_KEY);
        if !self.panel_open { ctx.data_mut(|d| d.remove::<bool>(home_id)); }
        let home = ctx.data_mut(|d| d.get_temp::<bool>(home_id).unwrap_or(false));
        let (mut menu, mut search, mut settings, mut single, mut compare) = (false, false, false, false, false);
        let (mut mrms, mut spc, mut outlook_day) = (false, false, None);
        let toolbar = egui::Area::new("clearview_toolbar".into())
            .order(egui::Order::Foreground)
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 16.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .corner_radius(10.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(25.0, 25.0)));
                            ui.label(egui::RichText::new("HookEcho").size(18.0).strong());
                            menu = ui.add(egui::Button::new(egui::RichText::new(format!("{} Menu", ph::SQUARES_FOUR)).color(egui::Color32::WHITE))
                                .fill(egui::Color32::from_rgb(40, 83, 151))
                                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(117, 162, 225)))
                                .min_size(egui::vec2(0.0, 34.0)))
                                .named_toggle("Quick Launch menu", self.panel_open && home && !self.drawer.is_open()).clicked();
                            search = ui.add_sized([34.0, 34.0], egui::Button::new(ph::MAGNIFYING_GLASS))
                                .named("Search places, layers and tools").on_hover_text("Search").clicked();
                            settings = ui.add_sized([34.0, 34.0], egui::Button::new(ph::SLIDERS_HORIZONTAL))
                                .named("Open settings").on_hover_text("Settings").clicked();
                        });
                    });
            });
        self.mobile_occlusion.push(toolbar.response.rect);
        self.tour_anchors.menu = Some(toolbar.response.rect);
        let modes = egui::Area::new("context_modes".into())
            .order(egui::Order::Foreground)
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 76.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252).inner_margin(egui::Margin::same(3)).corner_radius(5.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.spacing_mut().button_padding = egui::vec2(6.0, 4.0);
                    ui.horizontal(|ui| {
                        single = ui.selectable_label(!self.analyst_open, "◉ Radar")
                            .named_toggle("Single radar map", !self.analyst_open).clicked();
                        compare = ui.selectable_label(self.analyst_open, format!("{} Four panes", ph::SQUARES_FOUR))
                            .named_toggle("Compare four radar panels", self.analyst_open).clicked();
                        ui.separator();
                        let mrms_on = self.views[self.active].fields_on.contains(&crate::render::FieldLayer::Mrms);
                        mrms = ui.selectable_label(mrms_on, egui::RichText::new("● MRMS").color(if mrms_on {
                            egui::Color32::from_rgb(125, 242, 213)
                        } else { egui::Color32::from_rgb(190, 206, 215) }))
                            .named_toggle("MRMS national mosaic", mrms_on).clicked();
                        let spc_on = self.filters.outlook_day != 0;
                        spc = ui.selectable_label(spc_on, egui::RichText::new("● SPC Outlook").color(if spc_on {
                            egui::Color32::from_rgb(249, 203, 115)
                        } else { egui::Color32::from_rgb(190, 206, 215) }))
                            .named_toggle("SPC Outlook", spc_on).clicked();
                        if spc_on {
                            ui.menu_button(format!("Day {} ▾", self.filters.outlook_day), |ui| {
                                for day in 1..=3 {
                                    if ui.selectable_label(self.filters.outlook_day == day, format!("Day {day}")).clicked() {
                                        outlook_day = Some(day);
                                        ui.close();
                                    }
                                }
                            });
                        }
                        if self.analyst_open {
                            ui.menu_button(format!("{} Options", ph::SLIDERS_HORIZONTAL), |ui| {
                                ui.label(format!("Active pane {}", self.active + 1));
                                ui.horizontal(|ui| {
                                    ui.label("Panes");
                                    for n in [1, 2, 4] {
                                        if ui.selectable_label(self.views.len() == n, n.to_string()).clicked() { self.set_pane_count(n); }
                                    }
                                });
                                ui.checkbox(&mut self.link_cameras, "Link map positions");
                                ui.checkbox(&mut self.link_times, "Link timelines");
                                ui.checkbox(&mut self.analyst_inspector_open, "Show inspector");
                                ui.separator();
                                ui.label("Comparison presets");
                                for (i, label) in ["Tornado", "Hail", "Mesoscale", "Forecast"].iter().enumerate() {
                                    if ui.button(*label).clicked() {
                                        self.apply_palette(PaletteAction::ApplyAnalystPreset(i as u8), ctx);
                                        ui.close();
                                    }
                                }
                            });
                        }
                    });
                });
            });
        self.mobile_occlusion.push(modes.response.rect);
        if menu {
            self.panel_open = launch_home_open(self.panel_open, self.drawer.is_open(), home);
            self.sidebar_focus_search = false;
            self.layers_query.clear();
            self.show_alert_panel = false;
            self.panel_section = PanelSection::Radar;
            self.drawer.show_search();
            ctx.data_mut(|d| {
                d.insert_temp(home_id, true);
                d.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page"));
                d.remove::<bool>(egui::Id::new("context_all_controls"));
            });
        }
        if single || compare {
            self.switch_mode(if single { ViewMode::Radar } else { ViewMode::Analyst }, ctx);
            if compare { self.set_pane_count(4); }
            self.panel_open = false;
        }
        if mrms { self.apply_palette(PaletteAction::ToggleField(crate::render::FieldLayer::Mrms), ctx); }
        if spc { self.apply_palette(PaletteAction::ToggleOutlook, ctx); }
        if let Some(day) = outlook_day { self.apply_palette(PaletteAction::SetOutlookDay(day), ctx); }
        if search || settings {
            ctx.data_mut(|d| d.remove::<bool>(home_id));
        }
        if search { self.apply_action(BindableAction::CommandSearch, ctx); }
        if settings { self.apply_palette(PaletteAction::OpenWindow(AppWindow::Settings), ctx); }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_launch_opens_from_a_task_or_drawer_and_closes_from_home() {
        assert!(launch_home_open(false, false, false));
        assert!(launch_home_open(false, false, true));
        assert!(launch_home_open(true, false, false));
        assert!(launch_home_open(true, true, true));
        assert!(!launch_home_open(true, false, true));
    }
}
