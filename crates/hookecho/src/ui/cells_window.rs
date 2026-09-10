//! Storm attributes table: every tracked cell at once, sortable.
//!
//! The per-cell popup answers "what is this storm doing" once you've already found the storm. On a
//! busy day the question is the other way round — which of the thirty cells on screen is the one
//! worth looking at. This is the table that answers it: sort by hail size or reflectivity, click
//! the worst row, fly there.

use wxdata::level3::Cell;

/// Which column the table is ordered by.
#[derive(Default, PartialEq, Clone, Copy)]
pub enum SortCol {
    /// The composite severity score — the default, because "which of these thirty" is the
    /// question the table exists to answer.
    #[default]
    Rank,
    Id,
    Range,
    MaxDbz,
    Top,
    Vil,
    Poh,
    Posh,
    Hail,
}

#[derive(Default)]
pub struct CellsWindow {
    pub open: bool,
    sort: SortCol,
    /// Descending by default — the interesting storms are the big numbers.
    desc: bool,
    first_run: bool,
    query: String,
    selected: Option<String>,
}

impl CellsWindow {
    /// Toggle the window, resetting to the default sort on a fresh open.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open && !self.first_run {
            self.first_run = true;
            self.sort = SortCol::Rank;
            self.desc = true;
        }
    }
}

/// Missing values sort last regardless of direction — a cell with no hail estimate is not
/// "smallest hail", it's unknown, and burying it keeps the top of the table meaningful.
fn cmp_opt<T: PartialOrd>(a: Option<T>, b: Option<T>, desc: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
            if desc {
                o.reverse()
            } else {
                o
            }
        }
    }
}

/// Order `cells` by `sort`, returning indices. Kept separate from the widget so it's testable.
pub fn sorted_indices(cells: &[Cell], scores: &[u8], sort: SortCol, desc: bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..cells.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (&cells[a], &cells[b]);
        match sort {
            SortCol::Rank => cmp_opt(scores.get(a), scores.get(b), desc),
            SortCol::Id => {
                let o = x.id.cmp(&y.id);
                if desc {
                    o.reverse()
                } else {
                    o
                }
            }
            SortCol::Range => cmp_opt(x.range_nm, y.range_nm, desc),
            SortCol::MaxDbz => cmp_opt(x.max_dbz, y.max_dbz, desc),
            SortCol::Top => cmp_opt(x.top_kft, y.top_kft, desc),
            SortCol::Vil => cmp_opt(x.vil, y.vil, desc),
            SortCol::Poh => cmp_opt(x.poh, y.poh, desc),
            SortCol::Posh => cmp_opt(x.posh, y.posh, desc),
            SortCol::Hail => cmp_opt(x.hail_in, y.hail_in, desc),
        }
    });
    idx
}

/// The table as CSV, in `order` — what's on screen, in the order it's on screen. Unknowns are
/// empty fields rather than the em dash the table draws, so a spreadsheet reads them as blanks.
pub fn to_csv(cells: &[Cell], scores: &[u8], order: &[usize]) -> String {
    fn c<T: std::fmt::Display>(v: Option<T>) -> String {
        v.map(|x| x.to_string()).unwrap_or_default()
    }
    let mut s = String::from(
        "severity,id,azimuth_deg,range_nm,movement_deg,movement_kt,max_dbz,top_kft,vil,poh,posh,hail_in,tvs,meso\n",
    );
    for &i in order {
        let x = &cells[i];
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            scores.get(i).copied().unwrap_or(0),
            x.title,
            c(x.az_deg),
            c(x.range_nm),
            c(x.mvt_deg),
            c(x.mvt_kt),
            c(x.max_dbz),
            c(x.top_kft),
            c(x.vil),
            c(x.poh),
            c(x.posh),
            c(x.hail_in),
            u8::from(x.tvs.is_some()),
            u8::from(x.meso.is_some()),
        ));
    }
    s
}

/// Show the table. Returns the id of a clicked cell, if any.
#[allow(clippy::too_many_arguments)] // one call site, flat; a params struct buys nothing
pub fn show(
    w: &mut CellsWindow,
    ctx: &egui::Context,
    cells: &[Cell],
    // Composite severity 0-100 per cell, parallel to `cells` (see [`wxdata::cellscore`]).
    scores: &[u8],
    // Cell ids with a ZDR column detected near them — an updraft the storm table cannot see on
    // its own, badged next to the rotation flags it already carries.
    zdr_cells: &std::collections::HashSet<String>,
    // Per-cell-id history across volumes, oldest→newest — the same map the attributes popup
    // draws its trend rows from.
    trends: &std::collections::HashMap<String, Vec<crate::ui::cell_window::CellSample>>,
    _accent: egui::Color32,
    _drawer: &mut crate::ui::drawer::Drawer,
) -> Option<String> {
    if !w.open {
        return None;
    }
    let mut chosen = None;
    let mut open = w.open;
    let order: Vec<_> = sorted_indices(cells, scores, w.sort, w.desc)
        .into_iter()
        .filter(|i| {
            cells[*i]
                .id
                .to_lowercase()
                .contains(&w.query.to_lowercase())
        })
        .collect();
    if !order
        .iter()
        .any(|i| Some(&cells[*i].id) == w.selected.as_ref())
    {
        w.selected = order.first().map(|i| cells[*i].id.clone());
    }
    let width = (ctx.content_rect().width() - 48.0).clamp(300.0, 960.0);
    egui::Window::new("Storm attributes")
        .open(&mut open)
        .default_width(width)
        .default_pos(egui::pos2(24.0, 64.0))
        .default_height((ctx.content_rect().height() - 128.0).clamp(300.0, 600.0))
        .resizable(true)
        .vscroll(true)
        .collapsible(false)
        .frame(
            egui::Frame::window(&ctx.style_of(ctx.theme()))
                .fill(egui::Color32::from_rgb(17, 23, 31))
                .corner_radius(16)
                .inner_margin(16),
        )
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.weak(format!("{} cells", cells.len()));
                ui.add(
                    egui::TextEdit::singleline(&mut w.query)
                        .hint_text("Find a cell…")
                        .desired_width(150.0),
                );
                egui::ComboBox::from_id_salt("cell_sort")
                    .selected_text("Sort by…")
                    .show_ui(ui, |ui| {
                        for (label, key) in [
                            ("Severity", SortCol::Rank),
                            ("Cell ID", SortCol::Id),
                            ("Range", SortCol::Range),
                            ("Reflectivity", SortCol::MaxDbz),
                            ("Cell top", SortCol::Top),
                            ("VIL", SortCol::Vil),
                            ("Hail probability", SortCol::Poh),
                            ("Severe hail", SortCol::Posh),
                            ("Hail size", SortCol::Hail),
                        ] {
                            ui.selectable_value(&mut w.sort, key, label);
                        }
                    });
                ui.checkbox(&mut w.desc, "Descending");
                ui.menu_button("Export CSV", |ui| {
                    crate::ui::csv_buttons(
                        ui,
                        "cells.csv",
                        "Filtered cells in current sort",
                        || to_csv(cells, scores, &order),
                    );
                });
            });
            ui.add_space(12.0);
            if order.is_empty() {
                ui.weak("No matching storm cells.");
                return;
            }
            let draw_list = |ui: &mut egui::Ui, selected: &mut Option<String>| {
                ui.horizontal(|ui| {
                    ui.strong("Cell");
                    ui.weak("     Range · Peak reflectivity");
                });
                egui::ScrollArea::vertical()
                    .id_salt("storm_list")
                    .max_height(440.0)
                    .show(ui, |ui| {
                        for &i in &order {
                            let c = &cells[i];
                            let range = c
                                .range_nm
                                .map(|v| format!("{v:.0} NM"))
                                .unwrap_or_else(|| "—".into());
                            let peak = c
                                .max_dbz
                                .map(|v| format!("{v:.0} dBZ"))
                                .unwrap_or_else(|| "—".into());
                            if ui
                                .add_sized(
                                    [ui.available_width(), 44.0],
                                    egui::Button::new(format!("{}     {range} · {peak}", c.id))
                                        .selected(selected.as_ref() == Some(&c.id)),
                                )
                                .clicked()
                            {
                                *selected = Some(c.id.clone());
                            }
                        }
                    });
            };
            let detail = |ui: &mut egui::Ui, selected: &Option<String>| {
                if let Some(c) = cells.iter().find(|c| Some(&c.id) == selected.as_ref()) {
                    ui.horizontal(|ui| {
                        ui.heading(format!("Cell {}", c.id));
                        if ui.button("Center on map").clicked() {
                            chosen = Some(c.id.clone());
                        }
                    });
                    let score = cells
                        .iter()
                        .position(|x| x.id == c.id)
                        .and_then(|i| scores.get(i));
                    if let Some(score) = score {
                        ui.weak(format!("Severity score {score}/100"));
                    }
                    if zdr_cells.contains(&c.id) {
                        ui.label("ZDR column detected");
                    }
                    crate::ui::cell_window::attributes(
                        ui,
                        c,
                        trends.get(&c.id).map(Vec::as_slice).unwrap_or(&[]),
                    );
                }
            };
            let mut detail = detail;
            if ui.available_width() >= 740.0 {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(290.0, 440.0),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| draw_list(ui, &mut w.selected),
                    );
                    ui.separator();
                    ui.vertical(|ui| detail(ui, &w.selected));
                });
            } else {
                draw_list(ui, &mut w.selected);
                ui.separator();
                detail(ui, &w.selected);
            }
        });
    w.open = open;
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(id: &str, hail: Option<f32>, dbz: Option<f32>) -> Cell {
        Cell {
            id: id.into(),
            title: id.into(),
            hail_in: hail,
            max_dbz: dbz,
            ..Default::default()
        }
    }

    #[test]
    fn sorts_descending_with_unknowns_last() {
        let cells = [
            cell("A", Some(0.5), None),
            cell("B", None, None),
            cell("C", Some(2.5), None),
        ];
        let order = sorted_indices(&cells, &[], SortCol::Hail, true);
        assert_eq!(order, vec![2, 0, 1], "biggest hail first, unknown last");
    }

    #[test]
    fn severity_sorts_the_table_and_a_missing_score_sinks() {
        let cells = [
            cell("A", Some(2.0), Some(60.0)),
            cell("B", Some(0.2), Some(45.0)),
            cell("C", None, None),
        ];
        // Only two scores for three cells: the third is unknown, and unknown sorts last either way.
        assert_eq!(
            sorted_indices(&cells, &[30, 88], SortCol::Rank, true),
            vec![1, 0, 2]
        );
        assert_eq!(
            sorted_indices(&cells, &[30, 88], SortCol::Rank, false),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn csv_follows_the_table() {
        let cells = [cell("A", Some(0.5), Some(60.0)), cell("B", None, None)];
        let order = sorted_indices(&cells, &[], SortCol::Hail, true);
        let csv = to_csv(&cells, &[71, 12], &order);
        let lines: Vec<&str> = csv.lines().collect();
        assert!(
            lines[0].starts_with("severity,id,azimuth_deg"),
            "header first"
        );
        assert!(
            lines[1].starts_with("71,A,"),
            "sorted order, not input order"
        );
        assert_eq!(lines[2], "12,B,,,,,,,,,,,0,0", "unknowns are empty fields");
    }

    #[test]
    fn unknowns_stay_last_when_ascending() {
        let cells = [
            cell("A", Some(0.5), None),
            cell("B", None, None),
            cell("C", Some(2.5), None),
        ];
        let order = sorted_indices(&cells, &[], SortCol::Hail, false);
        assert_eq!(order, vec![0, 2, 1], "smallest first, unknown still last");
    }

    #[test]
    fn sorts_by_id_alphabetically() {
        let cells = [cell("Q7", None, None), cell("B3", None, None)];
        assert_eq!(sorted_indices(&cells, &[], SortCol::Id, false), vec![1, 0]);
        assert_eq!(sorted_indices(&cells, &[], SortCol::Id, true), vec![0, 1]);
    }
}
