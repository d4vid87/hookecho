//! Weather radar data acquisition and domain model for Hook Echo-WX.

pub mod afd;
pub mod airnow;
pub mod alerts;
pub mod archive_warnings;
pub mod aviation;
pub mod banding;
pub mod clock;
pub mod contour;
pub mod dat;
pub mod dealias;
pub mod derived;
pub mod dualpol;
pub mod dwd;
pub mod dotcams;
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
pub mod mosaic;
pub mod mping;
pub mod mrms;
pub mod ndbc;
pub mod net;
pub mod nohrsc;
pub mod obs;
pub mod odim;
pub mod openmeteo;
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
pub mod synoptic;
pub mod task;
pub mod tds;
pub mod tfr;
pub mod tdwr;
pub mod torclimo;
pub mod towers;
pub mod tropical;
pub mod tz;
pub mod verify;
pub mod vtec;
pub mod volume3d;
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
/// and folds in the TDWR and DWD tables — everything that resolves a site id gets terminal and
/// German radars for free, instead of each call site remembering there are three networks.
pub mod sites {
    pub use nexrad_model::meta::registry::{nearest_site, sites, SiteEntry};

    /// The site with this id, from any network (case-insensitive).
    pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
        nexrad_model::meta::registry::site_by_id(id)
            .or_else(|| crate::tdwr::site_by_id(id))
            .or_else(|| crate::dwd::site_by_id(id))
    }

    /// Whether `id` is a WSR-88D — the only network with a Level 2 stream, a volume archive and
    /// Level 3 algorithm products. TDWR volumes are synthesized from Level 3 tilts and DWD's are
    /// assembled from ODIM files, so every archive/stream/algorithm path asks this instead of
    /// assuming the site it was handed is a NEXRAD.
    pub fn is_nexrad(id: &str) -> bool {
        !crate::tdwr::is_tdwr(id) && !crate::dwd::is_dwd(id)
    }

    /// Every site the app can fetch: WSR-88Ds first, then TDWRs, then Germany's DWD network.
    pub fn all() -> impl Iterator<Item = &'static SiteEntry> {
        sites().iter().chain(crate::tdwr::SITES).chain(crate::dwd::SITES)
    }
}
