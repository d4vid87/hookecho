//! Per-site WSR-88D antenna height above the ground elevation the site registry carries.
//!
//! The beam-height model used one flat 20 m for every site, with a `ponytail:` note saying the
//! towers were "all in this neighbourhood". They are not: across the 152 sites below they run
//! from 9.8 m to 54.8 m, a spread of 45.0 m. At 10 km that is about a fifth of a
//! degree of beam elevation, which is the difference between a ridge clearing the beam and
//! blocking it.
//!
//! Derived, not transcribed: NCEI's `nexrad-stations.txt` publishes each site's elevation in feet
//! measured at the feedhorn, and the site registry publishes the ground elevation in metres. The
//! tower is the difference. Both sources round (whole feet, whole metres), so each value here is
//! good to about half a metre — far better than the 10 m error the flat constant carried.
//!
//! ponytail: WSR-88D only. TDWR and every other radar falls back to [`DEFAULT_TOWER_M`].

/// Median of the table, used for any site not in it.
pub const DEFAULT_TOWER_M: f64 = 29.8;

/// `(ICAO, tower height in metres)`, sorted by id.
const TOWERS: &[(&str, f64)] = &[
    ("KABR", 24.5),
    ("KABX", 24.9),
    ("KAKQ", 43.7),
    ("KAMA", 35.7),
    ("KAMX", 29.8),
    ("KAPX", 29.8),
    ("KARX", 24.6),
    ("KATX", 44.7),
    ("KBBX", 14.4),
    ("KBGM", 29.1),
    ("KBIS", 29.9),
    ("KBLX", 31.7),
    ("KBMX", 34.3),
    ("KBOX", 34.7),
    ("KBRO", 19.8),
    ("KBUF", 29.8),
    ("KBYX", 24.1),
    ("KCAE", 35.2),
    ("KCBW", 35.1),
    ("KCBX", 33.8),
    ("KCCX", 24.7),
    ("KCLE", 29.1),
    ("KCLX", 39.8),
    ("KCRP", 29.3),
    ("KCXX", 34.4),
    ("KCYS", 19.6),
    ("KDAX", 34.9),
    ("KDDC", 24.1),
    ("KDFX", 19.5),
    ("KDGX", 36.6),
    ("KDIX", 25.1),
    ("KDLH", 35.0),
    ("KDMX", 34.8),
    ("KDTX", 43.6),
    ("KDVN", 29.4),
    ("KDYX", 19.2),
    ("KEAX", 29.8),
    ("KEMX", 35.2),
    ("KENX", 32.8),
    ("KEOX", 31.7),
    ("KEPZ", 34.6),
    ("KESX", 25.2),
    ("KEVX", 24.4),
    ("KEWX", 40.8),
    ("KEYX", 35.7),
    ("KFCX", 29.7),
    ("KFDR", 14.8),
    ("KFDX", 15.0),
    ("KFFC", 34.3),
    ("KFSD", 19.7),
    ("KFSX", 29.3),
    ("KFTG", 35.2),
    ("KFWS", 28.8),
    ("KGGW", 32.6),
    ("KGJX", 33.8),
    ("KGLD", 19.6),
    ("KGRB", 42.9),
    ("KGRK", 19.8),
    ("KGRR", 29.7),
    ("KGSP", 29.8),
    ("KGWX", 34.8),
    ("KGYX", 19.5),
    ("KHDX", 14.5),
    ("KHGX", 30.1),
    ("KHNX", 29.6),
    ("KHPX", 9.8),
    ("KHTX", 30.6),
    ("KICT", 19.7),
    ("KICX", 47.7),
    ("KILN", 34.6),
    ("KILX", 45.8),
    ("KIND", 29.4),
    ("KINX", 24.3),
    ("KIWA", 21.6),
    ("KIWX", 28.9),
    ("KJAX", 38.8),
    ("KJGX", 29.4),
    ("KJKL", 30.3),
    ("KLBB", 36.6),
    ("KLCH", 37.8),
    ("KLIX", 47.6),
    ("KLNX", 42.8),
    ("KLOT", 29.6),
    ("KLRX", 45.6),
    ("KLSX", 35.1),
    ("KLTX", 25.2),
    ("KLVX", 34.9),
    ("KLWX", 40.1),
    ("KLZK", 24.8),
    ("KMAF", 28.8),
    ("KMAX", 14.6),
    ("KMBX", 29.6),
    ("KMHX", 35.2),
    ("KMKX", 19.8),
    ("KMLB", 34.4),
    ("KMOB", 25.1),
    ("KMPX", 47.6),
    ("KMQT", 34.8),
    ("KMRX", 29.1),
    ("KMSX", 37.7),
    ("KMTX", 40.9),
    ("KMUX", 25.0),
    ("KMVX", 29.1),
    ("KMXX", 48.7),
    ("KNKX", 29.6),
    ("KNQA", 46.6),
    ("KOAX", 34.7),
    ("KOHX", 30.0),
    ("KOKX", 34.7),
    ("KOTX", 18.5),
    ("KPAH", 35.2),
    ("KPBZ", 24.9),
    ("KPDT", 19.6),
    ("KPOE", 20.2),
    ("KPUX", 34.6),
    ("KRAX", 34.8),
    ("KRGX", 29.1),
    ("KRIW", 19.9),
    ("KRLX", 40.7),
    ("KRTX", 47.7),
    ("KSFX", 19.5),
    ("KSGF", 29.1),
    ("KSHV", 35.0),
    ("KSJT", 34.8),
    ("KSOX", 23.7),
    ("KSRX", 29.6),
    ("KTBW", 24.2),
    ("KTFX", 27.8),
    ("KTLH", 34.9),
    ("KTLX", 19.5),
    ("KTWX", 14.3),
    ("KTYX", 35.4),
    ("KUDX", 54.8),
    ("KUEX", 25.0),
    ("KVAX", 46.6),
    ("KVBX", 39.7),
    ("KVNX", 14.4),
    ("KVTX", 24.6),
    ("KVWX", 35.5),
    ("KYUX", 19.8),
    ("PABC", 10.8),
    ("PACG", 19.9),
    ("PAEC", 11.4),
    ("PAHG", 34.5),
    ("PAIH", 20.2),
    ("PAKC", 24.9),
    ("PGUA", 39.7),
    ("PHKI", 48.6),
    ("PHKM", 47.8),
    ("PHMO", 24.1),
    ("PHWA", 24.3),
    ("TJUA", 34.6),
];

/// Antenna height above ground for `id`, or [`DEFAULT_TOWER_M`] if the site is not a WSR-88D we
/// have a measurement for.
pub fn tower_m(id: &str) -> f64 {
    TOWERS
        .binary_search_by(|(k, _)| k.cmp(&id))
        .map(|i| TOWERS[i].1)
        .unwrap_or(DEFAULT_TOWER_M)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_plausible() {
        assert!(
            TOWERS.windows(2).all(|w| w[0].0 < w[1].0),
            "binary_search needs sorted ids"
        );
        for (id, h) in TOWERS {
            assert!(*h > 5.0 && *h < 80.0, "{id}: {h} m is not a radar tower");
        }
    }

    #[test]
    fn known_sites_resolve_and_unknown_ones_fall_back() {
        // KTLX's tower is a shade under 20 m; KMXX's is nearly 50. The flat constant was right
        // for one of them.
        assert!((tower_m("KTLX") - 19.5).abs() < 0.05);
        assert!((tower_m("KMXX") - 48.7).abs() < 0.05);
        assert!(tower_m("KMXX") > tower_m("KTLX") + 20.0);
        // TDWR ids and nonsense both fall back rather than panicking.
        assert_eq!(tower_m("TOKC"), DEFAULT_TOWER_M);
        assert_eq!(tower_m(""), DEFAULT_TOWER_M);
    }
}
