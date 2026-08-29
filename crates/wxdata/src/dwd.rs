//! Germany's radar network from `opendata.dwd.de` — the one keyless international volume feed.
//!
//! DWD publishes every sweep of every C-band radar as a single-sweep ODIM_H5 file, no
//! registration and no key, which makes it the only foreign network this app can actually carry:
//! Environment Canada's Datamart has rendered GIFs only and keeps its ODIM volumes behind HTTP
//! basic auth, Australia's BOM volumetric feed requires registration, and OPERA's composite is
//! licensed. Bytes land in [`crate::odim`] and come back out as a [`Scan`], so rendering,
//! palettes, SRV and thresholds treat Essen exactly like they treat Oklahoma City.
//!
//! The five-minute volume scan is ten separate files per moment, one per elevation, and they are
//! *not* stored in elevation order — index 5 is the 0.5° base tilt and index 9 is the 25° bird
//! bath. Reflectivity has a `-LATEST-` symlink beside each file, which is what makes this feed
//! cheap to poll: no directory index, ten predictable URLs, fetched together.
//!
//! Velocity has no such symlink, and DWD's directory index is a megabyte of ungzipped HTML per
//! moment per site. It does not need one: every product of one elevation is named after that
//! sweep's end time, and the reflectivity file states its own end time in
//! `/dataset1/what/endtime`. So the velocity URL falls out of a file already in hand — see
//! [`crate::odim::end_stamp`] — and the same stamp names the dual-pol products, which are
//! fetched the same way for the same reason (their `-LATEST-` symlinks would let a fetch straddle
//! two volumes).
//!
//! Every build offers it. `opendata.dwd.de` sends no `Access-Control-Allow-Origin`, so the browser
//! reaches it through the same `/proxy/` route as every other feed (`crate::net::fetch_url`),
//! which puts a whole volume on the shared edge cache. That is real bandwidth, and it is the price
//! of a radar app that draws Germany in a browser.
//!
//! ponytail: no per-moment toggles and no partial retry — a moment that fails to arrive is simply
//! absent from the sweep, exactly as it was when this module fetched reflectivity alone.

use chrono::{DateTime, Utc};
use nexrad_model::data::{MomentData, Radial, Scan, Sweep};
use nexrad_model::meta::registry::SiteEntry;

const BASE: &str = "https://opendata.dwd.de/weather/radar/sites";

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

/// The `-LATEST-` reflectivity symlink for one elevation slot of one site.
///
/// `th` is total (unfiltered) horizontal reflectivity. The clutter-filtered `dbzh` product has no
/// `-LATEST-` symlink, so unfiltered is the only version reachable without a directory index.
fn sweep_url(slug: &str, wmo: u16, tilt: u8) -> String {
    format!(
        "{BASE}/sweep_vol_z/{slug}/unfiltered/ras07-vol5minng01_sweeph5onem_th_{tilt:02}-LATEST-{slug}-{wmo}-hd5"
    )
}

/// The moments merged into each reflectivity sweep: the product directory under `sites/`, the
/// subdirectory holding it, and the token its filenames use.
///
/// Velocity is the cheap one — DWD's filtered velocity files are ~70 KB against reflectivity's
/// ~190 KB. The three dual-pol products are ~540 KB each, so carrying them multiplies a volume
/// from ~2 MB to ~16 MB. Dropping them is deleting three rows.
const MOMENTS: [(&str, &str, &str); 4] = [
    ("sweep_vol_v", "hdf5/filter_polarimetric", "vradh"),
    ("sweep_vol_zdr", "unfiltered", "uzdr"),
    ("sweep_vol_rhohv", "unfiltered", "urhohv"),
    ("sweep_vol_phidp", "unfiltered", "uphidp"),
];

/// The timestamped file for one moment of one elevation, named after that sweep's end time.
fn moment_url(m: &(&str, &str, &str), slug: &str, wmo: u16, tilt: u8, stamp: &str) -> String {
    let (dir, sub, name) = *m;
    format!(
        "{BASE}/{dir}/{slug}/{sub}/\
         ras07-vol5minng01_sweeph5onem_{name}_{tilt:02}-{stamp}-{slug}-{wmo}-hd5"
    )
}

/// Fold the moments of `extra` into `base`, gate arrays and all.
///
/// Every product of one elevation is the same scan sampled at once, so ray `i` of one file is ray
/// `i` of the others. Callers drop any sweep whose ray count or elevation angle disagrees, which
/// is the check that keeps that assumption honest.
pub(crate) fn merge(base: &Sweep, extra: &[Sweep]) -> Sweep {
    let radials = base
        .radials()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let pick = |f: fn(&Radial) -> Option<&MomentData>| {
                f(r).or_else(|| extra.iter().find_map(|s| s.radials().get(i).and_then(f)))
                    .cloned()
            };
            Radial::new(
                r.collection_timestamp(),
                r.azimuth_number(),
                r.azimuth_angle_degrees(),
                r.azimuth_spacing_degrees(),
                r.radial_status(),
                r.elevation_number(),
                r.elevation_angle_degrees(),
                pick(Radial::reflectivity),
                pick(Radial::velocity),
                pick(Radial::spectrum_width),
                pick(Radial::differential_reflectivity),
                pick(Radial::differential_phase),
                pick(Radial::correlation_coefficient),
                None,
            )
        })
        .collect();
    Sweep::new(base.elevation_number(), radials)
}

/// Order decoded single-sweep scans into one volume, lowest tilt first.
///
/// DWD's slot numbering is not elevation order, and everything downstream — the tilt selector,
/// cross sections, the derived grids — assumes sweep 1 is the base tilt.
pub(crate) fn assemble(mut parts: Vec<(f32, Sweep)>) -> Vec<Sweep> {
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
    current_name: Option<&str>,
) -> anyhow::Result<Option<(String, DateTime<Utc>, Scan)>> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not a DWD radar"))?;
    let (slug, wmo) = path_for(id).ok_or_else(|| anyhow::anyhow!("{id} has no DWD path"))?;

    let get = |url: String| {
        let http = http.clone();
        async move {
            let bytes = http
                .get(crate::net::fetch_url(&url))
                .send()
                .await
                .ok()?
                .bytes()
                .await
                .ok()?;
            crate::stats::net(bytes.len());
            Some((url, bytes.to_vec()))
        }
    };

    // Is there anything new? DWD republishes every `-LATEST-` sweep on a five-minute cycle at a
    // URL that never changes, so there is nothing to list and nothing to compare but the data
    // itself. The lowest tilt's reflectivity is the cheapest thing that answers: ~190 KB against
    // the ~2 MB and ~50 requests a whole volume costs, and every file in a volume carries the
    // same start time, so its stamp names the volume.
    //
    // A GET and not a HEAD: the proxies the browser build goes through are GET-only, and a probe
    // that behaved differently per platform would be a second code path to be wrong in. The bytes
    // are handed to the sweep job below rather than thrown away, so when the volume *has* moved
    // on, the probe costs nothing at all.
    let probe_url = sweep_url(slug, wmo, 0);
    let probe = match probe_sweep(http, &probe_url, id, current_name).await {
        Probe::UpToDate => {
            crate::stats::bump(crate::stats::Counter::FetchSkipped);
            return Ok(None);
        }
        Probe::Fresh(bytes) => Some((probe_url, bytes)),
        Probe::Unreadable => None,
    };
    let mut probe = probe;

    let jobs = (0..TILTS).map(|tilt| {
        let get = &get;
        // Tilt 0 was already fetched by the probe above; the rest go over the wire now.
        let prefetched = (tilt == 0).then(|| probe.take()).flatten();
        async move {
            let (url, bytes) = match prefetched {
                Some(pair) => pair,
                None => get(sweep_url(slug, wmo, tilt)).await?,
            };
            // The stamp has to come off the reflectivity file before anything else is asked for:
            // it is the only name the other moments answer to — and it comes out of the same
            // decode, rather than a second open and parse of the same ~190 KB.
            let (time, scan, stamp) = match crate::odim::decode_with_stamp(bytes) {
                Ok(v) => v,
                Err(e) => {
                    // One unreadable elevation shouldn't cost the whole volume.
                    log::warn!("dwd: skipping {url}: {e}");
                    return None;
                }
            };
            // One file is one sweep; a volume with more would mean the feed changed shape.
            let base = scan.sweeps().first()?;
            let angle = base.radials().first()?.elevation_angle_degrees();

            let Some(stamp) = stamp else {
                log::warn!("dwd: {url} states no sweep end time; reflectivity only");
                return Some((time, angle, base.clone()));
            };
            let extras = futures_util::future::join_all(
                MOMENTS
                    .iter()
                    .map(|m| get(moment_url(m, slug, wmo, tilt, &stamp))),
            )
            .await;
            let extras: Vec<Sweep> = extras
                .into_iter()
                .flatten()
                .filter_map(|(url, bytes)| match crate::odim::decode(bytes) {
                    Ok((_, scan)) => scan.sweeps().first().cloned(),
                    Err(e) => {
                        // A moment that fails to arrive is simply absent from the sweep.
                        log::warn!("dwd: skipping {url}: {e}");
                        None
                    }
                })
                // Merging is by ray index, so a file that disagrees about the scan it describes
                // would smear another elevation's gates across this one.
                .filter(|s| {
                    s.radials().len() == base.radials().len()
                        && s.radials()
                            .first()
                            .is_some_and(|r| r.elevation_angle_degrees() == angle)
                })
                .collect();
            Some((time, angle, merge(base, &extras)))
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
    let time = found
        .iter()
        .map(|(t, _, _)| *t)
        .min()
        .unwrap_or_else(Utc::now);
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
    Ok(Some((
        volume_name(id, time),
        time,
        Scan::with_site(meta.to_site(), vcp, sweeps),
    )))
}

/// What the probe learned about the newest volume.
enum Probe {
    /// The feed still holds the volume the caller is already showing.
    UpToDate,
    /// A newer volume — here are the reflectivity bytes for its lowest tilt, already paid for.
    Fresh(Vec<u8>),
    /// The probe could not be fetched or could not be decoded. Not an answer; carry on and let
    /// the full fetch report whatever is really wrong.
    Unreadable,
}

/// Ask the feed whether the volume has moved on, as cheaply as it can be asked.
///
/// Two layers, and the second is why the first is safe. `opendata.dwd.de` honours both `ETag` and
/// `If-Modified-Since` on these files, so an unchanged poll can be a header exchange with no body
/// at all — but a 304 says only "this file has not changed since *you* last read it", which means
/// "you are up to date" only if what the caller holds is what that read produced. So the volume
/// name is remembered next to the validators and checked against the caller's; when they disagree
/// the request is reissued unconditionally, which is exactly the behaviour of not having the
/// store at all.
///
/// Without a 304 the probe still pays for one ~190 KB file rather than a ~2 MB volume, and those
/// bytes are handed back to be used as the tilt-0 sweep.
async fn probe_sweep(
    http: &reqwest::Client,
    url: &str,
    id: &str,
    current_name: Option<&str>,
) -> Probe {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Only ever conditional when the caller has something to be up to date *with*.
        if current_name.is_some() {
            let remembered = crate::net::validators::get(url);
            let req = crate::net::validators::apply(http.get(crate::net::fetch_url(url)), url);
            if let Ok(resp) = req.send().await {
                if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
                    crate::stats::bump(crate::stats::Counter::NetNotModified);
                    crate::stats::net(0);
                    // The file is unchanged; is it the volume the caller holds?
                    if remembered.and_then(|e| e.tag).as_deref() == current_name {
                        return Probe::UpToDate;
                    }
                    // It is not — fall through and fetch it for real, unconditionally.
                } else if resp.status().is_success() {
                    return finish_probe(url, id, current_name, resp).await;
                } else {
                    return Probe::Unreadable;
                }
            } else {
                return Probe::Unreadable;
            }
        }
    }
    match http.get(crate::net::fetch_url(url)).send().await {
        Ok(resp) if resp.status().is_success() => finish_probe(url, id, current_name, resp).await,
        _ => Probe::Unreadable,
    }
}

/// Read a probe response's body, remember its validators against the volume it turned out to be,
/// and say whether the caller already has that volume.
async fn finish_probe(
    url: &str,
    id: &str,
    current_name: Option<&str>,
    resp: reqwest::Response,
) -> Probe {
    #[cfg(not(target_arch = "wasm32"))]
    let validators = (resp.headers().clone(), url.to_string());
    let Ok(bytes) = resp.bytes().await else {
        return Probe::Unreadable;
    };
    crate::stats::net(bytes.len());
    let bytes = bytes.to_vec();
    // Header only: the probe asks which volume this is, not what is in it.
    let Ok(time) = crate::odim::start_time(bytes.clone()) else {
        return Probe::Unreadable;
    };
    let name = volume_name(id, time);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (headers, url) = validators;
        crate::net::validators::remember_headers(&url, &headers, Some(name.clone()));
    }
    let _ = url;
    if current_name == Some(name.as_str()) {
        return Probe::UpToDate;
    }
    Probe::Fresh(bytes)
}

/// The name a DWD volume is known by: the site and the volume's start time.
///
/// One function because the probe above has to predict exactly what a full fetch would return —
/// two spellings of this format that drifted apart would mean a volume that never looks up to
/// date, and a poll that downloads two megabytes every thirty seconds forever.
fn volume_name(id: &str, time: DateTime<Utc>) -> String {
    format!("{id}-{}", time.format("%Y%m%d%H%M%S"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe skips a poll by predicting the name a full fetch would return. If the two ever
    /// spell it differently, every poll downloads a whole volume forever and nothing looks wrong
    /// — so the format lives in one function, and this is the check that it is the one the caller
    /// compares against.
    #[test]
    fn volume_name_is_the_site_and_the_volume_time() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-08-29T15:31:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(volume_name("DEBO", t), "DEBO-20260829153100");
    }

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
            assert!(
                slug.chars().all(|c| c.is_ascii_lowercase()),
                "{} slug",
                s.id
            );
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

    /// The stamp is the whole reason velocity costs no directory index, and it has to match the
    /// filename convention exactly — one digit out and the URL 404s.
    #[test]
    fn the_end_stamp_names_the_sibling_files() {
        let stamp = crate::odim::end_stamp(fixture("00")).expect("end stamp");
        // The decode reads it out of the file it is already holding; the two must agree, or a
        // volume's dual-pol moments would be asked for under a name that does not exist.
        let (_, _, from_decode) =
            crate::odim::decode_with_stamp(fixture("00")).expect("decode with stamp");
        assert_eq!(from_decode.as_deref(), Some(stamp.as_str()));
        assert_eq!(stamp.len(), 16, "{stamp}");
        assert!(stamp.chars().all(|c| c.is_ascii_digit()), "{stamp}");
        assert!(stamp.ends_with("00"), "centiseconds: {stamp}");
        assert_eq!(
            moment_url(&MOMENTS[0], "boo", 10132, 5, &stamp),
            format!(
                "https://opendata.dwd.de/weather/radar/sites/sweep_vol_v/boo/\
                 hdf5/filter_polarimetric/\
                 ras07-vol5minng01_sweeph5onem_vradh_05-{stamp}-boo-10132-hd5"
            )
        );
    }

    /// Merging must not disturb the moments the base sweep already carries — reflectivity is the
    /// one moment every DWD sweep has, and it is the one the map draws first.
    #[test]
    fn merge_adds_moments_without_replacing_the_ones_already_there() {
        let (_, base) = decoded("05");
        let gates = MomentData::from_fixed_point(4, 125, 250, 8, 2.0, 0.0, vec![1, 2, 3, 4]);
        let velocity_only = Sweep::new(
            1,
            base.radials()
                .iter()
                .map(|r| {
                    Radial::new(
                        r.collection_timestamp(),
                        r.azimuth_number(),
                        r.azimuth_angle_degrees(),
                        r.azimuth_spacing_degrees(),
                        r.radial_status(),
                        r.elevation_number(),
                        r.elevation_angle_degrees(),
                        None,
                        Some(gates.clone()),
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                })
                .collect(),
        );
        let before = base.radials()[0].reflectivity().map(|m| m.values().len());
        let merged = merge(&base, &[velocity_only]);
        let r = &merged.radials()[0];
        assert_eq!(r.reflectivity().map(|m| m.values().len()), before);
        assert!(r.velocity().is_some(), "velocity came from the extra sweep");
        assert_eq!(merged.radials().len(), base.radials().len());
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
            sweeps
                .iter()
                .map(|s| s.elevation_number())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// The `-LATEST-` symlinks are DWD's convention, not a standard, and they are what makes this
    /// feed pollable at all. Run with `--ignored` when a German site stops updating.
    /// The point of the probe, against the live feed: a second poll that is already showing the
    /// newest volume must cost one request instead of ~50, and must say so rather than handing
    /// back a volume the caller then throws away. With validators remembered from the first
    /// fetch, that one request carries no body at all.
    #[tokio::test]
    #[ignore = "network"]
    async fn a_second_poll_of_an_unchanged_volume_costs_one_request() {
        let client = reqwest::Client::new();
        let (name, _, _) = fetch_volume(&client, "DEBO", None)
            .await
            .unwrap()
            .expect("first fetch returns a volume");
        let before = crate::stats::snapshot();
        let again = fetch_volume(&client, "DEBO", Some(&name)).await.unwrap();
        let after = crate::stats::snapshot();
        let requests = |v: &Vec<(&'static str, u64)>| {
            v.iter().find(|(l, _)| *l == "net_requests").unwrap().1
        };
        let bytes =
            |v: &Vec<(&'static str, u64)>| v.iter().find(|(l, _)| *l == "net_bytes").unwrap().1;
        assert!(again.is_none(), "unchanged volume should be skipped");
        assert_eq!(
            requests(&after) - requests(&before),
            1,
            "the probe is one request, not a whole volume"
        );
        assert_eq!(
            bytes(&after) - bytes(&before),
            0,
            "an unchanged probe is answered 304, with no body"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_dwd_volume_assembles() {
        let client = reqwest::Client::new();
        let (name, time, scan) = fetch_volume(&client, "DEBO", None)
            .await
            .unwrap()
            .expect("a fresh fetch is never up to date");
        let angles: Vec<f32> = scan
            .sweeps()
            .iter()
            .map(|s| s.radials()[0].elevation_angle_degrees())
            .collect();
        println!("{name} at {time}: {angles:?}");
        assert_eq!(
            scan.sweeps().len(),
            TILTS as usize,
            "one sweep per elevation"
        );
        // Velocity is reached by a name derived from the reflectivity file, not by a symlink or a
        // directory index; if DWD ever renames its timestamped files, this is what notices.
        let base = &scan.sweeps()[0].radials()[0];
        assert!(base.velocity().is_some(), "base tilt carries velocity");
        assert!(base.correlation_coefficient().is_some(), "and dual-pol");
        assert!(
            angles.windows(2).all(|w| w[0] <= w[1]),
            "ascending: {angles:?}"
        );
        assert!(
            angles[0] < 1.0,
            "base tilt is 0.5 degrees, got {}",
            angles[0]
        );
        assert!(
            (Utc::now() - time).num_minutes() < 30,
            "{time} is not a live volume"
        );
    }
}
