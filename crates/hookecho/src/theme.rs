//! "Modern dark pro" egui styling: a tuned dark palette with a single accent, consistent
//! spacing/rounding, subtle window borders + shadow, and a slightly larger type scale.
//!
//! Applied every frame from [`crate::app`] (a cheap style clone) so it survives runtime theme
//! switches. `// ponytail: no bundled font — egui's default proportional face is fine; drop an
//! Inter/IBM-Plex TTF into data/fonts/ and install it here if a distinct face is wanted.`

use crate::settings::Theme;
use crate::ui::m3::{self, Density};
use egui::{Color32, CornerRadius, Margin, Stroke, Style, Visuals};

/// Default accent (matches the built-in Dark theme). Used only as a fallback where the selected
/// theme isn't in scope; live UI accent comes from [`accent`] / `ui.visuals().hyperlink_color`.
pub const ACCENT: Color32 = Color32::from_rgb(77, 163, 255); // #4da3ff

fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// A full color palette for one theme. Dark/Light reproduce the original hardcoded tuning; the
/// five vibrant themes drive deep tinted backgrounds and high-chroma accents through the same
/// generic `tune`.
#[derive(Clone, Copy)]
struct Palette {
    is_dark: bool,
    /// Panel + window fill.
    bg: Color32,
    /// Deepest surface (plots, extreme_bg).
    extreme: Color32,
    /// Faint raised surface (stat cards).
    faint: Color32,
    /// Window / separator stroke.
    stroke: Color32,
    /// Inactive widget fill.
    widget: Color32,
    /// Hovered widget fill.
    widget_hover: Color32,
    /// Body text.
    text: Color32,
    /// Primary accent (selection, active widgets, links, map markers).
    accent: Color32,
}

fn palette(theme: Theme, system_dark: bool) -> Palette {
    match theme {
        Theme::System => {
            if system_dark {
                palette(Theme::Dark, true)
            } else {
                palette(Theme::Light, false)
            }
        }
        // Original Dark tuning, unchanged.
        Theme::Dark => Palette {
            is_dark: true,
            bg: c(0x14161b),
            extreme: c(0x0e1013),
            faint: c(0x1a1d23),
            stroke: c(0x2a2f38),
            widget: c(0x1c1f26),
            widget_hover: c(0x262b34),
            text: c(0xc8d0da),
            accent: ACCENT,
        },
        // Original Light tuning, unchanged (fills come from Visuals::light()).
        Theme::Light => Palette {
            is_dark: false,
            bg: c(0xf6f7f9),
            extreme: c(0xffffff),
            faint: c(0xeceef1),
            stroke: c(0xd0d3d8),
            widget: c(0xe8eaed),
            widget_hover: c(0xdfe2e6),
            text: c(0x1b1e24),
            accent: ACCENT,
        },
        // Synthwave — neon on violet night.
        Theme::Synthwave => Palette {
            is_dark: true,
            bg: c(0x1b1338),
            extreme: c(0x120b28),
            faint: c(0x241a4a),
            stroke: c(0x3a2a6e),
            widget: c(0x241a4a),
            widget_hover: c(0x322459),
            text: c(0xe8e2ff),
            accent: c(0xff3d9e),
        },
        // Aurora — northern lights over ice.
        Theme::Aurora => Palette {
            is_dark: true,
            bg: c(0x0f1a29),
            extreme: c(0x081119),
            faint: c(0x142235),
            stroke: c(0x27435c),
            widget: c(0x142235),
            widget_hover: c(0x1c2f45),
            text: c(0xd6f2ea),
            accent: c(0x3dffb0),
        },
        // High contrast — black, white and one saturated yellow, for low vision and for direct
        // sunlight, which is the same problem from a different direction. In this theme the
        // overlay and vector stroke widths are scaled via `overlay_stroke_scale` / `vector_stroke_scale`
        // and radar colormaps switch to the high-contrast alternates in `colormap.rs` (see
        // `high_contrast_alt_name`), so the information layers respond, not just the chrome.
        Theme::HighContrast => Palette {
            is_dark: true,
            bg: c(0x000000),
            extreme: c(0x000000),
            faint: c(0x101010),
            stroke: c(0xffffff),
            widget: c(0x1a1a1a),
            widget_hover: c(0x333333),
            text: c(0xffffff),
            accent: c(0xffe600),
        },
        // OLED black — Dark's chrome over a true-black background. On an OLED panel a #000000
        // pixel is an off pixel: less battery on a phone at 3am, and no backlight glow around
        // the map at night. Unlike High contrast this keeps the normal accent and stroke, so it
        // is the everyday dark theme rather than an accessibility mode.
        Theme::Oled => Palette {
            is_dark: true,
            bg: c(0x000000),
            extreme: c(0x000000),
            faint: c(0x0b0d10),
            stroke: c(0x23272f),
            widget: c(0x121418),
            widget_hover: c(0x1c1f25),
            text: c(0xc8d0da),
            accent: ACCENT,
        },
    }
}

/// The primary accent color for a theme (map markers, active-pane outline, status highlights).
pub fn accent(theme: Theme) -> Color32 {
    if let Some(c) = accent_override() {
        return c;
    }
    // system_dark doesn't affect any accent (Dark and Light share ACCENT), so pass true.
    palette(theme, true).accent
}

/// User accent override, packed as `0xFF_RR_GG_BB` (0 = none).
///
/// ponytail: a process-global instead of threading `&Settings` through 19 `accent()` call sites —
/// the accent is one app-wide value and `apply()` is the only writer. If accent ever becomes
/// per-window, pass it explicitly instead.
static ACCENT_OVERRIDE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn set_accent_override(rgb: Option<[u8; 3]>) {
    let packed = rgb.map_or(0, |[r, g, b]| {
        0xFF00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
    });
    ACCENT_OVERRIDE.store(packed, std::sync::atomic::Ordering::Relaxed);
}

fn accent_override() -> Option<Color32> {
    let p = ACCENT_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    (p != 0).then(|| Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8))
}

/// The background fill for a theme, for the settings swatch preview.
pub fn preview_bg(theme: Theme) -> Color32 {
    palette(theme, true).bg
}

pub fn apply(
    ctx: &egui::Context,
    theme: Theme,
    system_dark: bool,
    density: Density,
    accent_rgb: Option<[u8; 3]>,
) {
    set_accent_override(accent_rgb);
    let mut pal = palette(theme, system_dark);
    if let Some(c) = accent_override() {
        pal.accent = c;
    }
    let mut visuals = if pal.is_dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    tune(&mut visuals, &pal);

    let mut style = Style {
        visuals,
        ..Default::default()
    };

    // Spacing and type come from one token table: touch on Android, else the density the user
    // picked (`m3::COMFORT` / `m3::COMPACT`). No widget branches on density.
    let m = m3::metrics(density, cfg!(target_os = "android"));
    style.spacing.item_spacing = m.item_spacing;
    style.spacing.button_padding = m.button_padding;
    style.spacing.interact_size.y = m.interact_h;
    style.spacing.window_margin = Margin::same(m.window_margin);
    style.spacing.menu_margin = Margin::same(m.menu_margin);

    // Rounding on every widget state.
    let r = CornerRadius::same(6);
    for w in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        w.corner_radius = r;
    }
    style.visuals.window_corner_radius = CornerRadius::same(8);
    style.visuals.menu_corner_radius = CornerRadius::same(8);

    // Type scale (egui's default face; sizes only).
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(m.heading, Proportional)),
        (TextStyle::Body, FontId::new(m.body, Proportional)),
        (TextStyle::Button, FontId::new(m.button, Proportional)),
        (TextStyle::Small, FontId::new(m.small, Proportional)),
        (
            TextStyle::Monospace,
            FontId::new(m.mono, egui::FontFamily::Monospace),
        ),
    ]
    .into();

    let egui_theme = if pal.is_dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_style_of(egui_theme, style);
    ctx.options_mut(|o| {
        o.theme_preference = if pal.is_dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        }
    });
}

/// Does `theme` request the high-contrast information layers (thicker strokes, high-contrast ramps)?
pub fn is_high_contrast(theme: Theme) -> bool {
    matches!(theme, Theme::HighContrast)
}

/// Stroke-width multiplier for geographic overlay polygons (warnings, outlooks, etc.).
/// High contrast roughly doubles the outline so it stays legible in direct sunlight and for
/// low-vision users — the chrome uses `palette` instead, this is purely for map geometry.
pub fn overlay_stroke_scale(theme: Theme) -> f32 {
    if is_high_contrast(theme) { 2.2 } else { 1.0 }
}

/// Stroke-width multiplier for vector basemap strokes (roads, boundaries, waterways).
pub fn vector_stroke_scale(theme: Theme) -> f32 {
    if is_high_contrast(theme) { 1.8 } else { 1.0 }
}

/// Boost fill/stroke alphas for warning polygons under high contrast so the polygon remains
/// legible against both dark satellite imagery and bright radar. Returns `(fill_alpha, stroke_alpha)` 0..255.
pub fn warning_alpha_for(theme: Theme) -> (u8, u8) {
    if is_high_contrast(theme) {
        (90, 255)
    } else {
        (45, 235)
    }
}

/// Which built-in colormap alternate to prefer when `theme` is high contrast, if the user
/// hasn't chosen a custom `.pal`. `None` means keep the default. The alternates live in
/// `colormap.rs` (`REF-HC.pal`, `VEL-HC.pal`) and are exposed via `colormap::high_contrast_alt_name`.
pub fn high_contrast_alt_name(moment: wxdata::level2::Moment) -> Option<&'static str> {
    if moment == wxdata::level2::Moment::Reflectivity {
        Some("High contrast (reflectivity)")
    } else if moment == wxdata::level2::Moment::Velocity {
        Some("High contrast (velocity)")
    } else {
        None
    }
}

/// Generic tuning shared by every theme. Backgrounds/strokes/text come from the palette; the
/// accent drives active widgets, selection, links, and a subtle glow on hover + window edges.
fn tune(v: &mut Visuals, p: &Palette) {
    let a = p.accent;
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.extreme;
    v.faint_bg_color = p.faint;
    v.window_stroke = Stroke::new(1.0, p.stroke);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if p.is_dark { 120 } else { 60 }),
    };
    v.popup_shadow = v.window_shadow;

    v.widgets.noninteractive.bg_fill = p.bg;
    v.widgets.noninteractive.weak_bg_fill = p.bg;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text.gamma_multiply(0.85));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke.gamma_multiply(0.8));

    v.widgets.inactive.bg_fill = p.widget;
    v.widgets.inactive.weak_bg_fill = p.widget;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke);

    // Hover flashes the accent (the dopamine): a wash of accent over the hover fill.
    v.widgets.hovered.bg_fill = p.widget_hover;
    v.widgets.hovered.weak_bg_fill = p.widget_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, a.gamma_multiply(0.55));

    v.widgets.active.bg_fill = a.gamma_multiply(0.85);
    v.widgets.active.weak_bg_fill = a.gamma_multiply(0.85);
    v.widgets.active.fg_stroke = Stroke::new(1.0, if p.is_dark { Color32::WHITE } else { p.text });
    v.widgets.active.bg_stroke = Stroke::new(1.0, a);

    v.selection.bg_fill = a.gamma_multiply(if p.is_dark { 0.45 } else { 0.35 });
    v.selection.stroke = Stroke::new(1.0, a);
    v.hyperlink_color = a;
    v.override_text_color = Some(p.text);
}

/// A compact stat card: a faint rounded panel with a small weak label over a strong value.
/// Sized to a fixed width so several tile neatly in a `horizontal_wrapped` row.
pub fn stat_card(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(108.0);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(label.to_uppercase()).size(9.5).weak())
                        .truncate(),
                );
                ui.label(egui::RichText::new(value).size(15.0).strong());
            });
        });
}

/// A min-max normalized sparkline of `vals` (oldest→newest) in a fixed-height row.
pub fn sparkline(ui: &mut egui::Ui, vals: &[f32], color: Color32) {
    let size = egui::vec2(ui.available_width().min(300.0), 34.0);
    sparkline_sized(ui, vals, color, size);
}

/// [`sparkline`] at a caller-chosen size, for places with a row height to fit inside — a table
/// cell has ~20 px, where the default 34 would overlap its neighbours.
///
/// Returns the response so a caller can hang a tooltip on it.
pub fn sparkline_sized(
    ui: &mut egui::Ui,
    vals: &[f32],
    color: Color32,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    if vals.len() < 2 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no data",
            egui::FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );
        return response;
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in vals {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let span = (hi - lo).max(1e-3);
    let pad = 4.0;
    let pts: Vec<egui::Pos2> = vals
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x =
                rect.left() + pad + (rect.width() - 2.0 * pad) * i as f32 / (vals.len() - 1) as f32;
            let y = rect.bottom() - pad - (rect.height() - 2.0 * pad) * (v - lo) / span;
            egui::pos2(x, y)
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
    let weak = ui.visuals().weak_text_color();
    painter.text(
        rect.right_top() + egui::vec2(-3.0, 1.0),
        egui::Align2::RIGHT_TOP,
        format!("{hi:.0}"),
        egui::FontId::proportional(9.0),
        weak,
    );
    painter.text(
        rect.right_bottom() + egui::vec2(-3.0, -1.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{lo:.0}"),
        egui::FontId::proportional(9.0),
        weak,
    );
    response
}

/// A collapsible, accent-labelled section with consistent inner spacing.
pub fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let heading = egui::RichText::new(title.to_uppercase())
        .color(ui.visuals().hyperlink_color)
        .size(11.5)
        .strong();
    egui::CollapsingHeader::new(heading)
        .default_open(true)
        .show_unindented(ui, |ui| {
            ui.add_space(2.0);
            let r = add(ui);
            ui.add_space(4.0);
            r
        })
        .body_returned
}
