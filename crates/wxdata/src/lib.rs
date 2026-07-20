//! Weather radar data acquisition and domain model for Hook Echo-WX.

pub mod afd;
pub mod alerts;
pub mod archive_warnings;
pub mod aviation;
pub mod dealias;
pub mod hrrr;
pub mod level2;
pub mod level3;
pub mod lsr;
pub mod metar;
pub mod live;
pub mod mrms;
pub mod obs;
pub mod overlay;
pub mod placefile;
pub mod probsevere;
pub mod sounding;
pub mod spc;
pub mod spotters;
pub mod tds;
pub mod torclimo;
pub mod tropical;
pub mod volume3d;
pub mod xsection;

/// NEXRAD site registry (id, city, state, lat/lon, elevation) for the ~319 US sites.
///
/// Re-exported from `nexrad-model` so the app has one dependency surface for site data.
pub use nexrad_model::meta::registry as sites;
