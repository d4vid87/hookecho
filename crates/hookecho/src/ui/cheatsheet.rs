//! The `?` keyboard cheat sheet.
//!
//! A dimmed full-screen overlay listing every binding in force, grouped the way the drawer groups
//! commands. It reads the live binding table, so a rebind shows up here without a second edit.

use crate::app::PaletteEntry;
use crate::hotkeys::{self, Binding, BindableAction};
use crate::ui::style;

/// Draw the sheet. Returns false when the user dismissed it.
pub(crate) fn show(
    ctx: &egui::Context,
    bindings: &[Binding],
    entries: &[PaletteEntry],
    accent: egui::Color32,
) -> bool {
    // Fades in; the same animation id drives the dim and the card so they arrive together.
    let t = ctx.animate_bool_with_time(egui::Id::new("cheatsheet_fade"), true, 0.12);
    let screen = ctx.content_rect();

    let mut open = true;
    egui::Area::new(egui::Id::new("cheatsheet"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.painter().rect_filled(
                screen,
                0.0,
                egui::Color32::from_black_alpha((150.0 * t) as u8),
            );
        });

    egui::Area::new(egui::Id::new("cheatsheet_card"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            style::glass(ui, (235.0 * t) as u8).show(ui, |ui| {
                ui.set_max_width((screen.width() - 80.0).min(720.0));
                ui.label(
                    egui::RichText::new("Keyboard shortcuts")
                        .size(style::FONT_LG)
                        .color(accent)
                        .strong(),
                );
                ui.add_space(6.0);
                columns(ui, ctx, bindings, entries);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "Ctrl + = / − / 0 resize the interface · Escape closes whatever is in \
                         front · rebind anything in Settings → Hotkeys",
                    )
                    .size(style::FONT_SM)
                    .weak(),
                );
            });
        });

    // Any key or click closes it — an overlay you have to aim at is worse than no overlay.
    if ctx.input(|i| {
        i.pointer.any_click()
            || i.key_pressed(egui::Key::Escape)
            || i.key_pressed(egui::Key::Questionmark)
    }) {
        open = false;
    }
    open
}

/// Two columns of `category → rows`, balanced by row count.
fn columns(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    bindings: &[Binding],
    entries: &[PaletteEntry],
) {
    let mut groups: Vec<(&'static str, Vec<(String, String)>)> = Vec::new();
    for b in bindings {
        let (cat, label) = match b.action {
            BindableAction::Palette(p) => match entries.iter().find(|e| e.action == p) {
                Some(e) => (e.category, e.label.clone()),
                None => continue,
            },
            other => ("App", hotkeys::label(other).unwrap_or("").to_string()),
        };
        let key = ctx.format_shortcut(&b.shortcut);
        match groups.iter_mut().find(|(c, _)| *c == cat) {
            Some((_, rows)) => rows.push((key, label)),
            None => groups.push((cat, vec![(key, label)])),
        }
    }

    let total: usize = groups.iter().map(|(_, r)| r.len() + 1).sum();
    let mut split = 0;
    let mut seen = 0;
    for (i, (_, rows)) in groups.iter().enumerate() {
        seen += rows.len() + 1;
        if seen * 2 >= total {
            split = i + 1;
            break;
        }
    }

    ui.columns(2, |cols| {
        for (i, (cat, rows)) in groups.iter().enumerate() {
            let ui = &mut cols[usize::from(i >= split)];
            ui.label(egui::RichText::new(*cat).size(style::FONT_SM).weak());
            for (key, label) in rows {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(key)
                            .monospace()
                            .size(style::FONT_SM)
                            .background_color(egui::Color32::from_white_alpha(18)),
                    );
                    ui.label(egui::RichText::new(label).size(style::FONT_BASE));
                });
            }
            ui.add_space(6.0);
        }
    });
}
