//! The docked toolbar and the top chip row.
//!
//! Five actions, no more: Material 3 caps a docked toolbar at five, and the old dock spent two of
//! its slots ("Layers" and "⋯ More") opening the same drawer. Play moved into the sheet summary
//! where the scrubber is, which freed a slot for the alert badge — the one control that actually
//! needs to be visible without opening anything.

use egui::{vec2, Align, Align2, Color32, Id, Layout, Rect, RichText, Stroke};
use egui_phosphor::regular as ph;

use super::sheet::icon_button;
use super::MobileSheet;
use crate::ui::m3;

impl crate::app::HookEchoApp {
    /// The toolbar row, docked along the bottom of the persistent sheet.
    ///
    /// It draws inside the sheet's own Area rather than one of its own: egui promotes a layer to
    /// the front when you interact with it, so a separate toolbar Area vanished behind the sheet
    /// the first time the sheet was dragged.
    pub(crate) fn mobile_toolbar(
        &mut self,
        parent: &mut egui::Ui,
        rect: Rect,
        alert_count: usize,
        max_esc: u8,
    ) {
        let accent = crate::theme::accent(self.settings.theme);
        parent.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            {
                ui.horizontal(|ui| {
                    ui.set_height(rect.height());
                    // No vertical gap anywhere in the row: the glyph is already 48pt and the
                    // caption has to fit under it inside the bar.
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let n = 5.0;
                    let slot = rect.width() / n;
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let sheet = self.mobile_sheet;
                    let slot_ui =
                        |ui: &mut egui::Ui, glyph: &str, label: &str, active: bool| -> bool {
                            ui.allocate_ui_with_layout(
                                vec2(slot, rect.height()),
                                Layout::top_down(Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    let clicked = icon_button(ui, glyph, active, accent).clicked();
                                    ui.label(RichText::new(label).size(m3::T_LABEL_SM).color(
                                        if active {
                                            accent
                                        } else {
                                            ui.visuals().weak_text_color()
                                        },
                                    ));
                                    clicked
                                },
                            )
                            .inner
                        };
                    if slot_ui(ui, ph::STACK, "Layers", sheet == MobileSheet::Menu) {
                        self.mobile_sheet = if sheet == MobileSheet::Menu {
                            MobileSheet::None
                        } else {
                            MobileSheet::Menu
                        };
                    }
                    if slot_ui(ui, ph::RADIO_BUTTON, "Site", self.site_dialog.is_some())
                        && self.site_dialog.is_none()
                    {
                        self.site_dialog = Some(Default::default());
                    }
                    if slot_ui(ui, ph::CUBE, "3D", self.show_3d) {
                        self.build_volume3d();
                    }
                    if slot_ui(ui, ph::CAMERA, "Capture", sheet == MobileSheet::Capture) {
                        self.mobile_sheet = if sheet == MobileSheet::Capture {
                            MobileSheet::None
                        } else {
                            MobileSheet::Capture
                        };
                    }
                    // The badge carries its own count, so it draws itself rather than going
                    // through `slot_ui`.
                    let on = sheet == MobileSheet::Alerts;
                    let (glyph, tint) = match (alert_count, max_esc) {
                        (0, _) => (ph::BELL.to_string(), None),
                        (n, e) if e >= 2 => (n.to_string(), Some(Color32::from_rgb(220, 40, 40))),
                        (n, _) => (n.to_string(), Some(super::sheet::LIVE_TINT)),
                    };
                    let clicked = ui
                        .allocate_ui_with_layout(
                            vec2(slot, rect.height()),
                            Layout::top_down(Align::Center),
                            |ui| {
                                let c = tint.unwrap_or(accent);
                                let hit = icon_button(ui, &glyph, on || tint.is_some(), c);
                                ui.label(
                                    RichText::new("Alerts").size(m3::T_LABEL_SM).color(
                                        tint.unwrap_or_else(|| ui.visuals().weak_text_color()),
                                    ),
                                );
                                hit.clicked()
                            },
                        )
                        .inner;
                    if clicked {
                        self.mobile_sheet = if on {
                            MobileSheet::None
                        } else {
                            MobileSheet::Alerts
                        };
                    }
                });
            }
        });
    }

    /// The slim chip row along the top edge: site + VCP on the left, the armed-tool status in the
    /// middle, the chrome-hide eye on the right. Everything else that used to live in the top bar
    /// is one tap into a sheet.
    pub(crate) fn mobile_top_chips(
        &mut self,
        ctx: &egui::Context,
        content: Rect,
        site: &str,
        vcp: &str,
    ) -> Rect {
        let accent = crate::theme::accent(self.settings.theme);
        // Clear the colorbar + field strips painted at the very top of the content rect, and in
        // landscape clear the side rail too.
        let y = content.top() + 26.0;
        let x = if m3::is_landscape(content) {
            m3::RAIL_W.min(content.width() * 0.45) + m3::SP_3
        } else {
            m3::SP_3
        };
        let mut occl = Rect::NOTHING;
        egui::Area::new(Id::new("m_chips"))
            .anchor(Align2::LEFT_TOP, vec2(x, y - content.top()))
            .show(ctx, |ui| {
                let resp = pill(ui, |ui| {
                    ui.label(
                        RichText::new(ph::MAGNIFYING_GLASS)
                            .size(m3::T_LABEL_LG)
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.label(
                        RichText::new(site)
                            .size(m3::T_TITLE_SM)
                            .strong()
                            .color(ui.visuals().strong_text_color()),
                    );
                    ui.label(
                        RichText::new(vcp)
                            .size(m3::T_LABEL_SM)
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                occl = resp.rect;
                if resp.interact(egui::Sense::click()).clicked() && self.site_dialog.is_none() {
                    self.site_dialog = Some(Default::default());
                }
            });

        // The armed-tool hint: the desktop says this in its status bar, which the phone hides, so
        // a two-point tool otherwise reads as "the first tap did nothing".
        let hint = match self.tool {
            crate::app::MapTool::Measure => Some("Tap two points to measure"),
            crate::app::MapTool::Marker => Some("Tap the map to drop a marker"),
            crate::app::MapTool::CrossSection => Some("Tap two points for a cross-section"),
            crate::app::MapTool::Sounding => Some("Tap a point for a sounding"),
            crate::app::MapTool::Climatology => Some("Tap a point for tornado climatology"),
            _ => None,
        };
        if let Some(text) = hint {
            egui::Area::new(Id::new("m_toolhint"))
                .anchor(Align2::CENTER_TOP, vec2(0.0, y - content.top() + 56.0))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(accent)
                        .corner_radius(m3::R_FULL)
                        .inner_margin(egui::Margin::symmetric(m3::SP_4 as i8, m3::SP_2 as i8))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(text)
                                    .size(m3::T_LABEL_LG)
                                    .strong()
                                    .color(Color32::BLACK),
                            );
                        });
                });
        }
        occl
    }
}

/// A rounded translucent container for a top chip.
fn pill<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::Response {
    egui::Frame::new()
        .fill(super::sheet::sheet_fill(ui).gamma_multiply(0.96))
        .corner_radius(m3::R_FULL)
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 22),
        ))
        .inner_margin(egui::Margin::symmetric(m3::SP_3 as i8, m3::SP_2 as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = m3::SP_2;
                add(ui);
            });
        })
        .response
}
