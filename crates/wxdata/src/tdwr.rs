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

use chrono::{DateTime, NaiveDate, Timelike, Utc};
use nexrad_model::data::{
    MomentData, PulseWidth, Radial, RadialStatus, Scan, Sweep, VolumeCoveragePattern,
};
use nexrad_model::meta::registry::SiteEntry;
use nexrad_model::meta::Site;
use crate::level2::Identifier;

const BUCKET: &str = "https://unidata-nexrad-level3.s3.amazonaws.com";
// An archive product can need two listings and a download. Bound each request on web too,
// leaving time to publish the products that did arrive before the pane's volume deadline.
const ARCHIVE_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
pub const ALL_ARCHIVE_HOURS: u32 = (1 << 24) - 1;

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
    site("TDJT", "West Palm Beach", "FL", 26.688, -80.273, 133),
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
    SITES.iter().find(|s| s.id.eq_ignore_ascii_case(canonical_site_id(id)))
}

/// NWS SCN 26-61 renamed the Palm Beach SPG in August 2026. Accept saved TPBI selections and links,
/// but use TDJT for the live Level 3 keys and new frames.
pub fn canonical_site_id(id: &str) -> &str {
    if id.eq_ignore_ascii_case("TPBI") { "TDJT" } else { id }
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

/// The bucket's timestamped keys under one narrow prefix. Hour prefixes stay below S3's
/// default 1,000-key page even on the one-minute low tilt, so a complete hour needs no cursor.
async fn keys_for_prefix(http: &reqwest::Client, prefix: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("{BUCKET}/?list-type=2&prefix={prefix}");
    let xml = http
        .get(crate::net::fetch_url(&url))
        .timeout(crate::net::FEED_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    crate::stats::net(xml.len());
    anyhow::ensure!(!xml.contains("<IsTruncated>true</IsTruncated>"), "TDWR listing exceeded one S3 page");
    Ok(xml.match_indices("<Key>").filter_map(|(i, _)| {
        xml[i + 5..].split_once("</Key>").map(|(key, _)| key.to_owned())
    }).collect())
}

fn latest_key_at_or_before(keys: Vec<String>, anchor: DateTime<Utc>) -> Option<String> {
    keys.into_iter()
        .filter_map(|key| key_time(&key).map(|time| (key, time)))
        .filter(|(_, time)| *time <= anchor && anchor.signed_duration_since(*time) <= chrono::Duration::minutes(10))
        .max_by_key(|(_, time)| *time)
        .map(|(key, _)| key)
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

fn frame_name(site: &str, time: DateTime<Utc>) -> String {
    format!("{site}{}_TDWR", time.format("%Y%m%d_%H%M%S"))
}

fn product_revision(keys: &[Option<String>]) -> String {
    keys.iter().map(|key| key.as_deref().unwrap_or("-")).collect::<Vec<_>>().join("|")
}

fn archive_site_id(id: &str, date: NaiveDate) -> &str {
    if canonical_site_id(id) == "TDJT"
        && date < NaiveDate::from_ymd_opt(2026, 8, 3).unwrap()
    {
        "TPBI"
    } else {
        canonical_site_id(id)
    }
}

/// List every low-tilt TDWR reflectivity scan on a UTC day for the shared archive timeline.
/// One hour per request avoids S3's default pagination limit and admits midnight boundaries.
pub async fn list_volumes(http: &reqwest::Client, id: &str, date: NaiveDate) -> anyhow::Result<Vec<Identifier>> {
    let mut listing = list_archive_hours(http, id, date, ALL_ARCHIVE_HOURS).await?;
    if listing.missing_hours != 0 {
        let retry = list_archive_hours(http, id, date, listing.missing_hours).await?;
        anyhow::ensure!(retry.missing_hours == 0, "TDWR hourly listings incomplete for {id} {date}");
        listing.frames.extend(retry.frames);
        listing.frames.sort_by(|a, b| a.name().cmp(b.name()));
    }
    Ok(listing.frames)
}

pub struct ArchiveListing {
    pub frames: Vec<Identifier>,
    /// UTC hours whose listing failed. A successful empty listing clears its bit.
    pub missing_hours: u32,
}

pub async fn list_archive_hours(
    http: &reqwest::Client,
    id: &str,
    date: NaiveDate,
    hours: u32,
) -> anyhow::Result<ArchiveListing> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not a TDWR"))?;
    let source = archive_site_id(meta.id, date);
    let short = &source[1..];
    let day = date.format("%Y_%m_%d").to_string();
    Ok(collect_archive_hours(source, date, hours, |hour| {
        let prefix = format!("{short}_TZ0_{day}_{hour:02}");
        async move { crate::task::timeout(ARCHIVE_REQUEST_TIMEOUT, keys_for_prefix(http, &prefix)).await? }
    }).await)
}

async fn collect_archive_hours<F>(
    source: &str,
    date: NaiveDate,
    hours: u32,
    list: impl Fn(u32) -> F,
) -> ArchiveListing
where
    F: std::future::Future<Output = anyhow::Result<Vec<String>>>,
{
    let listings = futures_util::future::join_all((0..24).filter(|hour| hours & (1 << hour) != 0).map(|hour| {
        let listing = list(hour);
        async move { (hour, listing.await) }
    })).await;
    let mut listing = ArchiveListing { frames: Vec::new(), missing_hours: 0 };
    for (hour, keys) in listings {
        match keys {
            Ok(keys) => listing.frames.extend(keys.into_iter().filter_map(|key| key_time(&key))
                .filter(|time| time.date_naive() == date && time.hour() == hour)
                .map(|time| Identifier::new(frame_name(source, time)))),
            Err(error) => {
                log::debug!("TDWR listing {source} {date} {hour:02} UTC: {error}");
                listing.missing_hours |= 1 << hour;
            }
        }
    }
    listing.frames.sort_by(|a, b| a.name().cmp(b.name()));
    listing.frames.dedup_by(|a, b| a.name() == b.name());
    listing
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
    collection_timestamp_ms: i64,
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
                collection_timestamp_ms,
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
/// Returns the scan plus the volume name and time. The name follows the newest low-tilt
/// reflectivity key, so later product arrivals cannot rename an already displayed frame. The
/// revision identifies the usable product-key set, so failed downloads are retried even when
/// the listing is unchanged and a same-name frame can gain tilts.
pub async fn fetch_volume(
    http: &reqwest::Client,
    id: &str,
    current_name: Option<&str>,
    current_revision: Option<&str>,
) -> anyhow::Result<Option<(String, DateTime<Utc>, Scan, String)>> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not a TDWR"))?;
    let short = &meta.id[1..];
    // A low-tilt reflectivity key anchors the volume identity and timeline frame. Fetch the
    // current hour for every product concurrently, then fall back to the preceding hour only
    // where needed. Selecting all products at or before the anchor avoids mixing a newer tilt
    // into a frame identified by an older low-tilt scan.
    let now = Utc::now();
    let listings = futures_util::future::join_all(PRODUCTS.iter().map(|(product, _)| {
        let (http, prefix) = (http.clone(), format!("{short}_{product}"));
        async move {
            let hour = format!("{prefix}_{}", now.format("%Y_%m_%d_%H"));
            keys_for_prefix(&http, &hour).await.unwrap_or_default()
        }
    }))
    .await;
    let current_anchor = listings.first().and_then(|keys| keys.iter().filter_map(|key| key_time(key)).max());
    let anchor = if let Some(anchor) = current_anchor {
        anchor
    } else {
        let previous = format!("{short}_TZ0_{}", (now - chrono::Duration::hours(1)).format("%Y_%m_%d_%H"));
        keys_for_prefix(http, &previous).await.unwrap_or_default().iter().filter_map(|key| key_time(key)).max()
            .ok_or_else(|| anyhow::anyhow!("no recent low-tilt TDWR reflectivity for {id}"))?
    };
    let name = frame_name(meta.id, anchor);
    let fallback_hour = if anchor.timestamp().div_euclid(3600) == now.timestamp().div_euclid(3600) {
        anchor - chrono::Duration::hours(1)
    } else {
        anchor
    };
    let keys = futures_util::future::join_all(PRODUCTS.iter().zip(listings).map(|((product, _), current)| {
        let (http, prefix) = (http.clone(), format!("{short}_{product}"));
        async move {
            if let Some(key) = latest_key_at_or_before(current, anchor) { return Some(key); }
            let fallback = format!("{prefix}_{}", fallback_hour.format("%Y_%m_%d_%H"));
            latest_key_at_or_before(keys_for_prefix(&http, &fallback).await.unwrap_or_default(), anchor)
        }
    })).await;
    anyhow::ensure!(keys.first().and_then(|key| key.as_ref()).is_some(), "TDWR low tilt unavailable at selected anchor");
    let revision = product_revision(&keys);
    if current_name == Some(name.as_str()) && current_revision == Some(revision.as_str()) {
        crate::stats::bump(crate::stats::Counter::FetchSkipped);
        return Ok(None);
    }
    assemble_volume(meta, meta.id, keys, anchor, |key| fetch_product(http, key)).await.map(Some)
}

/// Fetch the product set that was current at a listed low-tilt frame, for playback and scrubbing.
pub async fn fetch_volume_at(http: &reqwest::Client, id: &Identifier) -> anyhow::Result<(String, DateTime<Utc>, Scan)> {
    let (name, time, mut scan, missing) = fetch_archive_volume(http, id, None).await?;
    if missing != 0 {
        let (_, _, retried, remaining) = fetch_archive_volume(http, id, Some((&scan, missing))).await?;
        anyhow::ensure!(remaining == 0, "TDWR archive products unavailable for {name}");
        scan = retried;
    }
    Ok((name, time, scan))
}

/// Return the usable scan plus a bit for each product still unresolved. A retry receives the
/// previous scan for this same identifier so only unresolved products need network requests.
pub async fn fetch_archive_volume(
    http: &reqwest::Client,
    id: &Identifier,
    retry: Option<(&Scan, u8)>,
) -> anyhow::Result<(String, DateTime<Utc>, Scan, u8)> {
    let site = id.site().ok_or_else(|| anyhow::anyhow!("TDWR frame has no site"))?;
    let meta = site_by_id(site).ok_or_else(|| anyhow::anyhow!("{site} is not a TDWR"))?;
    let anchor = id.date_time().ok_or_else(|| anyhow::anyhow!("TDWR frame has no valid time"))?;
    anyhow::ensure!(id.name() == frame_name(site, anchor), "TDWR frame name malformed");
    let short = &site[1..];
    let (scan, missing) = collect_archive_products(meta, retry, |index, product, gate_len| async move {
        let key = if index == 0 {
            // The frame name came from this exact low-tilt object. No listing is needed.
            Some(format!("{short}_{product}_{}", anchor.format("%Y_%m_%d_%H_%M_%S")))
        } else {
            let prefix = format!("{short}_{product}");
            archive_product_key(&prefix, anchor, |prefix| async move {
                crate::task::timeout(ARCHIVE_REQUEST_TIMEOUT, keys_for_prefix(http, &prefix)).await?
            }).await?
        };
        let Some(key) = key else { return Ok(None); };
        let p = crate::task::timeout(ARCHIVE_REQUEST_TIMEOUT, fetch_product(http, key))
            .await??;
        let sweep = decode_sweep(&p, index, product, gate_len)
            .ok_or_else(|| anyhow::anyhow!("TDWR {product} contained no usable sweep"))?;
        Ok(Some(sweep))
    }).await?;
    Ok((id.name().to_owned(), anchor, scan, missing))
}

async fn archive_product_key<F>(
    prefix: &str, anchor: DateTime<Utc>, list: impl Fn(String) -> F,
) -> anyhow::Result<Option<String>>
where
    F: std::future::Future<Output = anyhow::Result<Vec<String>>>,
{
    let current = format!("{prefix}_{}", anchor.format("%Y_%m_%d_%H"));
    let key = latest_key_at_or_before(list(current).await?, anchor);
    // A previous-hour key can only be within the ten-minute window near the hour boundary.
    if key.is_some() || anchor.minute() >= 10 {
        return Ok(key);
    }
    let previous = format!("{prefix}_{}", (anchor - chrono::Duration::hours(1)).format("%Y_%m_%d_%H"));
    Ok(latest_key_at_or_before(list(previous).await?, anchor))
}

async fn collect_archive_products<F>(
    meta: &SiteEntry,
    retry: Option<(&Scan, u8)>,
    load: impl Fn(usize, &'static str, u16) -> F,
) -> anyhow::Result<(Scan, u8)>
where
    F: std::future::Future<Output = anyhow::Result<Option<Sweep>>>,
{
    let jobs = PRODUCTS.iter().enumerate().map(|(index, &(product, gate_len))| {
        let load = &load;
        async move {
            if let Some((scan, missing)) = retry {
                if missing & (1 << index) == 0 {
                    return Ok(scan.sweeps().iter().find(|sweep| sweep.elevation_number() == index as u8 + 1).cloned());
                }
            }
            load(index, product, gate_len).await
        }
    });
    let found = futures_util::future::join_all(jobs).await;
    anyhow::ensure!(matches!(found.first(), Some(Ok(Some(_)))), "no usable TDWR low-tilt reflectivity for {}", meta.id);
    let mut missing = 0;
    let mut sweeps = Vec::new();
    for (index, result) in found.into_iter().enumerate() {
        match result {
            Ok(Some(sweep)) => sweeps.push(sweep),
            Ok(None) => {}
            Err(error) => {
                log::debug!("TDWR archive {} {}: {error}", meta.id, PRODUCTS[index].0);
                missing |= 1 << index;
            }
        }
    }
    Ok((build_scan(meta, sweeps), missing))
}

async fn fetch_product(http: &reqwest::Client, key: String) -> anyhow::Result<nexrad_level3::Level3Product> {
    let bytes = http
        .get(crate::net::fetch_url(&format!("{BUCKET}/{key}")))
        .timeout(crate::net::FEED_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    crate::stats::net(bytes.len());
    Ok(nexrad_level3::decode(&bytes)?)
}

async fn assemble_volume<F>(
    meta: &SiteEntry,
    frame_site: &str,
    mut keys: Vec<Option<String>>,
    anchor: DateTime<Utc>,
    fetch: impl Fn(String) -> F,
) -> anyhow::Result<(String, DateTime<Utc>, Scan, String)>
where
    F: std::future::Future<Output = anyhow::Result<nexrad_level3::Level3Product>>,
{
    // Six products. Serially that was most of the wait for a TDWR site; they're independent, so
    // fetch and decode them all at once.
    let mut jobs = Vec::new();
    for ((n, (product, gate_len)), key) in PRODUCTS.iter().enumerate().zip(keys.iter().cloned()) {
        let fetch = &fetch;
        let (product, gate_len) = (*product, *gate_len);
        jobs.push(async move {
            let key = key?;
            let p = match fetch(key.clone()).await {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("tdwr fetch/decode {key}: {e}");
                    return None;
                }
            };
            decode_sweep(&p, n, product, gate_len)
        });
    }
    // `join_all` and not a JoinSet: these are six round trips with a small decode each, so the
    // concurrency that matters is the IO, and this way the web build compiles the same code.
    let found = futures_util::future::join_all(jobs).await;
    anyhow::ensure!(found.first().is_some_and(Option::is_some), "no usable TDWR low-tilt reflectivity for {}", meta.id);
    for (key, sweep) in keys.iter_mut().zip(&found) {
        if sweep.is_none() {
            *key = None;
        }
    }
    let revision = product_revision(&keys);
    // join_all preserves product order, including the elevation numbers of missing sweeps.
    Ok((frame_name(frame_site, anchor), anchor, build_scan(meta, found.into_iter().flatten().collect()), revision))
}

fn decode_sweep(p: &nexrad_level3::Level3Product, index: usize, product: &str, gate_len: u16) -> Option<Sweep> {
    // Reflectivity and velocity at one tilt need distinct sweep numbers for split-cut binning.
    // Packet 16 has no per-radial acquisition clock, so this is the product generation time.
    let timestamp = crate::level3::product_time(p)?;
    sweep_from(p, index as u8 + 1, gate_len, product.starts_with("TV"), timestamp.timestamp_millis())
}

fn build_scan(meta: &SiteEntry, sweeps: Vec<Sweep>) -> Scan {
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
    Scan::with_site(site, vcp, sweeps)
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
        assert_eq!(site_by_id("TPBI").unwrap().id, "TDJT");
        assert_eq!(site_by_id("TDJT").unwrap().id, "TDJT");
        assert!(!SITES.iter().any(|site| site.id == "TPBI"));
        assert_eq!(archive_site_id("TDJT", NaiveDate::from_ymd_opt(2026, 8, 2).unwrap()), "TPBI");
        assert_eq!(archive_site_id("TPBI", NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()), "TDJT");
    }

    #[test]
    fn key_time_parses_the_s3_naming() {
        let t = key_time("OKC_TZ0_2026_08_01_17_27_13").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-08-01T17:27:13+00:00");
        assert!(key_time("OKC_TZ0_garbage").is_none());
    }

    #[test]
    fn live_products_do_not_advance_past_the_low_tilt_frame() {
        let anchor = key_time("DFW_TZ0_2026_10_15_15_04_31").unwrap();
        let keys = [
            "DFW_TV0_2026_10_15_14_50_00",
            "DFW_TV0_2026_10_15_15_03_30",
            "DFW_TV0_2026_10_15_15_05_31",
        ].into_iter().map(str::to_owned).collect();
        assert_eq!(latest_key_at_or_before(keys, anchor).as_deref(), Some("DFW_TV0_2026_10_15_15_03_30"));
    }

    /// A later velocity object must not change the identity of a low-tilt reflectivity frame.
    #[test]
    fn low_tilt_key_names_the_volume() {
        let low_tilt = key_time("OKC_TZ0_2026_08_01_17_27_13").unwrap();
        assert_eq!(frame_name("TOKC", low_tilt), "TOKC20260801_172713_TDWR");
        assert!(latest_key_at_or_before(vec!["OKC_TV0_2026_08_01_17_28_02".to_owned()], low_tilt).is_none());
    }

    #[tokio::test]
    async fn archive_hours_retry_only_failed_listings() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let partial = collect_archive_hours("TOKC", date, (1 << 8) | (1 << 9) | (1 << 10), |hour| {
            std::future::ready(match hour {
                8 => Ok(vec!["OKC_TZ0_2026_08_01_08_27_13".into()]),
                9 => Err(anyhow::anyhow!("temporarily unavailable")),
                10 => Ok(Vec::new()),
                _ => unreachable!(),
            })
        }).await;
        assert_eq!(partial.frames[0].name(), "TOKC20260801_082713_TDWR");
        assert_eq!(partial.missing_hours, 1 << 9);

        let retry = collect_archive_hours("TOKC", date, partial.missing_hours, |hour| {
            assert_eq!(hour, 9);
            std::future::ready(Ok(vec!["OKC_TZ0_2026_08_01_09_00_00".into()]))
        }).await;
        assert_eq!(retry.frames[0].name(), "TOKC20260801_090000_TDWR");
        assert_eq!(retry.missing_hours, 0);
    }

    #[tokio::test]
    async fn archive_product_lookup_distinguishes_an_empty_hour_from_an_error() {
        let anchor = key_time("OKC_TZ0_2026_08_01_17_02_13").unwrap();
        let missing = archive_product_key("OKC_TV0", anchor, |prefix| {
            std::future::ready(if prefix.ends_with("_17") {
                Ok(Vec::new())
            } else {
                Err(anyhow::anyhow!("previous hour unavailable"))
            })
        }).await;
        assert!(missing.is_err());

        let found = archive_product_key("OKC_TV0", anchor, |prefix| {
            std::future::ready(Ok(if prefix.ends_with("_16") {
                vec!["OKC_TV0_2026_08_01_16_59_50".into()]
            } else {
                Vec::new()
            }))
        }).await.unwrap();
        assert_eq!(found.as_deref(), Some("OKC_TV0_2026_08_01_16_59_50"));
    }

    #[tokio::test]
    async fn partial_archive_volume_keeps_reflectivity_and_retries_only_velocity() {
        let meta = site_by_id("TOKC").unwrap();
        let (partial, missing) = collect_archive_products(meta, None, |index, product, gate_len| {
            std::future::ready(match index {
                0 => fixture_product(product).map(|p| decode_sweep(&p, index, product, gate_len)),
                3 => Err(anyhow::anyhow!("velocity listing unavailable")),
                _ => Ok(None),
            })
        }).await.unwrap();
        assert_eq!(missing, 1 << 3);
        assert!(bin_scan_opts(&partial, Moment::Reflectivity, 0, false).is_ok());
        assert!(bin_scan_opts(&partial, Moment::Velocity, 0, false).is_err());

        let (full, missing) = collect_archive_products(meta, Some((&partial, missing)), |index, product, gate_len| {
            assert_eq!(index, 3);
            let key = format!("OKC_{product}_2026_08_01_17_27_13");
            std::future::ready(fixture_product(&key).map(|p| decode_sweep(&p, index, product, gate_len)))
        }).await.unwrap();
        assert_eq!(missing, 0);
        assert_eq!(full.sweeps()[0], partial.sweeps()[0]);
        assert!(bin_scan_opts(&full, Moment::Velocity, 0, false).is_ok());
    }

    fn advertised_products() -> Vec<Option<String>> {
        vec![
            Some("OKC_TZ0_2026_08_01_17_27_13".into()),
            None,
            None,
            Some("OKC_TV0_2026_08_01_17_27_13".into()),
            None,
            None,
        ]
    }

    fn fixture_product(key: &str) -> anyhow::Result<nexrad_level3::Level3Product> {
        let mut product = nexrad_level3::decode(include_bytes!("../../nexrad-level3/tests/data/tz0_okc.l3"))?;
        if key.contains("_TV0_") {
            product.code = 182;
        }
        Ok(product)
    }

    #[tokio::test]
    async fn failed_advertised_product_retries_with_unchanged_keys() {
        let keys = advertised_products();
        let anchor = key_time(keys[0].as_deref().unwrap()).unwrap();
        let meta = site_by_id("TOKC").unwrap();
        let velocity_attempts = std::cell::Cell::new(0);
        let fetch = |key: String| {
            let result = if key.contains("_TV0_") && velocity_attempts.replace(velocity_attempts.get() + 1) == 0 {
                Err(anyhow::anyhow!("temporary velocity download failure"))
            } else {
                fixture_product(&key)
            };
            std::future::ready(result)
        };

        let (name, time, partial, revision) = assemble_volume(meta, meta.id, keys.clone(), anchor, &fetch)
            .await.expect("live reflectivity remains usable");
        assert!(bin_scan_opts(&partial, Moment::Reflectivity, 0, false).is_ok());
        assert!(bin_scan_opts(&partial, Moment::Velocity, 0, false).is_err());
        assert_ne!(revision, product_revision(&keys), "the next poll must not skip the unchanged listing");

        let (next_name, next_time, complete, next_revision) = assemble_volume(meta, meta.id, keys.clone(), anchor, &fetch)
            .await.expect("retry succeeds with the same keys");
        assert_eq!((next_name, next_time), (name, time));
        assert_eq!(velocity_attempts.get(), 2);
        assert_eq!(next_revision, product_revision(&keys), "a complete volume can skip the next unchanged poll");
        assert!(bin_scan_opts(&complete, Moment::Velocity, 0, false).is_ok());
    }

    #[tokio::test]
    async fn failed_or_unusable_anchor_never_returns_a_velocity_only_volume() {
        let keys = advertised_products();
        let anchor = key_time(keys[0].as_deref().unwrap()).unwrap();
        let meta = site_by_id("TOKC").unwrap();
        for undecodable in [true, false] {
            let fetch = |key: String| {
                let result = if key.contains("_TZ0_") && undecodable {
                    Err(anyhow::anyhow!("low reflectivity download failed"))
                } else {
                    fixture_product(&key).map(|mut product| {
                        if key.contains("_TZ0_") { product.radial = None; }
                        product
                    })
                };
                std::future::ready(result)
            };
            let result = assemble_volume(meta, meta.id, keys.clone(), anchor, fetch).await;
            assert!(result.unwrap_err().to_string().contains("low-tilt reflectivity"));
        }
        let mut missing_anchor = keys;
        missing_anchor[0] = None;
        assert!(assemble_volume(meta, meta.id, missing_anchor, anchor, |key| std::future::ready(fixture_product(&key)))
            .await.is_err());
    }

    #[tokio::test]
    async fn archive_requires_usable_low_tilt_reflectivity() {
        let meta = site_by_id("TOKC").unwrap();
        let result = collect_archive_products(meta, None, |index, product, gate_len| {
            std::future::ready(match index {
                0 => Err(anyhow::anyhow!("low reflectivity download failed")),
                3 => fixture_product("OKC_TV0_2026_08_01_17_27_13")
                    .map(|p| decode_sweep(&p, index, product, gate_len)),
                _ => Ok(None),
            })
        }).await;
        assert!(result.unwrap_err().to_string().contains("low-tilt reflectivity"));
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
            modified_julian_date: 1,
            seconds_since_midnight: 0,
            volume_scan_date: 1,
            volume_scan_time: 0,
            generation_date: 1,
            generation_time: 0,
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
        let timestamp = 1_791_842_420_000;
        let sweep = sweep_from(&p, 1, 150, false, timestamp).expect("sweep");
        assert_eq!(sweep.radials()[0].collection_timestamp(), timestamp);
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
        let (name, time, scan, revision) = fetch_volume(&http, "TOKC", None, None)
            .await
            .expect("TOKC volume")
            .expect("a fresh fetch is never up to date");
        assert!(name.starts_with("TOKC"), "{name}");
        assert!(revision.contains("OKC_TZ0_"));
        // A failed optional download is retried even if the usable subset stays unchanged.
        fetch_volume(&http, "TOKC", Some(&name), Some(&revision))
            .await.expect("TDWR revision probe");
        assert!(Identifier::new(name).date_time().is_some());
        let tilts = crate::level2::elevation_angles(&scan);
        assert!(!tilts.is_empty(), "at least one tilt");
        assert!(bin_scan_opts(&scan, Moment::Reflectivity, 0, false).is_ok());
        for sweep in scan.sweeps() {
            for radial in sweep.radials() {
                let at = radial.collection_timestamp();
                // The Level 3 product header can trail its S3 key by several seconds.
                assert!(at <= (time + chrono::Duration::minutes(2)).timestamp_millis(), "a tilt is much newer than its frame: product {at}, key {}", time.timestamp_millis());
                assert!(at >= (time - chrono::Duration::minutes(10)).timestamp_millis(), "a tilt is older than its frame window: product {at}, key {}", time.timestamp_millis());
            }
        }
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn lists_and_fetches_distinct_tdwr_archive_frames() {
        let http = reqwest::Client::new();
        let frames = list_volumes(&http, "TDFW", Utc::now().date_naive()).await.expect("TDFW listing");
        assert!(frames.len() >= 2, "need two real TDFW frames for playback");
        let earlier = &frames[frames.len() - 2];
        let latest = &frames[frames.len() - 1];
        assert!(earlier.date_time() < latest.date_time());
        for id in [earlier, latest] {
            let (name, time, scan) = fetch_volume_at(&http, id).await.expect("historical TDWR volume");
            assert_eq!(name, id.name());
            assert_eq!(Some(time), id.date_time());
            assert!(bin_scan_opts(&scan, Moment::Reflectivity, 0, false).is_ok());
            assert!(bin_scan_opts(&scan, Moment::Velocity, 0, false).is_ok());
        }
    }
}
