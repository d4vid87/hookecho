//! The Layers panel: a searchable, categorized list of every layer/product/tool in the app,
//! rendered as big full-width toggle rows. Desktop shows it as a right-edge slide-in; Android
//! hosts the same body in a bottom sheet. Both read the one action registry
//! (`HookEchoApp::palette_entries`), so they can never drift apart.

use crate::app::{PaletteAction, PaletteEntry};
use egui::{vec2, Align, Color32, Layout, RichText, Stroke};

/// Category order in the panel (anything else falls to the bottom, in registry order).
pub(crate) const CATEGORIES: [&str; 7] =
    ["Radar", "National", "Severe", "Obs", "Models", "Reference", "Tools"];

/// Case-insensitive subsequence match with a compactness score: lower is a tighter match.
/// `None` = no match. Empty needle matches everything at score 0.
pub(crate) fn fuzzy(needle: &str, hay: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = hay.to_lowercase().chars().collect();
    let mut score = 0usize;
    let mut at = 0usize;
    for nc in needle.to_lowercase().chars() {
        if nc == ' ' {
            continue;
        }
        let found = hay[at..].iter().position(|h| *h == nc)?;
        score += found; // characters skipped between matches — tighter matches score lower
        at += found + 1;
    }
    Some(score)
}

/// Filter + sort entry indices for `query` (best match first, registry order within a tie).
pub(crate) fn matches(entries: &[PaletteEntry], query: &str) -> Vec<usize> {
    let mut hits: Vec<(usize, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| fuzzy(query, &e.label).map(|s| (s, i)))
        .collect();
    hits.sort_by_key(|(s, i)| (*s, *i));
    hits.into_iter().map(|(_, i)| i).collect()
}

/// One 40 px full-width row: label left, state pill right. Buttons (not table rows) so the
/// whole strip is a touch target on Android.
fn row(ui: &mut egui::Ui, e: &PaletteEntry, accent: Color32) -> bool {
    let on = e.on.unwrap_or(false);
    let (fg, bg) = if on {
        (accent, Color32::from_rgba_unmultiplied(255, 255, 255, 26))
    } else {
        (Color32::from_gray(224), Color32::from_rgba_unmultiplied(255, 255, 255, 10))
    };
    let w = ui.available_width();
    let resp = ui.add(
        egui::Button::new(RichText::new(&e.label).size(14.0).color(fg))
            .min_size(vec2(w, 40.0))
            .fill(bg)
            .corner_radius(10.0)
            .stroke(if on { Stroke::new(1.0, accent.gamma_multiply(0.7)) } else { Stroke::NONE }),
    );
    // State pill, drawn over the button's right edge (a nested layout inside a Button isn't a thing).
    if let Some(on) = e.on {
        let txt = if on { "ON" } else { "OFF" };
        let col = if on { accent } else { Color32::from_gray(120) };
        ui.painter().text(
            resp.rect.right_center() - vec2(12.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            txt,
            egui::FontId::proportional(11.0),
            col,
        );
    }
    resp.clicked()
}

/// The panel body: search box + categorized rows. Returns the clicked action, if any.
pub(crate) fn body(
    ui: &mut egui::Ui,
    entries: &[PaletteEntry],
    query: &mut String,
    accent: Color32,
    max_height: f32,
) -> Option<PaletteAction> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS).size(15.0).color(Color32::from_gray(170)));
        ui.add(
            egui::TextEdit::singleline(query)
                .hint_text("Search layers…")
                .desired_width(ui.available_width() - 4.0),
        );
    });
    ui.add_space(6.0);
    let order = matches(entries, query);
    egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
        if order.is_empty() {
            ui.add_space(8.0);
            ui.weak("No matches.");
            return;
        }
        if !query.is_empty() {
            // Searching: one flat best-first list — categories only add noise here.
            for i in &order {
                if row(ui, &entries[*i], accent) {
                    chosen = Some(entries[*i].action);
                }
                ui.add_space(4.0);
            }
            return;
        }
        for cat in CATEGORIES {
            let in_cat: Vec<usize> = order.iter().copied().filter(|i| entries[*i].category == cat).collect();
            if in_cat.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(cat).size(13.0).strong().color(accent));
            });
            ui.add_space(3.0);
            for i in in_cat {
                if row(ui, &entries[i], accent) {
                    chosen = Some(entries[i].action);
                }
                ui.add_space(4.0);
            }
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_subsequence_and_ranking() {
        assert!(fuzzy("vel", "Velocity").is_some());
        assert!(fuzzy("srv", "Storm-Relative Velocity").is_some());
        assert!(fuzzy("gau", "River gauges (NWPS)").is_some());
        assert!(fuzzy("zzz", "Velocity").is_none());
        // Empty query matches everything.
        assert_eq!(fuzzy("", "anything"), Some(0));
        // A tighter (contiguous) match must rank ahead of a scattered one.
        let tight = fuzzy("cape", "CAPE").unwrap();
        let loose = fuzzy("cape", "Cell arrival probability estimate").unwrap();
        assert!(tight < loose, "tight {tight} should beat loose {loose}");
    }
}
