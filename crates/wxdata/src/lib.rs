//! Weather radar data acquisition and domain model for HookEcho.

pub mod afd;
pub mod airnow;
pub mod alerts;
pub mod archive_warnings;
pub mod aviation;
pub mod banding;
pub mod cellscore;
pub mod clock;
pub mod contour;
pub mod dat;
pub mod dealias;
pub mod derived;
pub mod dotcams;
pub mod dualpol;
pub mod dwd;
pub mod eccc;
pub mod efield;
pub mod ero;
pub mod forecast;
pub mod fronts;
pub mod geocode;
pub mod glm;
pub mod global;
pub mod hrrr;
pub mod kdp;
pub mod level2;
pub mod level3;
pub mod live;
pub mod lsr;
pub mod metar;
pub mod meteoalarm;
pub mod mosaic;
pub mod mping;
pub mod mrms;
pub mod ndbc;
pub mod net;
pub mod nohrsc;
pub mod obs;
pub mod odim;
pub mod openmeteo;
pub mod opera;
pub mod overlay;
pub mod placefile;
pub mod probsevere;
pub mod raob;
pub mod recon;
pub mod river;
pub mod rotation;
pub mod severe;
pub mod sounding;
pub mod spc;
pub mod spoken;
pub mod spotters;
pub mod stations;
pub mod stats;
pub mod synoptic;
pub mod task;
pub mod tds;
pub mod tdwr;
pub mod tfr;
pub mod torclimo;
pub mod towers;
pub mod tropical;
pub mod tz;
pub mod verify;
pub mod volume3d;
pub mod vtec;
/// Off-main-thread Level 2 decode in the browser.
#[cfg(target_arch = "wasm32")]
pub mod wasm_worker;
pub mod webcams;
pub mod wfigs;
pub mod wssi;
pub mod xsection;

/// Radar site registry (id, city, state, lat/lon, elevation).
///
/// Wraps `nexrad-model`'s WSR-88D registry so the app has one dependency surface for site data,
/// and folds in the TDWR, DWD and OPERA tables — everything that resolves a site id gets terminal,
/// German and European radars for free, instead of each call site remembering how many networks
/// there are.
pub mod sites {
    pub use nexrad_model::meta::registry::{nearest_site, sites, SiteEntry};

    /// Which feed a site id belongs to.
    ///
    /// The fourth network is where a chain of `is_x` booleans stops paying: every call site that
    /// wanted "not this one, not that one either" had to be edited again, and one of them was
    /// always missed. One `match` per question instead, and adding a network makes the compiler
    /// list the questions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Network {
        /// WSR-88D — the US network, and the only one with a Level 2 stream and a volume archive.
        Nexrad,
        /// Terminal Doppler Weather Radar, synthesized from Level 3 tilt products.
        Tdwr,
        /// Germany's network, assembled from `opendata.dwd.de` ODIM files.
        Dwd,
        /// Europe's, from EUMETNET's OpenRadarData bucket.
        Opera,
    }

    /// The network `id` belongs to. An id no table knows reads as [`Network::Nexrad`], which is
    /// what every path assumed before there was more than one network.
    pub fn network(id: &str) -> Network {
        if crate::tdwr::is_tdwr(id) {
            Network::Tdwr
        } else if crate::dwd::is_dwd(id) {
            Network::Dwd
        } else if crate::opera::is_opera(id) {
            Network::Opera
        } else {
            Network::Nexrad
        }
    }

    /// The site with this id, from any network (case-insensitive).
    pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
        nexrad_model::meta::registry::site_by_id(id)
            .or_else(|| crate::tdwr::site_by_id(id))
            .or_else(|| crate::dwd::site_by_id(id))
            .or_else(|| crate::opera::site_by_id(id))
    }

    /// Whether `id` is a WSR-88D — the only network with a Level 2 stream, a volume archive and
    /// Level 3 algorithm products. Every other network's volumes are assembled from ODIM files or
    /// synthesized from Level 3 tilts, so every archive/stream/algorithm path asks this instead of
    /// assuming the site it was handed is a NEXRAD.
    pub fn is_nexrad(id: &str) -> bool {
        network(id) == Network::Nexrad
    }

    /// Every site the app can fetch: WSR-88Ds first, then TDWRs, then DWD, then OPERA.
    pub fn all() -> impl Iterator<Item = &'static SiteEntry> {
        sites()
            .iter()
            .chain(crate::tdwr::SITES)
            .chain(crate::dwd::SITES)
            .chain(crate::opera::SITES)
    }
}
