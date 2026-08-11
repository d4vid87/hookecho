//! Hurricane-hunter reconnaissance: high-density observations (HDOB) from the flight track.
//!
//! Every 30 seconds an aircraft inside a tropical cyclone transmits its position, flight-level
//! wind, and — the number forecasters actually wait for — the SFMR surface wind beneath it. No
//! satellite or radar product measures that; a plane flying through the eyewall does.
//!
//! The bulletins are plain text under `tgftp.nws.noaa.gov/data/raw/ur/`, one file per issuing
//! centre, holding the most recent transmission. Format is the NHC HDOB spec: fixed
//! whitespace-separated columns with `/` filling anything missing.

use chrono::{DateTime, Duration, NaiveDate, Utc};

const BASE: &str = "https://tgftp.nws.noaa.gov/data/raw/ur";

/// The HDOB bulletins worth polling: Atlantic (`urnt15`) and Pacific (`urpn15`), from each centre
/// that issues them.
const BULLETINS: [&str; 5] = [
    "urnt15.knhc..txt",
    "urnt15.kwbc..txt",
    "urnt15.kbix..txt",
    "urpn15.knhc..txt",
    "urpn15.kwbc..txt",
];

/// One 30-second high-density observation.
#[derive(Debug, Clone, PartialEq)]
pub struct HdobOb {
    pub time: DateTime<Utc>,
    pub lat: f64,
    pub lon: f64,
    /// Static air pressure at flight level (mb).
    pub press_mb: Option<f32>,
    /// Geopotential height at flight level (m).
    pub height_m: Option<f32>,
    pub fl_temp_c: Option<f32>,
    pub wdir_deg: Option<f32>,
    /// Flight-level wind speed (kt).
    pub wspd_kt: Option<f32>,
    /// Peak 10-second flight-level wind (kt).
    pub peak_kt: Option<f32>,
    /// SFMR-measured surface wind (kt) — the number that decides an intensity estimate.
    pub sfmr_kt: Option<f32>,
    /// Aircraft/mission id from the bulletin header, e.g. "AF307".
    pub mission: String,
}

/// A group is missing when it is all slashes.
fn field(s: &str) -> Option<&str> {
    (!s.is_empty() && !s.starts_with('/')).then_some(s)
}

/// `2833N` / `09423W` → signed degrees. Degrees are all but the last two digits; the last two
/// are whole minutes.
fn coord(s: &str) -> Option<f64> {
    // Split at the last *character*, not the last byte: corrupted text can end in a multi-byte
    // character, and splitting inside one panics (found by fuzz/fuzz_targets/hdob_parse.rs).
    let (num, hemi) = s.split_at(s.char_indices().next_back()?.0);
    let deg: f64 = num.get(..num.len().checked_sub(2)?)?.parse().ok()?;
    let min: f64 = num.get(num.len() - 2..)?.parse().ok()?;
    let v = deg + min / 60.0;
    Some(match hemi {
        "S" | "W" => -v,
        _ => v,
    })
}

/// Signed tenths ("-155" → −15.5). Missing groups are slashes.
fn tenths(s: &str) -> Option<f32> {
    field(s)?.parse::<f32>().ok().map(|v| v / 10.0)
}

/// Parse one HDOB bulletin.
///
/// The header line carries the mission id and the observation date; the data lines carry only
/// `HHMMSS`, so the date comes from the header and rolls back a day when the time is ahead of it.
pub fn parse_hdob(text: &str) -> Vec<HdobOb> {
    let mut out = Vec::new();
    let mut mission = String::new();
    let mut date: Option<NaiveDate> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() {
            continue;
        }
        // Header: "AF307 WXWXA 260730143955307    HDOB 14 20260730"
        if let Some(i) = f.iter().position(|w| *w == "HDOB") {
            mission = f[0].to_string();
            date = f
                .get(i + 2)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok());
            continue;
        }
        // Data lines start with a six-digit HHMMSS and carry at least through the wind group.
        if f.len() < 9 || f[0].len() != 6 || !f[0].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let (Some(day), Some(lat), Some(lon)) = (date, coord(f[1]), coord(f[2])) else {
            continue;
        };
        let (Ok(h), Ok(m), Ok(sec)) = (
            f[0][0..2].parse::<u32>(),
            f[0][2..4].parse::<u32>(),
            f[0][4..6].parse::<u32>(),
        ) else {
            continue;
        };
        let Some(time) = day.and_hms_opt(h, m, sec).map(|t| t.and_utc()) else {
            continue;
        };
        // Pressure drops its leading 1 above 1000 mb, so a small value means a high pressure.
        let press_mb = tenths(f[3]).map(|p| if p < 100.0 { p + 1000.0 } else { p });
        // `get` rather than a slice: the six characters are digits in every real bulletin, but a
        // corrupted fetch can put a multi-byte character here and slicing across it panics
        // (found by fuzz/fuzz_targets/hdob_parse.rs).
        let (wdir_deg, wspd_kt) = match field(f[8]).filter(|w| w.len() == 6) {
            Some(w) => match (w.get(..3), w.get(3..)) {
                (Some(d), Some(s)) => (d.parse::<f32>().ok(), s.parse::<f32>().ok()),
                _ => (None, None),
            },
            None => (None, None),
        };
        out.push(HdobOb {
            time,
            lat,
            lon,
            press_mb,
            height_m: field(f[4]).and_then(|v| v.parse().ok()),
            fl_temp_c: tenths(f[6]),
            wdir_deg,
            wspd_kt,
            peak_kt: f.get(9).and_then(|v| field(v)).and_then(|v| v.parse().ok()),
            sfmr_kt: f
                .get(10)
                .and_then(|v| field(v))
                .and_then(|v| v.parse().ok()),
            mission: mission.clone(),
        });
    }
    out
}

/// Fetch the current HDOB bulletins, keeping observations from the last `max_age_hours`.
///
/// Between missions — and out of season entirely — this is legitimately empty.
pub async fn fetch(http: &reqwest::Client, max_age_hours: i64) -> anyhow::Result<Vec<HdobOb>> {
    let cutoff = Utc::now() - Duration::hours(max_age_hours);
    let mut out = Vec::new();
    for name in BULLETINS {
        let Ok(resp) = http
            .get(crate::net::fetch_url(&format!("{BASE}/{name}")))
            .header("User-Agent", crate::alerts::USER_AGENT)
            .send()
            .await
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(text) = resp.text().await else {
            continue;
        };
        out.extend(parse_hdob(&text).into_iter().filter(|o| o.time >= cutoff));
    }
    out.sort_by_key(|o| o.time);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wind group with a multi-byte character in it used to panic on a byte slice across the
    /// character boundary (found by fuzzing). Corrupted text is a thing to skip, not to die on.
    #[test]
    fn a_mangled_wind_group_is_skipped_not_fatal() {
        let text = "AF307 WXWXA 260730143955307    HDOB 14 20260730\n\
                    165030 2833N 09423W 3926 07809 0492 -155 -199 \u{fffd}\u{fffd}015 015\n";
        let obs = parse_hdob(text);
        assert_eq!(obs.len(), 1);
        assert!(obs[0].wdir_deg.is_none());
    }

    /// Same story one field over: a coordinate ending in a multi-byte character.
    #[test]
    fn a_mangled_coordinate_is_skipped_not_fatal() {
        let text = "AF307 WXWXA 260730143955307    HDOB 14 20260730\n\
                    165030 2833\u{fffd} 09423W 3926 07809 0492 -155 -199 069015 015\n";
        // Unknown hemisphere reads as north/east, same as any other unexpected letter — the
        // point is that it comes back rather than taking the process down.
        let obs = parse_hdob(text);
        assert_eq!(obs.len(), 1);
        assert!(obs[0].lat > 0.0);
    }

    const BULLETIN: &str = "\
URNT15 KNHC 301700
AF307 WXWXA 260730143955307    HDOB 14 20260730
165030 2833N 09423W 3926 07809 0492 -155 -199 069015 015 /// /// 03
165100 2835N 09424W 3926 07809 0493 -152 -198 066014 014 052 001 03
165130 2837N 09426W //// ///// 0493 //// //// otherwise 014 /// /// 03";

    #[test]
    fn hdobs_decode_position_wind_and_sfmr() {
        let obs = parse_hdob(BULLETIN);
        assert_eq!(obs.len(), 3);
        let o = &obs[0];
        assert_eq!(o.mission, "AF307");
        assert_eq!(o.time.to_rfc3339(), "2026-07-30T16:50:30+00:00");
        // 2833N is 28 degrees 33 minutes north; 09423W is west, so negative.
        assert!((o.lat - 28.55).abs() < 1e-6, "{}", o.lat);
        assert!((o.lon - -94.383_333).abs() < 1e-5, "{}", o.lon);
        assert_eq!(o.press_mb, Some(392.6));
        assert_eq!(o.height_m, Some(7809.0));
        assert_eq!(o.fl_temp_c, Some(-15.5));
        assert_eq!(o.wdir_deg, Some(69.0));
        assert_eq!(o.wspd_kt, Some(15.0));
        assert_eq!(o.peak_kt, Some(15.0));
        // Slashes mean the SFMR reported nothing, not a surface wind of zero.
        assert_eq!(o.sfmr_kt, None);
        assert_eq!(obs[1].sfmr_kt, Some(52.0));
    }

    #[test]
    fn missing_groups_cost_only_their_own_field() {
        let o = &parse_hdob(BULLETIN)[2];
        assert_eq!(o.press_mb, None);
        assert_eq!(o.height_m, None);
        assert_eq!(o.fl_temp_c, None);
        assert_eq!(o.wdir_deg, None, "an unparsable wind group is not a wind");
        // Position and time still decoded, so the point still plots.
        assert!((o.lat - 28.616_666).abs() < 1e-5);
    }

    #[test]
    fn pressure_regains_its_dropped_leading_one() {
        // "0134" is 1013.4 mb: above 1000, the leading 1 is dropped in the bulletin.
        let line = "165030 2833N 09423W 0134 00099 0492 -155 -199 069015 015 /// /// 03";
        let text = format!("AF307 WXWXA 1    HDOB 14 20260730\n{line}");
        assert_eq!(parse_hdob(&text)[0].press_mb, Some(1013.4));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_current_bulletins() {
        let http = reqwest::Client::new();
        // Between missions this is empty; only a parse or transport failure is a bug.
        let obs = fetch(&http, 6).await.expect("bulletins");
        for o in &obs {
            assert!(o.lat.abs() <= 90.0 && o.lon.abs() <= 180.0);
        }
    }
}
