//! Europe's radars from EUMETNET's OpenRadarData feed — the fourth network, and the first that
//! spans more than one country.
//!
//! ORD is an OGC EDR API on `api.meteogate.eu`, but the API only serves CoverageJSON: the ODIM
//! files themselves live in a public S3 bucket, `openradar-24h`, one object per volume per moment,
//! keyed `YYYY/MM/DD/CC/nod/PVOL/nod@YYYYMMDDThhmm@<elevations>@<moments>.h5`. Nordic sites
//! instead publish one `SCAN` object per elevation. Anonymous, no key,
//! no registration, CC BY 4.0, and the bucket lists with a prefix — so finding the newest volume
//! is one ~5 KB listing of the current hour rather than the megabyte of HTML DWD charges for the
//! same question.
//!
//! The object is a whole volume: every elevation and (for most publishers) every moment in one
//! ~1 MB file that [`crate::odim::decode`] already reads. That makes a European site an order of
//! magnitude cheaper than a German one, which needs forty files. Publishers that split moments
//! into one file per quantity are handled by fetching the rest of the group and folding them in.
//!
//! The site table is curated from live published objects rather than every registered radar.
//! Publisher-specific differences are handled explicitly:
//!
//! * Malta writes big-endian numeric metadata, now read by `hdf5lite`, but currently publishes
//!   reflectivity only. Norway's 4-byte-address volumes and delayed velocity are supported below.
//! * France publishes `SCAN/` cuts at consecutive minutes within a five-minute cycle, unlike the
//!   Nordic, Swiss, Estonian and Lithuanian SCAN publishers' equal-timestamp cuts.
//! * Spain alternates two- and three-cut PVOL strategies; the newest two-cut strategy carries both
//!   reflectivity and velocity. Slovakia publishes one PVOL object per moment at each timestamp.
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
    site("CHALB", "Albis", "CH", 47.2843, 8.5120, 938),
    site("CHDOL", "La Dole", "CH", 46.4251, 6.0994, 1682),
    site("CHPPM", "Plaine Morte", "CH", 46.3706, 7.4866, 2937),
    site("CHWEI", "Weissfluhgipfel", "CH", 46.8350, 9.7945, 2850),
    site("DKBOR", "Bornholm", "DK", 55.1127, 14.8875, 171),
    site("DKROM", "Romo", "DK", 55.1731, 8.5520, 15),
    site("DKSAM", "Samso", "DK", 55.8119, 10.5853, 48),
    site("DKSIN", "Sindal", "DK", 57.4893, 10.1365, 109),
    site("DKSTE", "Stevns", "DK", 55.3262, 12.4493, 53),
    site("EEHAR", "Harku", "EE", 59.3971, 24.6021, 70),
    site("EESUR", "Sürgavere", "EE", 58.4823, 25.5187, 157),
    site("ESAHR", "Alhaurín el Grande", "ES", 36.6134, -4.6593, 1159),
    site("ESATN", "Artenara", "ES", 28.0188, -15.6145, 1777),
    site("ESGLD", "Gelida", "ES", 41.4082, 1.8849, 662),
    site("ESLID", "Autilla del Pino", "ES", 41.9956, -4.6028, 887),
    site("ESNJR", "Níjar", "ES", 36.8324, -2.0821, 499),
    site("ESPDG", "Perdiguera", "ES", 41.7340, -0.5459, 835),
    site("ESSFT", "Sierra de Fuentes", "ES", 39.4288, -6.2853, 667),
    site("ESSSE", "Bakio", "ES", 43.4033, -2.8419, 625),
    site("ESTJV", "Torrejón de Velasco", "ES", 40.1759, -3.7137, 717),
    site("FRABB", "Abbeville", "FR", 50.1360, 1.8347, 84),
    site("FRAJA", "Ajaccio", "FR", 41.9531, 8.7005, 784),
    site("FRALE", "Aléria", "FR", 42.1297, 9.4964, 63),
    site("FRAVE", "Avesnes", "FR", 50.1283, 3.8118, 209),
    site("FRBLA", "Blaisy-Haut", "FR", 47.3552, 4.7759, 607),
    site("FRBOL", "Bollène", "FR", 44.3231, 4.7622, 325),
    site("FRBOR", "Bordeaux", "FR", 44.8315, -0.6919, 70),
    site("FRBOU", "Bourges", "FR", 47.0586, 2.3596, 174),
    site("FRCAE", "Falaise", "FR", 48.9272, -0.1496, 167),
    site("FRCOL", "Collobrières", "FR", 43.2166, 6.3729, 654),
    site("FRGRE", "Grèzes", "FR", 45.1044, 1.3697, 360),
    site("FRMCL", "Montclar", "FR", 43.9905, 2.6096, 679),
    site("FRMOM", "Momuy", "FR", 43.6245, -0.6094, 146),
    site("FRMTC", "Montancy", "FR", 47.3686, 7.0190, 926),
    site("FRNAN", "Nancy", "FR", 48.7158, 6.5816, 296),
    site("FRNIM", "Nîmes", "FR", 43.8061, 4.5027, 78),
    site("FRNIZ", "Saint Nizier", "FR", 46.0678, 4.4454, 920),
    site("FROPO", "Opoul", "FR", 42.9184, 2.8650, 718),
    site("FRPLA", "Plabennec", "FR", 48.4609, -4.4298, 111),
    site("FRTOU", "Toulouse", "FR", 43.5743, 1.3763, 187),
    site("FRTRA", "Trappes", "FR", 48.7746, 2.0083, 190),
    site("FRTRE", "Treillères", "FR", 47.3374, -1.6563, 81),
    site("FRTRO", "Arcis-sur-Aube", "FR", 48.4621, 4.3093, 165),
    site("FIKAN", "Kankaanpää", "FI", 61.8108, 22.5020, 175),
    site("FIKAU", "Kaunispää", "FI", 68.4344, 27.4440, 463),
    site("FIKES", "Kesälahti", "FI", 61.9070, 29.7977, 174),
    site("FIKOR", "Korpo", "FI", 60.1285, 21.6434, 61),
    site("FIKUO", "Kuopio", "FI", 62.8626, 27.3815, 268),
    site("FILUO", "Luosto", "FI", 67.1391, 26.8969, 533),
    site("FINUR", "Nurmes", "FI", 63.8379, 29.4489, 323),
    site("FIPET", "Petäjävesi", "FI", 62.3045, 25.4401, 271),
    site("FIUTA", "Utajärvi", "FI", 64.7749, 26.3189, 118),
    site("FIVIH", "Vihti", "FI", 60.5562, 24.4956, 181),
    site("FIVIM", "Vimpeli", "FI", 63.1048, 23.8209, 200),
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
    site("LTLAU", "Laukuva", "LT", 55.6090, 22.2395, 200),
    site("LTVIL", "Vilnius", "LT", 54.6262, 25.1068, 193),
    site("MTGUD", "Gudja", "MT", 35.8528, 14.4747, 100),
    site("NLDHL", "Den Helder", "NL", 52.9528, 4.7906, 55),
    site("NLHRW", "Herwijnen", "NL", 51.8369, 5.1381, 25),
    site("NOAND", "Andøya", "NO", 69.2414, 16.0030, 441),
    site("NOBER", "Berlevåg", "NO", 70.5107, 29.0184, 474),
    site("NOBML", "Bømlo", "NO", 59.8540, 5.0900, 102),
    site("NOHAS", "Hasvik", "NO", 70.6052, 22.4430, 446),
    site("NOHFJ", "Hafjell", "NO", 61.2318, 10.5273, 1080),
    site("NOHGB", "Hægebostad", "NO", 58.3601, 7.1648, 631),
    site("NOHUR", "Hurum", "NO", 59.6271, 10.5645, 363),
    site("NORSA", "Rissa", "NO", 63.6900, 10.2040, 616),
    site("NORSG", "Rassegalvarri", "NO", 69.2186, 23.4398, 604),
    site("NORST", "Røst", "NO", 67.5307, 12.0986, 17),
    site("NOSMN", "Sømna", "NO", 65.2199, 11.9926, 315),
    site("NOSTA", "Stad", "NO", 62.1871, 5.1275, 508),
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
    site("SEANG", "Ängelholm", "SE", 56.3675, 12.8517, 209),
    site("SEATV", "Åtvidaberg", "SE", 58.1059, 15.9365, 222),
    site("SEBAA", "Bålsta", "SE", 59.6110, 17.5833, 83),
    site("SEHUV", "Hudiksvall", "SE", 61.5771, 16.7144, 389),
    site("SEKAA", "Karlskrona", "SE", 56.2955, 15.6102, 132),
    site("SEKRN", "Kiruna", "SE", 67.7088, 20.6178, 647),
    site("SELEK", "Leksand", "SE", 60.7230, 14.8776, 457),
    site("SELLA", "Luleå (Rosvik)", "SE", 65.4309, 21.8650, 69),
    site("SEOER", "Örnsköldsvik", "SE", 63.6395, 18.4019, 522),
    site("SEOSD", "Östersund", "SE", 63.2951, 14.7591, 466),
    site("SKJAV", "Malý Javorník", "SK", 48.2556, 17.1524, 600),
    site("SKKOJ", "Kojšovská hoľa", "SK", 48.7827, 20.9873, 1256),
    site("SKKUB", "Kubínska hoľa", "SK", 49.2717, 19.2493, 1425),
    site("SKLAZ", "Španí laz", "SK", 48.2404, 19.2573, 661),
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
        "{BUCKET}?list-type=2&prefix={}/{country}/{nod}/{}/{nod}@{}",
        hour.format("%Y/%m/%d"),
        if matches!(country, "SE" | "FI" | "CH" | "EE" | "LT" | "FR") { "SCAN" } else { "PVOL" },
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

/// Most Norwegian sites publish DBZH/TH at a five-minute mark, with VRADH about one minute later
/// on every other strategy. That velocity volume omits the 0.5° reflectivity cut. Rissa publishes
/// VRADH with the same timestamp and elevations. Prefer the latest complete pair over a newer
/// reflectivity-only scan, so both moments share matching tilts.
fn newest_norwegian_group<'a>(xml: &'a str, nod: &str) -> Vec<&'a str> {
    let all = keys(xml);
    for base in all.iter().rev().copied().filter(|key| key.ends_with("@DBZH.h5")) {
        let Some(name) = base.rsplit('/').next() else { continue };
        let mut fields = name.split('@');
        if fields.next() != Some(nod) { continue; }
        let (Some(stamp), Some(angles)) = (fields.next(), fields.next()) else { continue };
        // These scans start at :00/:05/…/:55, so the delayed file is always in this hour.
        let Some(minute) = stamp.get(11..13).and_then(|s| s.parse::<u8>().ok()).filter(|m| *m < 59) else { continue };
        let velocity_angles = angles.strip_prefix("0.5_").unwrap_or(angles);
        let next_stamp = format!("{}{:02}", &stamp[..11], minute + 1);
        let velocity_name = format!("{nod}@{next_stamp}@{velocity_angles}@VRADH.h5");
        let same_time_name = format!("{nod}@{stamp}@{angles}@VRADH.h5");
        let Some(velocity) = all.iter().copied().find(|key| key.rsplit('/').next() == Some(same_time_name.as_str()))
            .or_else(|| all.iter().copied().find(|key| key.rsplit('/').next() == Some(velocity_name.as_str()))) else { continue };
        let prefix = base.rsplit_once('@').map(|(p, _)| p).unwrap_or(base);
        let mut group = vec![base];
        group.extend(all.iter().copied().filter(|key| *key != base && key.rsplit_once('@').is_some_and(|(p, _)| p == prefix)));
        if !group.contains(&velocity) { group.push(velocity); }
        return group;
    }
    Vec::new()
}

/// Pick the latest *complete enough* SCAN volume. Individual elevations arrive separately;
/// choosing a timestamp after only its base tilt lands would repeatedly replace a full volume
/// with a one-tilt one. Swiss scans publish two cuts; other supported sites publish at least five.
fn newest_scan_group<'a>(xml: &'a str, nod: &str, minimum: usize) -> Vec<&'a str> {
    let all = keys(xml);
    let mut stamps: Vec<&str> = all.iter().filter_map(|key| {
        let mut parts = key.rsplit('/').next()?.split('@');
        (parts.next()? == nod).then(|| parts.next()).flatten()
    }).collect();
    stamps.sort_unstable();
    stamps.dedup();
    for stamp in stamps.into_iter().rev() {
        let prefix = format!("{nod}@{stamp}@");
        let group: Vec<&str> = all.iter().copied().filter(|key| {
            key.rsplit('/').next().is_some_and(|name| name.starts_with(&prefix) && name.contains("DBZH"))
        }).collect();
        if group.len() >= minimum {
            return group;
        }
    }
    Vec::new()
}

/// French cuts are stamped at each minute of a five-minute cycle, not all at its start. Require
/// all five minute slots before presenting a cycle, with the final (lowest) cut as its timestamp.
fn newest_french_scan_group<'a>(xml: &'a str, nod: &str) -> Vec<&'a str> {
    let mut cycles: std::collections::BTreeMap<(&str, u8), Vec<(u8, &'a str)>> =
        std::collections::BTreeMap::new();
    for key in keys(xml) {
        let Some(name) = key.rsplit('/').next() else { continue };
        let mut fields = name.split('@');
        if fields.next() != Some(nod) { continue; }
        let (Some(stamp), Some(_angle), Some(moment)) =
            (fields.next(), fields.next(), fields.next()) else { continue };
        if !moment.contains("DBZH") || stamp.len() != 13 { continue; }
        let Some(minute) = stamp.get(11..13).and_then(|s| s.parse::<u8>().ok()) else { continue };
        if minute >= 60 { continue; }
        cycles.entry((&stamp[..11], minute / 5)).or_default().push((minute % 5, key));
    }
    for (_, cuts) in cycles.into_iter().rev() {
        if cuts.len() < 5 || !(0..5).all(|slot| cuts.iter().any(|(minute, _)| *minute == slot)) {
            continue;
        }
        let mut group: Vec<&str> = cuts.into_iter().map(|(_, key)| key).collect();
        group.sort_unstable();
        let base = group.pop().expect("a complete cycle has a final cut");
        group.insert(0, base);
        return group;
    }
    Vec::new()
}

/// Spanish PVOL alternates a three-cut DBZH/TH-only strategy with a two-cut DBZH/VRADH strategy.
/// Choose the latest dual-moment volume so the velocity picker does not silently lose its data
/// every ten minutes when a reflectivity-only object arrives.
fn newest_spanish_group<'a>(xml: &'a str, nod: &str) -> Vec<&'a str> {
    keys(xml)
        .into_iter()
        .rev()
        .find(|key| key.rsplit('/').next().is_some_and(|name| {
            name.starts_with(&format!("{nod}@")) && name.ends_with("@DBZH_VRADH.h5")
        }))
        .into_iter()
        .collect()
}

/// Slovak reflectivity, TH and velocity arrive as separate objects. Wait for velocity instead of
/// replacing a complete volume with a momentarily newer reflectivity-only one.
fn newest_slovak_group<'a>(xml: &'a str, nod: &str) -> Vec<&'a str> {
    let all = keys(xml);
    for base in all.iter().rev().copied().filter(|key| {
        key.rsplit('/').next().is_some_and(|name| name.starts_with(&format!("{nod}@"))
            && name.ends_with("@DBZH.h5"))
    }) {
        let Some((prefix, _)) = base.rsplit_once('@') else { continue };
        let velocity_name = format!("{prefix}@VRAD.h5");
        if !all.contains(&velocity_name.as_str()) { continue; }
        let mut group = vec![base];
        group.extend(all.iter().copied().filter(|key| *key != base
            && key.rsplit_once('@').is_some_and(|(p, _)| p == prefix)));
        return group;
    }
    Vec::new()
}

/// The newest volume for `id`, assembled from the objects EUMETNET published for it.
pub async fn fetch_volume(
    http: &reqwest::Client,
    id: &str,
    current_name: Option<&str>,
) -> anyhow::Result<Option<(String, DateTime<Utc>, Scan)>> {
    let meta = site_by_id(id).ok_or_else(|| anyhow::anyhow!("{id} is not an OPERA radar"))?;
    let nod = id.to_ascii_lowercase();

    let text = |url: String| {
        let http = http.clone();
        async move {
            http.get(crate::net::fetch_url(&url))
            .timeout(crate::net::FEED_TIMEOUT)
                .send()
                .await
                .ok()?
                .text()
                .await
                .ok()
                .inspect(|t: &String| crate::stats::net(t.len()))
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
        group = (if meta.state == "FR" {
            newest_french_scan_group(&index, &nod)
        } else if meta.state == "ES" {
            newest_spanish_group(&index, &nod)
        } else if meta.state == "SK" {
            newest_slovak_group(&index, &nod)
        } else if matches!(meta.state, "SE" | "FI" | "CH" | "EE" | "LT") {
            newest_scan_group(&index, &nod, if meta.state == "CH" { 2 } else { 5 })
        } else if meta.state == "NO" {
            newest_norwegian_group(&index, &nod)
        } else {
            newest_group(&index, &nod)
        })
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

    let fetch = |key: String| {
        let http = http.clone();
        let url = format!("{BUCKET}/{key}");
        async move {
            let bytes = http
                .get(crate::net::fetch_url(&url))
                .timeout(crate::net::FEED_TIMEOUT)
                .send()
                .await
                .ok()?
                .bytes()
                .await
                .ok()?;
            crate::stats::net(bytes.len());
            match crate::odim::decode(bytes.to_vec()) {
                Ok(v) => Some(v),
                Err(e) => {
                    // A moment that fails to arrive is simply absent from the volume.
                    log::warn!("opera: skipping {url}: {e}");
                    None
                }
            }
        }
    };

    // Reflectivity first, alone, and then ask whether the rest is worth fetching. The key carries
    // a timestamp, but only to the minute, and the name this returns is built from the decoded
    // volume time — so comparing keys would be comparing something subtly different from what the
    // caller holds. Decoding the one object the volume is anchored on answers exactly, and it is
    // the object every other moment gets folded into, so nothing is wasted when the answer is
    // "yes, this is new".
    let (time, base) = fetch(group[0].clone())
        .await
        .ok_or_else(|| anyhow::anyhow!("no decodable OPERA volume for {id}"))?;
    if current_name == Some(volume_name(id, time).as_str()) {
        crate::stats::bump(crate::stats::Counter::FetchSkipped);
        return Ok(None);
    }

    let files = futures_util::future::join_all(group[1..].iter().cloned().map(&fetch)).await;
    let files: Vec<Scan> = files.into_iter().flatten().map(|(_, scan)| scan).collect();
    if matches!(meta.state, "SE" | "FI" | "CH" | "EE" | "LT" | "FR") {
        let parts: Vec<(f32, Sweep)> = std::iter::once(&base)
            .chain(files.iter())
            .filter_map(|scan| {
                (scan.sweeps().len() == 1).then(|| {
                    let sweep = scan.sweeps()[0].clone();
                    (angle(&sweep), sweep)
                })
            })
            .filter(|(angle, _)| angle.is_finite())
            .collect();
        anyhow::ensure!(parts.len() >= if meta.state == "CH" { 2 } else { 5 }, "incomplete SCAN volume for {id}");
        return Ok(Some((
            volume_name(id, time),
            time,
            Scan::with_site(meta.to_site(), base.coverage_pattern().clone(), crate::dwd::assemble(parts)),
        )));
    }
    // Every object in the group is the same scan sampled at once, so sweep `i` of one is sweep `i`
    // of the others; a file that disagrees about the scan it describes is dropped rather than
    // smeared across it.
    let extras: Vec<Scan> = files
        .into_iter()
        .filter(|s| {
            meta.state == "NO" || (s.sweeps().len() == base.sweeps().len()
                && std::iter::zip(s.sweeps(), base.sweeps()).all(|(a, b)| angle(a) == angle(b)))
        })
        .collect();
    let merged: Vec<Sweep> = base
        .sweeps()
        .iter()
        .enumerate()
        .map(|(i, sweep)| {
            let siblings: Vec<Sweep> = extras
                .iter()
                .filter_map(|s| {
                    if meta.state == "NO" {
                        s.sweeps().iter().find(|candidate| angle(candidate) == angle(sweep)
                            && candidate.radials().len() == sweep.radials().len()).cloned()
                    } else { s.sweeps().get(i).cloned() }
                })
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

    Ok(Some((
        volume_name(id, time),
        time,
        // The decoder already built the pattern-0 placeholder ODIM's free-text scan strategy
        // deserves; there is nothing better to say about it here.
        Scan::with_site(
            meta.to_site(),
            base.coverage_pattern().clone(),
            crate::dwd::assemble(ordered),
        ),
    )))
}

/// The name an OPERA volume is known by. Same shape as the DWD one and for the same reason: the
/// probe above has to predict exactly what a full fetch returns.
fn volume_name(id: &str, time: DateTime<Utc>) -> String {
    format!("{id}-{}", time.format("%Y%m%d%H%M%S"))
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

    /// Same contract as the DWD one: the probe's predicted name and the fetched volume's name are
    /// the same function, or the "already showing it" comparison never fires.
    #[test]
    fn volume_name_is_the_site_and_the_volume_time() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-08-29T15:31:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(volume_name("HRBIL", t), "HRBIL-20260829153100");
    }

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

    #[test]
    fn swedish_scan_waits_for_multiple_elevations() {
        let entry = |stamp: &str, angle: &str| format!(
            "<Key>2026/10/15/SE/seang/SCAN/seang@{stamp}@{angle}@DBZH_TH_VRADH.h5</Key>"
        );
        let mut xml = String::new();
        for angle in ["0.5", "1.0", "1.5", "2.0", "2.5", "4.0"] {
            xml.push_str(&entry("20261015T0730", angle));
        }
        xml.push_str(&entry("20261015T0735", "0.5"));
        assert_eq!(newest_scan_group(&xml, "seang", 5).len(), 6);
        for angle in ["1.0", "1.5", "2.0", "2.5"] {
            xml.push_str(&entry("20261015T0735", angle));
        }
        let picked = newest_scan_group(&xml, "seang", 5);
        assert_eq!(picked.len(), 5);
        assert!(picked.iter().all(|key| key.contains("T0735")));
    }

    #[test]
    fn french_scan_waits_for_the_final_minute_of_every_cycle() {
        let entry = |minute: u8, angle: &str| format!(
            "<Key>2026/10/15/FR/frabb/SCAN/frabb@20261015T08{minute:02}@{angle}@DBZH_TH_VRADH.h5</Key>"
        );
        let mut xml = String::new();
        for (minute, angle) in [(45, "7.5"), (45, "90.0"), (46, "0.9"),
            (47, "1.51"), (48, "4.5"), (49, "0.4")] {
            xml.push_str(&entry(minute, angle));
        }
        for (minute, angle) in [(50, "6.5"), (50, "9.5"), (51, "0.9"),
            (52, "1.51"), (53, "3.5")] {
            xml.push_str(&entry(minute, angle));
        }
        let picked = newest_french_scan_group(&xml, "frabb");
        assert_eq!(picked.len(), 6);
        assert!(picked[0].contains("T0849@0.4"));
        xml.push_str(&entry(54, "0.4"));
        let picked = newest_french_scan_group(&xml, "frabb");
        assert_eq!(picked.len(), 6);
        assert!(picked[0].contains("T0854@0.4"));
        assert!(picked.iter().all(|key| key.contains("T085")));
    }

    #[test]
    fn spanish_volume_prefers_recent_dual_moment_strategy() {
        let entry = |stamp: &str, angles: &str, moments: &str| format!(
            "<Key>2026/10/15/ES/eslid/PVOL/eslid@{stamp}@{angles}@{moments}.h5</Key>"
        );
        let xml = format!("{}{}{}", entry("20261015T0856", "0.5_1.4", "DBZH_VRADH"),
            entry("20261015T0900", "0.5_1.4_2.3", "DBZH_TH"),
            entry("20261015T0906", "0.5_1.4", "DBZH_VRADH"));
        let picked = newest_spanish_group(&xml, "eslid");
        assert_eq!(picked.len(), 1);
        assert!(picked[0].contains("T0906"));
        assert!(newest_spanish_group(&xml, "other").is_empty());
    }

    #[test]
    fn slovak_group_waits_for_velocity_object() {
        let entry = |stamp: &str, moment: &str| format!(
            "<Key>2026/10/15/SK/skjav/PVOL/skjav@{stamp}@0.0_0.5_1.0@{moment}.h5</Key>"
        );
        let xml = format!("{}{}{}{}", entry("20261015T0850", "DBZH"),
            entry("20261015T0850", "VRAD"), entry("20261015T0855", "DBZH"),
            entry("20261015T0855", "TH"));
        let picked = newest_slovak_group(&xml, "skjav");
        assert_eq!(picked.len(), 2);
        assert!(picked.iter().all(|key| key.contains("T0850")));
    }

    #[test]
    fn norwegian_group_waits_for_matching_delayed_velocity() {
        let row = |time: &str, angles: &str, moment: &str| format!(
            "<Key>2026/10/15/NO/noand/PVOL/noand@{time}@{angles}@{moment}.h5</Key>"
        );
        let full = "0.5_1.0_2.6_5.2_8.6_13.0_18.6_25.8_35.0_90.0";
        let short = "1.0_2.6_5.2_8.6_13.0_18.6_25.8_35.0_90.0";
        let xml = format!("{}{}{}{}", row("20261015T0755", full, "DBZH"),
            row("20261015T0755", full, "TH"), row("20261015T0756", short, "VRADH"),
            row("20261015T0800", full, "DBZH"));
        let group = newest_norwegian_group(&xml, "noand");
        assert_eq!(group.len(), 3);
        assert!(group[0].contains("T0755"));
        assert!(group[2].contains("T0756") && group[2].contains("VRADH"));
        let rissa = format!("{}{}", row("20261015T0800", full, "DBZH"),
            row("20261015T0800", full, "VRADH")).replace("/noand/", "/norsa/").replace("noand@", "norsa@");
        let paired = newest_norwegian_group(&rissa, "norsa");
        assert_eq!(paired.len(), 2);
        assert!(paired[1].contains("T0800") && paired[1].contains("VRADH"));
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
        let (name, _, scan) = fetch_volume(&http, "HRBIL", None)
            .await
            .expect("volume")
            .expect("a fresh fetch is never up to date");
        assert!(name.starts_with("HRBIL-"), "{name}");
        assert!(scan.sweeps().len() >= 5, "{} sweeps", scan.sweeps().len());
        let base = &scan.sweeps()[0].radials()[0];
        assert!(
            base.reflectivity().is_some(),
            "base tilt carries reflectivity"
        );
        assert!(base.velocity().is_some(), "and velocity");
    }

    #[tokio::test]
    #[ignore]
    async fn live_swedish_scan_assembles_reflectivity_and_velocity() {
        let http = reqwest::Client::new();
        let (_, _, scan) = fetch_volume(&http, "SEANG", None)
            .await.expect("volume").expect("fresh volume");
        assert!(scan.sweeps().len() >= 5);
        let base = &scan.sweeps()[0].radials()[0];
        assert!(base.reflectivity().is_some());
        assert!(base.velocity().is_some());
        for tilt in 0..scan.sweeps().len() {
            crate::level2::bin_scan(&scan, crate::level2::Moment::Reflectivity, tilt)
                .unwrap_or_else(|e| panic!("SEANG tilt {tilt} reflectivity: {e}"));
            crate::level2::bin_scan(&scan, crate::level2::Moment::Velocity, tilt)
                .unwrap_or_else(|e| panic!("SEANG tilt {tilt} velocity: {e}"));
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_finnish_scan_assembles_reflectivity_and_velocity() {
        let http = reqwest::Client::new();
        for id in ["FIKAN", "FIKAU", "FIKES", "FIKOR", "FIKUO", "FILUO", "FINUR", "FIPET", "FIUTA", "FIVIH", "FIVIM"] {
            let (_, _, scan) = fetch_volume(&http, id, None)
                .await.unwrap_or_else(|e| panic!("{id}: {e}"))
                .expect("fresh volume");
            assert!(scan.sweeps().len() >= 5, "{id}");
            let base = &scan.sweeps()[0].radials()[0];
            assert!(base.reflectivity().is_some(), "{id}: reflectivity");
            assert!(base.velocity().is_some(), "{id}: velocity");
            for tilt in 0..5 {
                crate::level2::bin_scan(&scan, crate::level2::Moment::Reflectivity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} reflectivity: {e}"));
                crate::level2::bin_scan(&scan, crate::level2::Moment::Velocity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} velocity: {e}"));
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_swiss_baltic_scans_assemble_every_elevation() {
        let http = reqwest::Client::new();
        for id in ["CHALB", "CHDOL", "CHPPM", "CHWEI", "EEHAR", "EESUR", "LTLAU", "LTVIL"] {
            let (_, _, scan) = fetch_volume(&http, id, None)
                .await.unwrap_or_else(|e| panic!("{id}: {e}"))
                .expect("fresh volume");
            let minimum = if id.starts_with("CH") { 2 } else { 5 };
            assert!(scan.sweeps().len() >= minimum, "{id}");
            for tilt in 0..scan.sweeps().len() {
                crate::level2::bin_scan(&scan, crate::level2::Moment::Reflectivity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} reflectivity: {e}"));
                crate::level2::bin_scan(&scan, crate::level2::Moment::Velocity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} velocity: {e}"));
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_maltese_volume_has_reflectivity_only() {
        let http = reqwest::Client::new();
        let (_, _, scan) = fetch_volume(&http, "MTGUD", None)
            .await.expect("volume").expect("fresh volume");
        assert!(scan.sweeps().len() >= 5);
        crate::level2::bin_scan(&scan, crate::level2::Moment::Reflectivity, 0).expect("reflectivity");
        assert!(!crate::level2::available_moments(&scan)[crate::level2::Moment::Velocity.index()]);
    }

    #[tokio::test]
    #[ignore]
    async fn live_french_cycles_assemble_every_elevation() {
        let http = reqwest::Client::new();
        for site in SITES.iter().filter(|site| site.state == "FR") {
            let id = site.id;
            let (_, _, scan) = fetch_volume(&http, id, None)
                .await.unwrap_or_else(|e| panic!("{id}: {e}"))
                .expect("fresh volume");
            assert!(scan.sweeps().len() >= 5, "{id}");
            for tilt in 0..scan.sweeps().len() {
                crate::level2::bin_scan(&scan, crate::level2::Moment::Reflectivity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} reflectivity: {e}"));
                crate::level2::bin_scan(&scan, crate::level2::Moment::Velocity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} velocity: {e}"));
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_spanish_and_slovak_volumes_assemble_every_elevation() {
        let http = reqwest::Client::new();
        for site in SITES.iter().filter(|site| matches!(site.state, "ES" | "SK")) {
            let id = site.id;
            let (_, _, scan) = fetch_volume(&http, id, None)
                .await.unwrap_or_else(|e| panic!("{id}: {e}"))
                .expect("fresh volume");
            let minimum = if site.state == "ES" { 2 } else { 12 };
            assert!(scan.sweeps().len() >= minimum, "{id}");
            for tilt in 0..scan.sweeps().len() {
                crate::level2::bin_scan(&scan, crate::level2::Moment::Reflectivity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} reflectivity: {e}"));
                crate::level2::bin_scan(&scan, crate::level2::Moment::Velocity, tilt)
                    .unwrap_or_else(|e| panic!("{id} tilt {tilt} velocity: {e}"));
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_norwegian_volume_assembles_reflectivity_and_velocity() {
        let http = reqwest::Client::new();
        let (_, _, scan) = fetch_volume(&http, "NOAND", None)
            .await.expect("volume").expect("fresh volume");
        assert!(scan.sweeps().len() >= 5);
        let base = &scan.sweeps()[0].radials()[0];
        assert!(base.reflectivity().is_some());
        // Norway's separate velocity scan starts at 1.0°, not the 0.5° base cut.
        assert!(scan.sweeps().iter().any(|s| s.radials()[0].velocity().is_some()));
    }
}
