//! Warning window: a click on a warning/watch polygon opens a stack of alert cards; clicking a
//! card opens the official bulletin directly in a scrollable glass reading panel.

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
pub fn show(
    ctx: &egui::Context,
    popup: &mut WarningPopup,
    popovers: &mut crate::ui::popover::Popovers,
) -> bool {
    let mut open = true;
    let mut close = false;
    popovers
        .card(ctx, "warning", egui::Window::new("Weather alerts"))
        .open(&mut open)
        .frame(crate::ui::popover::glass_frame())
        .collapsible(false)
        .title_bar(false)
        .default_size([460.0, 520.0])
        .show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(225, 234, 244));
            ui.horizontal(|ui| {
                ui.weak("WEATHER ALERT");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Close ×").clicked();
                });
            });
            match popup.selected {
                Some(i) if i < popup.cards.len() => {
                    detail_view(ui, &popup.cards[i], &mut popup.selected)
                }
                _ => stack_view(ui, &popup.cards, &mut popup.selected),
            }
        });
    open && !close
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
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(6.0, 16.0), egui::Sense::hover());
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
                        ui.add(
                            egui::Label::new(egui::RichText::new(&a.area).weak().small())
                                .truncate(),
                        );
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
    ui.add_space(12.0);
    let icon = if a.event.contains("Statement") {
        egui_phosphor::regular::INFO
    } else {
        egui_phosphor::regular::WARNING
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(icon)
                .size(28.0)
                .color(color32(card.color)),
        );
        ui.label(egui::RichText::new(&a.event).size(26.0).strong());
    });
    if !a.headline.is_empty() && a.headline != a.event {
        ui.label(&a.headline);
    }

    if let Some(expires) = a.expires {
        ui.add_space(8.0);
        ui.label(format!("Expires {}", expires.format("%b %-d · %H:%M UTC")));
    }
    ui.add_space(12.0);
    ui.separator();
    ui.label(egui::RichText::new("Official bulletin").size(18.0).strong());
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt((&a.id, "bulletin"))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .max_height((ui.available_height() - 32.0).max(100.0))
        .show(ui, |ui| {
            if !a.area.is_empty() {
                ui.strong(&a.area);
                ui.add_space(10.0);
            }
            ui.horizontal_wrapped(|ui| {
                if let Some(w) = &a.max_wind {
                    ui.label(format!("Wind: {w}"));
                }
                if let Some(h) = a.max_hail_in {
                    ui.label(format!("Hail: {h:.2} in"));
                }
                if let Some(d) = &a.damage_threat {
                    ui.label(format!("Damage threat: {d}"));
                }
                if let Some(t) = &a.tornado_detection {
                    ui.label(format!("Tornado: {t}"));
                }
            });
            let mut body = a.description.clone();
            if !a.instruction.is_empty() {
                body.push_str("\n\nPRECAUTIONARY/PREPAREDNESS ACTIONS...\n");
                body.push_str(&a.instruction);
            }
            ui.add(egui::Label::new(egui::RichText::new(body).size(16.0)).wrap());
            if let Some(source) = &a.source {
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("Reported source: {source}")).small(),
                    )
                    .wrap(),
                );
            }
        });
    ui.separator();
    ui.weak("Official alert bulletin · source text as issued");
}

/// "Expires in N min" / "Expires in H h M min" / "EXPIRED" from the alert expiry.
pub(crate) fn countdown(a: &AlertInfo) -> String {
    let Some(exp) = a.expires else {
        return "No expiry".into();
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_and_statements_show_bulletin_without_expanding() {
        for event in ["Special Marine Warning", "Special Weather Statement"] {
            let ctx = egui::Context::default();
            let mut popup = WarningPopup {
                selected: Some(0),
                cards: vec![WarnCard {
                    color: [240, 160, 60, 255],
                    info: AlertInfo {
                        id: event.into(),
                        event: event.into(),
                        headline: "Issued by NWS".into(),
                        area: "Sample area".into(),
                        description: "Official sample bulletin text.".into(),
                        instruction: "Sample preparedness instructions.".into(),
                        expires: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                        max_hail_in: None,
                        max_wind: None,
                        tornado_detection: None,
                        damage_threat: None,
                        source: None,
                        motion: None,
                        vtec: None,
                    },
                }],
            };
            let mut popovers = crate::ui::popover::Popovers::default();
            let mut labels = String::new();
            for _ in 0..3 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        assert!(show(ui.ctx(), &mut popup, &mut popovers));
                    },
                );
                labels = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) if s.clip_rect.contains(t.pos) => {
                            Some(t.galley.job.text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            assert!(labels.contains(event), "{labels}");
            assert!(
                labels.contains("Official sample bulletin text."),
                "{labels}"
            );
            assert!(
                labels.contains("Sample preparedness instructions."),
                "{labels}"
            );
            assert!(labels.contains("Expires"), "{labels}");
        }
    }
}
