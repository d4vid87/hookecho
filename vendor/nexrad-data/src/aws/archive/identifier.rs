use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

/// Identifying metadata for a NEXRAD archive volume file.
#[derive(Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct Identifier(String);

impl Identifier {
    /// Constructs a new identifier from the provided name.
    pub fn new(name: String) -> Self {
        Identifier(name)
    }

    /// The file name.
    pub fn name(&self) -> &str {
        &self.0
    }

    /// The radar site this file was produced at, e.g. KDMX.
    pub fn site(&self) -> Option<&str> {
        self.0.get(0..self.site_len())
    }

    fn site_len(&self) -> usize {
        // OPERA NOD identifiers are five letters; other archive formats remain four.
        if self.0.contains("_OPERA") && self.0.as_bytes().get(..5)
            .is_some_and(|id| id.iter().all(u8::is_ascii_alphabetic)) { 5 } else { 4 }
    }

    /// This file's data collection time.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        let start = self.site_len();
        let date_string = self.0.get(start..start + 8)?;
        if let Ok(date) = NaiveDate::parse_from_str(date_string, "%Y%m%d") {
            let time_string = self.0.get(start + 9..start + 15)?;
            if let Ok(time) = NaiveTime::parse_from_str(time_string, "%H%M%S") {
                let naive_datetime = NaiveDateTime::new(date, time);
                return Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc));
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn international_names_preserve_archive_identifier_contract() {
        for (name, site) in [("KTLX20261104_123000_V06", "KTLX"),
            ("DEBO20261104_123000_DWD", "DEBO"),
            ("FRABB20261104_123000_OPERA", "FRABB"),
            ("IEDUB20261104_123000_OPERA~202611041230", "IEDUB")] {
            let id = Identifier::new(name.into());
            assert_eq!(id.site(), Some(site));
            assert_eq!(id.date_time().unwrap().to_rfc3339(), "2026-11-04T12:30:00+00:00");
        }
        assert!(Identifier::new("é_OPERA".into()).date_time().is_none());
    }
}
