//! "Modern dark pro" egui styling: a tuned dark palette with a single accent, consistent
//! spacing/rounding, subtle window borders + shadow, and a slightly larger type scale.
//!
//! Applied every frame from [`crate::app`] (a cheap style clone) so it survives runtime theme
//! switches. `// ponytail: no bundled font — egui's default proportional face is fine; drop an
//! Inter/IBM-Plex TTF into data/fonts/ and install it here if a distinct face is wanted.`

use crate::settings::Theme;
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
        // Acid Storm — radar scope on energy drinks.
        Theme::AcidStorm => Palette {
            is_dark: true,
            bg: c(0x111a11),
            extreme: c(0x0a0f0a),
            faint: c(0x162216),
            stroke: c(0x2a3f2a),
            widget: c(0x162216),
            widget_hover: c(0x1f2f1f),
            text: c(0xe4ffe0),
            accent: c(0xaaff00),
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
        // Magma — lava glow, warm and loud.
        Theme::Magma => Palette {
            is_dark: true,
            bg: c(0x221310),
            extreme: c(0x160b09),
            faint: c(0x2e1a15),
            stroke: c(0x502a20),
            widget: c(0x2e1a15),
            widget_hover: c(0x3d221a),
            text: c(0xffe8d6),
            accent: c(0xff6b1a),
        },
        // Bubblegum — candy-shop light theme.
        Theme::Bubblegum => Palette {
            is_dark: false,
            bg: c(0xffe9f2),
            extreme: c(0xfff4f8),
            faint: c(0xffdcea),
            stroke: c(0xf5b8d2),
            widget: c(0xffd4e6),
            widget_hover: c(0xffc2da),
            text: c(0x3a1030),
            accent: c(0xff2d78),
        },
        // Riptide — electric cyan on deep ocean navy.
        Theme::Riptide => Palette {
            is_dark: true,
            bg: c(0x0a1a2a),
            extreme: c(0x06101a),
            faint: c(0x0e2236),
            stroke: c(0x1a4a66),
            widget: c(0x0e2236),
            widget_hover: c(0x132d46),
            text: c(0xd8f4ff),
            accent: c(0x00e5ff),
        },
        // Ultraviolet — electric purple under blacklight.
        Theme::Ultraviolet => Palette {
            is_dark: true,
            bg: c(0x190e2b),
            extreme: c(0x100821),
            faint: c(0x22133a),
            stroke: c(0x452a70),
            widget: c(0x22133a),
            widget_hover: c(0x2e1a4e),
            text: c(0xece0ff),
            accent: c(0xb44bff),
        },
        // Voltage — cyber yellow hi-vis HUD on graphite.
        Theme::Voltage => Palette {
            is_dark: true,
            bg: c(0x141418),
            extreme: c(0x0d0d10),
            faint: c(0x1c1c22),
            stroke: c(0x3a3a26),
            widget: c(0x1c1c22),
            widget_hover: c(0x26262e),
            text: c(0xf2f0dc),
            accent: c(0xffe600),
        },
        // Redline — signal crimson on ember maroon.
        Theme::Redline => Palette {
            is_dark: true,
            bg: c(0x1c0d11),
            extreme: c(0x120809),
            faint: c(0x29141a),
            stroke: c(0x55222e),
            widget: c(0x29141a),
            widget_hover: c(0x371b23),
            text: c(0xffdfe3),
            accent: c(0xff2b4a),
        },
        // Glacier — vivid azure on polar-morning frost (light).
        Theme::Glacier => Palette {
            is_dark: false,
            bg: c(0xe6f1fc),
            extreme: c(0xf2f8fe),
            faint: c(0xd9e8f8),
            stroke: c(0xa9c9e8),
            widget: c(0xd2e3f6),
            widget_hover: c(0xc2d9f2),
            text: c(0x0d2440),
            accent: c(0x0077ff),
        },
    }
}

/// The primary accent color for a theme (map markers, active-pane outline, status highlights).
pub fn accent(theme: Theme) -> Color32 {
    // system_dark doesn't affect any accent (Dark and Light share ACCENT), so pass true.
    palette(theme, true).accent
}

/// The background fill for a theme, for the settings swatch preview.
pub fn preview_bg(theme: Theme) -> Color32 {
    palette(theme, true).bg
}

pub fn apply(ctx: &egui::Context, theme: Theme, system_dark: bool) {
    let pal = palette(theme, system_dark);
    let mut visuals = if pal.is_dark { Visuals::dark() } else { Visuals::light() };
    tune(&mut visuals, &pal);

    let mut style = Style {
        visuals,
        ..Default::default()
    };

    // Spacing.
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.window_margin = Margin::same(10);
    style.spacing.menu_margin = Margin::same(8);

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
        (TextStyle::Heading, FontId::new(15.0, Proportional)),
        (TextStyle::Body, FontId::new(13.5, Proportional)),
        (TextStyle::Button, FontId::new(13.5, Proportional)),
        (TextStyle::Small, FontId::new(11.0, Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, egui::FontFamily::Monospace)),
    ]
    .into();

    let egui_theme = if pal.is_dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_style_of(egui_theme, style);
    ctx.options_mut(|o| {
        o.theme_preference = if pal.is_dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        }
    });
}

/// Generic tuning shared by every theme. Backgrounds/strokes/text come from the palette; the
/// accent drives active widgets, selection, links, and a subtle glow on hover + window edges.
fn tune(v: &mut Visuals, p: &Palette) {
    let a = p.accent;
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.extreme;
    v.faint_bg_color = p.faint;
    // Window/menu edges glow faintly in the theme accent.
    v.window_stroke = Stroke::new(1.0, if p.is_dark { p.stroke } else { p.stroke });
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
                ui.add(egui::Label::new(
                    egui::RichText::new(label.to_uppercase()).size(9.5).weak(),
                ).truncate());
                ui.label(egui::RichText::new(value).size(15.0).strong());
            });
        });
}

/// A min-max normalized sparkline of `vals` (oldest→newest) in a fixed-height row.
pub fn sparkline(ui: &mut egui::Ui, vals: &[f32], color: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width().min(300.0), 34.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    if vals.len() < 2 {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "no data",
            egui::FontId::proportional(10.0), ui.visuals().weak_text_color());
        return;
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
            let x = rect.left() + pad + (rect.width() - 2.0 * pad) * i as f32 / (vals.len() - 1) as f32;
            let y = rect.bottom() - pad - (rect.height() - 2.0 * pad) * (v - lo) / span;
            egui::pos2(x, y)
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
    let weak = ui.visuals().weak_text_color();
    painter.text(rect.right_top() + egui::vec2(-3.0, 1.0), egui::Align2::RIGHT_TOP,
        format!("{hi:.0}"), egui::FontId::proportional(9.0), weak);
    painter.text(rect.right_bottom() + egui::vec2(-3.0, -1.0), egui::Align2::RIGHT_BOTTOM,
        format!("{lo:.0}"), egui::FontId::proportional(9.0), weak);
}

/// A collapsible, accent-labelled toolbox section with consistent inner spacing.
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
