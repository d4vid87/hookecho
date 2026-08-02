//! Material 3 design tokens for the mobile chrome.
//!
//! Spacing, shape, type, motion and state-layer values in one place so the phone UI stops
//! hand-picking numbers. Colors deliberately do NOT live here — they come from the active theme
//! (`theme::apply` puts the palette in `ui.visuals()`, and `style::glass` builds the card fill), so
//! all 13 themes keep working instead of fighting a second palette.
//!
//! Compiles everywhere; only the mobile code calls it.

use egui::{vec2, Color32, Response, RichText, Sense, Stroke, Vec2};

// ---------- Spacing: the 4dp grid ----------
pub const SP_1: f32 = 4.0;
pub const SP_2: f32 = 8.0;
pub const SP_3: f32 = 12.0;
pub const SP_4: f32 = 16.0;
pub const SP_6: f32 = 24.0;

// ---------- Shape ----------
pub const R_XS: f32 = 4.0;
pub const R_SM: f32 = 8.0;
pub const R_MD: f32 = 12.0;
pub const R_LG: f32 = 16.0;
/// Sheet top corners and the extra-large containers M3 Expressive leans on.
pub const R_XL: f32 = 28.0;
pub const R_FULL: f32 = 9999.0;

// ---------- Type scale ----------
// Display is omitted: nothing on this surface wants 45pt.
pub const T_HEADLINE: f32 = 24.0;
pub const T_TITLE: f32 = 20.0;
pub const T_TITLE_SM: f32 = 16.0;
pub const T_BODY_LG: f32 = 16.0;
pub const T_BODY: f32 = 14.0;
pub const T_LABEL_LG: f32 = 14.0;
pub const T_LABEL: f32 = 12.0;
pub const T_LABEL_SM: f32 = 11.0;

// ---------- Touch + component metrics ----------
/// M3's minimum touch target. Anything tappable is at least this tall.
pub const MIN_TARGET: f32 = 48.0;
// 68, not M3's bare 64: these slots carry a caption under the glyph, and ten unlabeled icons
// would be a guessing game.
pub const TOOLBAR_H: f32 = 68.0;
pub const CHIP_H: f32 = 32.0;
pub const ROW_H: f32 = 56.0;
pub const SHEET_HANDLE: Vec2 = vec2(32.0, 4.0);
/// Handle + one 48pt row + padding. Excludes the bottom safe-area inset.
pub const SHEET_PEEK: f32 = 88.0;
/// Landscape side-rail width.
pub const RAIL_W: f32 = 340.0;

// ---------- State layers (M3 percentages as u8 alpha) ----------
pub const A_HOVER: u8 = 20;
pub const A_PRESSED: u8 = 31;
pub const A_SELECTED: u8 = 41;
pub const A_SCRIM: u8 = 82;
pub const SURF_LOW: u8 = 13;
pub const SURF_MED: u8 = 20;
pub const SURF_HIGH: u8 = 28;

// ---------- Motion ----------
pub const DUR_SHORT: f32 = 0.15;
pub const DUR_MED: f32 = 0.25;
pub const DUR_LONG: f32 = 0.40;

/// MD3 window size classes.
///
/// ponytail: only `Compact` has layouts today; medium/expanded map to compact until tablets
/// matter. The seam exists so adding them later is a match arm, not a rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthClass {
    Compact,
    Medium,
    Expanded,
}

pub fn width_class(w_pts: f32) -> WidthClass {
    if w_pts < 600.0 {
        WidthClass::Compact
    } else if w_pts < 840.0 {
        WidthClass::Medium
    } else {
        WidthClass::Expanded
    }
}

/// Landscape drives the sheet→rail switch. A square-ish window counts as portrait.
pub fn is_landscape(r: egui::Rect) -> bool {
    r.width() > r.height() * 1.15
}

/// Overlay the M3 state layer for a widget's current interaction state.
pub fn state_layer(base: Color32, resp: &Response) -> Color32 {
    let a = if resp.is_pointer_button_down_on() {
        A_PRESSED
    } else if resp.hovered() {
        A_HOVER
    } else {
        return base;
    };
    let wash = if base.r() as u32 + base.g() as u32 + base.b() as u32 > 384 {
        Color32::from_rgba_unmultiplied(0, 0, 0, a)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, a)
    };
    blend(base, wash)
}

/// Source-over blend of `over` onto opaque `base`.
pub fn blend(base: Color32, over: Color32) -> Color32 {
    let a = over.a() as f32 / 255.0;
    let mix = |b: u8, o: u8| (b as f32 * (1.0 - a) + o as f32 * a) as u8;
    Color32::from_rgba_unmultiplied(
        mix(base.r(), over.r()),
        mix(base.g(), over.g()),
        mix(base.b(), over.b()),
        base.a(),
    )
}

/// The 32×4 drag pill at the top of a bottom sheet, in a full-width 29pt hit strip.
///
/// Returns the response for the whole strip so the caller can drag the sheet from it — and it
/// senses clicks too, because a tap on the handle is how M3 expects you to step through the snap
/// points without dragging anything.
pub fn drag_handle(ui: &mut egui::Ui) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, MIN_TARGET * 0.6), Sense::click_and_drag());
    let c = ui.visuals().weak_text_color();
    let pill = egui::Rect::from_center_size(rect.center(), SHEET_HANDLE);
    ui.painter()
        .rect_filled(pill, SHEET_HANDLE.y / 2.0, c.gamma_multiply(0.8));
    resp
}

/// An M3 assist/filter chip.
pub fn chip(ui: &mut egui::Ui, text: &str, selected: bool) -> Response {
    let accent = ui.visuals().selection.bg_fill;
    let (fg, bg) = if selected {
        (
            ui.visuals().strong_text_color(),
            accent.gamma_multiply(0.45),
        )
    } else {
        (ui.visuals().text_color(), Color32::TRANSPARENT)
    };
    let resp = ui.add(
        egui::Button::new(RichText::new(text).size(T_LABEL_LG).color(fg))
            .min_size(vec2(0.0, CHIP_H))
            .corner_radius(R_SM)
            .fill(bg)
            .stroke(Stroke::new(
                1.0,
                ui.visuals().widgets.inactive.bg_stroke.color,
            )),
    );
    resp
}

/// A 56pt two-line M3 list row: leading glyph, title, optional supporting text.
pub fn list_row(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    sub: Option<&str>,
    selected: bool,
) -> Response {
    let w = ui.available_width();
    let h = if sub.is_some() { ROW_H } else { MIN_TARGET };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let vis = ui.visuals();
    let accent = vis.selection.bg_fill;
    let base = if selected {
        accent.gamma_multiply(A_SELECTED as f32 / 255.0)
    } else {
        Color32::TRANSPARENT
    };
    let fill = if resp.is_pointer_button_down_on() {
        blend(
            base,
            Color32::from_rgba_unmultiplied(255, 255, 255, A_PRESSED),
        )
    } else {
        base
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, R_MD, fill);
    }
    let text_c = if selected {
        vis.strong_text_color()
    } else {
        vis.text_color()
    };
    let weak = vis.weak_text_color();
    let p = ui.painter();
    let mut x = rect.left() + SP_4;
    if !icon.is_empty() {
        p.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(T_TITLE),
            text_c,
        );
        x += SP_6 + SP_2;
    }
    match sub {
        Some(s) => {
            p.text(
                egui::pos2(x, rect.center().y - 9.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(T_BODY_LG),
                text_c,
            );
            p.text(
                egui::pos2(x, rect.center().y + 10.0),
                egui::Align2::LEFT_CENTER,
                s,
                egui::FontId::proportional(T_LABEL),
                weak,
            );
        }
        None => {
            p.text(
                egui::pos2(x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(T_BODY_LG),
                text_c,
            );
        }
    }
    resp
}

/// Nearest snap for a sheet drag, biased by fling velocity.
///
/// `pos` and the snaps are sheet *heights* in points; positive `vel` is a downward drag (which
/// shrinks the sheet). A fling faster than 500 pt/s wins over proximity.
pub fn snap_to(pos: f32, vel: f32, snaps: &[f32]) -> f32 {
    debug_assert!(!snaps.is_empty());
    if vel.abs() > 500.0 {
        // Downward fling → the next smaller snap; upward → the next larger. The 24pt deadband
        // stops a fling that starts within a hair of a snap from "moving" to that same snap:
        // flinging down at Half must reach Peek, not sit back down on Half.
        const NEAR: f32 = 24.0;
        let mut best: Option<f32> = None;
        for &s in snaps {
            let ok = if vel > 0.0 {
                s < pos - NEAR
            } else {
                s > pos + NEAR
            };
            if ok && best.is_none_or(|b| (s - pos).abs() < (b - pos).abs()) {
                best = Some(s);
            }
        }
        if let Some(b) = best {
            return b;
        }
    }
    snaps.iter().copied().fold(snaps[0], |a, s| {
        if (s - pos).abs() < (a - pos).abs() {
            s
        } else {
            a
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_class_breakpoints() {
        assert_eq!(width_class(360.0), WidthClass::Compact);
        assert_eq!(width_class(599.9), WidthClass::Compact);
        assert_eq!(width_class(600.0), WidthClass::Medium);
        assert_eq!(width_class(839.9), WidthClass::Medium);
        assert_eq!(width_class(840.0), WidthClass::Expanded);
    }

    #[test]
    fn snaps_pick_nearest_when_slow() {
        let s = [88.0, 400.0, 760.0];
        assert_eq!(snap_to(120.0, 0.0, &s), 88.0);
        assert_eq!(snap_to(360.0, 10.0, &s), 400.0);
        assert_eq!(snap_to(700.0, -20.0, &s), 760.0);
    }

    #[test]
    fn fling_beats_proximity() {
        let s = [88.0, 400.0, 760.0];
        // Sitting near Half but flung down hard → collapse to Peek.
        assert_eq!(snap_to(410.0, 1200.0, &s), 88.0);
        // Flung up from Peek → Half, not straight to Full.
        assert_eq!(snap_to(90.0, -1200.0, &s), 400.0);
        // Flung down from the bottom snap has nowhere to go.
        assert_eq!(snap_to(88.0, 1200.0, &s), 88.0);
    }

    #[test]
    fn landscape_needs_a_real_aspect_change() {
        let port = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(360.0, 800.0));
        let land = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 360.0));
        let square = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 380.0));
        assert!(!is_landscape(port));
        assert!(is_landscape(land));
        assert!(!is_landscape(square));
    }
}
