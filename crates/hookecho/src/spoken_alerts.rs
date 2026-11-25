//! Session-local deduplication for visible-map speech, independent of push notifications.
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use wxdata::overlay::AlertInfo;

#[derive(Default)]
pub struct SpokenAlerts {
    seen: HashMap<String, SpokenEvent>,
}

struct SpokenEvent {
    highest_tier: u8,
    instructions: HashSet<String>,
    expires: DateTime<Utc>,
}

impl SpokenAlerts {
    /// Pick one current bulletin per nearby event before touching the speech ledger.
    pub fn new_warnings<'a>(
        &mut self,
        alerts: impl IntoIterator<Item = &'a AlertInfo>,
        now: DateTime<Utc>,
        min_tier: u8,
    ) -> Vec<(u8, &'a AlertInfo)> {
        self.seen.retain(|_, seen| seen.expires > now);
        let mut active = HashMap::new();
        for alert in alerts {
            let Some(expires) = alert.expires.filter(|expires| *expires > now) else {
                continue;
            };
            let tier = wxdata::alerts::escalation(alert);
            if tier < min_tier {
                continue;
            }
            // Favor severity over recency. Old snapshots without issuance times still have
            // a stable order, independent of the order in which geometry parts arrive.
            let rank = (tier, alert.sent_at, expires, alert.id.as_str());
            let (selected, selected_rank, event_expires) = active
                .entry(alert.event_key())
                .or_insert((alert, rank, expires));
            if rank > *selected_rank {
                (*selected, *selected_rank) = (alert, rank);
            }
            *event_expires = (*event_expires).max(expires);
        }

        let mut announcements = Vec::new();
        for (key, (alert, (tier, ..), expires)) in active {
            let instructions = alert
                .instruction
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            match self.seen.entry(key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(SpokenEvent {
                        highest_tier: tier,
                        instructions: HashSet::from([instructions]),
                        expires,
                    });
                    announcements.push((tier, alert));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let seen = entry.get_mut();
                    seen.expires = seen.expires.max(expires);
                    let escalated = tier > seen.highest_tier;
                    if tier >= seen.highest_tier {
                        seen.highest_tier = tier;
                        let changed_instructions =
                            !instructions.is_empty() && seen.instructions.insert(instructions);
                        if escalated || changed_instructions {
                            announcements.push((tier, alert));
                        }
                    }
                }
            }
        }
        announcements.sort_unstable_by(|(tier, alert), (other_tier, other)| {
            other_tier.cmp(tier).then_with(|| alert.id.cmp(&other.id))
        });
        announcements
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_bulletins_for_one_event_read_once_then_stay_silent() {
        let mut ledger = SpokenAlerts::default();
        let now = Utc::now();
        let mut original = wxdata::spoken::demo_alert();
        original.expires = Some(now + chrono::Duration::minutes(30));
        original.vtec = Some("/O.NEW.KFWD.TO.W.0001.261124T2300Z-261125T0200Z/".into());
        let mut continuation = original.clone();
        continuation.id = "continued".into();
        continuation.vtec = Some("/O.CON.KFWD.TO.W.0001.261124T2300Z-261125T0200Z/".into());
        continuation.headline = "Continued tornado warning".into();
        assert_eq!(original.event_key(), continuation.event_key());
        assert_eq!(
            wxdata::spoken::warning_script(&original, "", ""),
            wxdata::spoken::warning_script(&continuation, "", "")
        );
        original.sent_at = Some(now - chrono::Duration::minutes(10));
        continuation.sent_at = Some(now);
        assert_eq!(
            ledger.new_warnings([&original, &continuation], now, 0),
            vec![(2, &continuation)]
        );
        assert!(ledger
            .new_warnings([&continuation, &original], now, 0)
            .is_empty());
        assert!(ledger.new_warnings([&original], now, 0).is_empty());
    }

    #[test]
    fn editorial_changes_are_silent_but_escalations_speak() {
        let mut ledger = SpokenAlerts::default();
        let now = Utc::now();
        let mut alert = wxdata::spoken::demo_alert();
        alert.expires = Some(now + chrono::Duration::minutes(30));
        alert.tornado_detection = None;
        alert.damage_threat = None;
        assert_eq!(ledger.new_warnings([&alert], now, 0).len(), 1);
        alert.headline = "Warning continued".into();
        alert.area.push_str("; Grady, OK");
        alert.expires = Some(now + chrono::Duration::minutes(45));
        assert!(ledger.new_warnings([&alert], now, 0).is_empty());
        alert.damage_threat = Some("CATASTROPHIC".into());
        assert_eq!(ledger.new_warnings([&alert], now, 0), vec![(2, &alert)]);
        assert!(ledger.new_warnings([&alert], now, 0).is_empty());
    }

    #[test]
    fn repeats_are_silent_but_changed_instructions_speak() {
        let mut ledger = SpokenAlerts::default();
        let now = Utc::now();
        let mut alert = wxdata::spoken::demo_alert();
        alert.expires = Some(now + chrono::Duration::minutes(30));
        assert_eq!(ledger.new_warnings([&alert], now, 0).len(), 1);
        assert!(ledger.new_warnings([&alert], now, 0).is_empty());
        let original = alert.clone();
        alert.instruction.push_str(" Stay indoors.");
        alert.sent_at = Some(now);
        assert_eq!(ledger.new_warnings([&alert], now, 0).len(), 1);
        assert!(ledger.new_warnings([&original, &alert], now, 0).is_empty());
        assert!(ledger.new_warnings([&original], now, 0).is_empty());
        assert!(ledger
            .new_warnings([&alert], now + chrono::Duration::hours(1), 0)
            .is_empty());
        alert.expires = None;
        assert!(ledger.new_warnings([&alert], now, 0).is_empty());
    }
}
