//! WSV3-style desktop chrome primitives.
//!
//! The look this reproduces: a docked ribbon of labelled control groups painted over a
//! navy→black vertical gradient, glossy stadium "pill" buttons for the mode switches, faint
//! hairline dividers between groups, a docked horizontal colour scale directly under the ribbon,
//! and a thin status bar along the bottom edge. It is the opposite trade from the minimal
//! chrome — the map gives back a strip of height in exchange for every control being visible at
//! once.
//!
//! Everything here is a free function: no `self`, no app state. `app/chrome/ribbon.rs` composes
//! these into the actual ribbon and wires them to `PaletteAction`s.

use crate::colormap::ColorTable;
use egui::{
    pos2, vec2, Align2, Color32, FontId, Mesh, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

/// Ribbon gradient endpoints — a blue-tinted charcoal at the top fading to near-black.
pub const RIBBON_TOP: Color32 = Color32::from_rgb(0x2b, 0x30, 0x3c);
pub const RIBBON_BOT: Color32 = Color32::from_rgb(0x0c, 0x0d, 0x11);
/// Group-header text: a letter-spaced light blue-grey, drawn small.
pub const HEADER: Color32 = Color32::from_rgb(0xa9, 0xb4, 0xcc);
/// Body text on the ribbon.
pub const INK: Color32 = Color32::from_rgb(0xd6, 0xdc, 0xe8);
/// The signature WSV3 blue used on active pills and highlighted list rows.
pub const WSV3_BLUE: Color32 = Color32::from_rgb(0x3d, 0x7b, 0xd6);
/// Unselected pill fill.
pub const PILL_BG: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x28);
/// Status bar.
pub const STATUS_BG: Color32 = Color32::from_rgb(0x09, 0x0a, 0x0d);
pub const STATUS_FG: Color32 = Color32::from_rgb(0x86, 0x8c, 0x9a);

/// Ribbon control-row height (the gradient band).
pub const RIBBON_H: f32 = 130.0;
/// Docked colour scale height, drawn immediately below the ribbon.
pub const COLORBAR_H: f32 = 26.0;
/// Bottom status bar height.
pub const STATUS_H: f32 = 22.0;
/// Right-edge keep-out for the window min/max/close buttons in this layout (they anchor 70 px in
/// from the edge and are ~100 px wide).
pub const WINDOW_BTN_KEEPOUT: f32 = 178.0;

/// Paint the vertical ribbon gradient across `rect` and a hard hairline along its bottom edge.
pub fn ribbon_gradient(painter: &egui::Painter, rect: Rect) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), RIBBON_TOP);
    mesh.colored_vertex(rect.right_top(), RIBBON_TOP);
    mesh.colored_vertex(rect.right_bottom(), RIBBON_BOT);
    mesh.colored_vertex(rect.left_bottom(), RIBBON_BOT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, Color32::from_black_alpha(170)),
    );
    painter.hline(
        rect.x_range(),
        rect.bottom() - 1.5,
        Stroke::new(1.0, Color32::from_white_alpha(10)),
    );
}

/// A glossy stadium pill. `selected` fills it with the accent; otherwise it is a dark chip with a
/// hairline ring. The returned [`Response`] senses clicks.
pub fn pill(ui: &mut Ui, label: &str, selected: bool, accent: Color32) -> Response {
    pill_sized(ui, label, selected, accent, 0.0)
}

/// [`pill`] with a minimum width (so a row of related pills lines up).
pub fn pill_sized(ui: &mut Ui, label: &str, selected: bool, accent: Color32, min_w: f32) -> Response {
    let font = FontId::proportional(13.0);
    let text_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let w = (text_w + 22.0).max(min_w);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 27.0), Sense::click());
    let p = ui.painter();
    let radius = rect.height() * 0.5;
    let base = if selected {
        accent
    } else if resp.hovered() {
        Color32::from_rgb(0x26, 0x2b, 0x38)
    } else {
        PILL_BG
    };
    p.rect_filled(rect, radius, base);
    // Gloss: a lighter wash over the top half.
    let top = Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y));
    p.rect_filled(
        top,
        radius,
        Color32::from_white_alpha(if selected { 26 } else { 12 }),
    );
    p.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0,
            if selected {
                accent
            } else {
                Color32::from_white_alpha(46)
            },
        ),
        StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        font,
        if selected { Color32::WHITE } else { INK },
    );
    resp
}

/// The small uppercase label that titles a ribbon group.
pub fn group_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(9.0)
            .color(HEADER)
            .strong(),
    );
    ui.add_space(2.0);
}

/// A full-height faint vertical divider between two ribbon groups.
pub fn vsep(ui: &mut Ui) {
    ui.add_space(7.0);
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, RIBBON_H - 14.0), Sense::hover());
    let x = rect.center().x;
    let p = ui.painter();
    let yr = egui::Rangef::new(rect.top() + 6.0, rect.bottom() - 6.0);
    p.vline(x - 0.5, yr, Stroke::new(1.0, Color32::from_black_alpha(150)));
    p.vline(x + 0.5, yr, Stroke::new(1.0, Color32::from_white_alpha(12)));
    ui.add_space(7.0);
}

/// A compact toggle used for the checkbox-style options in the ribbon (Smoothing, Map legend…).
/// Returns `true` on the frame it is clicked.
pub fn check(ui: &mut Ui, label: &str, on: &mut bool) -> bool {
    let font = FontId::proportional(12.0);
    let tw = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let (rect, resp) = ui.allocate_exact_size(vec2(tw + 20.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let p = ui.painter();
    let box_r = Rect::from_center_size(pos2(rect.left() + 7.0, rect.center().y), vec2(12.0, 12.0));
    p.rect_filled(box_r, 3.0, if *on { WSV3_BLUE } else { PILL_BG });
    p.rect_stroke(
        box_r,
        3.0,
        Stroke::new(1.0, Color32::from_white_alpha(60)),
        StrokeKind::Inside,
    );
    if *on {
        p.text(
            box_r.center(),
            Align2::CENTER_CENTER,
            "\u{2713}",
            FontId::proportional(11.0),
            Color32::WHITE,
        );
    }
    p.text(
        pos2(rect.left() + 17.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        INK,
    );
    resp.clicked()
}

/// The docked horizontal colour scale under the ribbon. Mirrors [`crate::ui::legend::draw_vertical`]
/// — same [`ColorTable`], so the bar and the radar LUT never diverge — but laid out left→right with
/// tick labels riding under the bar.
pub fn colorbar(
    painter: &egui::Painter,
    rect: Rect,
    table: &ColorTable,
    disp_factor: f32,
    disp_label: &str,
) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(0x05, 0x05, 0x07));
    let (vmin, vmax) = match (table.stops.first(), table.stops.last()) {
        (Some(a), Some(b)) if b.value > a.value => (a.value, b.value),
        _ => return,
    };
    let span = (vmax - vmin).max(f32::EPSILON);
    let bar = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top() + 3.0),
        pos2(rect.right() - 6.0, rect.top() + 14.0),
    );
    let x_of = |v: f32| bar.left() + ((v - vmin) / span).clamp(0.0, 1.0) * bar.width();
    let col = |c: [u8; 4]| Color32::from_rgb(c[0], c[1], c[2]);

    let mut mesh = Mesh::default();
    let mut quad = |x0: f32, x1: f32, c0: Color32, c1: Color32| {
        if x1 <= x0 {
            return;
        }
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x0, bar.top()), c0);
        mesh.colored_vertex(pos2(x1, bar.top()), c1);
        mesh.colored_vertex(pos2(x1, bar.bottom()), c1);
        mesh.colored_vertex(pos2(x0, bar.bottom()), c0);
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };
    for (i, s) in table.stops.iter().enumerate() {
        let x0 = x_of(s.value);
        match table.stops.get(i + 1) {
            Some(n) => {
                let x1 = x_of(n.value);
                if s.solid {
                    quad(x0, x1, col(s.rgba), col(s.rgba));
                } else {
                    quad(x0, x1, col(s.rgba), col(s.end.unwrap_or(n.rgba)));
                }
            }
            None => quad(x0 - 1.0, x0 + 1.0, col(s.rgba), col(s.rgba)),
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.rect_stroke(
        bar,
        0.0,
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        StrokeKind::Outside,
    );

    // Ticks: the table's own Step if it declared one, else eight even divisions.
    let step = table
        .step
        .filter(|s| *s > 0.0 && (vmax - vmin) / *s <= 24.0);
    let mut ticks: Vec<f32> = Vec::new();
    match step {
        Some(st) => {
            let mut v = (vmin / st).ceil() * st;
            while v <= vmax + 1e-3 {
                ticks.push(v);
                v += st;
            }
        }
        None => {
            for k in 0..=8 {
                ticks.push(vmin + span * k as f32 / 8.0);
            }
        }
    }
    let font = FontId::proportional(9.5);
    let mut last_x = f32::NEG_INFINITY;
    for v in ticks {
        let x = x_of(v);
        if x - last_x < 34.0 {
            continue;
        }
        last_x = x;
        painter.vline(
            x,
            egui::Rangef::new(bar.bottom(), bar.bottom() + 3.0),
            Stroke::new(1.0, Color32::from_white_alpha(90)),
        );
        painter.text(
            pos2(x, bar.bottom() + 4.0),
            Align2::CENTER_TOP,
            format!("{:.0}", v * disp_factor),
            font.clone(),
            STATUS_FG,
        );
    }
    if !disp_label.is_empty() {
        painter.text(
            pos2(rect.left() + 4.0, bar.center().y),
            Align2::LEFT_CENTER,
            disp_label.trim(),
            FontId::proportional(9.0),
            Color32::from_white_alpha(150),
        );
    }
}
