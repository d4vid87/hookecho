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
    /// A backtest the user asked for, picked up by the app (which owns the runtime) on its next
    /// frame: rule index, and the UTC day to replay.
    #[cfg(not(target_arch = "wasm32"))]
    pub backtest_request: Option<(usize, chrono::NaiveDate)>,
    /// The run in flight or the last one's result.
    #[cfg(not(target_arch = "wasm32"))]
    pub backtest: Option<crate::backtest::Shared>,
    /// The day the buttons replay, edited as text because a date picker is a dependency.
    #[cfg(not(target_arch = "wasm32"))]
    pub backtest_day: String,
}

impl RulesWindow {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

pub fn show(
    state: &mut RulesWindow,
    ctx: &egui::Context,
    settings: &mut Settings,
    drawer: &mut crate::ui::drawer::Drawer,
) -> bool {
    if !state.open {
        return false;
    }
    let mut open = state.open;
    let mut changed = false;
    // The gear carries the one knob that decides whether any of these rules can reach the user
    // at all — the page is pointless with it off, and it lives three tabs deep in Settings.
    let gear = drawer.gear;
    let Some(window) = drawer.page(
        ctx,
        "Alert rules",
        &mut open,
        true,
        egui::Window::new("Alert rules"),
    ) else {
        state.open = open;
        return false;
    };
    window.show(ctx, |ui| {
        if gear {
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut settings.mute_alerts, "Mute all alerts")
                    .changed()
                {
                    changed = true;
                }
            });
            ui.separator();
        }
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
                            .on_hover_text(
                                "How long before this rule may fire for the same place again",
                            )
                            .changed();
                        if ui.button("🗑").on_hover_text("Delete this rule").clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.end_row();

                    // Second row per rule: the parts that are usually left alone — extra
                    // conditions, a sound, a snapshot. Below the rule rather than beside it, so
                    // the grid stays readable for the common single-condition case.
                    ui.label("");
                    ui.label("");
                    extra_row(ui, i, &mut settings.alert_rules[i], &mut changed);
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

        #[cfg(not(target_arch = "wasm32"))]
        backtest_bar(ui, state, settings);

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
                if ui.selectable_label(&rule.place == place, label).clicked() {
                    rule.place = place.clone();
                    changed = true;
                }
            }
        });
    changed
}

/// Replay one rule against an archive day, and say what it would have done.
///
/// The window has no tokio runtime and no HTTP client, so the button records a request the app
/// picks up on its next frame — the same handover the voice download and the file picker use.
#[cfg(not(target_arch = "wasm32"))]
fn backtest_bar(ui: &mut egui::Ui, state: &mut RulesWindow, settings: &Settings) {
    ui.add_space(6.0);
    ui.separator();
    if state.backtest_day.is_empty() {
        state.backtest_day = chrono::Utc::now().date_naive().to_string();
    }
    let running = state
        .backtest
        .as_ref()
        .and_then(|s| s.lock().ok().map(|p| p.finished.is_none()))
        .unwrap_or(false);
    ui.horizontal(|ui| {
        ui.strong("Backtest");
        ui.label("day (UTC):");
        ui.add(
            egui::TextEdit::singleline(&mut state.backtest_day)
                .desired_width(96.0)
                .hint_text("YYYY-MM-DD"),
        );
        let day = state.backtest_day.parse::<chrono::NaiveDate>().ok();
        let armed: Vec<usize> = settings
            .alert_rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.trigger.is_scan())
            .map(|(i, _)| i)
            .collect();
        ui.add_enabled_ui(day.is_some() && !running && !armed.is_empty(), |ui| {
            egui::ComboBox::from_id_salt("backtest_rule")
                .selected_text("Run a rule…")
                .show_ui(ui, |ui| {
                    for i in armed {
                        if ui.button(settings.alert_rules[i].title()).clicked() {
                            if let Some(d) = day {
                                state.backtest_request = Some((i, d));
                            }
                        }
                    }
                });
        });
    });
    ui.weak(format!(
        "Replays up to {} volumes from the archive against one scan rule — the same detectors the \
         live path runs. Feed triggers (warnings, ProbSevere, lightning) and extra conditions are \
         not replayable.",
        crate::backtest::MAX_VOLUMES
    ));
    let Some(shared) = &state.backtest else {
        return;
    };
    let Ok(p) = shared.lock() else { return };
    if let Some(msg) = &p.finished {
        ui.label(msg);
    } else {
        ui.label(format!("checking volume {} of {}…", p.done, p.total.max(1)));
    }
    if !p.fired.is_empty() {
        ui.label(
            p.fired
                .iter()
                .map(|t| t.format("%H:%MZ").to_string())
                .collect::<Vec<_>>()
                .join("  "),
        );
    }
}

/// The per-rule extras: one level of AND/OR, a sound, and a snapshot attachment.
///
/// One level deliberately — a condition list with a combinator, not an expression tree. A rule
/// nobody can read at 3am is a rule nobody trusts, and the second level is where that starts.
fn extra_row(ui: &mut egui::Ui, i: usize, rule: &mut AlertRule, changed: &mut bool) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("rule_combine", i))
            .width(90.0)
            .selected_text(rule.combine.label())
            .show_ui(ui, |ui| {
                for c in crate::settings::RuleCombinator::ALL {
                    *changed |= ui
                        .selectable_value(&mut rule.combine, c, c.label())
                        .changed();
                }
            });
        let mut drop: Option<usize> = None;
        for (j, cond) in rule.conditions.iter_mut().enumerate() {
            egui::ComboBox::from_id_salt(("rule_cond", i, j))
                .width(130.0)
                .selected_text(cond.trigger.label())
                .show_ui(ui, |ui| {
                    for t in RuleTrigger::ALL {
                        let label = t.label();
                        if ui.selectable_label(cond.trigger == t, label).clicked() {
                            cond.threshold = t.threshold_hint().map(|(_, d)| d);
                            cond.trigger = t;
                            *changed = true;
                        }
                    }
                });
            if let Some((unit, default)) = cond.trigger.threshold_hint() {
                let mut v = cond.threshold.unwrap_or(default);
                if ui
                    .add(egui::DragValue::new(&mut v).suffix(format!(" {unit}")))
                    .changed()
                {
                    cond.threshold = Some(v);
                    *changed = true;
                }
            }
            if ui.small_button("✕").clicked() {
                drop = Some(j);
            }
        }
        if let Some(j) = drop {
            rule.conditions.remove(j);
            *changed = true;
        }
        if ui
            .button("➕ condition")
            .on_hover_text(format!(
                "Also require another signature within {:.0} km and {:.0} minutes",
                crate::rules::COMPOUND_NEAR_KM,
                crate::rules::COMPOUND_WINDOW_MIN,
            ))
            .clicked()
        {
            rule.conditions.push(crate::settings::RuleCondition {
                trigger: RuleTrigger::Tds,
                threshold: None,
            });
            *changed = true;
        }
    });
    ui.horizontal(|ui| {
        let mut on = rule.sound.is_some();
        if ui
            .checkbox(&mut on, "Sound")
            .on_hover_text("Play a sound when this rule fires")
            .changed()
        {
            rule.sound = on.then(crate::settings::AlertSound::default);
            *changed = true;
        }
        if let Some(sound) = &mut rule.sound {
            egui::ComboBox::from_id_salt(("rule_sound", i))
                .width(110.0)
                .selected_text(sound.label())
                .show_ui(ui, |ui| {
                    for s in crate::settings::AlertSound::BUILTINS {
                        *changed |= ui.selectable_value(sound, s.clone(), s.label()).changed();
                    }
                });
        }
        *changed |= ui
            .checkbox(&mut rule.snapshot, "Snapshot")
            .on_hover_text("Attach a picture of the map to this rule's push (desktop only)")
            .changed();
    });
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
