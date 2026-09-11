//! Session-local deduplication for visible-map speech, independent of push notifications.
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use wxdata::overlay::AlertInfo;

#[derive(Default)]
pub struct SpokenAlerts {
    seen: HashMap<String, (String, DateTime<Utc>)>,
}

impl SpokenAlerts {
    /// Call only after geographic, mute and quiet-hours checks. Never consume an alert that
    /// was not eligible to speak. Expiration bounds both memory and replay suppression.
    pub fn take(&mut self, alert: &AlertInfo, now: DateTime<Utc>) -> bool {
        self.seen.retain(|_, (_, expires)| *expires > now);
        let Some(expires) = alert.expires.filter(|expires| *expires > now) else { return false };
        let key = alert.event_key();
        let revision = format!("{}\n{}\n{}\n{}\n{}", alert.event, alert.headline,
            alert.area, alert.instruction, expires.timestamp());
        if self.seen.get(&key).is_some_and(|(old, _)| old == &revision) {
            return false;
        }
        self.seen.insert(key, (revision, expires));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeats_are_silent_but_changed_instructions_speak() {
        let mut ledger = SpokenAlerts::default();
        let now = Utc::now();
        let mut alert = wxdata::spoken::demo_alert();
        alert.expires = Some(now + chrono::Duration::minutes(30));
        assert!(ledger.take(&alert, now));
        assert!(!ledger.take(&alert, now));
        alert.instruction.push_str(" Stay indoors.");
        assert!(ledger.take(&alert, now));
        assert!(!ledger.take(&alert, now + chrono::Duration::hours(1)));
        alert.expires = None;
        assert!(!ledger.take(&alert, now));
    }
}
