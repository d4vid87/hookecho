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

pub fn sort_cards(cards: &mut [WarnCard]) {
    cards.sort_by_key(|card| {
        std::cmp::Reverse((
            crate::ui::alert_panel::severity_rank(&card.info.event),
            wxdata::alerts::escalation(&card.info),
        ))
    });
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
        .frame(crate::ui::popover::glass_frame(ctx))
        .collapsible(false)
        .title_bar(false)
        .default_size([460.0, 520.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.weak("WEATHER ALERT");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Close ×").clicked();
                });
            });
            match popup.selected {
                Some(i) if i < popup.cards.len() => {
                    detail_view(ui, &popup.cards, i, &mut popup.selected)
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

fn detail_view(ui: &mut egui::Ui, cards: &[WarnCard], index: usize, selected: &mut Option<usize>) {
    let card = &cards[index];
    let a = &card.info;
    ui.horizontal(|ui| {
        if ui.button("‹ All alerts").clicked() {
            *selected = None;
        }
        ui.label(countdown(a));
    });
    if cards.len() > 1 {
        let mut choice = index;
        egui::ComboBox::from_id_salt("overlapping_alerts")
            .selected_text(format!("Alert {} of {}", index + 1, cards.len()))
            .show_ui(ui, |ui| {
                for (i, item) in cards.iter().enumerate() {
                    ui.selectable_value(&mut choice, i, &item.info.event);
                }
            });
        *selected = Some(choice);
    }
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
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(crate::ui::style::RADIUS_SM)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.label(egui::RichText::new("ALERT SUMMARY").small().weak());
            ui.label(if a.area.is_empty() {
                "Affected area unavailable"
            } else {
                &a.area
            });
            ui.label(a.expires.map_or_else(
                || "Validity unavailable".to_string(),
                |expires| format!("Valid until {}", expires.format("%b %-d · %H:%M UTC")),
            ));
            let mut hazards = Vec::new();
            if let Some(w) = &a.max_wind {
                hazards.push(format!("Wind {w}"));
            }
            if let Some(h) = a.max_hail_in {
                hazards.push(format!("Hail {h:.2} in"));
            }
            if let Some(d) = &a.damage_threat {
                hazards.push(format!("Damage {d}"));
            }
            ui.label(if hazards.is_empty() {
                "Hazard details unavailable".to_string()
            } else {
                hazards.join(" · ")
            });
            ui.label(a.source.as_deref().map_or_else(
                || "Source details unavailable".to_string(),
                |source| format!("Source: {source}"),
            ));
        });
    ui.add_space(12.0);
    ui.separator();
    if !a.description.is_empty() && ui.button("Read official bulletin aloud").clicked() {
        crate::speech::enable();
        crate::speech::speak(&format!(
            "{}. {}. {}\n{}",
            a.event, a.area, a.description, a.instruction
        ));
    }
    ui.label(egui::RichText::new("Official bulletin").size(18.0).strong());
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt((&a.id, "bulletin"))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        // A fixed reading viewport must not grow with the window's previous content size.
        .max_height((ui.ctx().content_rect().height() * 0.4).clamp(120.0, 360.0))
        .auto_shrink([false, false])
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
            if a.description.is_empty() {
                ui.weak(
                    "The source provides the watch area and timing, but no official bulletin text.",
                );
                return;
            }
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
    ui.weak(if a.description.is_empty() {
        "Alert metadata · official bulletin unavailable"
    } else {
        "Official alert bulletin · source text as issued"
    });
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

    #[test]
    fn overlapping_alerts_put_the_highest_priority_first() {
        let card = |event: &str| WarnCard {
            color: [255; 4],
            info: AlertInfo {
                event: event.into(),
                id: event.into(),
                headline: String::new(),
                area: String::new(),
                description: String::new(),
                instruction: String::new(),
                expires: None,
                max_hail_in: None,
                max_wind: None,
                tornado_detection: None,
                damage_threat: None,
                source: None,
                motion: None,
                vtec: None,
            },
        };
        let mut cards = vec![card("Flood Advisory"), card("Tornado Warning")];
        sort_cards(&mut cards);
        assert_eq!(cards[0].info.event, "Tornado Warning");
    }
}
