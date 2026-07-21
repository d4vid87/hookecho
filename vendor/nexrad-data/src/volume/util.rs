use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

/// Given a "modified" Julian date (date count since 1/1/1970) and a count of milliseconds since
/// midnight on that date, return an appropriate DateTime.
pub(crate) fn get_datetime(
    modified_julian_date: u16,
    past_midnight: Duration,
) -> Option<DateTime<Utc>> {
    let count_start = NaiveDate::from_ymd_opt(1970, 1, 1)?;
    let date = count_start + Duration::days(modified_julian_date as i64 - 1);
    let time = NaiveTime::from_num_seconds_from_midnight_opt(0, 0)? + past_midnight;

    Some(DateTime::from_naive_utc_and_offset(
        NaiveDateTime::new(date, time),
        Utc,
    ))
}
