//! Local timezone for each NEXRAD site.
//!
//! `nexrad-model`'s [`SiteEntry`](crate::sites::SiteEntry) carries no timezone column and lives in
//! an upstream crate, so the mapping lives here as a side table keyed by site id.
//!
//! ponytail: hand-authored match instead of a geo lookup — `tzf-rs` would embed multi-megabyte
//! timezone polygons into a 28 MB APK to answer 157 fixed questions. The three tests below make
//! the table self-policing: `every_site_has_a_timezone` fails the moment the registry grows a site
//! this table doesn't know about.

pub use chrono_tz::Tz;

/// Timezone of the radar site `id` (e.g. `"KTLX"`), or `None` if the id is not a known site.
pub fn site_tz(id: &str) -> Option<Tz> {
    use chrono_tz::Tz::*;
    Some(match id {
        // Alaska. Southeast (Sitka/Yakutat) keeps its own zone name; everything else is Anchorage.
        "KACG" | "PACG" => America__Sitka,
        "KABC" | "KAIH" | "KAKC" | "PABC" | "PAEC" | "PAHG" | "PAIH" | "PAKC" => America__Anchorage,

        // Hawaii, Guam, Puerto Rico.
        "PHKI" | "PHKM" | "PHMO" | "PHWA" => Pacific__Honolulu,
        "PGUA" => Pacific__Guam,
        "TJUA" => America__Puerto_Rico,

        // Pacific.
        "KATX" | "KOTX" => America__Los_Angeles, // WA
        "KMAX" | "KPDT" | "KRTX" => America__Los_Angeles, // OR — Pendleton is Pacific, not Mountain
        "KESX" | "KLRX" | "KRGX" => America__Los_Angeles, // NV
        "KBBX" | "KDAX" | "KEYX" | "KHNX" | "KMUX" | "KNKX" | "KSOX" | "KVBX" | "KVTX" => {
            America__Los_Angeles // CA
        }

        // Mountain, no DST.
        "KEMX" | "KFSX" | "KIWA" | "KYUX" => America__Phoenix, // AZ

        // Mountain.
        "KCBX" | "KSFX" => America__Boise,                    // ID
        "KBLX" | "KGGW" | "KMSX" | "KTFX" => America__Denver, // MT
        "KCYS" | "KRIW" => America__Denver,                   // WY
        "KICX" | "KMTX" => America__Denver,                   // UT
        "KFTG" | "KGJX" | "KPUX" => America__Denver,          // CO
        "KABX" | "KFDX" | "KHDX" => America__Denver,          // NM
        "KGLD" => America__Denver,                            // KS — Goodland is Mountain
        "KUDX" => America__Denver,                            // SD — Rapid City is Mountain
        "KEPZ" => America__Denver,                            // TX — El Paso is Mountain

        // Central.
        "KABR" | "KFSD" => America__Chicago,          // SD
        "KBIS" | "KMBX" | "KMVX" => America__Chicago, // ND
        "KLNX" | "KOAX" | "KUEX" => America__Chicago, // NE
        "KDDC" | "KICT" | "KTWX" => America__Chicago, // KS
        "KFDR" | "KINX" | "KTLX" | "KVNX" => America__Chicago, // OK
        "KAMA" | "KBRO" | "KCRP" | "KDFX" | "KDYX" | "KEWX" | "KFWS" | "KGRK" | "KHGX" | "KLBB"
        | "KMAF" | "KSJT" => America__Chicago, // TX
        "KDLH" | "KMPX" => America__Chicago,          // MN
        "KARX" | "KGRB" | "KMKX" => America__Chicago, // WI
        "KDMX" | "KDVN" => America__Chicago,          // IA
        "KILX" | "KLOT" => America__Chicago,          // IL
        "KEAX" | "KLSX" | "KSGF" => America__Chicago, // MO
        "KLZK" | "KSRX" => America__Chicago,          // AR
        "KLCH" | "KLIX" | "KPOE" | "KSHV" => America__Chicago, // LA
        "KDGX" | "KGWX" => America__Chicago,          // MS
        "KBMX" | "KEOX" | "KHTX" | "KMOB" | "KMXX" => America__Chicago, // AL
        "KNQA" | "KOHX" => America__Chicago,          // TN — Memphis and Nashville
        "KHPX" | "KPAH" => America__Chicago,          // KY — Fort Campbell and Paducah
        "KVWX" => America__Chicago,                   // IN — Gibson County is Central
        "KEVX" => America__Chicago,                   // FL — Eglin AFB is in the Central panhandle

        // Eastern, with their own zone names.
        "KIND" | "KIWX" => America__Indiana__Indianapolis,
        "KAPX" | "KDTX" | "KGRR" | "KMQT" => America__Detroit, // MI — Marquette is Eastern

        // Eastern.
        "KCBW" | "KGYX" => America__New_York, // ME
        "KBOX" => America__New_York,          // MA
        "KCXX" => America__New_York,          // VT
        "KBGM" | "KBUF" | "KENX" | "KOKX" | "KTYX" => America__New_York, // NY
        "KCCX" | "KDIX" | "KPBZ" => America__New_York, // PA
        "KCLE" | "KILN" => America__New_York, // OH
        "KRLX" => America__New_York,          // WV
        "KAKQ" | "KFCX" | "KLWX" => America__New_York, // VA
        "KLTX" | "KMHX" | "KRAX" => America__New_York, // NC
        "KCAE" | "KCLX" | "KGSP" => America__New_York, // SC
        "KFFC" | "KJGX" | "KVAX" => America__New_York, // GA
        "KAMX" | "KBYX" | "KJAX" | "KMLB" | "KTBW" | "KTLH" => America__New_York, // FL
        "KJKL" | "KLVX" => America__New_York, // KY — Jackson and Fort Knox
        "KMRX" => America__New_York,          // TN — Knoxville

        // TDWRs (terminal radars at airports), in the same geographic order.
        "TLAS" => America__Los_Angeles,                         // NV
        "TPHX" => America__Phoenix,                             // AZ, no DST
        "TDEN" => America__Denver,                              // CO
        "TSLC" => America__Denver,                              // UT
        "TDAL" | "TDFW" | "THOU" | "TIAH" => America__Chicago,  // TX
        "TOKC" | "TTUL" => America__Chicago,                    // OK
        "TMCI" | "TSTL" => America__Chicago,                    // MO
        "TMSP" => America__Chicago,                             // MN
        "TMKE" => America__Chicago,                             // WI
        "TMDW" | "TORD" => America__Chicago,                    // IL
        "TMEM" | "TBNA" => America__Chicago,                    // TN
        "TMSY" => America__Chicago,                             // LA
        "TSDF" => America__New_York,                            // KY — Louisville is Eastern
        "TIDS" => America__Indiana__Indianapolis,               // IN
        "TDTW" => America__Detroit,                             // MI
        "TCMH" | "TCVG" | "TDAY" | "TLVE" => America__New_York, // OH
        "TPIT" | "TPHL" => America__New_York,                   // PA
        "TEWR" => America__New_York,                            // NJ
        "TJFK" => America__New_York,                            // NY
        "TBOS" => America__New_York,                            // MA
        "TADW" | "TBWI" | "TDCA" | "TIAD" => America__New_York, // DC / MD / VA
        "TCLT" | "TRDU" => America__New_York,                   // NC
        "TATL" => America__New_York,                            // GA
        "TFLL" | "TMCO" | "TMIA" | "TPBI" | "TTPA" => America__New_York, // FL
        "TSJU" => America__Puerto_Rico,                         // PR

        // Germany is one zone, so the DWD table needs no per-site entry — but it does need to be
        // asked, or every German radar falls through to `None` and reads UTC.
        _ if crate::dwd::is_dwd(id) => Europe__Berlin,

        // OPERA sites carry an ISO country code instead of a state, and every country in the
        // table is a single zone, so the zone is a property of the country and not of the radar.
        // `every_site_has_a_timezone` fails the moment the site table grows a country this match
        // does not answer for.
        _ => match crate::opera::site_by_id(id)?.state {
            "BE" => Europe__Brussels,
            "CZ" => Europe__Prague,
            "DK" => Europe__Copenhagen,
            "HR" => Europe__Zagreb,
            "IE" => Europe__Dublin,
            "IS" => Atlantic__Reykjavik,
            "NL" => Europe__Amsterdam,
            "PL" => Europe__Warsaw,
            "RO" => Europe__Bucharest,
            "SI" => Europe__Ljubljana,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Offset, TimeZone};
    use chrono_tz::Tz::*;

    /// Tripwire: the registry is upstream, so a new site must not silently fall through to `None`.
    #[test]
    fn every_site_has_a_timezone() {
        let missing: Vec<_> = crate::sites::all()
            .filter(|s| site_tz(s.id).is_none())
            .map(|s| format!("{} ({}, {})", s.id, s.city, s.state))
            .collect();
        assert!(missing.is_empty(), "sites with no timezone: {missing:?}");
    }

    /// Sites that do not follow their state's default zone.
    #[test]
    fn state_boundary_exceptions() {
        for (id, want) in [
            ("KEPZ", America__Denver),                // TX, Mountain
            ("KGLD", America__Denver),                // KS, Mountain
            ("KUDX", America__Denver),                // SD, Mountain
            ("KABR", America__Chicago),               // SD, Central
            ("KEVX", America__Chicago),               // FL, Central
            ("KTLH", America__New_York),              // FL, Eastern
            ("KVWX", America__Chicago),               // IN, Central
            ("KIND", America__Indiana__Indianapolis), // IN, Eastern
            ("KHPX", America__Chicago),               // KY, Central
            ("KLVX", America__New_York),              // KY, Eastern
            ("KNQA", America__Chicago),               // TN, Central
            ("KMRX", America__New_York),              // TN, Eastern
            ("KMQT", America__Detroit),               // MI, Eastern
            ("KIWA", America__Phoenix),               // AZ, no DST
            ("PACG", America__Sitka),                 // AK southeast
        ] {
            assert_eq!(site_tz(id), Some(want), "{id}");
        }
        assert_eq!(site_tz("XXXX"), None);
    }

    /// Gross-typo catcher: a zone's January standard offset should roughly track its longitude.
    #[test]
    fn standard_offset_tracks_longitude() {
        for s in crate::sites::sites() {
            let tz = site_tz(s.id).expect("covered by every_site_has_a_timezone");
            let jan = tz
                .with_ymd_and_hms(2026, 1, 15, 12, 0, 0)
                .single()
                .expect("noon in January is unambiguous");
            assert_eq!(jan.month(), 1);
            let solar = f64::from(jan.offset().fix().local_minus_utc()) / 3600.0 * 15.0;
            let delta = (solar - f64::from(s.longitude)).abs();
            assert!(
                delta < 35.0,
                "{} ({}): {tz} is {delta:.0}° off",
                s.id,
                s.city
            );
        }
    }
}
