//! Terminal Doppler Weather Radar (TDWR) support.
//!
//! TDWRs are the FAA's airport radars: C-band, ~150 m gates, and a volume every minute or so.
//! Near a major airport they see low-level detail no WSR-88D can, which is why every serious
//! radar app carries them.
//!
//! There is no Level 2 feed, so a "volume" here is synthesized from the Level 3 tilt products the
//! Unidata bucket carries — `TZ0`/`TZ1`/`TZ2` (base reflectivity, product 180) and
//! `TV0`/`TV1`/`TV2` (velocity, 182) — decoded through the same packet-16 reader the DVL/EET
//! grids use, then packed into a [`Scan`] so the rest of the app (rendering, palettes, SRV,
//! thresholds) treats a TDWR exactly like any other radar.
//!
//! Keys are `SSS_PPP_YYYY_MM_DD_HH_MM_SS` with the *three*-letter site id (`OKC_TZ0_…`), while
//! the app displays the four-letter id (`TOKC`) people actually say.
//!
//! Deliberately not fetched: `TZL` (long-range reflectivity) is the same 0.5° tilt as `TZ0` at
//! half the resolution, so it would collide with it in the tilt list for no gain.

use chrono::{DateTime, Utc};
use nexrad_model::data::{
    MomentData, PulseWidth, Radial, RadialStatus, Scan, Sweep, VolumeCoveragePattern,
};
use nexrad_model::meta::registry::SiteEntry;
use nexrad_model::meta::Site;

const BUCKET: &str = "https://unidata-nexrad-level3.s3.amazonaws.com";

/// Every TDWR the Unidata Level 3 bucket carries, as registry entries so they are
/// interchangeable with WSR-88Ds everywhere the app looks a site up. Coordinates and heights are
/// the ones the radars themselves report in their product headers, not a transcribed table.
///
/// `id` is the four-letter form the UI shows; the S3 keys use `id[1..]`.
pub const SITES: &[SiteEntry] = &[
    site("TADW", "Andrews AFB", "MD", 38.695, -76.845, 346),
    site("TATL", "Atlanta", "GA", 33.647, -84.262, 1075),
    site("TBNA", "Nashville", "TN", 35.980, -86.662, 817),
    site("TBOS", "Boston", "MA", 42.158, -70.933, 264),
    site("TBWI", "Baltimore", "MD", 39.090, -76.630, 297),
    site("TCLT", "Charlotte", "NC", 35.337, -80.885, 871),
    site("TCMH", "Columbus", "OH", 40.006, -82.715, 1148),
    site("TCVG", "Cincinnati", "OH", 38.898, -84.580, 1053),
    site("TDAL", "Dallas Love", "TX", 32.926, -96.968, 622),
    site("TDAY", "Dayton", "OH", 40.022, -84.123, 1019),
    site("TDCA", "Washington National", "DC", 38.759, -76.962, 345),
    site("TDEN", "Denver", "CO", 39.727, -104.526, 5701),
    site("TDFW", "Dallas-Fort Worth", "TX", 33.065, -96.918, 585),
    site("TDTW", "Detroit", "MI", 42.111, -83.515, 772),
    site("TEWR", "Newark", "NJ", 40.594, -74.270, 136),
    site("TFLL", "Fort Lauderdale", "FL", 26.143, -80.344, 120),
    site("THOU", "Houston Hobby", "TX", 29.516, -95.242, 116),
    site("TIAD", "Washington Dulles", "VA", 39.084, -77.529, 473),
    site(
        "TIAH",
        "Houston Intercontinental",
        "TX",
        30.065,
        -95.567,
        253,
    ),
    site("TIDS", "Indianapolis", "IN", 39.637, -86.435, 847),
    site("TJFK", "New York JFK", "NY", 40.589, -73.880, 112),
    site("TLAS", "Las Vegas", "NV", 36.144, -115.007, 2058),
    site("TLVE", "Cleveland", "OH", 41.290, -82.008, 931),
    site("TMCI", "Kansas City", "MO", 39.499, -94.742, 1090),
    site("TMCO", "Orlando", "FL", 28.344, -81.326, 169),
    site("TMDW", "Chicago Midway", "IL", 41.651, -87.730, 763),
    site("TMEM", "Memphis", "TN", 34.896, -89.993, 483),
    site("TMIA", "Miami", "FL", 25.758, -80.491, 125),
    site("TMKE", "Milwaukee", "WI", 42.819, -88.046, 933),
    site("TMSP", "Minneapolis", "MN", 44.871, -92.933, 1120),
    site("TMSY", "New Orleans", "LA", 30.022, -90.403, 99),
    site("TOKC", "Oklahoma City", "OK", 35.276, -97.510, 1308),
    site("TORD", "Chicago O'Hare", "IL", 41.797, -87.858, 744),
    site("TPBI", "West Palm Beach", "FL", 26.688, -80.273, 133),
    site("TPHL", "Philadelphia", "PA", 39.949, -75.070, 153),
    site("TPHX", "Phoenix", "AZ", 33.420, -112.163, 1089),
    site("TPIT", "Pittsburgh", "PA", 40.501, -80.486, 1386),
    site("TRDU", "Raleigh-Durham", "NC", 36.002, -78.697, 515),
    site("TSDF", "Louisville", "KY", 38.046, -85.611, 731),
    site("TSJU", "San Juan", "PR", 18.474, -66.180, 157),
    site("TSLC", "Salt Lake City", "UT", 40.967, -111.930, 4295),
    site("TSTL", "St. Louis", "MO", 38.805, -90.489, 647),
    site("TTPA", "Tampa", "FL", 27.860, -82.518, 93),
    site("TTUL", "Tulsa", "OK", 36.071, -95.826, 823),
];

/// Table constructor: takes the height in feet the products report and stores metres.
const fn site(
    id: &'static str,
    city: &'static str,
    state: &'static str,
    latitude: f32,
    longitude: f32,
    height_ft: i16,
) -> SiteEntry {
    SiteEntry {
        id,
        city,
        state,
        latitude,
        longitude,
        elevation_meters: (height_ft as i32 * 3048 / 10000) as i16,
    }
}

/// The TDWR with this four-letter id, if any.
pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
    SITES.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

/// Whether `id` names a TDWR rather than a WSR-88D. Note that not every `T…` id is one:
/// TJUA (San Juan) is a WSR-88D, which is why this checks the table rather than the first letter.
pub fn is_tdwr(id: &str) -> bool {
    site_by_id(id).is_some()
}

/// The tilt products a synthesized volume is built from: `(mnemonic, gate length in metres)`.
/// All three reflectivity tilts and all three velocity tilts share the 150 m TDWR gate.
const PRODUCTS: [(&str, u16); 6] = [
    ("TZ0", 150),
    ("TZ1", 150),
    ("TZ2", 150),
    ("TV0", 150),
    ("TV1", 150),
    ("TV2", 150),
];

/// Newest S3 key under `prefix`, or `None` when the listing is empty or unreachable.
async fn newest_key(http: &reqwest::Client, prefix: &str) -> Option<String> {
    let url = format!("{BUCKET}/?list-type=2&prefix={prefix}");
    let xml = http.get(&url).send().await.ok()?.text().await.ok()?;
    // Keys sort by their embedded timestamp, so the last one listed is the newest.
    let (i, _) = xml.rmatch_indices("<Key>").next()?;
    let rest = &xml[i + 5..];
    rest.find("</Key>").map(|e| rest[..e].to_string())
}

/// Timestamp encoded in an S3 key (`OKC_TZ0_2026_08_01_17_27_13`).
fn key_time(key: &str) -> Option<DateTime<Utc>> {
    let tail = key.rsplit('/').next()?;
    let parts: Vec<&str> = tail.split('_').collect();
    let n = parts.len();
    if n < 8 {
        return None;
    }
    let f = |i: usize| parts[n - 8 + i].parse::<u32>().ok();
    let dt = chrono::NaiveDate::from_ymd_opt(f(2)? as i32, f(3)?, f(4)?)?.and_hms_opt(
        f(5)?,
        f(6)?,
        f(7)?,
    )?;
    Some(dt.and_utc())
}

/// Turn one decoded tilt product into a sweep of radials carrying `moment`.
///
/// The product's data levels are kept as the raw bytes and replayed through the fixed-point
/// decoding the Level 2 path already uses: with `scale = 1/increment` and
/// `offset = 2 − minimum/increment`, `(raw − offset)/scale` reproduces the product's own
/// `minimum + (level − 2) × increment` exactly, and levels 0/1 keep their below-threshold and
/// range-folded meanings.
fn sweep_from(
    p: &nexrad_level3::Level3Product,
    elevation_number: u8,
    gate_len_m: u16,
    velocity: bool,
) -> Option<Sweep> {
    let ra = p.radial.as_ref()?;
    let elevation = p.elevation_deg?;
    let min = p.thresholds[0] as f32 / 10.0;
    let inc = p.thresholds[1] as f32 / 10.0;
    if inc == 0.0 || ra.radials.is_empty() {
        return None;
    }
    let (scale, offset) = (1.0 / inc, 2.0 - min / inc);
    let first_gate = ra.first_bin * gate_len_m;
    let spacing = ra.radials[0].delta_deg;
    let radials = ra
        .radials
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let data = MomentData::from_fixed_point(
                r.levels.len() as u16,
                first_gate,
                gate_len_m,
                8,
                scale,
                offset,
                r.levels.clone(),
            );
            let (refl, vel) = if velocity {
                (None, Some(data))
            } else {
                (Some(data), None)
            };
            Radial::new(
                0,
                i as u16,
                r.start_deg,
                spacing,
                RadialStatus::ScanStart,
                elevation_number,
                elevation,
                refl,
                vel,
                None,
                None,
                None,
                None,
                None,
            )
        })
        .collect();
    Some(Sweep::new(elevation_number, radials))
}

/// Fetch the newest tilt products for a TDWR and assemble them into a volume.
///
/// Returns the scan plus the volume name and time. The name is the newest contributing key, so
/// the app's "has this volume changed?" comparison works unchanged.
pub async fn fetch_volume(
    http: &reqwest::Client,
    id: &str,
) -> anyhow::Result<(String, DateTime<Utc>, Scan)> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not a TDWR"))?;
    let short = &meta.id[1..];
    let day = Utc::now().format("%Y_%m_%d").to_string();

    let mut sweeps = Vec::new();
    let mut newest: Option<(String, DateTime<Utc>)> = None;
    for (n, (product, gate_len)) in PRODUCTS.iter().enumerate() {
        let Some(key) = newest_key(http, &format!("{short}_{product}_{day}")).await else {
            continue;
        };
        let Ok(resp) = http.get(format!("{BUCKET}/{key}")).send().await else {
            continue;
        };
        let Ok(bytes) = resp.bytes().await else {
            continue;
        };
        let p = match nexrad_level3::decode(&bytes) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("tdwr decode {key}: {e}");
                continue;
            }
        };
        // Elevation numbers must be unique per sweep; reflectivity and velocity at one tilt stay
        // separate sweeps, which the split-cut-aware binning already handles.
        if let Some(s) = sweep_from(&p, n as u8 + 1, *gate_len, product.starts_with("TV")) {
            sweeps.push(s);
        }
        if let Some(t) = key_time(&key) {
            if newest.as_ref().is_none_or(|(_, prev)| t > *prev) {
                newest = Some((key, t));
            }
        }
    }
    let (name, time) = newest.ok_or_else(|| anyhow::anyhow!("no TDWR products for {id}"))?;
    if sweeps.is_empty() {
        anyhow::bail!("no decodable TDWR tilts for {id}");
    }

    // No separate tower height is published for these.
    let site: Site = meta.to_site();
    // TDWRs have no VCP number; 80 is what the products report as their scan strategy.
    let vcp = VolumeCoveragePattern::new(
        80,
        1,
        0.5,
        PulseWidth::Short,
        false,
        0,
        false,
        0,
        false,
        false,
        0,
        false,
        false,
        Vec::new(),
    );
    Ok((name, time, Scan::with_site(site, vcp, sweeps)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level2::{bin_scan_opts, Moment};

    #[test]
    fn the_table_is_well_formed() {
        assert!(SITES.len() >= 40);
        for s in SITES {
            assert_eq!(s.id.len(), 4, "{} is not a four-letter id", s.id);
            assert!(s.id.starts_with('T'), "{} lacks the T prefix", s.id);
            assert!(
                s.latitude.abs() <= 90.0 && s.longitude.abs() <= 180.0,
                "{}",
                s.id
            );
        }
        // TJUA is a WSR-88D despite the leading T — the old first-letter heuristic got this wrong.
        assert!(!is_tdwr("TJUA"));
        assert!(is_tdwr("TOKC") && is_tdwr("tokc"));
        assert_eq!(site_by_id("TOKC").unwrap().city, "Oklahoma City");
    }

    #[test]
    fn key_time_parses_the_s3_naming() {
        let t = key_time("OKC_TZ0_2026_08_01_17_27_13").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-08-01T17:27:13+00:00");
        assert!(key_time("OKC_TZ0_garbage").is_none());
    }

    /// A synthetic tilt product round-trips through the fixed-point packing back to its own dBZ.
    #[test]
    fn synthesized_sweeps_preserve_product_values() {
        use nexrad_level3::{Level3Product, Radial as L3Radial, RadialArray};
        let mut thresholds = [0i16; 16];
        thresholds[0] = -320; // −32.0 dBZ minimum
        thresholds[1] = 5; // 0.5 dB increment
        let radials = (0..360)
            .map(|deg| L3Radial {
                start_deg: deg as f32,
                delta_deg: 1.0,
                // 0/1 are below-threshold and range-folded; 144 decodes to 39.0 dBZ.
                levels: vec![0, 1, 144],
            })
            .collect();
        let p = Level3Product {
            code: 180,
            lat: 35.276,
            lon: -97.51,
            height_ft: 1308,
            cells: vec![],
            hail: vec![],
            meso: vec![],
            past_tracks: vec![],
            tabular: None,
            graphic: None,
            raw_text: None,
            radial: Some(RadialArray {
                first_bin: 0,
                nbins: 3,
                radials,
            }),
            thresholds,
            elevation_deg: Some(0.5),
        };
        let sweep = sweep_from(&p, 1, 150, false).expect("sweep");
        let site = Site::new(*b"TOKC", 35.276, -97.51, 398, 0);
        let vcp = VolumeCoveragePattern::new(
            80,
            1,
            0.5,
            PulseWidth::Short,
            false,
            0,
            false,
            0,
            false,
            false,
            0,
            false,
            false,
            Vec::new(),
        );
        let scan = Scan::with_site(site, vcp, vec![sweep]);
        let binned = bin_scan_opts(&scan, Moment::Reflectivity, 0, false).expect("bin");
        assert_eq!(binned.gate_count, 3);
        assert!((binned.gate_interval_km - 0.15).abs() < 1e-6, "150 m gates");
        // Reflectivity bins over [−32, 95]; 39 dBZ lands at the same index the Level 2 path gives.
        let row = 0; // azimuth 0
        assert_eq!(binned.data[row], 0, "below threshold");
        assert_eq!(binned.data[row + 1], 1, "range folded");
        let want = 2 + (((39.0f32 + 32.0) / 127.0) * 253.0) as u8;
        assert_eq!(binned.data[row + 2], want, "39 dBZ");
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_a_live_tdwr_volume() {
        let http = reqwest::Client::new();
        let (name, _time, scan) = fetch_volume(&http, "TOKC").await.expect("TOKC volume");
        assert!(name.starts_with("OKC_"), "{name}");
        let tilts = crate::level2::elevation_angles(&scan);
        assert!(!tilts.is_empty(), "at least one tilt");
        assert!(bin_scan_opts(&scan, Moment::Reflectivity, 0, false).is_ok());
    }
}
