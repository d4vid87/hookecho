//! WPC coded surface analysis: fronts and pressure centers — the "TV weather map" layer.
//!
//! Source is the `CODSUS` bulletin (WMO `ASUS01 KWBC`), served by the NWS API as product type
//! `COD` at location `SUS`. It is a fixed-vocabulary text product:
//!
//! ```text
//! VALID 072600Z
//! HIGHS 1019 2888 1017 25106 ...
//! LOWS  1004 44101 ...
//! STNRY WK 3582 3583 3684 ...
//! COLD WK 3771 3771 3575 ...
//! ```
//!
//! Coordinate groups are whole degrees, north/west: the first two digits are latitude, the rest
//! are longitude — so `2888` is 28°N 88°W and `25106` is 25°N 106°W. Anything that doesn't parse
//! is skipped rather than failing the bulletin; a garbled segment shouldn't cost you the map.

use crate::alerts::USER_AGENT;
use chrono::{DateTime, Utc};

const API: &str = "https://api.weather.gov";

/// The kind of boundary, which decides both color and the shape of the pips drawn along it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontKind {
    Cold,
    Warm,
    Stationary,
    Occluded,
    /// A trough — drawn as a dashed line with no pips.
    Trough,
}

impl FrontKind {
    fn from_token(t: &str) -> Option<FrontKind> {
        Some(match t {
            "COLD" => FrontKind::Cold,
            "WARM" => FrontKind::Warm,
            "STNRY" => FrontKind::Stationary,
            "OCFNT" => FrontKind::Occluded,
            "TROF" => FrontKind::Trough,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            FrontKind::Cold => "Cold front",
            FrontKind::Warm => "Warm front",
            FrontKind::Stationary => "Stationary front",
            FrontKind::Occluded => "Occluded front",
            FrontKind::Trough => "Trough",
        }
    }

    /// Map colors follow the convention every weather map uses.
    pub fn color(self) -> [u8; 3] {
        match self {
            FrontKind::Cold => [60, 120, 240],
            FrontKind::Warm => [225, 70, 70],
            FrontKind::Stationary => [180, 60, 160],
            FrontKind::Occluded => [160, 80, 210],
            FrontKind::Trough => [200, 160, 60],
        }
    }
}

/// One frontal boundary as a polyline of `(lon, lat)`.
#[derive(Debug, Clone)]
pub struct Front {
    pub kind: FrontKind,
    pub pts: Vec<(f64, f64)>,
}

/// A pressure center.
#[derive(Debug, Clone, Copy)]
pub struct Center {
    /// `true` for a high, `false` for a low.
    pub high: bool,
    pub mb: i32,
    pub lon: f64,
    pub lat: f64,
}

#[derive(Debug, Clone, Default)]
pub struct SurfaceAnalysis {
    pub valid: Option<DateTime<Utc>>,
    pub fronts: Vec<Front>,
    pub centers: Vec<Center>,
}

/// Decode one coordinate group: 2-digit latitude, then 2- or 3-digit west longitude.
fn coord(tok: &str) -> Option<(f64, f64)> {
    if !tok.bytes().all(|b| b.is_ascii_digit()) || !(4..=5).contains(&tok.len()) {
        return None;
    }
    let lat: f64 = tok[0..2].parse().ok()?;
    let lon: f64 = tok[2..].parse().ok()?;
    if lat > 90.0 || lon > 180.0 {
        return None;
    }
    Some((-lon, lat))
}

/// Parse the bulletin body.
pub fn parse(text: &str) -> SurfaceAnalysis {
    let mut out = SurfaceAnalysis::default();
    // Records continue across newlines until the next keyword, so flatten first.
    let flat = text.replace(['\r', '\n'], " ");
    let toks: Vec<&str> = flat.split_whitespace().collect();

    #[derive(PartialEq)]
    enum Mode {
        None,
        Centers { high: bool },
        Front(FrontKind),
    }
    let mut mode = Mode::None;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    // Pressure centers alternate <millibars> <coord>.
    let mut pending_mb: Option<i32> = None;

    let flush = |mode: &Mode, pts: &mut Vec<(f64, f64)>, out: &mut SurfaceAnalysis| {
        if let Mode::Front(kind) = mode {
            if pts.len() >= 2 {
                out.fronts.push(Front {
                    kind: *kind,
                    pts: std::mem::take(pts),
                });
            }
        }
        pts.clear();
    };

    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        if t == "VALID" {
            // `VALID 072600Z` — day-of-month + hour, in the current month.
            if let Some(v) = toks.get(i + 1) {
                out.valid = parse_valid(v);
            }
            flush(&mode, &mut pts, &mut out);
            mode = Mode::None;
            i += 2;
            continue;
        }
        if t == "HIGHS" || t == "LOWS" {
            flush(&mode, &mut pts, &mut out);
            mode = Mode::Centers { high: t == "HIGHS" };
            pending_mb = None;
            i += 1;
            continue;
        }
        if let Some(kind) = FrontKind::from_token(t) {
            flush(&mode, &mut pts, &mut out);
            mode = Mode::Front(kind);
            // Strength qualifiers (WK / MDT / STRG / DISS) follow the type and carry no geometry.
            i += 1;
            while matches!(toks.get(i), Some(&"WK" | &"MDT" | &"STRG" | &"DISS")) {
                i += 1;
            }
            continue;
        }

        match &mode {
            Mode::Centers { high } => {
                // Millibars are 3-4 digits and never a valid coordinate group here (a 4-digit
                // coordinate would put a center north of 90° or the pressure below 100 mb), so
                // the alternation is what disambiguates: pressure, position, pressure, position.
                match pending_mb {
                    None => pending_mb = t.parse::<i32>().ok(),
                    Some(mb) => {
                        if let Some((lon, lat)) = coord(t) {
                            out.centers.push(Center {
                                high: *high,
                                mb,
                                lon,
                                lat,
                            });
                        }
                        pending_mb = None;
                    }
                }
            }
            Mode::Front(_) => {
                if let Some(p) = coord(t) {
                    // The bulletin repeats a point when a segment doubles back; a zero-length
                    // step would break the pip spacing walk downstream.
                    if pts.last() != Some(&p) {
                        pts.push(p);
                    }
                }
            }
            Mode::None => {}
        }
        i += 1;
    }
    flush(&mode, &mut pts, &mut out);
    out
}

/// `VALID 072600Z` is `MMDDHH`: month 07, day 26, hour 00Z. (Not `DDHHMM` — a bulletin issued at
/// `260128` carrying `VALID 072600Z` only reconciles as July 26th 00Z.) The year isn't in the
/// product, so it comes from the clock, rolling back at a year boundary.
fn parse_valid(tok: &str) -> Option<DateTime<Utc>> {
    let t = tok.trim_end_matches('Z');
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let month: u32 = t[0..2].parse().ok()?;
    let day: u32 = t[2..4].parse().ok()?;
    let hour: u32 = t[4..6].parse().ok()?;
    use chrono::{Datelike, TimeZone};
    let now = Utc::now();
    // A December bulletin read in January belongs to the previous year.
    let year = if month == 12 && now.month() == 1 {
        now.year() - 1
    } else {
        now.year()
    };
    Utc.with_ymd_and_hms(year, month, day, hour.min(23), 0, 0)
        .single()
}

/// Fetch the newest coded surface analysis.
pub async fn fetch(http: &reqwest::Client) -> anyhow::Result<SurfaceAnalysis> {
    let list = http
        .get(format!("{API}/products/types/COD/locations/SUS"))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/ld+json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let lv: serde_json::Value = serde_json::from_str(&list)?;
    let id = lv
        .pointer("/@graph/0/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no coded surface analysis available"))?;

    let body = http
        .get(format!("{API}/products/{id}"))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/ld+json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let bv: serde_json::Value = serde_json::from_str(&body)?;
    let text = bv
        .get("productText")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("coded surface analysis has no text"))?;
    Ok(parse(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real ASUS01 KWBC bulletin (2026-07-26 0128Z).
    const SAMPLE: &str = "\
000
ASUS01 KWBC 260128
CODSUS

CODED SURFACE FRONTAL POSITIONS
NWS WEATHER PREDICTION CENTER COLLEGE PARK MD
928 PM EDT SAT JUL 25 2026

VALID 072600Z
HIGHS 1019 2888 1017 25106 1016 62139
LOWS 1004 44101 998 75153
TROF 2972 3071
TROF 2188 2189 2089 1891 1892
STNRY WK 3582 3583 3684 3684 3785 3786 3887
COLD WK 3771 3771 3575 3476 3378 3379
WARM WK 4159 4162 4164
OCFNT WK 5242 5145 5048
$$";

    #[test]
    fn parses_every_front_kind() {
        let a = parse(SAMPLE);
        let kinds: Vec<FrontKind> = a.fronts.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![
                FrontKind::Trough,
                FrontKind::Trough,
                FrontKind::Stationary,
                FrontKind::Cold,
                FrontKind::Warm,
                FrontKind::Occluded,
            ]
        );
    }

    #[test]
    fn decodes_two_and_three_digit_longitudes() {
        let a = parse(SAMPLE);
        let highs: Vec<(f64, f64)> = a
            .centers
            .iter()
            .filter(|c| c.high)
            .map(|c| (c.lon, c.lat))
            .collect();
        assert_eq!(highs[0], (-88.0, 28.0), "4-digit group");
        assert_eq!(highs[1], (-106.0, 25.0), "5-digit group");
    }

    #[test]
    fn reads_center_pressures_and_signs() {
        let a = parse(SAMPLE);
        assert_eq!(a.centers.iter().filter(|c| c.high).count(), 3);
        let lows: Vec<&Center> = a.centers.iter().filter(|c| !c.high).collect();
        assert_eq!(lows.len(), 2);
        assert_eq!(lows[0].mb, 1004);
        assert_eq!((lows[1].mb, lows[1].lat), (998, 75.0), "3-digit pressure");
    }

    #[test]
    fn strength_qualifiers_are_not_geometry() {
        let a = parse(SAMPLE);
        let cold = a.fronts.iter().find(|f| f.kind == FrontKind::Cold).unwrap();
        // "COLD WK 3771 3771 3575 3476 3378 3379" — six groups, one repeat collapsed.
        assert_eq!(cold.pts.len(), 5);
        assert_eq!(cold.pts[0], (-71.0, 37.0));
    }

    #[test]
    fn repeated_points_are_collapsed() {
        let a = parse(SAMPLE);
        let stnry = a
            .fronts
            .iter()
            .find(|f| f.kind == FrontKind::Stationary)
            .unwrap();
        // Seven groups with one duplicate pair (3684 3684).
        assert_eq!(stnry.pts.len(), 6);
        for w in stnry.pts.windows(2) {
            assert_ne!(w[0], w[1]);
        }
    }

    #[test]
    fn single_point_segments_are_dropped() {
        let a = parse("VALID 072600Z\nCOLD WK 3771\nTROF 2972 3071");
        assert_eq!(a.fronts.len(), 1, "a one-point front is not a line");
        assert_eq!(a.fronts[0].kind, FrontKind::Trough);
    }

    #[test]
    fn garbage_does_not_panic_or_poison() {
        let a = parse("VALID xxxx\nCOLD WK 99999 abcd 3771 3575\nnonsense");
        assert_eq!(a.fronts.len(), 1);
        assert_eq!(
            a.fronts[0].pts.len(),
            2,
            "bad groups skipped, good ones kept"
        );
        assert!(a.valid.is_none());
    }

    #[test]
    fn valid_time_is_month_day_hour() {
        let a = parse(SAMPLE);
        let v = a.valid.expect("valid time");
        use chrono::{Datelike, Timelike};
        assert_eq!((v.month(), v.day(), v.hour()), (7, 26, 0));
    }
}
