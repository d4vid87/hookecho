//! Weather radar data acquisition and domain model for Hook Echo-WX.

pub mod afd;
pub mod airnow;
pub mod alerts;
pub mod archive_warnings;
pub mod aviation;
pub mod contour;
pub mod dat;
pub mod dealias;
pub mod derived;
pub mod dotcams;
pub mod efield;
pub mod ero;
pub mod forecast;
pub mod fronts;
pub mod geocode;
pub mod glm;
pub mod global;
pub mod hrrr;
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
pub mod spotters;
pub mod stations;
pub mod task;
pub mod tds;
pub mod tdwr;
pub mod torclimo;
pub mod tropical;
pub mod tz;
pub mod verify;
pub mod volume3d;
pub mod webcams;
pub mod wfigs;
pub mod wssi;
pub mod xsection;

/// Radar site registry (id, city, state, lat/lon, elevation).
///
/// Wraps `nexrad-model`'s WSR-88D registry so the app has one dependency surface for site data,
/// and folds in the TDWR table — everything that resolves a site id gets terminal radars for
/// free, instead of each call site remembering there are two networks.
pub mod sites {
    pub use nexrad_model::meta::registry::{nearest_site, sites, SiteEntry};

    /// The site with this id, from either network (case-insensitive).
    pub fn site_by_id(id: &str) -> Option<&'static SiteEntry> {
        nexrad_model::meta::registry::site_by_id(id).or_else(|| crate::tdwr::site_by_id(id))
    }

    /// Every site in both networks: WSR-88Ds first, then TDWRs.
    pub fn all() -> impl Iterator<Item = &'static SiteEntry> {
        sites().iter().chain(crate::tdwr::SITES)
    }
}
