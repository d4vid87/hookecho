//! The one place the floating chrome gets its look and its position from.
//!
//! Desktop and mobile share the same glass-card language, so the frame builders live here rather
//! than in `app::mobile` (where desktop used to import them from). The `LANE_*` constants are the
//! other half: every floating surface anchors to a named lane instead of a hand-picked offset, so
//! two panels can't quietly land on top of each other.
//!
//! Two window idioms exist and both are fine:
//! - **Floating chrome** — `egui::Area` + [`glass`], anchored to a `LANE_*` offset. Map-first, no
//!   title bar, no drag. Everything in the always-on-screen layer uses this.
//! - **Tool window** — `crate::ui::fit_phone(egui::Window::new(..))`, titled, draggable, closable.
//!   Everything summoned on demand (soundings, settings, palette editor) uses this.

use egui::{vec2, Color32, Frame, Margin, RichText, Stroke};

/// RadarOmega's signature accents: orange for the active/live radar chrome, blue for actions.
pub const OMEGA_ORANGE: Color32 = Color32::from_rgb(0xF2, 0xA0, 0x33);
pub const OMEGA_BLUE: Color32 = Color32::from_rgb(0x2D, 0x9C, 0xDB);
/// Reserved for live/recording semantics only — the theme accent is the accent.
pub const OMEGA_GREEN: Color32 = Color32::from_rgb(0x3D, 0xD5, 0x6B);

/// Type scale for chrome text.
pub const FONT_SM: f32 = 11.0;
pub const FONT_BASE: f32 = 13.0;
pub const FONT_LG: f32 = 16.0;
pub const FONT_TITLE: f32 = 20.0;

/// Corner radii: `SM` for chips and buttons, `LG` for cards and panels.
pub const RADIUS_SM: f32 = 9.0;
pub const RADIUS_LG: f32 = 18.0;

/// Fill shared by every glass card, before the per-call alpha.
pub const CARD_FILL: (u8, u8, u8) = (12, 14, 18);

// ---------- Anchor lanes ----------
// Top edge, CENTER_TOP: banners sit above the search pill.
pub const LANE_TOP_BANNER: f32 = 8.0;
pub const LANE_TOP_SEARCH: f32 = 44.0;
// Right edge, RIGHT_TOP: the control column is outermost, panels open inboard of it, and
// status badges stack below both.
pub const LANE_RIGHT_CONTROLS: egui::Vec2 = vec2(-14.0, 44.0);
pub const LANE_RIGHT_PANEL: egui::Vec2 = vec2(-74.0, 44.0);
pub const LANE_RIGHT_BADGE_X: f32 = -14.0;
/// First free y below a control column of `buttons` 44 px buttons.
pub fn lane_right_badge_y(buttons: usize) -> f32 {
    LANE_RIGHT_CONTROLS.y + buttons as f32 * 50.0 + 8.0
}
// Bottom edge.
pub const LANE_BOTTOM_TIMELINE: f32 = -34.0;
pub const LANE_BOTTOM_CHIP: f32 = -8.0;
/// Bottom-left product pill, and the chase HUD stacked above it.
pub const LANE_BOTTOM_PRODUCT: f32 = -34.0;
pub const LANE_BOTTOM_CHASE: f32 = -92.0;

/// Translucent near-black card used by the floating bars.
pub fn glass(alpha: u8) -> Frame {
    let (r, g, b) = CARD_FILL;
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(r, g, b, alpha))
        .corner_radius(RADIUS_LG)
        .inner_margin(Margin::symmetric(12, 9))
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 22),
        ))
}

/// A ~44px rounded-square chrome button holding one Phosphor glyph.
pub fn square_btn(ui: &mut egui::Ui, glyph: &str, active: bool, accent: Color32) -> egui::Response {
    let (fg, bg) = if active {
        (Color32::BLACK, accent)
    } else {
        (
            Color32::from_gray(232),
            Color32::from_rgba_unmultiplied(255, 255, 255, 20),
        )
    };
    ui.add(
        egui::Button::new(RichText::new(glyph).size(FONT_TITLE).color(fg))
            .min_size(vec2(44.0, 44.0))
            .fill(bg)
            .corner_radius(13.0),
    )
}
