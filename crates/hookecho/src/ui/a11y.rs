//! Names for the chrome that has none.
//!
//! The floating chrome is icon-first by design: a stack glyph for the panel, a bell for alerts, a
//! caret for back. A screen reader handed that button reads out the private-use codepoint the
//! icon font happens to sit at, which is worse than silence. egui already carries an AccessKit
//! node per widget — it just has nothing to put in it unless the app says so.
//!
//! Every one of these buttons already has the words: they are in the tooltip. So the fix is one
//! call that spends the tooltip text twice, once for the pointer and once for the accessibility
//! tree, and the mechanical change is `.on_hover_text(x)` becoming `.named(x)`.
//!
//! ponytail: an extension trait on `Response` rather than wrappers around `Button`, because the
//! chrome builds its buttons half a dozen different ways (`square_btn`, bare `egui::Button`,
//! `selectable_label`) and they all end at a `Response`.

use egui::{Response, WidgetInfo, WidgetType};

pub trait Named {
    /// Name this control, for the pointer and for assistive technology alike.
    fn named(self, name: &str) -> Response;

    /// Same, for a control that is also on or off — a toggle in a control column, a tab.
    fn named_toggle(self, name: &str, on: bool) -> Response;
}

impl Named for Response {
    fn named(self, name: &str) -> Response {
        record_target(&self, name);
        let enabled = self.enabled();
        self.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
        self.on_hover_text(name.to_owned())
    }

    fn named_toggle(self, name: &str, on: bool) -> Response {
        record_target(&self, name);
        let enabled = self.enabled();
        self.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, on, name));
        self.on_hover_text(name.to_owned())
    }
}

/// Let browser checks locate real canvas controls without depending on their screen position.
/// The opt-in callback only reports geometry; input still goes through the normal pointer path.
pub fn record_target(response: &Response, name: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen(inline_js = "export function ui_target(name, pass, x, y, w, h, scale) { globalThis.__hookechoUiTarget?.(name, pass, x, y, w, h, scale); }")]
        extern "C" {
            fn ui_target(name: &str, pass: f64, x: f32, y: f32, w: f32, h: f32, scale: f32);
        }
        let rect = response.ctx.layer_transform_to_global(response.layer_id)
            .map_or(response.interact_rect, |transform| transform * response.interact_rect);
        let size = if response.enabled() && rect.is_positive() { rect.size() } else { egui::Vec2::ZERO };
        ui_target(name, response.ctx.cumulative_pass_nr() as f64,
            rect.min.x, rect.min.y, size.x, size.y, response.ctx.pixels_per_point());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (response, name);
}
