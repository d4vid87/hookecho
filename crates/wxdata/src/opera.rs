//! Europe's radars from EUMETNET's OpenRadarData feed — the fourth network, and the first that
//! spans more than one country.
//!
//! ORD is an OGC EDR API on `api.meteogate.eu`, but the API only serves CoverageJSON: the ODIM
//! files themselves live in a public S3 bucket, `openradar-24h`, one object per volume per moment,
//! keyed `YYYY/MM/DD/CC/nod/PVOL/nod@YYYYMMDDThhmm@<elevations>@<moments>.h5`. Anonymous, no key,
//! no registration, CC BY 4.0, and the bucket lists with a prefix — so finding the newest volume
//! is one ~5 KB listing of the current hour rather than the megabyte of HTML DWD charges for the
//! same question.
//!
//! The object is a whole volume: every elevation and (for most publishers) every moment in one
//! ~1 MB file that [`crate::odim::decode`] already reads. That makes a European site an order of
//! magnitude cheaper than a German one, which needs forty files. Publishers that split moments
//! into one file per quantity are handled by fetching the rest of the group and folding them in.
//!
//! ponytail: a curated table of the sites that verify live, not the whole 129-radar feed. Three
//! things disqualify a country, and all three were measured rather than assumed:
//!
//! * Norway (12 sites) writes HDF5 with 4-byte file offsets and Malta writes big-endian
//!   datatypes; `hdf5lite` reads neither. Nothing in this crate can fix that.
//! * Switzerland, Finland, France, Estonia, Lithuania and Sweden publish `SCAN/`, one file per
//!   elevation, which is DWD's shape and a different fetch loop. Later, if ever.
//! * Germany is already here through [`crate::dwd`], from the source rather than the aggregator.
//!
//! Growing the table is adding rows. Growing the machinery is not required.

use chrono::{DateTime, Utc};
use nexrad_model::data::{Scan, Sweep};
use nexrad_model::meta::registry::SiteEntry;

const BUCKET: &str = "https://s3.waw3-1.cloudferro.com/openradar-24h";

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

/// Every OPERA radar the build offers. The id is the ODIM `NOD` code upper-cased — five letters,
/// so it can never collide with a four-letter WSR-88D or TDWR id — and `state` carries the ISO
/// country code, which is what the site list shows and what [`crate::tz`] keys off.
///
/// Coordinates and heights are the ones each radar reports in its own ODIM `/where` group, read
/// out of a live file per site rather than transcribed from a table.
pub const SITES: &[SiteEntry] = &[
    site("BEHEL", "Helchteren", "BE", 51.0702, 5.4054, 144),
    site("BEJAB", "Jabbeke", "BE", 51.1917, 3.0642, 50),
    site("BEWID", "Wideumont", "BE", 49.9136, 5.5044, 585),
    site("CZBRD", "Brdy", "CZ", 49.6583, 13.8178, 916),
    site("CZSKA", "Skalky", "CZ", 49.5011, 16.7885, 767),
    site("DKBOR", "Bornholm", "DK", 55.1127, 14.8875, 171),
    site("DKROM", "Romo", "DK", 55.1731, 8.5520, 15),
    site("DKSAM", "Samso", "DK", 55.8119, 10.5853, 48),
    site("DKSIN", "Sindal", "DK", 57.4893, 10.1365, 109),
    site("DKSTE", "Stevns", "DK", 55.3262, 12.4493, 53),
    site("HRBIL", "Bilogora", "HR", 45.8835, 17.2005, 280),
    site("HRDEB", "Debeljak", "HR", 44.0452, 15.3764, 208),
    site("HRGOL", "Goli", "HR", 45.0205, 14.1223, 553),
    site("HRGRA", "Gradiste", "HR", 45.1592, 18.7033, 110),
    site("HRPUN", "Puntijarka", "HR", 45.9078, 15.9684, 1030),
    site("HRULJ", "Uljenje", "HR", 42.8944, 17.4783, 463),
    site("IESHA", "Shannon", "IE", 52.6928, -8.9200, 29),
    site("ISBJO", "Bjolfur", "IS", 65.2659, -14.0618, 1090),
    site("ISKEF", "Keflavik", "IS", 64.0257, -22.6354, 48),
    site("ISSKA", "Skagi", "IS", 66.0557, -20.2680, 164),
    site("ISX2", "Mobile 2", "IS", 63.8696, -20.2023, 42),
    site("NLDHL", "Den Helder", "NL", 52.9528, 4.7906, 55),
    site("NLHRW", "Herwijnen", "NL", 51.8369, 5.1381, 25),
    site("PLBRZ", "Brzuchania", "PL", 50.3942, 20.0832, 434),
    site("PLGDY", "Gdynia", "PL", 54.5009, 18.2718, 261),
    site("PLGSA", "Gora Sw Anny", "PL", 50.4639, 18.1532, 433),
    site("PLLEG", "Legionowo", "PL", 52.4053, 20.9611, 122),
    site("PLPAS", "Pastewnik", "PL", 50.8925, 16.0395, 692),
    site("PLPOZ", "Poznan", "PL", 52.4133, 16.7970, 123),
    site("PLRAM", "Ramza", "PL", 50.1513, 18.7251, 357),
    site("PLRZE", "Rzeszow", "PL", 50.1141, 22.0370, 241),
    site("PLSWI", "Swidwin", "PL", 53.7958, 15.8368, 147),
    site("PLUZR", "Uzranki", "PL", 53.8557, 21.4123, 237),
    site("ROBAR", "Barnova", "RO", 47.0118, 27.5825, 454),
    site("ROBOB", "Bobohalma", "RO", 46.3602, 24.2252, 567),
    site("ROBUC", "Bucuresti", "RO", 44.5127, 26.0773, 133),
    site("ROCRA", "Craiova", "RO", 44.3103, 23.8674, 218),
    site("ROMED", "Medgidia", "RO", 44.2434, 28.2506, 112),
    site("ROORA", "Oradea", "RO", 47.0922, 21.9429, 246),
    site("ROTIM", "Timisoara", "RO", 45.7717, 21.2577, 145),
    site("SILIS", "Lisca", "SI", 46.0678, 15.2849, 950),
    site("SIPAS", "Pasja ravan", "SI", 46.0980, 14.2282, 1043),
];

/// The OPERA radar with this id, if any (case-insensitive).
pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
    SITES.iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

/// Whether `id` names a radar reached through the OPERA feed.
pub fn is_opera(id: &str) -> bool {
    site_by_id(id).is_some()
}

/// The bucket listing that names every object published for `site` in the hour containing `hour`.
fn index_url(country: &str, nod: &str, hour: DateTime<Utc>) -> String {
    // The `@` in the key is left as-is: S3 accepts it unescaped in a prefix, and escaping it would
    // have to survive the proxy's path rewrite as well.
    format!(
        "{BUCKET}?list-type=2&prefix={}/{country}/{nod}/PVOL/{nod}@{}",
        hour.format("%Y/%m/%d"),
        hour.format("%Y%m%dT%H")
    )
}

/// Every `<Key>` in an S3 `ListObjectsV2` response, in the order the bucket returned them —
/// lexicographic, which for these keys is chronological.
///
/// ponytail: `split` over an XML parser. The one field wanted is a text element with no attributes
/// and no nesting, and the feed is treated as hostile: a run-away `<Key>` with no close tag ends
/// the scan rather than the process.
fn keys(xml: &str) -> Vec<&str> {
    xml.split("<Key>")
        .skip(1)
        .map_while(|rest| rest.split_once("</Key>").map(|(k, _)| k))
        .collect()
}

/// The newest published volume in `xml`, as the objects that make it up.
///
/// A key is `nod@stamp@elevations@moments.h5`, and publishers vary: some put every moment in one
/// object, some write one object per quantity, and some run two scan strategies whose objects
/// share a timestamp but not an elevation list. So the choice is anchored on reflectivity — the
/// newest object that carries `DBZH` — and its siblings are the objects agreeing with it on
/// everything left of the moment field. Anchoring on the timestamp alone would pick a
/// velocity-only volume whenever the velocity object of the next scan lands first, which on the
/// Norwegian and Polish publishers is most of the time.
fn newest_group<'a>(xml: &'a str, nod: &str) -> Vec<&'a str> {
    let all = keys(xml);
    let base = all
        .iter()
        .rev()
        .find(|k| {
            let name = k.rsplit('/').next().unwrap_or(k);
            name.starts_with(nod)
                && name
                    .rsplit_once('@')
                    .is_some_and(|(_, m)| m.contains("DBZH"))
        })
        .copied();
    let Some(base) = base else { return Vec::new() };
    let prefix = base.rsplit_once('@').map(|(p, _)| p).unwrap_or(base);
    // Reflectivity first: it is the base every other moment is folded into.
    let mut group = vec![base];
    group.extend(
        all.iter()
            .filter(|k| **k != base && k.rsplit_once('@').is_some_and(|(p, _)| p == prefix))
            .copied(),
    );
    group
}

/// The newest volume for `id`, assembled from the objects EUMETNET published for it.
pub async fn fetch_volume(
    http: &reqwest::Client,
    id: &str,
) -> anyhow::Result<(String, DateTime<Utc>, Scan)> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not an OPERA radar"))?;
    let nod = id.to_ascii_lowercase();

    let text = |url: String| {
        let http = http.clone();
        async move {
            http.get(crate::net::fetch_url(&url))
                .send()
                .await
                .ok()?
                .text()
                .await
                .ok()
        }
    };

    // The listing is keyed by hour, so a volume published at :02 is alone in its hour and the one
    // before it is in the previous one. Two listings is the worst case and they cost ~5 KB each.
    let now = Utc::now();
    let mut group: Vec<String> = Vec::new();
    for back in 0..2 {
        let index = text(index_url(
            meta.state,
            &nod,
            now - chrono::Duration::hours(back),
        ))
        .await
        .unwrap_or_default();
        group = newest_group(&index, &nod)
            .into_iter()
            .map(str::to_string)
            .collect();
        if !group.is_empty() {
            break;
        }
    }
    if group.is_empty() {
        anyhow::bail!("no OPERA volume published for {id} in the last two hours");
    }

    let files = futures_util::future::join_all(group.iter().map(|key| {
        let http = http.clone();
        let url = format!("{BUCKET}/{key}");
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
                Ok(v) => Some(v),
                Err(e) => {
                    // A moment that fails to arrive is simply absent from the volume.
                    log::warn!("opera: skipping {url}: {e}");
                    None
                }
            }
        }
    }))
    .await;

    let mut files = files.into_iter().flatten();
    let (time, base) = files
        .next()
        .ok_or_else(|| anyhow::anyhow!("no decodable OPERA volume for {id}"))?;
    // Every object in the group is the same scan sampled at once, so sweep `i` of one is sweep `i`
    // of the others; a file that disagrees about the scan it describes is dropped rather than
    // smeared across it.
    let extras: Vec<Scan> = files
        .map(|(_, s)| s)
        .filter(|s| {
            s.sweeps().len() == base.sweeps().len()
                && std::iter::zip(s.sweeps(), base.sweeps()).all(|(a, b)| angle(a) == angle(b))
        })
        .collect();
    let merged: Vec<Sweep> = base
        .sweeps()
        .iter()
        .enumerate()
        .map(|(i, sweep)| {
            let siblings: Vec<Sweep> = extras
                .iter()
                .filter_map(|s| s.sweeps().get(i).cloned())
                .collect();
            crate::dwd::merge(sweep, &siblings)
        })
        .collect();

    // Some publishers concatenate two scan strategies into one volume, which leaves an elevation
    // appearing more than once. The map draws tilts in angle order, so the duplicates would stack
    // invisibly on top of each other; the first one wins.
    let mut seen = Vec::new();
    let ordered: Vec<(f32, Sweep)> = merged
        .into_iter()
        .filter_map(|s| {
            let a = angle(&s);
            (!seen.contains(&a.to_bits())).then(|| {
                seen.push(a.to_bits());
                (a, s)
            })
        })
        .collect();

    let name = format!("{id}-{}", time.format("%Y%m%d%H%M%S"));
    Ok((
        name,
        time,
        // The decoder already built the pattern-0 placeholder ODIM's free-text scan strategy
        // deserves; there is nothing better to say about it here.
        Scan::with_site(
            meta.to_site(),
            base.coverage_pattern().clone(),
            crate::dwd::assemble(ordered),
        ),
    ))
}

/// The elevation a sweep was cut at, or `NaN` for an empty one — which no decoded sweep is.
fn angle(sweep: &Sweep) -> f32 {
    sweep
        .radials()
        .first()
        .map_or(f32::NAN, |r| r.elevation_angle_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "<ListBucketResult>\
        <Contents><Key>2026/08/29/PL/plgsa/PVOL/plgsa@20260829T1531@0.5_1.5@DBZH.h5</Key></Contents>\
        <Contents><Key>2026/08/29/PL/plgsa/PVOL/plgsa@20260829T1531@0.5_1.5@TH.h5</Key></Contents>\
        <Contents><Key>2026/08/29/PL/plgsa/PVOL/plgsa@20260829T1531@0.5_1.5@VRADH.h5</Key></Contents>\
        <Contents><Key>2026/08/29/PL/plgsa/PVOL/plgsa@20260829T1536@0.5_1.5@VRADH.h5</Key></Contents>\
        </ListBucketResult>";

    /// The reason the volume is chosen by reflectivity and not by timestamp: the newest object in
    /// the bucket is routinely the velocity half of a volume whose reflectivity has not landed.
    #[test]
    fn the_newest_complete_volume_wins_over_the_newest_object() {
        let g = newest_group(LISTING, "plgsa");
        assert_eq!(g.len(), 3, "{g:?}");
        assert!(
            g[0].ends_with("plgsa@20260829T1531@0.5_1.5@DBZH.h5"),
            "{g:?}"
        );
        assert!(g.iter().all(|k| k.contains("T1531")), "{g:?}");
    }

    /// Objects of a different scan strategy share the timestamp but not the elevation list, and
    /// merging is by sweep index — so they must not be mistaken for siblings.
    #[test]
    fn a_second_scan_strategy_is_not_a_sibling() {
        let xml = LISTING.replace("T1531@0.5_1.5@TH", "T1531@0.3_0.9@TH");
        let g = newest_group(&xml, "plgsa");
        assert_eq!(g.len(), 2, "{g:?}");
        assert!(g.iter().all(|k| k.contains("@0.5_1.5@")), "{g:?}");
    }

    /// The feed is hostile input: a truncated listing must end the scan, not the process.
    #[test]
    fn an_unterminated_key_ends_the_scan() {
        assert_eq!(keys("<Key>a</Key><Key>b"), ["a"]);
        assert!(newest_group("<Key>", "plgsa").is_empty());
    }

    /// One digit out in the prefix and the listing comes back empty.
    #[test]
    fn the_index_url_names_one_hour_of_one_site() {
        let hour = chrono::DateTime::parse_from_rfc3339("2026-08-29T15:36:30Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            index_url("PL", "plgsa", hour),
            "https://s3.waw3-1.cloudferro.com/openradar-24h\
             ?list-type=2&prefix=2026/08/29/PL/plgsa/PVOL/plgsa@20260829T15"
        );
    }

    /// The ids must not collide with the US networks — the site tables are separate and
    /// `site_by_id` resolves them in order, so a collision would silently shadow a radar — and
    /// every site must carry the two-letter country code the bucket path and the timezone table
    /// are built from.
    #[test]
    fn ids_are_five_letter_nod_codes_under_a_country() {
        for s in SITES {
            assert!(
                nexrad_model::meta::registry::site_by_id(s.id).is_none()
                    && crate::tdwr::site_by_id(s.id).is_none(),
                "{} collides with a US site id",
                s.id
            );
            assert!(s.id.starts_with(s.state), "{} is not in {}", s.id, s.state);
            assert_eq!(s.state.len(), 2, "{} is not an ISO country code", s.state);
            assert!(
                crate::sites::site_by_id(s.id).is_some(),
                "{} is unreachable",
                s.id
            );
        }
    }

    /// Costs real network and a live feed; run with `--ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_opera_volume_assembles() {
        let http = reqwest::Client::new();
        let (name, _, scan) = fetch_volume(&http, "HRBIL").await.expect("volume");
        assert!(name.starts_with("HRBIL-"), "{name}");
        assert!(scan.sweeps().len() >= 5, "{} sweeps", scan.sweeps().len());
        let base = &scan.sweeps()[0].radials()[0];
        assert!(
            base.reflectivity().is_some(),
            "base tilt carries reflectivity"
        );
        assert!(base.velocity().is_some(), "and velocity");
    }
}
