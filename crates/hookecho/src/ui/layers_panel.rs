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

/// Row height: one line, tall enough to stay a touch target on Android.
const ROW_H: f32 = 28.0;

/// The row's icon, picked from the label and falling back to the category.
///
/// Derived rather than stored: a per-entry `icon` field would be ~100 registry edits to keep in
/// sync by hand, and the labels already say what the thing is.
pub(crate) fn glyph(e: &PaletteEntry) -> &'static str {
    use egui_phosphor::regular as ph;
    let l = e.label.to_lowercase();
    let has = |w: &str| l.contains(w);
    match () {
        _ if has("velocity") || has("azshear") || has("rotation") => ph::ARROWS_CLOCKWISE,
        _ if has("hail") || has("mesh") => ph::CIRCLE,
        _ if has("tornado") || has("tds") => ph::TORNADO,
        _ if has("lightning") || has("glm") => ph::LIGHTNING,
        _ if has("snow") || has("winter") || has("ice") => ph::SNOWFLAKE,
        _ if has("rain") || has("qpe") || has("precip") || has("flood") => ph::DROP,
        _ if has("wind") => ph::WIND,
        _ if has("temp") || has("dewpoint") => ph::THERMOMETER,
        _ if has("satellite") || has("cloud") || has("smoke") => ph::CLOUD,
        _ if has("surge") || has("buoy") || has("wave") || has("river") => ph::WAVES,
        _ if has("pirep") || has("sigmet") || has("airmet") || has("recon") => ph::AIRPLANE_TILT,
        _ if has("warning") || has("alert") || has("outlook") || has("watch") => ph::WARNING,
        _ if has("sounding") || has("vad") || has("cape") || has("chart") => ph::CHART_LINE,
        _ if has("cross-section") || has("3d") || has("cappi") || has("volume") => ph::CUBE,
        _ if has("measure") || has("range") || has("distance") => ph::RULER,
        _ if has("marker") || has("place") || has("gauge") || has("spotter") => ph::MAP_PIN,
        _ if has("basemap") || has("map") || has("terrain") => ph::MAP_TRIFOLD,
        _ if has("site") || has("radar site") || has("mosaic") => ph::BROADCAST,
        _ if has("camera") || has("webcam") => ph::CAMERA,
        _ if has("setting") || has("preference") => ph::GEAR,
        _ => match e.category {
            "Radar" => ph::RADIO_BUTTON,
            "National" => ph::GLOBE,
            "Severe" => ph::WARNING,
            "Obs" => ph::THERMOMETER,
            "Models" => ph::CHART_LINE,
            "Reference" => ph::MAP_TRIFOLD,
            _ => ph::CROSSHAIR,
        },
    }
}

/// Move `drag` to sit where `before` is inside `seq`, and record the result in `pref` — the
/// persisted, cross-category label order. Only the moved category's labels are rewritten, so
/// reordering Radar leaves an earlier drag in Obs alone.
pub(crate) fn reorder(pref: &mut Vec<String>, seq: &[String], drag: &str, before: &str) {
    if drag == before {
        return;
    }
    let mut next: Vec<String> = seq.iter().filter(|s| *s != drag).cloned().collect();
    let at = next.iter().position(|s| s == before).unwrap_or(next.len());
    next.insert(at, drag.to_string());
    pref.retain(|s| !seq.iter().any(|q| q == s));
    pref.extend(next);
}

/// One full-width row: the name, a state dot on the right, the description on hover. It used to
/// be a two-line 52 px card, which turned a category into a wall and pushed everything below the
/// fold; the description is a hint, not something you read twenty times in a row.
/// `draggable` puts the icon on a drag handle; the returned response covers the whole row and is
/// what the caller tests for a drop.
fn row(
    ui: &mut egui::Ui,
    e: &PaletteEntry,
    accent: Color32,
    draggable: bool,
) -> (bool, egui::Response) {
    let on = e.on.unwrap_or(false);
    let (fg, bg) = if on {
        (accent, Color32::from_rgba_unmultiplied(255, 255, 255, 22))
    } else {
        (
            Color32::from_gray(216),
            Color32::from_rgba_unmultiplied(255, 255, 255, 8),
        )
    };
    let icon = RichText::new(glyph(e))
        .size(14.0)
        .color(if on { accent } else { Color32::from_gray(150) });
    let mut clicked = false;
    let outer = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // The icon doubles as the grip: a separate handle column costs width the label needs,
            // and dragging from the label itself would fight the click that toggles the layer.
            if draggable {
                ui.dnd_drag_source(
                    egui::Id::new(("layer_drag", &e.label)),
                    e.label.clone(),
                    |ui| ui.label(icon),
                )
                .response
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag to reorder");
            } else {
                ui.label(icon);
            }
            let w = ui.available_width();
            // A justified child layout, not a plain `add`: inside a horizontal row egui centers a
            // button's text, and a column of centered labels is unreadable.
            let mut resp = ui
                .allocate_ui_with_layout(
                    vec2(w, ROW_H),
                    egui::Layout::top_down_justified(egui::Align::LEFT),
                    |ui| {
                        ui.add(
                            egui::Button::new(RichText::new(&e.label).size(13.0).color(fg))
                                .min_size(vec2(w, ROW_H))
                                .fill(bg)
                                .corner_radius(7.0)
                                .stroke(if on {
                                    Stroke::new(1.0, accent.gamma_multiply(0.7))
                                } else {
                                    Stroke::NONE
                                }),
                        )
                    },
                )
                .inner;
            if !e.desc.is_empty() {
                resp = resp.on_hover_text(e.desc);
            }
            clicked = resp.clicked();
            resp
        })
        .inner;
    // The button's rect, not the whole strip: it's what the chips are drawn against and what a
    // drop is tested on, and it covers everything but the grip.
    let resp = outer;
    // Binding chip, left of where the state dot goes: the shortcut stays learnable from the row.
    if let Some(key) = &e.key {
        let dx = if e.on.is_some() { -22.0 } else { -8.0 };
        ui.painter().text(
            resp.rect.right_center() + vec2(dx, 0.0),
            egui::Align2::RIGHT_CENTER,
            key,
            egui::FontId::monospace(10.0),
            Color32::from_gray(140),
        );
    }
    // State dot, drawn over the button's right edge (a nested layout inside a Button isn't a thing).
    if let Some(on) = e.on {
        ui.painter().circle_filled(
            resp.rect.right_center() + vec2(-10.0, 0.0),
            3.5,
            if on { accent } else { Color32::from_gray(90) },
        );
    }
    (clicked, resp)
}

/// The panel body: search box + categorized rows. Returns the clicked action, if any.
/// `focus_search` grabs the search field this frame (Ctrl+K opens the drawer typing-ready).
/// `pref` is the persisted drag order and is rewritten in place when a row is dropped.
#[allow(clippy::too_many_arguments)] // two call sites, both flat; a params struct buys nothing
pub(crate) fn body(
    ui: &mut egui::Ui,
    entries: &[PaletteEntry],
    query: &mut String,
    accent: Color32,
    max_height: f32,
    focus_search: bool,
    pref: &mut Vec<String>,
    mut after_radar: impl FnMut(&mut egui::Ui),
) -> Option<PaletteAction> {
    let mut chosen = None;
    // (dragged label, label it was dropped on) — applied after the loop so the borrow of `pref`
    // doesn't have to live inside the scroll area.
    let mut moved: Option<(String, String)> = None;
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
    ui.add_space(4.0);
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
                    // No dragging in search results: the order you're looking at is the ranking,
                    // not the list you'd be reordering.
                    if row(ui, &entries[*i], accent, false).0 {
                        chosen = Some(entries[*i].action);
                    }
                    ui.add_space(2.0);
                }
                return;
            }
            // Not searching: one collapsible group per category, only Radar open. Seven headers
            // fit on screen at once, so the whole app is visible as an outline instead of as a
            // scroll. This replaces the category pills *and* the per-category "Show all"
            // expander — three scoping controls were two too many.
            for cat in CATEGORIES {
                let mut in_cat: Vec<usize> = order
                    .iter()
                    .copied()
                    .filter(|i| entries[*i].category == cat)
                    .collect();
                if in_cat.is_empty() {
                    continue;
                }
                // Dragged rows first, in the order they were dropped; then everyday entries;
                // then registry order. A row that was never dragged still has a stable place.
                in_cat.sort_by_key(|i| {
                    let dragged = pref.iter().position(|s| *s == entries[*i].label);
                    (dragged.unwrap_or(usize::MAX), !entries[*i].common)
                });
                let seq: Vec<String> = in_cat.iter().map(|i| entries[*i].label.clone()).collect();
                let head = RichText::new(format!("{cat}  ({})", in_cat.len()))
                    .size(12.0)
                    .strong()
                    .color(accent);
                egui::CollapsingHeader::new(head)
                    .id_salt(("cat", cat))
                    .default_open(cat == "Radar")
                    .show_unindented(ui, |ui| {
                        for i in in_cat {
                            let (clicked, resp) = row(ui, &entries[i], accent, true);
                            if clicked {
                                chosen = Some(entries[i].action);
                            }
                            // Insertion line above the row the pointer is over, so a drop lands
                            // where the preview says it will.
                            if resp.dnd_hover_payload::<String>().is_some() {
                                let r = resp.rect;
                                ui.painter().hline(
                                    r.x_range(),
                                    r.top() - 1.0,
                                    Stroke::new(2.0, accent),
                                );
                            }
                            if let Some(drag) = resp.dnd_release_payload::<String>() {
                                moved = Some((
                                    (*drag).clone(),
                                    entries[i].label.clone(),
                                ));
                            }
                            ui.add_space(2.0);
                        }
                    });
                // The knobs for the products right above them, not at the bottom of the panel:
                // a threshold or a forecast hour is read together with the layer it belongs to.
                if cat == "Radar" {
                    ui.add_space(2.0);
                    after_radar(ui);
                    ui.add_space(2.0);
                }
                if let Some((drag, before)) = moved.take() {
                    // Only the category the row was dropped in is rewritten — a cross-category
                    // drag would move a layer out of the group its label says it's in.
                    if seq.contains(&drag) {
                        reorder(pref, &seq, &drag, &before);
                    }
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
                key: None,
            },
            PaletteEntry {
                label: "MRMS Mosaic".into(),
                category: "National",
                action: PaletteAction::CycleBasemap,
                on: None,
                desc: "Every radar stitched together",
                common: true,
                key: None,
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
            key: None,
        };
        let entries = [
            e("Storm-Relative Velocity"),
            e("Velocity"),
            e("Reflectivity"),
        ];
        assert_eq!(matches(&entries, "velocity").first(), Some(&1));
        assert_eq!(matches(&entries, "refl").first(), Some(&2));
    }

    #[test]
    fn dropping_a_row_puts_it_where_the_preview_said() {
        let seq: Vec<String> = ["Reflectivity", "Velocity", "Spectrum Width"]
            .map(String::from)
            .to_vec();
        let mut pref = vec!["Some other category's row".to_string()];
        // Drop Spectrum Width onto Velocity: it lands *above* Velocity, matching the line drawn
        // along the hovered row's top edge.
        reorder(&mut pref, &seq, "Spectrum Width", "Velocity");
        assert_eq!(
            pref,
            [
                "Some other category's row",
                "Reflectivity",
                "Spectrum Width",
                "Velocity",
            ]
        );
        // A second drag rewrites the same labels rather than appending them twice.
        let seq2: Vec<String> = ["Reflectivity", "Spectrum Width", "Velocity"]
            .map(String::from)
            .to_vec();
        reorder(&mut pref, &seq2, "Velocity", "Reflectivity");
        assert_eq!(
            pref,
            [
                "Some other category's row",
                "Velocity",
                "Reflectivity",
                "Spectrum Width",
            ]
        );
        // Dropping a row on itself is a no-op, not a reshuffle.
        let before = pref.clone();
        reorder(&mut pref, &seq2, "Velocity", "Velocity");
        assert_eq!(pref, before);
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
