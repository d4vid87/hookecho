//! Germany's radar network from `opendata.dwd.de` — the one keyless international volume feed.
//!
//! DWD publishes every sweep of every C-band radar as a single-sweep ODIM_H5 file, no
//! registration and no key, which makes it the only foreign network this app can actually carry:
//! Environment Canada's Datamart has rendered GIFs only and keeps its ODIM volumes behind HTTP
//! basic auth, Australia's BOM volumetric feed requires registration, and OPERA's composite is
//! licensed. Bytes land in [`crate::odim`] and come back out as a [`Scan`], so rendering,
//! palettes, SRV and thresholds treat Essen exactly like they treat Oklahoma City.
//!
//! The five-minute volume scan is ten separate files, one per elevation, and they are *not*
//! stored in elevation order — index 5 is the 0.5° base tilt and index 9 is the 25° bird bath.
//! Each has a `-LATEST-` symlink beside it, which is what makes this feed cheap to poll: no
//! directory index, ten predictable URLs, fetched together.
//!
//! Every build offers it. `opendata.dwd.de` sends no `Access-Control-Allow-Origin`, so the browser
//! reaches it through the same `/proxy/` route as every other feed (`crate::net::fetch_url`),
//! which puts a volume — ten unfiltered files of ~440 KB — on the shared edge cache. That is real
//! bandwidth, and it is the price of a radar app that draws Germany in a browser.
//!
//! Reflectivity only. Velocity is published (`sweep_vol_v/`) but has no `-LATEST-` symlink, so
//! reaching it means downloading a 1 MB HTML directory index per poll to learn one filename.
//! ponytail: reflectivity-only until someone asks for velocity; the upgrade is one index fetch
//! per volume, whose newest-per-elevation names also unlock ZDR/RHOHV/PHIDP.

use chrono::{DateTime, Utc};
use nexrad_model::data::{Scan, Sweep};
use nexrad_model::meta::registry::SiteEntry;

const BASE: &str = "https://opendata.dwd.de/weather/radar/sites/sweep_vol_z";

/// Elevations in a `vol5minng01` scan. They are numbered by publication slot, not by angle.
const TILTS: u8 = 10;

const fn site(
    id: &'static str,
    city: &'static str,
    state: &'static str,
    latitude: f32,
    longitude: f32,
    elevation_meters: i16,
) -> SiteEntry {
    SiteEntry {
        id,
        city,
        state,
        latitude,
        longitude,
        elevation_meters,
    }
}

/// Every DWD radar the build offers, as registry entries so they interchange with WSR-88Ds
/// everywhere the app looks a site up. `state` carries the German Land, which is what the site
/// list shows.
///
/// Coordinates and heights are the ones each radar reports in its own ODIM `/where` group, read
/// out of a live file per site rather than transcribed from a table.
pub const SITES: &[SiteEntry] = &[
    site("DEAS", "Borkum", "NI", 53.5641, 6.7483, 36),
    site("DEBO", "Boostedt", "SH", 54.0044, 10.0469, 124),
    site("DEDR", "Dresden", "SN", 51.1246, 13.7686, 263),
    site("DEEI", "Eisberg", "BY", 49.5407, 12.4028, 799),
    site("DEES", "Essen", "NW", 51.4056, 6.9671, 185),
    site("DEFB", "Feldberg", "BW", 47.8736, 8.0036, 1516),
    site("DEFL", "Flechtdorf", "HE", 51.3112, 8.8020, 627),
    site("DEHN", "Hannover", "NI", 52.4601, 9.6945, 97),
    site("DEIS", "Isen", "BY", 48.1747, 12.1018, 677),
    site("DEME", "Memmingen", "BY", 48.0421, 10.2192, 724),
    site("DENE", "Neuhaus", "TH", 50.5001, 11.1350, 879),
    site("DENH", "Neuheilenbach", "RP", 50.1097, 6.5483, 585),
    site("DEOF", "Offenthal", "HE", 49.9847, 8.7129, 245),
    site("DEPR", "Prötzel", "BB", 52.6487, 13.8582, 193),
    site("DERO", "Rostock", "MV", 54.1757, 12.0581, 37),
    site("DETU", "Türkheim", "BW", 48.5854, 9.7827, 767),
    site("DEUM", "Ummendorf", "ST", 52.1601, 11.1761, 185),
];

/// The URL path slug and WMO number each site's files are named with. Neither is derivable from
/// the four-letter id (`DEAS` lives under `asb`), so both are carried explicitly.
const PATHS: [(&str, &str, u16); 17] = [
    ("DEAS", "asb", 10103),
    ("DEBO", "boo", 10132),
    ("DEDR", "drs", 10488),
    ("DEEI", "eis", 10780),
    ("DEES", "ess", 10410),
    ("DEFB", "fbg", 10908),
    ("DEFL", "fld", 10440),
    ("DEHN", "hnr", 10339),
    ("DEIS", "isn", 10873),
    ("DEME", "mem", 10950),
    ("DENE", "neu", 10557),
    ("DENH", "nhb", 10605),
    ("DEOF", "oft", 10629),
    ("DEPR", "pro", 10392),
    ("DERO", "ros", 10169),
    ("DETU", "tur", 10832),
    ("DEUM", "umd", 10356),
];

/// The DWD radar with this four-letter id, if any (case-insensitive).
pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
    SITES.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

/// Whether `id` names a DWD radar rather than a radar on a US network.
pub fn is_dwd(id: &str) -> bool {
    site_by_id(id).is_some()
}

fn path_for(id: &str) -> Option<(&'static str, u16)> {
    PATHS
        .iter()
        .find(|(site, _, _)| site.eq_ignore_ascii_case(id))
        .map(|(_, slug, wmo)| (*slug, *wmo))
}

/// The `-LATEST-` symlink for one elevation slot of one site.
///
/// `th` is total (unfiltered) horizontal reflectivity. The clutter-filtered `dbzh` product has no
/// `-LATEST-` symlink, so unfiltered is the only version reachable without a directory index.
fn sweep_url(slug: &str, wmo: u16, tilt: u8) -> String {
    format!(
        "{BASE}/{slug}/unfiltered/ras07-vol5minng01_sweeph5onem_th_{tilt:02}-LATEST-{slug}-{wmo}-hd5"
    )
}

/// Order decoded single-sweep scans into one volume, lowest tilt first.
///
/// DWD's slot numbering is not elevation order, and everything downstream — the tilt selector,
/// cross sections, the derived grids — assumes sweep 1 is the base tilt.
fn assemble(mut parts: Vec<(f32, Sweep)>) -> Vec<Sweep> {
    parts.sort_by(|a, b| a.0.total_cmp(&b.0));
    parts
        .into_iter()
        .enumerate()
        // ponytail: the radials are cloned because nexrad-model exposes sweeps by reference only;
        // it is one ~4 MB copy per volume, in a background task.
        .map(|(i, (_, sweep))| Sweep::new(i as u8 + 1, sweep.radials().to_vec()))
        .collect()
}

/// Fetch the newest volume for a DWD radar: ten elevations at once, assembled into one [`Scan`].
///
/// Returns the scan plus a volume name and time. The URLs never change, so the name is built from
/// the volume's own start time — that is what the app's "has this volume changed?" check compares.
pub async fn fetch_volume(
    http: &reqwest::Client,
    id: &str,
) -> anyhow::Result<(String, DateTime<Utc>, Scan)> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not a DWD radar"))?;
    let (slug, wmo) = path_for(id).ok_or_else(|| anyhow::anyhow!("{id} has no DWD path"))?;

    let jobs = (0..TILTS).map(|tilt| {
        let (http, url) = (http.clone(), sweep_url(slug, wmo, tilt));
        async move {
            let bytes = http
                .get(crate::net::fetch_url(&url))
                .send()
                .await
                .ok()?
                .bytes()
                .await
                .ok()?;
            match crate::odim::decode(bytes.to_vec()) {
                // One file is one sweep; a volume with more would mean the feed changed shape.
                Ok((time, scan)) => {
                    let sweep = scan.sweeps().first()?;
                    let angle = sweep.radials().first()?.elevation_angle_degrees();
                    Some((time, angle, sweep.clone()))
                }
                Err(e) => {
                    // One unreadable elevation shouldn't cost the whole volume.
                    log::warn!("dwd: skipping {url}: {e}");
                    None
                }
            }
        }
    });
    // `join_all` and not a JoinSet: ten independent round trips with a small decode each, and this
    // way the web build compiles the same code.
    let found: Vec<_> = futures_util::future::join_all(jobs)
        .await
        .into_iter()
        .flatten()
        .collect();
    if found.is_empty() {
        anyhow::bail!("no decodable DWD sweeps for {id}");
    }
    // Every file in a volume carries the same volume start time; the earliest is that time even
    // when a straggler from the next volume lands mid-fetch.
    let time = found.iter().map(|(t, _, _)| *t).min().unwrap_or_else(Utc::now);
    let sweeps = assemble(found.into_iter().map(|(_, a, s)| (a, s)).collect());

    // ODIM carries no VCP: `vol5minng01` is a free-text scan strategy. Pattern 0 says so rather
    // than borrowing a NEXRAD number that would imply elevations this radar never scanned.
    let vcp = nexrad_model::data::VolumeCoveragePattern::new(
        0,
        1,
        0.5,
        nexrad_model::data::PulseWidth::Short,
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
    let name = format!("{id}-{}", time.format("%Y%m%d%H%M%S"));
    Ok((name, time, Scan::with_site(meta.to_site(), vcp, sweeps)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two real sweeps of Boostedt, saved from the live feed: publication slot 0 is 5.5° and slot
    /// 5 is the 0.5° base tilt, which is the ordering [`assemble`] exists to fix.
    fn fixture(tilt: &str) -> Vec<u8> {
        let p = format!(
            "{}/tests/data/dwd-boo-tilt{tilt}.h5",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    fn decoded(tilt: &str) -> (f32, Sweep) {
        let (_, scan) = crate::odim::decode(fixture(tilt)).expect("decode");
        let sweep = scan.sweeps()[0].clone();
        let angle = sweep.radials()[0].elevation_angle_degrees();
        (angle, sweep)
    }

    #[test]
    fn every_site_has_a_path_and_the_two_tables_agree() {
        assert_eq!(SITES.len(), PATHS.len());
        for s in SITES {
            let (slug, wmo) = path_for(s.id).unwrap_or_else(|| panic!("{} has no path", s.id));
            assert_eq!(slug.len(), 3, "{} slug", s.id);
            assert!(slug.chars().all(|c| c.is_ascii_lowercase()), "{} slug", s.id);
            assert!((10_000..11_000).contains(&wmo), "{} wmo {wmo}", s.id);
        }
        assert!(is_dwd("debo") && is_dwd("DEBO"));
        assert!(!is_dwd("KTLX"));
    }

    #[test]
    fn the_url_names_the_latest_symlink() {
        assert_eq!(
            sweep_url("boo", 10132, 5),
            "https://opendata.dwd.de/weather/radar/sites/sweep_vol_z/boo/unfiltered/\
             ras07-vol5minng01_sweeph5onem_th_05-LATEST-boo-10132-hd5"
        );
    }

    #[test]
    fn assemble_puts_the_base_tilt_first_whatever_order_it_arrived_in() {
        let (a0, s0) = decoded("00");
        let (a5, s5) = decoded("05");
        assert!(a0 > a5, "slot 0 is the higher tilt: {a0} vs {a5}");

        // Arrived highest-first, as the ten concurrent fetches easily could.
        let sweeps = assemble(vec![(a0, s0), (a5, s5)]);
        let angles: Vec<f32> = sweeps
            .iter()
            .map(|s| s.radials()[0].elevation_angle_degrees())
            .collect();
        assert_eq!(angles, vec![a5, a0]);
        // Numbering must be renewed, not inherited: both files call themselves sweep 1.
        assert_eq!(
            sweeps.iter().map(|s| s.elevation_number()).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// The `-LATEST-` symlinks are DWD's convention, not a standard, and they are what makes this
    /// feed pollable at all. Run with `--ignored` when a German site stops updating.
    #[tokio::test]
    #[ignore]
    async fn live_dwd_volume_assembles() {
        let client = reqwest::Client::new();
        let (name, time, scan) = fetch_volume(&client, "DEBO").await.unwrap();
        let angles: Vec<f32> = scan
            .sweeps()
            .iter()
            .map(|s| s.radials()[0].elevation_angle_degrees())
            .collect();
        println!("{name} at {time}: {angles:?}");
        assert_eq!(scan.sweeps().len(), TILTS as usize, "one sweep per elevation");
        assert!(angles.windows(2).all(|w| w[0] <= w[1]), "ascending: {angles:?}");
        assert!(angles[0] < 1.0, "base tilt is 0.5 degrees, got {}", angles[0]);
        assert!(
            (Utc::now() - time).num_minutes() < 30,
            "{time} is not a live volume"
        );
    }
}
