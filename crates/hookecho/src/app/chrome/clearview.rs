//! Clearview's compact controls reuse the existing panel, drawer and live timeline.
use super::*;
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(super) fn clearview_controls(&mut self, ctx: &egui::Context) {
        use egui_phosphor::regular as ph;
        let logo = crate::icon::texture(ctx, 64);
        let mut search = false;
        let mut settings = false;
        let mut layers = false;
        let mut alerts = false;
        let toolbar = egui::Area::new("clearview_toolbar".into())
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 16.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .corner_radius(13.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.horizontal(|ui| {
                            ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(28.0, 28.0)));
                            ui.label(egui::RichText::new("HookEcho").size(20.0).strong());
                            search = ui.add_sized(egui::vec2(146.0, 36.0), egui::Button::new(
                                format!("{}  Search a place", ph::MAGNIFYING_GLASS)))
                                .named("Search places, layers and tools").clicked();
                            settings = ui.add_sized(egui::vec2(36.0, 36.0), egui::Button::new(ph::SLIDERS_HORIZONTAL))
                                .named("Open settings").on_hover_text("Settings").clicked();
                        });
                    });
            });
        self.mobile_occlusion.push(toolbar.response.rect);
        self.tour_anchors.menu = Some(toolbar.response.rect);
        let dock = egui::Area::new("clearview_tools".into())
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -140.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 252)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .corner_radius(24.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            layers = ui.add_sized(egui::vec2(84.0, 34.0), egui::Button::new(format!("{}  Layers", ph::STACK)))
                                .named_toggle("Layers", self.panel_open && !self.show_alert_panel).clicked();
                            let (count, _) = self.alert_badge();
                            let label = if count > 0 { format!("{}  Alerts ({count})", ph::WARNING) } else { format!("{}  Alerts", ph::WARNING) };
                            alerts = ui.add_sized(egui::vec2(84.0, 34.0), egui::Button::new(label))
                                .named_toggle("Nearby alerts", self.panel_open && self.show_alert_panel).clicked();
                            settings |= ui.add_sized(egui::vec2(90.0, 34.0), egui::Button::new(format!("{}  Settings", ph::SLIDERS_HORIZONTAL)))
                                .named("Open settings").clicked();
                        });
                    });
            });
        self.mobile_occlusion.push(dock.response.rect);
        if search { self.apply_action(BindableAction::CommandSearch, ctx); }
        if layers {
            self.panel_open = !(self.panel_open && !self.show_alert_panel);
            self.show_alert_panel = false;
            self.panel_section = PanelSection::Overlays;
            ctx.data_mut(|d| d.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page")));
        }
        if alerts { self.apply_action(BindableAction::ToggleAlertPanel, ctx); }
        if settings { self.apply_palette(PaletteAction::OpenWindow(AppWindow::Settings), ctx); }
    }
}
