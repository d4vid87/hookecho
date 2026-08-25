//! The window's own frame, when the OS isn't drawing one.
//!
//! A title bar is a strip of the screen that shows a name nobody reads and three buttons everybody
//! knows the shape of. On a map-first app it is also the only thing standing between the map and
//! the top edge of the display, so it goes: the window is borderless and the three buttons float
//! in the chrome with the rest.
//!
//! What the OS frame did for free has to come back by hand — dragging the window, resizing it from
//! any edge, double-click to maximize — which is what this file is. All of it is `ViewportCommand`
//! and none of it works on a platform without a desktop window, so the whole thing is a no-op
//! whenever [`crate::os_decorated`] says the OS is drawing the frame after all.

use super::*;
use egui::{pos2, vec2, Rect, ResizeDirection, ViewportCommand};

/// How far in from each edge counts as "grab to resize". Winit's own hit-test uses about this;
/// smaller and the window becomes annoying to catch, larger and the map loses its edges.
const GRIP: f32 = 6.0;
/// The drag strip along the top edge, in place of a title bar.
const DRAG_H: f32 = 44.0;
/// Kept clear on the right of the drag strip for the pane's color scale, which lives at the very
/// edge and is interactive.
const LEGEND_KEEPOUT: f32 = 60.0;

impl HookEchoApp {
    /// Draw the parts of the window the OS is no longer drawing. Call before the rest of the
    /// chrome: the drag strip covers the top edge, and whatever is drawn after it — the search
    /// pill, the window buttons below — takes its own clicks back.
    pub(crate) fn window_frame(&mut self, ctx: &egui::Context) {
        if crate::os_decorated() {
            return;
        }
        drag_strip(ctx);
        self.window_buttons(ctx);
        resize_grips(ctx);
    }

    /// Minimize / maximize / close, floating above the control column they line up with.
    fn window_buttons(&mut self, ctx: &egui::Context) {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        egui::Area::new(egui::Id::new("window_buttons"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::RIGHT_TOP, vec2(-70.0, 8.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if btn(ui, egui_phosphor::regular::MINUS, "Minimize") {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }
                    let (glyph, hint) = if maximized {
                        (egui_phosphor::regular::CORNERS_IN, "Restore")
                    } else {
                        (egui_phosphor::regular::CORNERS_OUT, "Maximize")
                    };
                    if btn(ui, glyph, hint) {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    }
                    if btn(ui, egui_phosphor::regular::X, "Close") {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });
    }
}

/// One small glassy chrome button, sized for a row of three rather than the 44 px column.
fn btn(ui: &mut egui::Ui, glyph: &str, hint: &str) -> bool {
    let (r, g, b) = crate::ui::style::CARD_FILL;
    ui.add(
        egui::Button::new(
            egui::RichText::new(glyph)
                .size(crate::ui::style::FONT_BASE)
                .color(egui::Color32::from_gray(238)),
        )
        .min_size(vec2(28.0, 24.0))
        .fill(egui::Color32::from_rgba_unmultiplied(r, g, b, 200))
        .stroke(egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26),
        ))
        .corner_radius(crate::ui::style::RADIUS_SM),
    )
    .on_hover_text(hint)
    .clicked()
}

/// The invisible strip along the top edge that drags the window, and double-clicks to maximize.
fn drag_strip(ctx: &egui::Context) {
    let screen = ctx.viewport_rect();
    let rect = Rect::from_min_max(
        screen.min,
        pos2(screen.right() - LEGEND_KEEPOUT, screen.top() + DRAG_H),
    );
    let resp = egui::Area::new(egui::Id::new("window_drag"))
        .order(egui::Order::Background)
        .fixed_pos(rect.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.allocate_response(rect.size(), egui::Sense::click_and_drag())
        })
        .inner;
    if resp.drag_started() {
        // StartDrag hands the window to the compositor, which finishes the gesture itself. egui
        // never sees the rest of the drag, which is why this is `drag_started` and not `dragged`.
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        // And it never sees the button come up either, so without this egui stays convinced a
        // drag is in progress and eats the *next* press to end it — every second grab would do
        // nothing at all.
        ctx.stop_dragging();
    }
    if resp.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

/// Eight invisible grips — four edges, four corners — that resize the window.
fn resize_grips(ctx: &egui::Context) {
    let s = ctx.viewport_rect();
    // How far along each side the corner squares reach.
    const END: f32 = 2.0 * GRIP;
    // The corner squares own their own pixels: the edges stop `2 * GRIP` short of each end
    // rather than running the full side. Overlapping them and relying on draw order does not
    // work — the edge takes the click either way, and a diagonal drag quietly resizes one axis.
    let grips = [
        (
            ResizeDirection::North,
            Rect::from_min_size(
                s.left_top() + vec2(END, 0.0),
                vec2(s.width() - 2.0 * END, GRIP),
            ),
        ),
        (
            ResizeDirection::South,
            Rect::from_min_size(
                s.left_bottom() + vec2(END, -GRIP),
                vec2(s.width() - 2.0 * END, GRIP),
            ),
        ),
        (
            ResizeDirection::West,
            Rect::from_min_size(
                s.left_top() + vec2(0.0, END),
                vec2(GRIP, s.height() - 2.0 * END),
            ),
        ),
        (
            ResizeDirection::East,
            Rect::from_min_size(
                s.right_top() + vec2(-GRIP, END),
                vec2(GRIP, s.height() - 2.0 * END),
            ),
        ),
        (
            ResizeDirection::NorthWest,
            Rect::from_min_size(s.left_top(), vec2(2.0 * GRIP, 2.0 * GRIP)),
        ),
        (
            ResizeDirection::NorthEast,
            Rect::from_min_size(
                s.right_top() - vec2(2.0 * GRIP, 0.0),
                vec2(2.0 * GRIP, 2.0 * GRIP),
            ),
        ),
        (
            ResizeDirection::SouthWest,
            Rect::from_min_size(
                s.left_bottom() - vec2(0.0, 2.0 * GRIP),
                vec2(2.0 * GRIP, 2.0 * GRIP),
            ),
        ),
        (
            ResizeDirection::SouthEast,
            Rect::from_min_size(
                s.max - vec2(2.0 * GRIP, 2.0 * GRIP),
                vec2(2.0 * GRIP, 2.0 * GRIP),
            ),
        ),
    ];
    for (dir, rect) in grips {
        let resp = egui::Area::new(egui::Id::new(("window_grip", dir as u8)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                ui.allocate_response(rect.size(), egui::Sense::click_and_drag())
            })
            .inner;
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(cursor(dir));
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            // Same lost mouse-up as the drag strip, same cure.
            ctx.stop_dragging();
        }
    }
}

fn cursor(dir: ResizeDirection) -> egui::CursorIcon {
    match dir {
        ResizeDirection::North => egui::CursorIcon::ResizeNorth,
        ResizeDirection::South => egui::CursorIcon::ResizeSouth,
        ResizeDirection::East => egui::CursorIcon::ResizeEast,
        ResizeDirection::West => egui::CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => egui::CursorIcon::ResizeNorthEast,
        ResizeDirection::NorthWest => egui::CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthEast => egui::CursorIcon::ResizeSouthEast,
        ResizeDirection::SouthWest => egui::CursorIcon::ResizeSouthWest,
    }
}
