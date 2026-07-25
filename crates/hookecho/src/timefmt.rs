//! Clock rendering for radar timestamps.
//!
//! Radar data is UTC on the wire, but a storm chaser reads the clock against the storm, not against
//! Greenwich. Every user-facing timestamp goes through here with the timezone of the site it came
//! from; `None` means "show Zulu" — either the user picked UTC in Settings or the site is unknown.

use chrono::{DateTime, Utc};
use wxdata::tz::Tz;

/// `"4:32:10 PM CDT"` / `"4:32 PM CDT"`, or `"20:32:10Z"` / `"20:32Z"` when `tz` is `None`.
pub fn fmt_clock(dt: DateTime<Utc>, tz: Option<Tz>, secs: bool) -> String {
    match tz {
        Some(tz) => dt
            .with_timezone(&tz)
            .format(if secs {
                "%-I:%M:%S %p %Z"
            } else {
                "%-I:%M %p %Z"
            })
            .to_string(),
        None => dt
            .format(if secs { "%H:%M:%SZ" } else { "%H:%MZ" })
            .to_string(),
    }
}

/// `"Jul 25, 4:32 PM CDT"`, or `"2026-07-25 20:32Z"` when `tz` is `None`.
pub fn fmt_date_clock(dt: DateTime<Utc>, tz: Option<Tz>) -> String {
    match tz {
        Some(tz) => dt
            .with_timezone(&tz)
            .format("%b %-d, %-I:%M %p %Z")
            .to_string(),
        None => dt.format("%Y-%m-%d %H:%MZ").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, m, d, h, 32, 10).unwrap()
    }

    #[test]
    fn zulu_when_no_zone() {
        assert_eq!(fmt_clock(utc(7, 25, 20), None, true), "20:32:10Z");
        assert_eq!(fmt_clock(utc(7, 25, 20), None, false), "20:32Z");
        assert_eq!(fmt_date_clock(utc(7, 25, 20), None), "2026-07-25 20:32Z");
    }

    /// `%Z` must render a zone abbreviation, and it must follow DST.
    #[test]
    fn central_time_follows_dst() {
        let chicago = wxdata::tz::site_tz("KTLX").unwrap();
        assert_eq!(
            fmt_clock(utc(7, 25, 20), Some(chicago), true),
            "3:32:10 PM CDT"
        );
        assert_eq!(
            fmt_clock(utc(1, 25, 20), Some(chicago), false),
            "2:32 PM CST"
        );
        assert_eq!(
            fmt_date_clock(utc(7, 25, 20), Some(chicago)),
            "Jul 25, 3:32 PM CDT"
        );
    }

    #[test]
    fn phoenix_never_shifts() {
        let phoenix = wxdata::tz::site_tz("KIWA").unwrap();
        assert_eq!(
            fmt_clock(utc(7, 25, 20), Some(phoenix), false),
            "1:32 PM MST"
        );
        assert_eq!(
            fmt_clock(utc(1, 25, 20), Some(phoenix), false),
            "1:32 PM MST"
        );
    }
}
