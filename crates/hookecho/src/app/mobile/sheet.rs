//! The modal bottom sheet: what the phone opens instead of a desktop panel.
//!
//! There used to be a persistent sheet here as well — a summary line the user dragged between
//! three snap points, with a five-slot toolbar docked along its bottom edge. It was M3's map-first
//! pattern done faithfully, and it cost the map a permanent strip of screen plus a second bar of
//! chrome to carry controls the shared registry already had. The floating chrome replaced both:
//! see [`crate::app::chrome::overlay`], which is now the phone's chrome too.
//!
//! What survives is the shell, because the phone still needs one thing the desktop does not — a
//! surface that comes up from the bottom edge, dims the map behind it, and is dismissed by
//! dragging it back down. The panels render into it.

use egui::{pos2, vec2, Align, Color32, Id, Layout, Rect, RichText, Sense, Stroke};

use egui_phosphor::regular as ph;

use crate::ui::m3;

/// The sheet's own surface color: the theme's panel fill lifted by an M3 surface tint so the sheet
/// reads as a layer above the map rather than a hole in it.
pub(crate) fn sheet_fill(ui: &egui::Ui) -> Color32 {
    let base = ui.visuals().panel_fill;
    let lift = if ui.visuals().dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, m3::SURF_MED)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, m3::SURF_LOW)
    };
    m3::blend(base.to_opaque(), lift)
}

/// A 48pt round icon button with an M3 state layer.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    glyph: &str,
    active: bool,
    accent: Color32,
) -> egui::Response {
    let fg = if active {
        accent
    } else {
        ui.visuals().text_color()
    };
    let resp = ui.add(
        egui::Button::new(RichText::new(glyph).size(m3::T_HEADLINE).color(fg))
            .min_size(vec2(m3::MIN_TARGET, m3::MIN_TARGET))
            .corner_radius(m3::R_FULL)
            .fill(if active {
                accent.gamma_multiply(0.18)
            } else {
                Color32::TRANSPARENT
            })
            .stroke(Stroke::NONE),
    );
    if resp.is_pointer_button_down_on() {
        ui.painter().circle_filled(
            resp.rect.center(),
            m3::MIN_TARGET / 2.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, m3::A_PRESSED),
        );
    }
    resp
}

/// A modal bottom sheet: scrim, drag handle, title bar with a close button, scrollable body.
///
/// This is what the phone opens instead of a desktop drawer. It is the same shape as the
/// persistent sheet (top corners, handle, surface tint) so the two read as one system, but it is
/// modal: the scrim dims the map and a tap outside dismisses it.
///
/// Returns the rect it covers, for gesture occlusion.
///
/// Drag the handle down to dismiss: past a quarter of the sheet's height it closes, otherwise it
/// springs back. The body keeps its own scroll drag, so only the handle and title strip move it.
///
/// ponytail: dismiss only, no snap ladder — these sheets are lists you came to read, not a
/// summary you peek at. Half-open states when one of them grows a map-adjacent preview.
pub(crate) fn modal_sheet<R>(
    ctx: &egui::Context,
    content: Rect,
    id: &str,
    title: &str,
    close: &mut bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Rect {
    let h = content.height() * 0.88;
    // Drag offset lives in egui's temp memory: it is per-sheet transient UI state with no
    // business in the app struct, and it dying with the sheet is the wanted behavior.
    let drag_id = Id::new((id, "drag_y"));
    let drag_y: f32 = ctx.memory(|m| m.data.get_temp(drag_id).unwrap_or(0.0));
    let rect = Rect::from_min_max(
        pos2(content.left(), content.bottom() - h + drag_y),
        pos2(content.right(), content.bottom() + drag_y),
    );

    // Scrim first, in its own layer under the sheet.
    egui::Area::new(Id::new(format!("{id}_scrim")))
        .order(egui::Order::Middle)
        .fixed_pos(content.min)
        .show(ctx, |ui| {
            let resp = ui.allocate_rect(content, Sense::click());
            ui.painter().rect_filled(
                content,
                egui::CornerRadius::ZERO,
                Color32::from_black_alpha(m3::A_SCRIM),
            );
            if resp.clicked()
                && !resp
                    .interact_pointer_pos()
                    .is_some_and(|p| rect.contains(p))
            {
                *close = true;
            }
        });

    egui::Area::new(Id::new(id))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            ui.allocate_rect(rect, Sense::click_and_drag());
            ui.set_clip_rect(rect);
            let painter = ui.painter().clone();
            painter.rect_filled(
                rect,
                egui::CornerRadius {
                    nw: m3::R_XL as u8,
                    ne: m3::R_XL as u8,
                    sw: 0,
                    se: 0,
                },
                sheet_fill(ui),
            );
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(rect.shrink2(vec2(m3::SP_4, 0.0))),
                |ui| {
                    ui.set_width(rect.width() - m3::SP_4 * 2.0);
                    let handle = m3::drag_handle(ui);
                    if handle.clicked() {
                        *close = true;
                    }
                    if handle.dragged() {
                        let dy = (drag_y + handle.drag_delta().y).max(0.0);
                        ctx.memory_mut(|m| m.data.insert_temp(drag_id, dy));
                    }
                    if handle.drag_stopped() {
                        // A quarter of the way down is a dismiss; anything less springs back.
                        if drag_y > h * 0.25 {
                            *close = true;
                        }
                        ctx.memory_mut(|m| m.data.insert_temp(drag_id, 0.0f32));
                    }
                    ui.horizontal(|ui| {
                        ui.set_height(m3::MIN_TARGET);
                        ui.label(
                            RichText::new(title)
                                .size(m3::T_TITLE)
                                .strong()
                                .color(ui.visuals().strong_text_color()),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let accent = ui.visuals().selection.bg_fill;
                            if icon_button(ui, ph::X, false, accent).clicked() {
                                *close = true;
                            }
                        });
                    });
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(rect.bottom() - ui.cursor().top() - m3::SP_2)
                        .show(ui, |ui| {
                            add(ui);
                            ui.add_space(m3::SP_6);
                        });
                },
            );
        });
    rect
}
