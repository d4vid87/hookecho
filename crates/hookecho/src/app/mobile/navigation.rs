//! Phone navigation. Data and actions stay in the shared app; this is its own presentation.

use egui::{vec2, Align2, Color32, RichText};
use egui_phosphor::regular as ph;

use super::super::{AppWindow, HookEchoApp, PaletteAction, PaletteEntry, PanelSection, ViewMode};
use crate::ui::a11y::Named as _;

impl HookEchoApp {
    pub(crate) fn mobile_navigation(&mut self, ctx: &egui::Context) {
        let site = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "Choose radar".into());
        let product = self.views[self.active].moment.short_name();
        let (freshness, freshness_color) =
            crate::ui::layers_panel::health_look(self.radar_health().state());
        let age = self.views[self.active].volume.as_ref().map(|scan| {
            (chrono::Utc::now() - scan.time).num_minutes().max(0)
        });
        let mut action = None;
        let mut mode = None;
        let mut product_anchor = None;
        let header = egui::Area::new("mobile_header".into())
            .order(egui::Order::Foreground)
            .anchor(
                Align2::LEFT_TOP,
                vec2(
                    self.chrome_rect.left() + 10.0,
                    self.chrome_rect.top() + 30.0,
                ),
            )
            .show(ctx, |ui| {
                ui.set_width((self.chrome_rect.width() - 20.0).max(200.0));
                ui.horizontal(|ui| {
                    let site_label = format!("{site} · {product}");
                    let radar = ui
                        .add_sized(
                            [ui.available_width() - 112.0, 48.0],
                            egui::Button::new(site_label),
                        )
                        .named("Open radar controls");
                    product_anchor = Some(radar.rect);
                    if radar.clicked() {
                        self.panel_section = PanelSection::Radar;
                        self.show_alert_panel = false;
                        self.panel_open = true;
                    }
                    if ui
                        .add_sized([48.0, 48.0], egui::Button::new(ph::MAGNIFYING_GLASS))
                        .named("Search places, sites, products, and tools")
                        .clicked()
                    {
                        self.panel_section = PanelSection::Tools;
                        self.show_alert_panel = false;
                        self.panel_open = true;
                        self.sidebar_focus_search = true;
                    }
                    if ui
                        .add_sized([48.0, 48.0], egui::Button::new(ph::MAP_PIN))
                        .named("Custom locations")
                        .clicked()
                    {
                        action = Some(PaletteAction::OpenWindow(AppWindow::Markers));
                    }
                });
                ui.colored_label(freshness_color, match age {
                    Some(minutes) => format!("● {freshness} · scan {minutes} min ago"),
                    None => format!("● {freshness} · waiting for scan"),
                });
                if self.analyst_open {
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized([108.0, 48.0], egui::Button::new("← Radar"))
                            .named("Back to Radar view")
                            .clicked()
                        {
                            mode = Some(ViewMode::Radar);
                        }
                        egui::ScrollArea::horizontal().show(ui, |ui| {
                            for i in 0..self.views.len() {
                                if ui
                                    .add_sized(
                                        [76.0, 48.0],
                                        egui::Button::new(format!("Pane {}", i + 1))
                                            .selected(i == self.active),
                                    )
                                    .named(
                                        [
                                            "Show analyst pane 1",
                                            "Show analyst pane 2",
                                            "Show analyst pane 3",
                                            "Show analyst pane 4",
                                        ].get(i).copied().unwrap_or("Show analyst pane"),
                                    )
                                    .clicked()
                                {
                                    self.active = i;
                                }
                            }
                        });
                    });
                }
            });
        self.mobile_occlusion.push(header.response.rect);
        self.tour_anchors.product = product_anchor;

        let bottom = self.chrome_rect.bottom() - 4.0;
        let destinations = egui::Area::new("mobile_destinations".into())
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(self.chrome_rect.left(), bottom - 68.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(22, 28, 36))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(53, 66, 80)))
                    .show(ui, |ui| {
                        ui.set_width(self.chrome_rect.width());
                        ui.horizontal(|ui| {
                            let (count, _) = self.alert_badge();
                            for (label, icon, section) in [
                                ("Radar".to_string(), ph::RADIO_BUTTON, PanelSection::Radar),
                                ("Layers".to_string(), ph::STACK, PanelSection::Overlays),
                                (format!("Alerts {count}"), ph::BELL, PanelSection::Alerts),
                                ("More".to_string(), ph::DOTS_THREE, PanelSection::Tools),
                            ] {
                                let selected = self.panel_open
                                    && self.panel_section == section
                                    && !self.sidebar_focus_search;
                                let button = egui::Button::new(
                                    RichText::new(format!("{icon}\n{label}")).color(if selected {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_gray(205)
                                    }),
                                )
                                .selected(selected)
                                .min_size(vec2((self.chrome_rect.width() - 28.0) / 4.0, 56.0));
                                let name = match section {
                                    PanelSection::Radar => "Radar controls",
                                    PanelSection::Overlays => "Layers",
                                    PanelSection::Alerts => "Alerts",
                                    PanelSection::Tools => "More",
                                };
                                if ui.add(button).named(name).clicked() {
                                    if selected {
                                        self.panel_open = false;
                                    } else {
                                        self.panel_section = section;
                                        self.show_alert_panel = section == PanelSection::Alerts;
                                        self.panel_open = true;
                                        self.sidebar_focus_search = false;
                                        ctx.data_mut(|data| data.remove::<Option<&'static str>>(egui::Id::new("panel_settings_page")));
                                    }
                                }
                            }
                        });
                    });
            });
        self.mobile_occlusion.push(destinations.response.rect);
        self.tour_anchors.menu = Some(destinations.response.rect);
        if let Some(action) = action {
            self.apply_palette(action, ctx);
        }
        if let Some(mode) = mode {
            self.switch_mode(mode, ctx);
        }
    }

    pub(crate) fn mobile_more(&mut self, ui: &mut egui::Ui, entries: &[PaletteEntry]) -> Option<PaletteAction> {
        let mut action = None;
        ui.spacing_mut().interact_size.y = 48.0;
        if ui
            .button(if self.analyst_open {
                "Analyst layout and presets"
            } else {
                "Open Analyst Workstation"
            })
            .clicked()
        {
            if !self.analyst_open {
                self.switch_mode(ViewMode::Analyst, ui.ctx());
            }
            self.panel_open = true;
            self.panel_section = PanelSection::Tools;
        }
        if self.analyst_open {
            ui.horizontal_wrapped(|ui| {
                for (i, label) in ["Tornado", "Hail", "Mesoscale", "Forecast"]
                    .iter()
                    .enumerate()
                {
                    if ui.button(*label).clicked() {
                        action = Some(PaletteAction::ApplyAnalystPreset(i as u8));
                    }
                }
            });
            ui.horizontal(|ui| {
                for n in [1, 2, 4] {
                    if ui
                        .selectable_label(self.views.len() == n, format!("{n} panes"))
                        .clicked()
                    {
                        self.set_pane_count(n);
                    }
                }
            });
            ui.checkbox(&mut self.link_cameras, "Link maps");
            ui.checkbox(&mut self.link_times, "Link time");
            ui.checkbox(&mut self.analyst_inspector_open, "Inspector");
        }
        ui.separator();
        for (label, window) in [
            ("Custom locations", AppWindow::Markers),
            ("Settings", AppWindow::Settings),
            ("Help", AppWindow::Help),
            ("Walkthrough", AppWindow::Tour),
        ] {
            if ui
                .add_sized([ui.available_width(), 48.0], egui::Button::new(label))
                .clicked()
            {
                action = Some(PaletteAction::OpenWindow(window));
            }
        }
        if ui
            .add_sized(
                [ui.available_width(), 48.0],
                egui::Button::new("Hide controls for full map"),
            )
            .clicked()
        {
            self.mobile_chrome_hidden = true;
            self.panel_open = false;
        }
        ui.separator();
        ui.label("Workspaces");
        if let Some(picked) = crate::ui::layers_panel::workspace_shortcuts(ui, entries) {
            action = Some(picked);
        }
        action
    }
}
