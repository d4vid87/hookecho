//! Storm attributes table: every tracked cell at once, sortable.
//!
//! The per-cell popup answers "what is this storm doing" once you've already found the storm. On a
//! busy day the question is the other way round — which of the thirty cells on screen is the one
//! worth looking at. This is the table that answers it: sort by hail size or reflectivity, click
//! the worst row, fly there.

use egui::{Align, Layout, RichText};
use egui_extras::{Column, TableBuilder};
use wxdata::level3::Cell;

/// Which column the table is ordered by.
#[derive(Default, PartialEq, Clone, Copy)]
pub enum SortCol {
    Id,
    Range,
    MaxDbz,
    Top,
    Vil,
    Poh,
    Posh,
    #[default]
    Hail,
}

#[derive(Default)]
pub struct CellsWindow {
    pub open: bool,
    sort: SortCol,
    /// Descending by default — the interesting storms are the big numbers.
    desc: bool,
    first_run: bool,
}

impl CellsWindow {
    /// Toggle the window, resetting to the default sort on a fresh open.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open && !self.first_run {
            self.first_run = true;
            self.sort = SortCol::Hail;
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
pub fn sorted_indices(cells: &[Cell], sort: SortCol, desc: bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..cells.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (&cells[a], &cells[b]);
        match sort {
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
pub fn to_csv(cells: &[Cell], order: &[usize]) -> String {
    fn c<T: std::fmt::Display>(v: Option<T>) -> String {
        v.map(|x| x.to_string()).unwrap_or_default()
    }
    let mut s = String::from(
        "id,azimuth_deg,range_nm,movement_deg,movement_kt,max_dbz,top_kft,vil,poh,posh,hail_in,tvs,meso\n",
    );
    for &i in order {
        let x = &cells[i];
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
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

fn num<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "—".into())
}

fn f1(v: Option<f32>) -> String {
    v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "—".into())
}

fn f0(v: Option<f32>) -> String {
    v.map(|x| format!("{x:.0}")).unwrap_or_else(|| "—".into())
}

/// One row's summary line, shared by the desktop table and the touch list.
fn movement(c: &Cell) -> String {
    match (c.mvt_deg, c.mvt_kt) {
        (Some(d), Some(k)) => format!("{d:.0}° {k:.0}kt"),
        _ => "—".into(),
    }
}

fn position(c: &Cell) -> String {
    match (c.az_deg, c.range_nm) {
        (Some(a), Some(r)) => format!("{a:.0}° {r:.0}nm"),
        _ => "—".into(),
    }
}

/// Show the table. Returns the id of a clicked cell, if any.
pub fn show(
    w: &mut CellsWindow,
    ctx: &egui::Context,
    cells: &[Cell],
    // Cell ids with a ZDR column detected near them — an updraft the storm table cannot see on
    // its own, badged next to the rotation flags it already carries.
    zdr_cells: &std::collections::HashSet<String>,
    accent: egui::Color32,
) -> Option<String> {
    if !w.open {
        return None;
    }
    let mut chosen = None;
    let mut open = w.open;
    let order = sorted_indices(cells, w.sort, w.desc);
    crate::ui::phone_surface(ctx, egui::Window::new("Storm attributes"))
        .open(&mut open)
        .default_size([720.0, 420.0])
        .show(ctx, |ui| {
            if cells.is_empty() {
                ui.weak("No tracked storm cells for this site right now.");
                ui.small("Cells come from the radar's own storm-tracking product (SCIT).");
                return;
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} cells", cells.len()))
                        .strong()
                        .color(accent),
                );
                ui.weak("· click a row to fly there");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    crate::ui::csv_buttons(ui, "cells.csv", "Every row, current sort", || {
                        to_csv(cells, &order)
                    });
                });
            });
            ui.separator();

            // Android: TableBuilder row clicks don't register under touch (nested scroll eats
            // them) — same trap the site dialog hit. Buttons take taps reliably.
            if cfg!(target_os = "android") {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for &i in &order {
                        let c = &cells[i];
                        let line = format!(
                            "{}   {}   {} dBZ   hail {}\"\n{}   top {} kft   VIL {}",
                            c.title,
                            position(c),
                            f0(c.max_dbz),
                            f1(c.hail_in),
                            movement(c),
                            f0(c.top_kft),
                            f0(c.vil),
                        );
                        let btn = egui::Button::new(RichText::new(line).size(14.0))
                            .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12))
                            .corner_radius(8.0)
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .min_size(egui::vec2(ui.available_width(), 46.0));
                        if ui.add(btn).clicked() {
                            chosen = Some(c.id.clone());
                        }
                        ui.add_space(3.0);
                    }
                });
                return;
            }

            let header = |ui: &mut egui::Ui, label: &str, col: SortCol, w: &mut CellsWindow| {
                let active = w.sort == col;
                let text = if active {
                    format!("{label} {}", if w.desc { "▼" } else { "▲" })
                } else {
                    label.to_string()
                };
                if ui.button(text).clicked() {
                    if active {
                        w.desc = !w.desc;
                    } else {
                        w.sort = col;
                        w.desc = true;
                    }
                }
            };

            // Horizontal scroll keeps every column reachable on a narrow window.
            egui::ScrollArea::horizontal().show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
                    .sense(egui::Sense::click())
                    .column(Column::exact(52.0)) // id
                    .column(Column::exact(92.0)) // az/range
                    .column(Column::exact(86.0)) // movement
                    .column(Column::exact(56.0)) // max dbz
                    .column(Column::exact(52.0)) // top
                    .column(Column::exact(52.0)) // vil
                    .column(Column::exact(48.0)) // poh
                    .column(Column::exact(52.0)) // posh
                    .column(Column::exact(56.0)) // hail
                    .column(Column::remainder()) // flags
                    .min_scrolled_height(0.0)
                    .header(22.0, |mut h| {
                        h.col(|ui| header(ui, "ID", SortCol::Id, w));
                        h.col(|ui| header(ui, "Pos", SortCol::Range, w));
                        h.col(|ui| {
                            ui.label("Moving");
                        });
                        h.col(|ui| header(ui, "dBZ", SortCol::MaxDbz, w));
                        h.col(|ui| header(ui, "Top", SortCol::Top, w));
                        h.col(|ui| header(ui, "VIL", SortCol::Vil, w));
                        h.col(|ui| header(ui, "POH", SortCol::Poh, w));
                        h.col(|ui| header(ui, "SVR", SortCol::Posh, w));
                        h.col(|ui| header(ui, "Hail", SortCol::Hail, w));
                        h.col(|ui| {
                            ui.label("Flags");
                        });
                    })
                    .body(|body| {
                        body.rows(20.0, order.len(), |mut row| {
                            let c = &cells[order[row.index()]];
                            row.col(|ui| {
                                ui.label(RichText::new(&c.title).strong());
                            });
                            row.col(|ui| {
                                ui.label(position(c));
                            });
                            row.col(|ui| {
                                ui.label(movement(c));
                            });
                            row.col(|ui| {
                                ui.label(f0(c.max_dbz));
                            });
                            row.col(|ui| {
                                ui.label(f0(c.top_kft));
                            });
                            row.col(|ui| {
                                ui.label(f0(c.vil));
                            });
                            row.col(|ui| {
                                ui.label(num(c.poh));
                            });
                            row.col(|ui| {
                                ui.label(num(c.posh));
                            });
                            row.col(|ui| {
                                // The number people scan for first.
                                let t = RichText::new(f1(c.hail_in));
                                ui.label(if c.hail_in.is_some_and(|h| h >= 1.0) {
                                    t.strong().color(egui::Color32::from_rgb(240, 170, 60))
                                } else {
                                    t
                                });
                            });
                            row.col(|ui| {
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    if c.tvs.is_some() {
                                        ui.label(
                                            RichText::new("TVS")
                                                .small()
                                                .strong()
                                                .color(egui::Color32::from_rgb(235, 70, 70)),
                                        );
                                    }
                                    if zdr_cells.contains(&c.id) {
                                        ui.label(
                                            RichText::new("Z\u{25b2}")
                                                .small()
                                                .strong()
                                                .color(egui::Color32::from_rgb(120, 230, 160)),
                                        )
                                        .on_hover_text(
                                            "A ZDR column reaches above the freezing level here \
                                             \u{2014} the updraft is deep",
                                        );
                                    }
                                    if c.meso.is_some() {
                                        ui.label(
                                            RichText::new("MESO")
                                                .small()
                                                .strong()
                                                .color(egui::Color32::from_rgb(235, 160, 60)),
                                        );
                                    }
                                });
                            });
                            if row.response().clicked() {
                                chosen = Some(c.id.clone());
                            }
                        });
                    });
            });
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
        let order = sorted_indices(&cells, SortCol::Hail, true);
        assert_eq!(order, vec![2, 0, 1], "biggest hail first, unknown last");
    }

    #[test]
    fn csv_follows_the_table() {
        let cells = [cell("A", Some(0.5), Some(60.0)), cell("B", None, None)];
        let order = sorted_indices(&cells, SortCol::Hail, true);
        let csv = to_csv(&cells, &order);
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("id,azimuth_deg"), "header first");
        assert!(lines[1].starts_with("A,"), "sorted order, not input order");
        assert_eq!(lines[2], "B,,,,,,,,,,,0,0", "unknowns are empty fields");
    }

    #[test]
    fn unknowns_stay_last_when_ascending() {
        let cells = [
            cell("A", Some(0.5), None),
            cell("B", None, None),
            cell("C", Some(2.5), None),
        ];
        let order = sorted_indices(&cells, SortCol::Hail, false);
        assert_eq!(order, vec![0, 2, 1], "smallest first, unknown still last");
    }

    #[test]
    fn sorts_by_id_alphabetically() {
        let cells = [cell("Q7", None, None), cell("B3", None, None)];
        assert_eq!(sorted_indices(&cells, SortCol::Id, false), vec![1, 0]);
        assert_eq!(sorted_indices(&cells, SortCol::Id, true), vec![0, 1]);
    }
}
