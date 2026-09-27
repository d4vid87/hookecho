//! Clearview's compact controls reuse the existing panel, drawer and live timeline.
use super::*;
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(super) fn clearview_controls(&mut self, ctx: &egui::Context) {
        use egui_phosphor::regular as ph;
        let logo = crate::icon::texture(ctx, 64);
        let mut search = false;
        let mut settings = false;
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
        self.priority_dock(ctx);
        if search { self.apply_action(BindableAction::CommandSearch, ctx); }
        if settings { self.apply_palette(PaletteAction::OpenWindow(AppWindow::Settings), ctx); }
    }
}
