//! User alert rules: does this detection, at this place, satisfy that rule?
//!
//! The app has five built-in alerts, and they fire on what somebody else decided was worth
//! waking up for. A chaser wants "rotation over 45 knots anywhere near the radar"; somebody with
//! a roof wants "a hail spike within twenty miles of home" and nothing else, ever. Same
//! detections, different questions.
//!
//! Everything here is pure: the app hands in a point and a strength, this says fire or not and
//! why. The cooldown, the notification and the layer plumbing live in `app.rs` — this file is
//! the part that can be tested without a GPU.

use crate::settings::{AlertRule, RuleCombinator, RulePlace, RuleTrigger, Settings};

/// One thing a rule could fire on: where it is, and how strong it is in the trigger's own units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    pub lon: f64,
    pub lat: f64,
    /// Vrot in knots, a ProbSevere percentage, a flash count — whatever the trigger measures.
    /// `None` for triggers that are their own answer, like a debris signature.
    pub strength: Option<f64>,
}

impl Detection {
    pub fn at(lon: f64, lat: f64) -> Self {
        Self {
            lon,
            lat,
            strength: None,
        }
    }

    pub fn with_strength(lon: f64, lat: f64, strength: f64) -> Self {
        Self {
            lon,
            lat,
            strength: Some(strength),
        }
    }
}

/// Does `rule` accept `hit`? Trigger matching is the caller's job — it only asks about rules
/// whose trigger it is currently evaluating — so this answers the other two questions: is the
/// detection strong enough, and is it somewhere this rule cares about.
pub fn matches(rule: &AlertRule, hit: &Detection, settings: &Settings) -> bool {
    rule.enabled && meets_threshold(rule, hit) && place_contains(&rule.place, hit, settings)
}

/// A rule with no threshold, or a trigger that takes none, accepts anything it is handed. A rule
/// with one and a detection that cannot report its strength does not fire: silence beats a
/// "45 kt" alert for something never measured.
pub fn meets_threshold(rule: &AlertRule, hit: &Detection) -> bool {
    match (rule.threshold, rule.trigger.threshold_hint()) {
        (Some(min), Some(_)) => hit.strength.is_some_and(|v| v >= min),
        _ => true,
    }
}

/// Is the detection inside the rule's place?
///
/// A marker uses its own watch radius — the same number the built-in proximity alerts use, so a
/// rule about home means the same distance as everything else about home. A zone is the drawn
/// polygon itself. A place that no longer exists (deleted marker, renamed zone) never matches;
/// the rules window shows those as dangling rather than silently firing everywhere.
pub fn place_contains(place: &RulePlace, hit: &Detection, settings: &Settings) -> bool {
    match place {
        RulePlace::Anywhere => true,
        RulePlace::Marker { id } => settings
            .markers
            .iter()
            .find(|m| &m.id == id)
            .is_some_and(|m| {
                let (km, _) = crate::geo::great_circle([m.lon, m.lat], [hit.lon, hit.lat]);
                km <= m.alert_radius_mi * crate::geo::KM_PER_MILE
            }),
        RulePlace::Zone { name } => settings
            .alert_polygons
            .iter()
            .find(|z| &z.name == name)
            .is_some_and(|z| wxdata::overlay::point_in_ring(&z.ring, hit.lon, hit.lat)),
    }
}

/// Something that fired recently, kept so a compound rule can ask "and was there also…".
#[derive(Debug, Clone, PartialEq)]
pub struct RecentHit {
    pub trigger: RuleTrigger2,
    pub hit: Detection,
    /// Minutes since it was seen.
    pub age_min: f64,
}

/// Compound conditions are matched on trigger *kind*, not on the warning text a `Warning` trigger
/// carries — a rule saying "rotation and also a warning" means any warning, and writing the text
/// twice would be a second place to get it wrong.
pub type RuleTrigger2 = RuleTrigger;

/// How recently an extra condition must have been satisfied to count. A storm does not do two
/// things in the same second, and it does not stay the same storm for an hour.
pub const COMPOUND_WINDOW_MIN: f64 = 15.0;

/// How close to the primary detection an extra condition must be. Two signatures 100 km apart are
/// two storms, and a rule about one of them should not fire on the other.
pub const COMPOUND_NEAR_KM: f64 = 20.0;

/// Do the rule's extra conditions hold around `hit`?
///
/// The rule's own trigger is not re-checked here — it is what started the evaluation. A rule with
/// no extra conditions is unchanged from before this existed, which is what makes every rule in
/// every existing settings file keep working.
pub fn compound_ok(rule: &AlertRule, hit: &Detection, recent: &[RecentHit]) -> bool {
    if rule.conditions.is_empty() {
        return true;
    }
    let holds = |c: &crate::settings::RuleCondition| {
        recent.iter().any(|r| {
            r.trigger == c.trigger
                && r.age_min <= COMPOUND_WINDOW_MIN
                && crate::geo::great_circle([hit.lon, hit.lat], [r.hit.lon, r.hit.lat]).0
                    <= COMPOUND_NEAR_KM
                && match c.threshold {
                    Some(min) => r.hit.strength.is_some_and(|v| v >= min),
                    None => true,
                }
        })
    };
    match rule.combine {
        RuleCombinator::And => rule.conditions.iter().all(holds),
        RuleCombinator::Or => rule.conditions.iter().any(holds),
    }
}

/// Does a warning rule match this event name? Empty text matches every warning, which is how a
/// rule says "anything the office issues near my house".
pub fn warning_matches(rule: &AlertRule, event: &str) -> bool {
    match &rule.trigger {
        RuleTrigger::Warning { event_contains } => {
            let want = event_contains.trim().to_ascii_lowercase();
            want.is_empty() || event.to_ascii_lowercase().contains(&want)
        }
        _ => false,
    }
}

/// What a place is called in a cooldown key and in the notification body.
pub fn place_label(place: &RulePlace, settings: &Settings) -> String {
    match place {
        RulePlace::Anywhere => "anywhere".to_string(),
        RulePlace::Marker { id } => settings
            .markers
            .iter()
            .find(|m| &m.id == id)
            .map_or_else(|| format!("marker {id}"), |m| m.name.clone()),
        RulePlace::Zone { name } => name.clone(),
    }
}

/// Does the place a rule names still exist? A rule pointed at a deleted marker can never fire,
/// and saying so is better than leaving the user to wonder why it is quiet.
pub fn place_exists(place: &RulePlace, settings: &Settings) -> bool {
    match place {
        RulePlace::Anywhere => true,
        RulePlace::Marker { id } => settings.markers.iter().any(|m| &m.id == id),
        RulePlace::Zone { name } => settings.alert_polygons.iter().any(|z| &z.name == name),
    }
}

/// The Severe percentage out of a ProbSevere feature's detail block.
///
/// ponytail: parsed back out of the text the overlay already built (`probsevere.rs`, "Severe:
/// 62%"), rather than widening `GeoFeature` with numeric fields for one consumer. Add the fields
/// if a second one turns up.
pub fn probsevere_percent(detail: &str) -> Option<f64> {
    let rest = detail.split("Severe:").nth(1)?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AlertPolygon, Marker};

    fn settings_with_home() -> Settings {
        let mut s = Settings::default();
        s.markers.push(Marker {
            id: "home1234".into(),
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
            ring: vec![
                [-98.0, 35.0],
                [-97.0, 35.0],
                [-97.0, 36.0],
                [-98.0, 36.0],
                [-98.0, 35.0],
            ],
        });
        s
    }

    fn rule(trigger: RuleTrigger, place: RulePlace) -> AlertRule {
        let mut r = AlertRule::new(trigger);
        r.place = place;
        r.enabled = true;
        r
    }

    #[test]
    fn a_marker_rule_fires_inside_its_own_watch_radius_and_not_outside() {
        let s = settings_with_home();
        let r = rule(
            RuleTrigger::Tds,
            RulePlace::Marker {
                id: "home1234".into(),
            },
        );
        // ~9 km east of home, well inside the 20-mile radius.
        assert!(matches(&r, &Detection::at(-97.4, 35.3), &s));
        // ~180 km away.
        assert!(!matches(&r, &Detection::at(-95.5, 35.3), &s));
        // A rule pointed at a marker that no longer exists fires nowhere.
        let orphan = rule(
            RuleTrigger::Tds,
            RulePlace::Marker {
                id: "deleted0".into(),
            },
        );
        assert!(!matches(&orphan, &Detection::at(-97.5, 35.3), &s));
        assert!(!place_exists(&orphan.place, &s));
    }

    #[test]
    fn a_zone_rule_uses_the_drawn_polygon() {
        let s = settings_with_home();
        let r = rule(
            RuleTrigger::Tbss,
            RulePlace::Zone {
                name: "The valley".into(),
            },
        );
        assert!(matches(&r, &Detection::at(-97.5, 35.5), &s));
        assert!(!matches(&r, &Detection::at(-96.0, 35.5), &s));
        assert_eq!(place_label(&r.place, &s), "The valley");
    }

    #[test]
    fn thresholds_gate_the_triggers_that_measure_something() {
        let s = Settings::default();
        let mut r = rule(RuleTrigger::Rotation, RulePlace::Anywhere);
        r.threshold = Some(45.0);
        assert!(matches(&r, &Detection::with_strength(-97.5, 35.3, 52.0), &s));
        assert!(!matches(&r, &Detection::with_strength(-97.5, 35.3, 30.0), &s));
        // Exactly at the threshold counts — the user asked for "45 and up".
        assert!(matches(&r, &Detection::with_strength(-97.5, 35.3, 45.0), &s));
        // A measured trigger with nothing measured stays quiet.
        assert!(!matches(&r, &Detection::at(-97.5, 35.3), &s));
        // A trigger that takes no threshold ignores one that got set anyway.
        let mut tds = rule(RuleTrigger::Tds, RulePlace::Anywhere);
        tds.threshold = Some(99.0);
        assert!(matches(&tds, &Detection::at(-97.5, 35.3), &s));
    }

    #[test]
    fn compound_conditions_need_the_same_storm_and_a_recent_one() {
        let mut r = rule(RuleTrigger::Rotation, RulePlace::Anywhere);
        r.conditions = vec![crate::settings::RuleCondition {
            trigger: RuleTrigger::Tds,
            threshold: None,
        }];
        let hit = Detection::with_strength(-97.5, 35.3, 50.0);
        let near = |age_min: f64, lon: f64| RecentHit {
            trigger: RuleTrigger::Tds,
            hit: Detection::at(lon, 35.3),
            age_min,
        };
        // Same storm, two minutes ago.
        assert!(compound_ok(&r, &hit, &[near(2.0, -97.45)]));
        // Same place, but an hour ago — a different storm by now.
        assert!(!compound_ok(&r, &hit, &[near(60.0, -97.45)]));
        // Recent, but 200 km away.
        assert!(!compound_ok(&r, &hit, &[near(2.0, -95.3)]));
        // Nothing at all.
        assert!(!compound_ok(&r, &hit, &[]));
    }

    #[test]
    fn and_needs_both_conditions_and_or_needs_one() {
        let mut r = rule(RuleTrigger::Rotation, RulePlace::Anywhere);
        r.conditions = vec![
            crate::settings::RuleCondition {
                trigger: RuleTrigger::Tds,
                threshold: None,
            },
            crate::settings::RuleCondition {
                trigger: RuleTrigger::ProbSevere,
                threshold: Some(70.0),
            },
        ];
        let hit = Detection::with_strength(-97.5, 35.3, 50.0);
        let tds = RecentHit {
            trigger: RuleTrigger::Tds,
            hit: Detection::at(-97.5, 35.3),
            age_min: 1.0,
        };
        let weak_ps = RecentHit {
            trigger: RuleTrigger::ProbSevere,
            hit: Detection::with_strength(-97.5, 35.3, 40.0),
            age_min: 1.0,
        };
        let strong_ps = RecentHit {
            trigger: RuleTrigger::ProbSevere,
            hit: Detection::with_strength(-97.5, 35.3, 85.0),
            age_min: 1.0,
        };
        r.combine = RuleCombinator::And;
        assert!(!compound_ok(&r, &hit, std::slice::from_ref(&tds)));
        assert!(!compound_ok(&r, &hit, &[tds.clone(), weak_ps.clone()]));
        assert!(compound_ok(&r, &hit, &[tds.clone(), strong_ps]));
        r.combine = RuleCombinator::Or;
        assert!(compound_ok(&r, &hit, &[tds]));
        assert!(!compound_ok(&r, &hit, &[weak_ps]));
    }

    #[test]
    fn an_old_rule_deserializes_with_no_conditions_and_behaves_as_before() {
        // The shape a settings file written before compound rules existed carries.
        let json = r#"{"id":"abc","trigger":"Tds","place":"Anywhere","cooldown_min":10,
                       "enabled":true}"#;
        let r: AlertRule = serde_json::from_str(json).expect("old rules must still read");
        assert!(r.conditions.is_empty());
        assert_eq!(r.combine, RuleCombinator::And);
        assert!(r.sound.is_none() && !r.snapshot);
        assert!(compound_ok(&r, &Detection::at(-97.5, 35.3), &[]));
    }

    #[test]
    fn a_disabled_rule_never_fires() {
        let s = Settings::default();
        let mut r = rule(RuleTrigger::Tds, RulePlace::Anywhere);
        r.enabled = false;
        assert!(!matches(&r, &Detection::at(-97.5, 35.3), &s));
    }

    #[test]
    fn warning_rules_match_on_the_event_name() {
        let all = AlertRule::new(RuleTrigger::Warning {
            event_contains: String::new(),
        });
        assert!(warning_matches(&all, "Special Marine Warning"));
        let tor = AlertRule::new(RuleTrigger::Warning {
            event_contains: "tornado".into(),
        });
        assert!(warning_matches(&tor, "Tornado Warning"));
        assert!(!warning_matches(&tor, "Severe Thunderstorm Warning"));
        // A non-warning trigger is not a warning match, whatever the text says.
        assert!(!warning_matches(&AlertRule::new(RuleTrigger::Tds), "Tornado Warning"));
    }

    #[test]
    fn probsevere_percentages_come_back_out_of_the_detail_text() {
        let detail = "ProbSevere storm 4213\nSevere: 62%\nTornado: 5%\nHail: 40%\nWind: 31%";
        assert_eq!(probsevere_percent(detail), Some(62.0));
        assert_eq!(probsevere_percent("Severe: 0%"), Some(0.0));
        assert_eq!(probsevere_percent("no percentages here"), None);
    }
}
