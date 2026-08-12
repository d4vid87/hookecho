//! The rules table: what the user wants to be told about, in their own terms.
//!
//! One row per rule, edited in place. Adding a rule creates it switched *off* — a rule armed the
//! instant it appears would start pushing before its threshold and place had been chosen, which
//! is exactly the behaviour that teaches people to ignore notifications.
//!
//! ponytail: delete-and-re-add rather than a modal editor, the same stance `AlertPolygon` takes.
//! Every field is on the row already; there is nothing a dialog would add but a dialog.

use crate::settings::{AlertRule, RulePlace, RuleTrigger, Settings};

#[derive(Default)]
pub struct RulesWindow {
    pub open: bool,
}

impl RulesWindow {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

pub fn show(state: &mut RulesWindow, ctx: &egui::Context, settings: &mut Settings) -> bool {
    if !state.open {
        return false;
    }
    let mut open = state.open;
    let mut changed = false;
    let window = egui::Window::new("Alert rules")
        .open(&mut open)
        .default_width(720.0)
        .resizable(true);
    crate::ui::phone_surface(ctx, window).show(ctx, |ui| {
        ui.weak(
            "Tell the app what is worth interrupting you for. New rules start switched off — \
             set the threshold and the place, then arm it.",
        );
        ui.separator();

        let places = place_options(settings);
        let mut remove: Option<usize> = None;
        egui::Grid::new("rules_grid")
            .num_columns(7)
            .spacing([8.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label("On");
                ui.label("Name");
                ui.label("Trigger");
                ui.label("At least");
                ui.label("Where");
                ui.label("Urgent");
                ui.label("Cooldown");
                ui.end_row();

                for i in 0..settings.alert_rules.len() {
                    let dangling = {
                        let r = &settings.alert_rules[i];
                        !crate::rules::place_exists(&r.place, settings)
                    };
                    let rule = &mut settings.alert_rules[i];
                    changed |= ui.checkbox(&mut rule.enabled, "").changed();

                    let name = ui.add(
                        egui::TextEdit::singleline(&mut rule.name)
                            .hint_text(rule.trigger.label())
                            .desired_width(120.0),
                    );
                    changed |= name.changed();

                    changed |= trigger_combo(ui, i, rule);
                    changed |= threshold_widget(ui, rule);
                    changed |= place_combo(ui, i, rule, &places);

                    changed |= ui
                        .checkbox(&mut rule.urgent, "")
                        .on_hover_text("Push through quiet hours")
                        .changed();

                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut rule.cooldown_min)
                                    .range(1..=240)
                                    .suffix(" min"),
                            )
                            .on_hover_text("How long before this rule may fire for the same place again")
                            .changed();
                        if ui.button("🗑").on_hover_text("Delete this rule").clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.end_row();

                    if dangling {
                        ui.label("");
                        ui.colored_label(
                            egui::Color32::from_rgb(240, 170, 60),
                            "⚠ that place no longer exists — this rule can never fire",
                        );
                        ui.end_row();
                    }
                }
            });

        if let Some(i) = remove {
            settings.alert_rules.remove(i);
            changed = true;
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("➕ Add rule").clicked() {
                settings.alert_rules.push(AlertRule::new(RuleTrigger::Tds));
                changed = true;
            }
            ui.weak(format!(
                "{} armed of {}",
                settings.alert_rules.iter().filter(|r| r.enabled).count(),
                settings.alert_rules.len()
            ));
        });
        if settings.alert_rules.is_empty() {
            ui.add_space(4.0);
            ui.weak(
                "Nothing here yet. The five built-in alerts (warnings, debris signatures, \
                 rotation, lightning and rain arrival) keep working regardless.",
            );
        }
    });
    state.open = open;
    changed
}

/// Every place a rule can name: anywhere, each marker, each drawn zone.
fn place_options(settings: &Settings) -> Vec<(RulePlace, String)> {
    let mut out = vec![(RulePlace::Anywhere, "Anywhere".to_string())];
    for m in &settings.markers {
        out.push((
            RulePlace::Marker { id: m.id.clone() },
            format!("{} ({:.0} mi)", m.name, m.alert_radius_mi),
        ));
    }
    for z in &settings.alert_polygons {
        out.push((
            RulePlace::Zone {
                name: z.name.clone(),
            },
            format!("zone: {}", z.name),
        ));
    }
    out
}

fn trigger_combo(ui: &mut egui::Ui, i: usize, rule: &mut AlertRule) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("rule_trigger", i))
        .selected_text(rule.trigger.label())
        .width(170.0)
        .show_ui(ui, |ui| {
            for t in RuleTrigger::ALL {
                let label = t.label();
                // Same variant, different payload: a warning rule keeps whatever text it has.
                let selected = std::mem::discriminant(&rule.trigger) == std::mem::discriminant(&t);
                if ui.selectable_label(selected, label).clicked() && !selected {
                    rule.threshold = t.threshold_hint().map(|(_, d)| d);
                    rule.trigger = t;
                    changed = true;
                }
            }
        });
    // A warning rule needs its text, and it is meaningless on any other trigger.
    if let RuleTrigger::Warning { event_contains } = &mut rule.trigger {
        changed |= ui
            .add(
                egui::TextEdit::singleline(event_contains)
                    .hint_text("any warning")
                    .desired_width(110.0),
            )
            .on_hover_text("Matches when the event name contains this — e.g. \"tornado\"")
            .changed();
    }
    changed
}

/// The threshold cell: a number in the trigger's own units, or a dash where it has none.
fn threshold_widget(ui: &mut egui::Ui, rule: &mut AlertRule) -> bool {
    match rule.trigger.threshold_hint() {
        Some((suffix, default)) => {
            let mut v = rule.threshold.unwrap_or(default);
            let changed = ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(0.0..=200.0)
                        .suffix(format!(" {suffix}")),
                )
                .changed();
            if changed {
                rule.threshold = Some(v);
            }
            changed
        }
        None => {
            ui.weak("—");
            false
        }
    }
}

fn place_combo(
    ui: &mut egui::Ui,
    i: usize,
    rule: &mut AlertRule,
    places: &[(RulePlace, String)],
) -> bool {
    let mut changed = false;
    let current = places
        .iter()
        .find(|(p, _)| p == &rule.place)
        .map(|(_, l)| l.clone())
        .unwrap_or_else(|| "(missing)".to_string());
    egui::ComboBox::from_id_salt(("rule_place", i))
        .selected_text(current)
        .width(170.0)
        .show_ui(ui, |ui| {
            for (place, label) in places {
                if ui
                    .selectable_label(&rule.place == place, label)
                    .clicked()
                {
                    rule.place = place.clone();
                    changed = true;
                }
            }
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AlertPolygon, Marker};

    #[test]
    fn the_place_list_offers_anywhere_then_every_marker_and_zone() {
        let mut s = Settings::default();
        s.markers.push(Marker {
            id: "abc12345".into(),
            name: "Home".into(),
            lat: 35.3,
            lon: -97.5,
            icon: None,
            alert_radius_mi: 20.0,
            video_url: String::new(),
            home: true,
        });
        s.alert_polygons.push(AlertPolygon {
            name: "The valley".into(),
            ring: vec![[-98.0, 35.0], [-97.0, 35.0], [-97.0, 36.0], [-98.0, 35.0]],
        });
        let places = place_options(&s);
        assert_eq!(places[0].0, RulePlace::Anywhere);
        assert_eq!(
            places[1].0,
            RulePlace::Marker {
                id: "abc12345".into()
            }
        );
        assert!(places[1].1.contains("Home") && places[1].1.contains("20 mi"));
        assert_eq!(
            places[2].0,
            RulePlace::Zone {
                name: "The valley".into()
            }
        );
    }
}
