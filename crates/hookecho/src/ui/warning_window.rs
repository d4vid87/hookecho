//! Warning window: a click on a warning/watch polygon opens a stack of alert cards; clicking a
//! card drills into the full NWS bulletin (WHAT TO EXPECT chips + raw text).

use crate::theme::{self, stat_card};
use wxdata::overlay::AlertInfo;

/// One card in the stack: the alert plus its polygon stroke color.
pub struct WarnCard {
    pub info: AlertInfo,
    pub color: [u8; 4],
}

/// The open warning popup: deduped alert cards, plus which one is drilled into.
pub struct WarningPopup {
    pub cards: Vec<WarnCard>,
    pub selected: Option<usize>,
}

/// Show the warning window. Returns `false` when it should close.
pub fn show(ctx: &egui::Context, popup: &mut WarningPopup) -> bool {
    let mut open = true;
    egui::Window::new("Active Warnings")
        .open(&mut open)
        .default_size([460.0, 560.0])
        .show(ctx, |ui| match popup.selected {
            Some(i) if i < popup.cards.len() => detail_view(ui, &popup.cards[i], &mut popup.selected),
            _ => stack_view(ui, &popup.cards, &mut popup.selected),
        });
    open
}

fn stack_view(ui: &mut egui::Ui, cards: &[WarnCard], selected: &mut Option<usize>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, card) in cards.iter().enumerate() {
            let a = &card.info;
            let resp = egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .stroke(egui::Stroke::new(1.0, color32(card.color)))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    // Colored header strip + event name.
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 16.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 1.0, color32(card.color));
                        ui.strong(&a.event);
                    });
                    // Summary line: hail · wind · countdown.
                    let mut bits: Vec<String> = Vec::new();
                    if let Some(h) = a.max_hail_in {
                        bits.push(format!("{h:.2}\" hail"));
                    }
                    if let Some(w) = &a.max_wind {
                        bits.push(w.clone());
                    }
                    bits.push(countdown(a));
                    ui.label(bits.join("  ·  "));
                    if !a.area.is_empty() {
                        ui.add(egui::Label::new(egui::RichText::new(&a.area).weak().small()).truncate());
                    }
                })
                .response;
            if resp.interact(egui::Sense::click()).clicked() {
                *selected = Some(i);
            }
            ui.add_space(4.0);
        }
    });
}

fn detail_view(ui: &mut egui::Ui, card: &WarnCard, selected: &mut Option<usize>) {
    let a = &card.info;
    ui.horizontal(|ui| {
        if ui.button("‹ Back").clicked() {
            *selected = None;
        }
        ui.label(countdown(a));
    });
    ui.separator();
    ui.heading(egui::RichText::new(&a.event).color(color32(card.color)));
    if !a.headline.is_empty() {
        ui.label(&a.headline);
    }

    theme::section(ui, "What to Expect", |ui| {
        ui.horizontal_wrapped(|ui| {
            if let Some(w) = &a.max_wind {
                stat_card(ui, "Max Wind", w);
            }
            if let Some(h) = a.max_hail_in {
                stat_card(ui, "Max Hail", &format!("{h:.2} in"));
            }
            if let Some(s) = &a.source {
                stat_card(ui, "Source", s);
            }
        });
    });

    if a.damage_threat.is_some() || a.tornado_detection.is_some() {
        theme::section(ui, "Expected Impacts", |ui| {
            if let Some(d) = &a.damage_threat {
                ui.label(format!("Damage threat: {d}"));
            }
            if let Some(t) = &a.tornado_detection {
                ui.label(format!("Tornado: {t}"));
            }
        });
    }

    ui.add_space(4.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut body = a.description.clone();
        if !a.instruction.is_empty() {
            body.push_str("\n\nPRECAUTIONARY/PREPAREDNESS ACTIONS...\n");
            body.push_str(&a.instruction);
        }
        ui.add(egui::Label::new(egui::RichText::new(body).monospace()).wrap());
    });
}

/// "Expires in N min" / "Expires in H h M min" / "EXPIRED" from the alert expiry.
pub(crate) fn countdown(a: &AlertInfo) -> String {
    let Some(exp) = a.expires else { return "No expiry".into() };
    let secs = (exp - chrono::Utc::now()).num_seconds();
    if secs <= 0 {
        return "EXPIRED".into();
    }
    let mins = secs / 60;
    if mins >= 60 {
        format!("Expires in {}h {}m", mins / 60, mins % 60)
    } else {
        format!("Expires in {mins} min")
    }
}

fn color32(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255)
}
