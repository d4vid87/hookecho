//! The Context bar reuses the existing action registry and panel state.
use super::*;
use crate::ui::a11y::Named as _;
pub(super) const CORNER_WIDTH: f32 = 330.0;
pub(super) const CORNER_RIGHT: f32 = 22.0;
pub(super) const CORNER_BOTTOM: f32 = 24.0;
fn select_section(open: bool, current: PanelSection, selected: PanelSection) -> bool {
    !open || current != selected
}
impl HookEchoApp {
    pub(super) fn clearview_controls(&mut self, ctx: &egui::Context) {
        use egui_phosphor::regular as ph;
        let logo = crate::icon::texture(ctx, 64);
        let (mut search, mut settings, mut selected) = (false, false, None);
        let current = if self.show_alert_panel {
            PanelSection::Alerts
        } else {
            self.panel_section
        };
        let toolbar = egui::Area::new("clearview_toolbar".into())
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 16.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .corner_radius(10.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Image::new(&logo).fit_to_exact_size(egui::vec2(24.0, 24.0)),
                            );
                            ui.label(egui::RichText::new("HookEcho").size(18.0).strong());
                            for (label, target) in [
                                ("Radar", PanelSection::Radar),
                                ("Layers", PanelSection::Overlays),
                                ("Alerts", PanelSection::Alerts),
                                ("Tools", PanelSection::Tools),
                            ] {
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .frame(false)
                                            .min_size(egui::vec2(0.0, 32.0))
                                            .selected(
                                                self.panel_open
                                                    && !self.drawer.is_open()
                                                    && current == target,
                                            ),
                                    )
                                    .named(label)
                                    .clicked()
                                {
                                    selected = Some(target);
                                }
                            }
                            search = ui
                                .add(
                                    egui::Button::new(format!("{} Search", ph::MAGNIFYING_GLASS))
                                        .min_size(egui::vec2(0.0, 32.0)),
                                )
                                .named("Search places, layers and tools")
                                .clicked();
                            settings = ui
                                .add_sized([32.0, 32.0], egui::Button::new(ph::SLIDERS_HORIZONTAL))
                                .named("Open settings")
                                .on_hover_text("Settings")
                                .clicked();
                        });
                    });
            });
        self.mobile_occlusion.push(toolbar.response.rect);
        self.tour_anchors.menu = Some(toolbar.response.rect);
        if let Some(target) = selected {
            self.panel_open =
                select_section(self.panel_open && !self.drawer.is_open(), current, target);
            self.panel_section = target;
            self.show_alert_panel = target == PanelSection::Alerts;
            self.layers_query.clear();
            self.sidebar_focus_search = false;
            self.drawer.show_search();
            ctx.data_mut(|d| {
                d.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page"));
                d.remove::<bool>(egui::Id::new("context_all_controls"));
            });
        }
        let modes = egui::Area::new("context_modes".into())
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(16.0, -CORNER_BOTTOM))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252)
                    .inner_margin(5)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_label(true, "Radar")
                                .on_hover_text("Single radar map");
                            if ui.selectable_label(false, "Analyst").clicked() {
                                self.switch_mode(ViewMode::Analyst, ctx);
                            }
                        });
                    });
            });
        self.mobile_occlusion.push(modes.response.rect);
        if search {
            self.apply_action(BindableAction::CommandSearch, ctx);
        }
        if settings {
            self.apply_palette(PaletteAction::OpenWindow(AppWindow::Settings), ctx);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_navigation_opens_switches_and_closes() {
        assert!(select_section(
            false,
            PanelSection::Radar,
            PanelSection::Radar
        ));
        assert!(select_section(
            true,
            PanelSection::Radar,
            PanelSection::Overlays
        ));
        assert!(!select_section(
            true,
            PanelSection::Overlays,
            PanelSection::Overlays
        ));
    }
}
