//! The Layers panel: a searchable, categorized list of every layer/product/tool in the app,
//! rendered as big full-width toggle rows. Desktop shows it as a right-edge slide-in; Android
//! hosts the same body in a bottom sheet. Both read the one action registry
//! (`HookEchoApp::palette_entries`), so they can never drift apart.

use crate::app::{PaletteAction, PaletteEntry};
use egui::{vec2, Color32, RichText, Stroke};

/// Category order in the panel (anything else falls to the bottom, in registry order).
pub(crate) const CATEGORIES: [&str; 7] = [
    "Radar",
    "National",
    "Severe",
    "Obs",
    "Models",
    "Reference",
    "Tools",
];

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

/// One full-width row: name over a plain-English description, state pill right. Buttons (not
/// table rows) so the whole strip is a touch target on Android.
fn row(ui: &mut egui::Ui, e: &PaletteEntry, accent: Color32) -> bool {
    let on = e.on.unwrap_or(false);
    let (fg, bg) = if on {
        (accent, Color32::from_rgba_unmultiplied(255, 255, 255, 26))
    } else {
        (
            Color32::from_gray(224),
            Color32::from_rgba_unmultiplied(255, 255, 255, 10),
        )
    };
    // Name over description, as one two-line button label: `Button` keeps the whole strip a single
    // touch target, which matters on Android where this same body is a bottom sheet.
    let text = if e.desc.is_empty() {
        RichText::new(&e.label).size(14.0).color(fg)
    } else {
        RichText::new(format!("{}\n{}", e.label, e.desc))
            .size(13.0)
            .color(fg)
    };
    let w = ui.available_width();
    let resp = ui.add(
        egui::Button::new(text)
            .min_size(vec2(w, if e.desc.is_empty() { 40.0 } else { 52.0 }))
            .fill(bg)
            .corner_radius(10.0)
            .stroke(if on {
                Stroke::new(1.0, accent.gamma_multiply(0.7))
            } else {
                Stroke::NONE
            }),
    );
    // State pill, drawn over the button's right edge (a nested layout inside a Button isn't a thing).
    if let Some(on) = e.on {
        let txt = if on { "ON" } else { "OFF" };
        let col = if on { accent } else { Color32::from_gray(120) };
        ui.painter().text(
            resp.rect.right_top() + vec2(-10.0, 9.0),
            egui::Align2::RIGHT_TOP,
            txt,
            egui::FontId::proportional(11.0),
            col,
        );
    }
    resp.clicked()
}

/// The panel body: search box + categorized rows. Returns the clicked action, if any.
/// `focus_search` grabs the search field this frame (Ctrl+K opens the drawer typing-ready).
pub(crate) fn body(
    ui: &mut egui::Ui,
    entries: &[PaletteEntry],
    query: &mut String,
    accent: Color32,
    max_height: f32,
    focus_search: bool,
) -> Option<PaletteAction> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                .size(15.0)
                .color(Color32::from_gray(170)),
        );
        let field = ui.add(
            egui::TextEdit::singleline(query)
                .hint_text("Search layers, tools, places…")
                .desired_width(ui.available_width() - 4.0),
        );
        if focus_search {
            field.request_focus();
        }
    });
    ui.add_space(6.0);
    let order = matches(entries, query);
    // Enter runs the top-ranked match. Type-and-Enter was the whole point of the command palette
    // this drawer replaced; without it the search box is a filter, not a launcher.
    if !query.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(i) = order.first() {
            return Some(entries[*i].action);
        }
    }
    let out = egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
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
                let in_cat: Vec<usize> = order
                    .iter()
                    .copied()
                    .filter(|i| entries[*i].category == cat)
                    .collect();
                if in_cat.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                // A plain label, not `with_layout`: inside a ScrollArea a left_to_right region
                // claims the whole remaining height, which pushed every row below it off-panel.
                ui.label(RichText::new(cat).size(13.0).strong().color(accent));
                ui.add_space(3.0);
                // Everyday rows first; the long tail hides behind one expander per category so the
                // panel reads as a short list instead of 81 flat rows. Nothing is removed — the
                // expander and the search box both still reach every entry.
                let expand_id = ui.id().with(("show_all", cat));
                let show_all = ui.data(|d| d.get_temp::<bool>(expand_id).unwrap_or(false));
                let total = in_cat.len();
                let (shown, hidden): (Vec<usize>, Vec<usize>) = if show_all {
                    (in_cat, Vec::new())
                } else {
                    in_cat.iter().copied().partition(|i| entries[*i].common)
                };
                for i in shown {
                    if row(ui, &entries[i], accent) {
                        chosen = Some(entries[i].action);
                    }
                    ui.add_space(4.0);
                }
                if !hidden.is_empty() || show_all {
                    let label = if show_all {
                        "Show less".to_string()
                    } else {
                        format!("Show all {total} ▾")
                    };
                    if ui.small_button(label).clicked() {
                        ui.data_mut(|d| d.insert_temp(expand_id, !show_all));
                    }
                    ui.add_space(4.0);
                }
            }
        });
    fade_out_bottom(ui, &out);
    chosen
}

/// Fade the last few pixels of the scroll viewport into the card colour when there's more below.
/// The viewport cuts wherever the height budget runs out, which lands mid-row often enough that a
/// half-drawn description ("Specific Differential Pha…") read as a rendering bug. A fade says
/// "keep scrolling" instead.
fn fade_out_bottom(ui: &mut egui::Ui, out: &egui::scroll_area::ScrollAreaOutput<()>) {
    const H: f32 = 22.0;
    let more_below = out.content_size.y > out.inner_rect.height() + 1.0
        && out.state.offset.y + out.inner_rect.height() < out.content_size.y - 1.0;
    if !more_below {
        return;
    }
    let r = out.inner_rect;
    let (cr, cg, cb) = crate::ui::style::CARD_FILL;
    let (clear, solid) = (
        Color32::from_rgba_unmultiplied(cr, cg, cb, 0),
        Color32::from_rgb(cr, cg, cb),
    );
    let mut mesh = egui::Mesh::default();
    for (p, c) in [
        (egui::pos2(r.left(), r.bottom() - H), clear),
        (egui::pos2(r.right(), r.bottom() - H), clear),
        (r.right_bottom(), solid),
        (r.left_bottom(), solid),
    ] {
        mesh.colored_vertex(p, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiering must never hide a row for good: the "Show all" expander and the search box both
    /// read the same registry, so every non-common entry has to still be in it.
    #[test]
    fn every_entry_is_reachable_from_the_registry() {
        let entries = [
            PaletteEntry {
                label: "Echo tops (L3)".into(),
                category: "National",
                action: PaletteAction::CycleBasemap,
                on: None,
                desc: "How tall the storm is",
                common: false,
            },
            PaletteEntry {
                label: "MRMS Mosaic".into(),
                category: "National",
                action: PaletteAction::CycleBasemap,
                on: None,
                desc: "Every radar stitched together",
                common: true,
            },
        ];
        // Empty query = the full list, common or not.
        assert_eq!(matches(&entries, "").len(), entries.len());
        // And an uncommon row is still findable by name.
        assert_eq!(matches(&entries, "echo"), vec![0]);
    }

    /// The drawer's Enter key runs `matches(...)[0]`, so the ranking has to put the obvious
    /// answer first for the labels people actually type.
    #[test]
    fn top_match_is_the_obvious_one() {
        let e = |label: &str| PaletteEntry {
            label: label.into(),
            category: "Radar",
            action: PaletteAction::CycleBasemap,
            on: None,
            desc: "",
            common: true,
        };
        let entries = [e("Storm-Relative Velocity"), e("Velocity"), e("Reflectivity")];
        assert_eq!(matches(&entries, "velocity").first(), Some(&1));
        assert_eq!(matches(&entries, "refl").first(), Some(&2));
    }

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
