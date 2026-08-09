//! Electric field, from two very different sources — read this before trusting a number.
//!
//! **PPEF** ([`fetch_ppef`]) is NOAA/NCEI's Prompt Penetration Electric Field model: the
//! *equatorial ionospheric* eastward field, in **mV/m**, predicted from ACE solar-wind data about
//! an hour ahead and published on a 5-minute cadence across twenty geographic longitudes. It is a
//! space-weather quantity, roughly 300 km up. It says nothing about the thunderstorm overhead.
//!
//! **A field mill** ([`fetch_mill`]) is the ground instrument storm chasers mean by "electric
//! field": a Boltek EFM-100 or similar reading the local fair-weather-to-thundercloud gradient in
//! **kV/m**, where a swing past a few kV/m means charge is building close enough to strike. No
//! national network publishes these, so this one is bring-your-own-URL: point it at a mill's JSON
//! and the card charts it. The two are six orders of magnitude and 300 km apart; the card labels
//! whichever it has and never blends them.

use crate::alerts::USER_AGENT;
use chrono::{DateTime, TimeZone, Utc};

/// NOAA's real-time PPEF model output (the `L` variant is the longitude table).
pub const PPEF_URL: &str =
    "https://electric-field-dot-gbp-oit-rc-hdgmrt-app-s8g.appspot.com/ppefmL";

/// One 5-minute PPEF row: the eastward field at each of the table's longitudes.
#[derive(Debug, Clone, PartialEq)]
pub struct PpefRow {
    pub time: DateTime<Utc>,
    /// Field in mV/m, one per entry of [`Ppef::longitudes`]. `NaN` where the model had no value.
    pub values: Vec<f32>,
}

/// A parsed PPEF table: the longitude grid, and the rows spanning roughly the last and next hour.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ppef {
    /// Geographic longitudes (°E, 0..360 as published) the columns correspond to.
    pub longitudes: Vec<f64>,
    pub rows: Vec<PpefRow>,
}

impl Ppef {
    /// The field at a longitude for the row nearest `at`, interpolated between the two straddling
    /// columns. `None` when the table is empty or the nearest row has no value there.
    pub fn value_at(&self, lon_deg: f64, at: DateTime<Utc>) -> Option<f32> {
        let row = self
            .rows
            .iter()
            .min_by_key(|r| (r.time - at).num_seconds().abs())?;
        self.interp_row(row, lon_deg)
    }

    /// The newest row's field at a longitude — what a card shows when it just wants "now".
    pub fn latest_at(&self, lon_deg: f64) -> Option<(DateTime<Utc>, f32)> {
        let row = self.rows.iter().max_by_key(|r| r.time)?;
        Some((row.time, self.interp_row(row, lon_deg)?))
    }

    /// The series at one longitude, oldest first — the card's sparkline.
    pub fn series_at(&self, lon_deg: f64) -> Vec<(DateTime<Utc>, f32)> {
        let mut out: Vec<_> = self
            .rows
            .iter()
            .filter_map(|r| self.interp_row(r, lon_deg).map(|v| (r.time, v)))
            .collect();
        out.sort_by_key(|(t, _)| *t);
        out
    }

    fn interp_row(&self, row: &PpefRow, lon_deg: f64) -> Option<f32> {
        if self.longitudes.len() < 2 || row.values.len() != self.longitudes.len() {
            return None;
        }
        // The grid wraps, so work in 0..360 and let the last column meet the first.
        let lon = lon_deg.rem_euclid(360.0);
        let n = self.longitudes.len();
        for i in 0..n {
            let (a, b) = (self.longitudes[i], self.longitudes[(i + 1) % n]);
            let span = (b - a).rem_euclid(360.0);
            let off = (lon - a).rem_euclid(360.0);
            if span > 0.0 && off <= span {
                let (va, vb) = (row.values[i], row.values[(i + 1) % n]);
                // Sitting exactly on a column needs no neighbour — and must not inherit its NaN.
                if off == 0.0 {
                    return (!va.is_nan()).then_some(va);
                }
                if va.is_nan() || vb.is_nan() {
                    return None;
                }
                return Some(va + (vb - va) * (off / span) as f32);
            }
        }
        None
    }
}

/// Parse the PPEF page. It is served as HTML with the data inside one `<span>`, rows separated by
/// literal `</br>` and columns by whitespace, so strip the markup and read it line by line.
pub fn parse_ppef(html: &str) -> Ppef {
    let text = html.replace("</br>", "\n").replace("&nbsp", " ");
    let mut longitudes = Vec::new();
    let mut rows = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('<') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            // The longitude header is the comment-free numeric line right after "# YR MO DA HH MM",
            // but the values themselves are on their own line — handled below.
            let _ = rest;
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Two all-numeric header rows precede the data: magnetic local time, then geographic
        // longitude. Both look alike, so keep the last one seen before the data starts — a data
        // row gives itself away by opening with a four-digit year.
        if rows.is_empty()
            && fields.len() >= 4
            && fields.iter().all(|f| f.parse::<f64>().is_ok())
            && fields[0].parse::<f64>().unwrap_or(0.0) < 1900.0
        {
            longitudes = fields
                .iter()
                .filter_map(|f| f.parse::<f64>().ok())
                .collect();
            continue;
        }
        // A data row: YR MO DA HH MM then one value per longitude.
        if fields.len() > 5 {
            let (Ok(y), Ok(mo), Ok(d), Ok(h), Ok(mi)) = (
                fields[0].parse::<i32>(),
                fields[1].parse::<u32>(),
                fields[2].parse::<u32>(),
                fields[3].parse::<u32>(),
                fields[4].parse::<u32>(),
            ) else {
                continue;
            };
            let Some(time) = Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).single() else {
                continue;
            };
            let values: Vec<f32> = fields[5..]
                .iter()
                .map(|f| f.parse::<f32>().unwrap_or(f32::NAN))
                .collect();
            rows.push(PpefRow { time, values });
        }
    }
    Ppef { longitudes, rows }
}

/// Fetch the current PPEF table.
pub async fn fetch_ppef(client: &reqwest::Client) -> anyhow::Result<Ppef> {
    let body = client
        .get(crate::net::fetch_url(PPEF_URL))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let p = parse_ppef(&body);
    if p.rows.is_empty() {
        anyhow::bail!("PPEF page carried no data rows");
    }
    Ok(p)
}

/// One reading from a ground field mill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MillReading {
    pub time: DateTime<Utc>,
    /// Vertical field in kV/m, positive as the mill reports it.
    pub kv_per_m: f32,
}

/// Parse a field-mill feed. Accepts either a bare object (`{"time":…, "kv_per_m":…}`, the latest
/// reading) or an array of them (a history, oldest or newest first — the caller sorts). Time may
/// be RFC 3339 or Unix seconds; the field may be named `kv_per_m`, `kvm`, or `field`.
pub fn parse_mill(json: &str) -> Vec<MillReading> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let items: Vec<&serde_json::Value> = match v.as_array() {
        Some(arr) => arr.iter().collect(),
        None => vec![&v],
    };
    items
        .into_iter()
        .filter_map(|o| {
            let kv = ["kv_per_m", "kvm", "field"]
                .iter()
                .find_map(|k| o.get(*k).and_then(|x| x.as_f64()))? as f32;
            let t = o.get("time").or_else(|| o.get("timestamp"))?;
            let time = t
                .as_i64()
                .and_then(|s| Utc.timestamp_opt(s, 0).single())
                .or_else(|| {
                    t.as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|d| d.with_timezone(&Utc))
                })?;
            Some(MillReading { time, kv_per_m: kv })
        })
        .collect()
}

/// Fetch a user-supplied field-mill endpoint.
pub async fn fetch_mill(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<MillReading>> {
    let body = client
        .get(crate::net::fetch_url(url))
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let r = parse_mill(&body);
    if r.is_empty() {
        anyhow::bail!("no readings in the field-mill response");
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    /// Trimmed from the live page: header, the longitude row, and three data rows.
    const PAGE: &str = r#"<html><body><span>
:Data type: Prompt penetration component
</br># Units: Electric Fields in mV/m
</br># YR MO DA  HH MM
000.0&nbsp 090.0&nbsp 180.0&nbsp 270.0&nbsp </br>
</br>#----------------------------------------
 </br>
2026 07 29  01 35    +0.200     -0.100     +0.000     +0.100  </br>
2026 07 29  01 40    +0.100     -0.050     +0.000     +0.050  </br>
2026 07 29  01 45    +0.000     NaN        +0.000     +0.000  </br>
</span></body></html>"#;

    #[test]
    fn parses_the_ppef_table() {
        let p = parse_ppef(PAGE);
        assert_eq!(p.longitudes, vec![0.0, 90.0, 180.0, 270.0]);
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.rows[0].time.timestamp(), 1785288900);
        assert_eq!(p.rows[0].values[0], 0.2);
    }

    #[test]
    fn interpolates_between_longitude_columns() {
        let p = parse_ppef(PAGE);
        let at = Utc.with_ymd_and_hms(2026, 7, 29, 1, 35, 0).unwrap();
        // Halfway from 0° (+0.200) to 90° (-0.100).
        let v = p.value_at(45.0, at).unwrap();
        assert!((v - 0.05).abs() < 1e-6, "got {v}");
        // Western longitudes wrap into the published 0..360 grid: -90° is 270°.
        assert!((p.value_at(-90.0, at).unwrap() - 0.1).abs() < 1e-6);
        // A NaN column poisons its span rather than reporting a fake zero.
        let late = Utc.with_ymd_and_hms(2026, 7, 29, 1, 45, 0).unwrap();
        assert_eq!(p.value_at(45.0, late), None);
    }

    #[test]
    fn latest_and_series() {
        let p = parse_ppef(PAGE);
        let (t, _) = p.latest_at(0.0).unwrap();
        assert_eq!(t.minute(), 45);
        let s = p.series_at(0.0);
        assert_eq!(s.len(), 3);
        assert!(s[0].0 < s[2].0, "series must run oldest first");
    }

    /// Live check against NOAA's real page — the layout is published as HTML for humans and can
    /// drift, so run this by hand (`--ignored`) when the readings look wrong.
    #[tokio::test]
    #[ignore]
    async fn live_ppef_page_still_parses() {
        let p = fetch_ppef(&reqwest::Client::new()).await.unwrap();
        assert_eq!(p.longitudes.len(), 20, "the grid is 20 longitudes wide");
        assert!(p.rows.len() > 10, "got {} rows", p.rows.len());
        // Oklahoma sits near 262 °E; the model is small but finite there.
        let (t, v) = p.latest_at(-98.0).unwrap();
        assert!(v.abs() < 20.0, "{v} mV/m at {t} is off the physical scale");
    }

    #[test]
    fn parses_field_mill_shapes() {
        // A single latest reading, Unix time.
        let one = parse_mill(r#"{"time":1785288900,"kv_per_m":-2.4}"#);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].kv_per_m, -2.4);
        // A history, RFC 3339, alternate field name.
        let many = parse_mill(
            r#"[{"timestamp":"2026-07-29T01:35:00Z","kvm":0.2},
                {"timestamp":"2026-07-29T01:36:00Z","field":8.1}]"#,
        );
        assert_eq!(many.len(), 2);
        assert_eq!(many[1].kv_per_m, 8.1);
        // Junk yields nothing rather than a panic.
        assert!(parse_mill("not json").is_empty());
        assert!(parse_mill(r#"[{"time":1}]"#).is_empty());
    }
}
