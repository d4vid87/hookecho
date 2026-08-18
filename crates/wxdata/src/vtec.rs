//! Parsing for the NWS P-VTEC string that rides on every warning-tier alert.
//!
//! An alert's `id` is per *message*, not per *event*. When an office continues a tornado warning
//! it issues a fresh message with a fresh id and the same warning underneath, which is why
//! deduping on `id` re-announces the same warning every few minutes. The VTEC string names the
//! event itself, and stays the same for the life of it:
//!
//! ```text
//! /O.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/
//!  │ │   │    │  │ │
//!  │ │   │    │  │ └─ event tracking number, unique per office/phenomenon/year
//!  │ │   │    │  └─── significance: W arning, A dvisory, Y (advisory), S tatement
//!  │ │   │    └────── phenomenon: TO rnado, SV severe thunderstorm, FF flash flood, …
//!  │ │   └─────────── issuing office
//!  │ └─────────────── action: NEW, CON, EXT, EXA, EXB, CAN, EXP, UPG, COR, ROU
//!  └───────────────── product class: O perational, T est, E xperimental
//! ```
//!
//! ponytail: P-VTEC only. The H-VTEC segment that follows it on hydrologic products carries flood
//! severity and crest times, which nothing here needs yet.

/// What an office is doing to the event with this message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A warning that did not exist before.
    New,
    /// Continued — the same warning, still in force. The common case, and the one that used to
    /// re-announce.
    Continued,
    /// Extended in time or area.
    Extended,
    /// Cancelled, expired, or upgraded away.
    Ended,
    /// Upgraded to a more serious product (severe thunderstorm to tornado, say).
    Upgraded,
    /// Correction, routine, or anything else.
    Other,
}

impl Action {
    fn parse(s: &str) -> Action {
        match s {
            "NEW" => Action::New,
            "CON" => Action::Continued,
            "EXT" | "EXA" | "EXB" => Action::Extended,
            "CAN" | "EXP" => Action::Ended,
            "UPG" => Action::Upgraded,
            _ => Action::Other,
        }
    }

    /// Does this action deserve a fresh notification, given we have already announced the event?
    ///
    /// Only an upgrade does. A continuation is the same warning restated, and announcing it again
    /// is the duplicate-notification bug.
    pub fn is_newsworthy_repeat(self) -> bool {
        matches!(self, Action::Upgraded)
    }
}

/// The parts of a P-VTEC string that identify an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vtec {
    pub action: Action,
    pub office: String,
    pub phenomenon: String,
    pub significance: String,
    pub etn: u16,
    /// Two-digit year from the event's start time, which is what makes `etn` unique — offices
    /// restart the numbering every January.
    pub year: u16,
}

impl Vtec {
    /// Identity of the *event*, stable across every message about it. This is what a seen-set
    /// should key on.
    pub fn event_key(&self) -> String {
        format!(
            "{}.{}.{}.{:04}.{}",
            self.office, self.phenomenon, self.significance, self.etn, self.year
        )
    }

    /// Parse the first P-VTEC line out of `s`, which may be a bare VTEC string or a block of text
    /// containing one.
    pub fn parse(s: &str) -> Option<Vtec> {
        for line in s.lines() {
            let line = line.trim();
            // A P-VTEC string is slash-delimited with exactly six dot-separated fields before the
            // time range.
            let Some(inner) = line.strip_prefix('/') else {
                continue;
            };
            let inner = inner.strip_suffix('/').unwrap_or(inner);
            // class.action.office.phenomenon.significance.etn.start-end
            let parts: Vec<&str> = inner.split('.').collect();
            if parts.len() < 7 {
                continue;
            }
            // parts[0] is the product class (O/T/E); a test product must never notify anyone.
            if parts[0] != "O" {
                continue;
            }
            let Ok(etn) = parts[5].parse::<u16>() else {
                continue;
            };
            // The time range is "130520T2012Z-130520T2045Z"; the first two digits are the year.
            // A continuation zeroes its start time, so fall back to the end time — the event does
            // not span a new year between messages.
            let Some(year) = parts[6]
                .split('-')
                .find(|t| !t.starts_with("000000"))
                .and_then(|t| t.get(..2))
                .and_then(|y| y.parse::<u16>().ok())
                .map(|y| 2000 + y)
            else {
                continue;
            };
            return Some(Vtec {
                action: Action::parse(parts[1]),
                office: parts[2].to_string(),
                phenomenon: parts[3].to_string(),
                significance: parts[4].to_string(),
                etn,
                year,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOR: &str = "/O.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/";
    const TOR_CON: &str = "/O.CON.KOUN.TO.W.0071.000000T0000Z-130520T2045Z/";

    #[test]
    fn parses_a_tornado_warning() {
        let v = Vtec::parse(TOR).unwrap();
        assert_eq!(v.action, Action::New);
        assert_eq!(v.office, "KOUN");
        assert_eq!(v.phenomenon, "TO");
        assert_eq!(v.significance, "W");
        assert_eq!(v.etn, 71);
        assert_eq!(v.year, 2013);
    }

    /// The whole point: a continuation is the *same event*, so it must produce the same key.
    #[test]
    fn a_continuation_shares_the_events_identity() {
        let new = Vtec::parse(TOR).unwrap();
        let con = Vtec::parse(TOR_CON).unwrap();
        assert_eq!(new.event_key(), con.event_key());
        assert_eq!(con.action, Action::Continued);
        assert!(!con.action.is_newsworthy_repeat());
    }

    /// Different offices, phenomena, significances and years never collide, because event
    /// tracking numbers are only unique within one of each.
    #[test]
    fn keys_separate_events_that_share_a_number() {
        let k = |s: &str| Vtec::parse(s).unwrap().event_key();
        let base = k("/O.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/");
        assert_ne!(base, k("/O.NEW.KFWD.TO.W.0071.130520T2012Z-130520T2045Z/"));
        assert_ne!(base, k("/O.NEW.KOUN.SV.W.0071.130520T2012Z-130520T2045Z/"));
        assert_ne!(base, k("/O.NEW.KOUN.TO.A.0071.130520T2012Z-130520T2045Z/"));
        assert_ne!(base, k("/O.NEW.KOUN.TO.W.0071.140520T2012Z-140520T2045Z/"));
    }

    #[test]
    fn an_upgrade_is_worth_announcing_and_a_continuation_is_not() {
        let upg = Vtec::parse("/O.UPG.KOUN.SV.W.0142.130520T2012Z-130520T2045Z/").unwrap();
        assert!(upg.action.is_newsworthy_repeat());
        for a in ["CON", "EXT", "EXA", "COR", "ROU"] {
            let s = format!("/O.{a}.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/");
            assert!(
                !Vtec::parse(&s).unwrap().action.is_newsworthy_repeat(),
                "{a} should not re-announce"
            );
        }
    }

    /// Test and experimental products carry the same shape and must never be treated as real.
    #[test]
    fn non_operational_products_are_not_events() {
        assert!(Vtec::parse("/T.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/").is_none());
        assert!(Vtec::parse("/E.NEW.KOUN.TO.W.0071.130520T2012Z-130520T2045Z/").is_none());
    }

    #[test]
    fn junk_is_rejected_rather_than_guessed_at() {
        assert!(Vtec::parse("").is_none());
        assert!(Vtec::parse("not a vtec string").is_none());
        assert!(Vtec::parse("/O.NEW.KOUN/").is_none());
        assert!(Vtec::parse("/O.NEW.KOUN.TO.W.XXXX.130520T2012Z-130520T2045Z/").is_none());
    }
}
