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
        self.0.get(0..4)
    }

    /// This file's data collection time.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        let date_string = self.0.get(4..12)?;
        if let Ok(date) = NaiveDate::parse_from_str(date_string, "%Y%m%d") {
            let time_string = self.0.get(13..19)?;
            if let Ok(time) = NaiveTime::parse_from_str(time_string, "%H%M%S") {
                let naive_datetime = NaiveDateTime::new(date, time);
                return Some(DateTime::from_naive_utc_and_offset(naive_datetime, Utc));
            }
        }

        None
    }
}
