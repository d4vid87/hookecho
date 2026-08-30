//! Async raster-tile fetching and visible-tile computation for the slippy map.
//!
//! The basemap source is a [`BasemapStyle`] (dark/light/satellite raster, or none). Switching
//! styles clears the pending/uploaded sets so the new source is refetched; the GPU tile cache
//! is cleared in the render layer via the callback's `clear_tiles` flag.

use crate::render::{mercator::Camera, PendingTile, TileId, VisibleTile};
use lru::LruCache;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Handle;

// Browser-prefixed so imagery hosts (e.g. Esri) that 403 bare library UAs still serve tiles,
// while still identifying the app.
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (compatible; hookecho/0.0; +github.com/d4vid87/hookecho)";

/// Which provider (if any) a style needs an API key for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// No key needed (built-in vector Dark/Light, USGS Satellite, None).
    Builtin,
    Mapbox,
    MapTiler,
}

/// Picker grouping for a [`BasemapStyle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Vector,
    Streets,
    Satellite,
    Topo,
    /// Rendered weather fields (national radar composites), not a map of the ground.
    Weather,
    Other,
}

impl Category {
    /// Groups in picker order.
    pub const ALL: [Category; 6] = [
        Category::Vector,
        Category::Streets,
        Category::Satellite,
        Category::Topo,
        Category::Weather,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Vector => "Vector",
            Category::Streets => "Streets",
            Category::Satellite => "Satellite",
            Category::Topo => "Terrain",
            Category::Weather => "Weather",
            Category::Other => "Other",
        }
    }
}

/// A selectable basemap under the radar. Dark/Light are the vector MVT basemap
/// (see [`crate::vector_tiles`]); Satellite is raster USGS imagery. The Mapbox*/MapTiler* styles
/// are provider raster tiles, available only when the matching Settings API key is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BasemapStyle {
    Dark,
    Light,
    Satellite,
    None,
    /// NOAA GOES-East GeoColor via NASA GIBS (near-real-time satellite).
    GoesEast,
    /// NOAA GOES-West GeoColor via NASA GIBS.
    GoesWest,
    /// GOES-East Band 13 clean longwave infrared (works at night).
    GoesEastIR,
    /// GOES-West Band 13 clean longwave infrared.
    GoesWestIR,
    /// GOES-East Air Mass RGB (jet/dry-air structure).
    GoesEastAirMass,
    GoesWestAirMass,
    /// GOES-East Dust RGB (blowing dust / haboobs).
    GoesEastDust,
    GoesWestDust,
    /// GOES-East Fire Temperature RGB (hot spots / wildfire).
    GoesEastFire,
    GoesWestFire,
    /// Himawari-9 Band 13 clean longwave infrared (the western Pacific and Australia).
    HimawariIR,
    /// Himawari-9 Air Mass RGB.
    HimawariAirMass,
    /// Himawari-9 Band 3 red visible, 1 km.
    HimawariVisible,
    /// GPM IMERG half-hourly precipitation rate: global, including the oceans and everywhere
    /// without a radar.
    ImergRate,
    /// DWD's German radar composite: the RV product, which is the analysis followed by a
    /// two-hour nowcast, in 5-minute steps.
    DwdRadarRV,
    /// DWD's German radar composite, analysis only (the WN product).
    DwdRadarWN,
    /// ECCC's North American 1-km radar composite, rain field.
    EcccRadarRain,
    /// ECCC's North American 1-km radar composite, snow field.
    EcccRadarSnow,
    MapboxStreets,
    MapboxSatellite,
    MapboxSatelliteStreets,
    MapboxOutdoors,
    MapboxDark,
    MapboxLight,
    MapboxNavDay,
    MapboxNavNight,
    MapTilerStreets,
    MapTilerSatellite,
    MapTilerOutdoor,
    MapTilerTopo,
    MapTilerBasic,
    MapTilerDatavizDark,
    // Keyless raster providers (no API key). Street/road emphasis noted where relevant.
    OsmStandard,
    OpenTopoMap,
    CartoPositron,
    CartoDarkMatter,
    CartoVoyager,
    EsriImagery,
    EsriStreets,
    EsriTopo,
    UsgsTopo,
    /// USGS aerial imagery with the topographic map drawn over it. The shipped default: keyless,
    /// so it works on a fresh install with no account anywhere, and it reads as terrain rather
    /// than as an abstraction, which is what a radar echo needs to sit on top of.
    #[default]
    UsgsImageryTopo,
    OsmHot,
    CyclOsm,
    /// Vector basemap, OSM Liberty look.
    VectorLiberty,
    /// Vector basemap, pale low-ink look.
    VectorBright,
    /// Vector basemap, near-monochrome look.
    VectorPositron,
    /// Vector basemap, night-drive look.
    VectorMidnight,
    EsriDarkGray,
    EsriLightGray,
    EsriNatGeo,
    EsriOcean,
    /// Esri World Imagery with our own vector roads/boundaries/labels drawn over it. Keyless, and
    /// the closest thing to the "hybrid" layer every commercial provider charges for.
    HybridSatellite,
    /// Follows the app theme: resolves to [`Self::Dark`] or [`Self::Light`]. Never rendered
    /// directly — see [`Self::resolve`].
    Auto,
    /// User-supplied `{z}/{x}/{y}` URL template from settings. Desktop and Android only; the web
    /// build's proxy is exact-host allowlisted, so an arbitrary host cannot be fetched there.
    CustomXyz,
}

impl BasemapStyle {
    /// Cycle order for the `z` hotkey; provider styles trail the built-ins.
    pub const ALL: [BasemapStyle; 59] = [
        BasemapStyle::Dark,
        BasemapStyle::Light,
        BasemapStyle::Satellite,
        BasemapStyle::None,
        BasemapStyle::OsmStandard,
        BasemapStyle::OpenTopoMap,
        BasemapStyle::CartoPositron,
        BasemapStyle::CartoDarkMatter,
        BasemapStyle::CartoVoyager,
        BasemapStyle::EsriImagery,
        BasemapStyle::EsriStreets,
        BasemapStyle::EsriTopo,
        BasemapStyle::UsgsTopo,
        BasemapStyle::UsgsImageryTopo,
        BasemapStyle::OsmHot,
        BasemapStyle::CyclOsm,
        BasemapStyle::VectorLiberty,
        BasemapStyle::VectorBright,
        BasemapStyle::VectorPositron,
        BasemapStyle::VectorMidnight,
        BasemapStyle::HybridSatellite,
        BasemapStyle::EsriDarkGray,
        BasemapStyle::EsriLightGray,
        BasemapStyle::EsriNatGeo,
        BasemapStyle::EsriOcean,
        BasemapStyle::Auto,
        BasemapStyle::CustomXyz,
        BasemapStyle::GoesEast,
        BasemapStyle::GoesWest,
        BasemapStyle::GoesEastIR,
        BasemapStyle::GoesWestIR,
        BasemapStyle::GoesEastAirMass,
        BasemapStyle::GoesWestAirMass,
        BasemapStyle::GoesEastDust,
        BasemapStyle::GoesWestDust,
        BasemapStyle::GoesEastFire,
        BasemapStyle::GoesWestFire,
        BasemapStyle::HimawariIR,
        BasemapStyle::HimawariAirMass,
        BasemapStyle::HimawariVisible,
        BasemapStyle::ImergRate,
        BasemapStyle::DwdRadarRV,
        BasemapStyle::DwdRadarWN,
        BasemapStyle::EcccRadarRain,
        BasemapStyle::EcccRadarSnow,
        BasemapStyle::MapboxStreets,
        BasemapStyle::MapboxSatellite,
        BasemapStyle::MapboxSatelliteStreets,
        BasemapStyle::MapboxOutdoors,
        BasemapStyle::MapboxDark,
        BasemapStyle::MapboxLight,
        BasemapStyle::MapboxNavDay,
        BasemapStyle::MapboxNavNight,
        BasemapStyle::MapTilerStreets,
        BasemapStyle::MapTilerSatellite,
        BasemapStyle::MapTilerOutdoor,
        BasemapStyle::MapTilerTopo,
        BasemapStyle::MapTilerBasic,
        BasemapStyle::MapTilerDatavizDark,
    ];

    /// The handful worth a one-tap chip on a phone: two vector styles, three ways of seeing the
    /// ground, roads, terrain, live satellite, and nothing at all. Everything else in [`Self::ALL`]
    /// is a variation on one of these and lives behind the picker's "All basemaps".
    pub const COMMON: [BasemapStyle; 8] = [
        // The shipped default leads: a phone's quick chips are the only basemap UI most people
        // will use, and a default they cannot get back to from there is not much of a default.
        BasemapStyle::UsgsImageryTopo,
        BasemapStyle::Dark,
        BasemapStyle::Light,
        BasemapStyle::EsriImagery,
        BasemapStyle::OsmStandard,
        BasemapStyle::OpenTopoMap,
        BasemapStyle::GoesEast,
        BasemapStyle::None,
    ];

    /// A chip-sized name. [`Self::label`] is the full one — "GOES-East (GeoColor)" wraps a phone
    /// chip onto two lines and pushes the row off the sheet.
    pub fn short_label(self) -> &'static str {
        match self {
            BasemapStyle::Satellite => "USGS",
            BasemapStyle::UsgsImageryTopo => "Imagery Topo",
            BasemapStyle::EsriImagery => "Satellite",
            BasemapStyle::HybridSatellite => "Hybrid",
            BasemapStyle::EsriDarkGray => "Dark Gray",
            BasemapStyle::EsriLightGray => "Light Gray",
            BasemapStyle::EsriNatGeo => "NatGeo",
            BasemapStyle::EsriOcean => "Ocean",
            BasemapStyle::CustomXyz => "Custom",
            BasemapStyle::OsmStandard => "Streets",
            BasemapStyle::OpenTopoMap => "Topo",
            BasemapStyle::GoesEast => "GOES-East",
            BasemapStyle::GoesWest => "GOES-West",
            _ => self.label(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BasemapStyle::Dark => "Dark",
            BasemapStyle::Light => "Light",
            BasemapStyle::Satellite => "USGS Imagery",
            BasemapStyle::None => "None",
            BasemapStyle::GoesEast => "GOES-East (GeoColor)",
            BasemapStyle::GoesWest => "GOES-West (GeoColor)",
            BasemapStyle::GoesEastIR => "GOES-East (infrared)",
            BasemapStyle::GoesWestIR => "GOES-West (infrared)",
            BasemapStyle::GoesEastAirMass => "GOES-East (air mass)",
            BasemapStyle::GoesWestAirMass => "GOES-West (air mass)",
            BasemapStyle::GoesEastDust => "GOES-East (dust)",
            BasemapStyle::GoesWestDust => "GOES-West (dust)",
            BasemapStyle::GoesEastFire => "GOES-East (fire temp)",
            BasemapStyle::GoesWestFire => "GOES-West (fire temp)",
            BasemapStyle::HimawariIR => "Himawari (infrared)",
            BasemapStyle::HimawariAirMass => "Himawari (air mass)",
            BasemapStyle::HimawariVisible => "Himawari (visible)",
            BasemapStyle::ImergRate => "IMERG precipitation rate",
            BasemapStyle::DwdRadarRV => "DWD radar (RV nowcast)",
            BasemapStyle::DwdRadarWN => "DWD radar (analysis)",
            BasemapStyle::EcccRadarRain => "ECCC radar (rain)",
            BasemapStyle::EcccRadarSnow => "ECCC radar (snow)",
            BasemapStyle::MapboxStreets => "Mapbox Streets",
            BasemapStyle::MapboxSatellite => "Mapbox Satellite",
            BasemapStyle::MapboxSatelliteStreets => "Mapbox Satellite Streets",
            BasemapStyle::MapboxOutdoors => "Mapbox Outdoors",
            BasemapStyle::MapboxDark => "Mapbox Dark",
            BasemapStyle::MapboxLight => "Mapbox Light",
            BasemapStyle::MapboxNavDay => "Mapbox Navigation (day)",
            BasemapStyle::MapboxNavNight => "Mapbox Navigation (night)",
            BasemapStyle::OsmStandard => "OpenStreetMap",
            BasemapStyle::OpenTopoMap => "OpenTopoMap",
            BasemapStyle::CartoPositron => "Carto Positron",
            BasemapStyle::CartoDarkMatter => "Carto Dark Matter",
            BasemapStyle::CartoVoyager => "Carto Voyager",
            BasemapStyle::EsriImagery => "Esri World Imagery",
            BasemapStyle::EsriStreets => "Esri Streets",
            BasemapStyle::EsriTopo => "Esri Topographic",
            BasemapStyle::UsgsTopo => "USGS Topo",
            BasemapStyle::UsgsImageryTopo => "USGS Imagery Topo",
            BasemapStyle::OsmHot => "OSM Humanitarian",
            BasemapStyle::CyclOsm => "CyclOSM",
            BasemapStyle::VectorLiberty => "Liberty",
            BasemapStyle::VectorBright => "Bright",
            BasemapStyle::VectorPositron => "Positron",
            BasemapStyle::VectorMidnight => "Midnight",
            BasemapStyle::EsriDarkGray => "Esri Dark Gray Canvas",
            BasemapStyle::EsriLightGray => "Esri Light Gray Canvas",
            BasemapStyle::EsriNatGeo => "Esri National Geographic",
            BasemapStyle::EsriOcean => "Esri Ocean",
            BasemapStyle::HybridSatellite => "Hybrid Satellite",
            BasemapStyle::Auto => "Auto (follow theme)",
            BasemapStyle::CustomXyz => "Custom (XYZ URL)",
            BasemapStyle::MapTilerStreets => "MapTiler Streets",
            BasemapStyle::MapTilerSatellite => "MapTiler Satellite",
            BasemapStyle::MapTilerOutdoor => "MapTiler Outdoor",
            BasemapStyle::MapTilerTopo => "MapTiler Topo",
            BasemapStyle::MapTilerBasic => "MapTiler Basic",
            BasemapStyle::MapTilerDatavizDark => "MapTiler Dataviz Dark",
        }
    }

    /// Command-line / settings slug for the `--basemap` argument.
    pub fn slug(self) -> &'static str {
        match self {
            BasemapStyle::Dark => "dark",
            BasemapStyle::Light => "light",
            BasemapStyle::Satellite => "satellite",
            BasemapStyle::None => "none",
            BasemapStyle::GoesEast => "goes-east",
            BasemapStyle::GoesWest => "goes-west",
            BasemapStyle::GoesEastIR => "goes-east-ir",
            BasemapStyle::GoesWestIR => "goes-west-ir",
            BasemapStyle::GoesEastAirMass => "goes-east-airmass",
            BasemapStyle::GoesWestAirMass => "goes-west-airmass",
            BasemapStyle::GoesEastDust => "goes-east-dust",
            BasemapStyle::GoesWestDust => "goes-west-dust",
            BasemapStyle::GoesEastFire => "goes-east-fire",
            BasemapStyle::GoesWestFire => "goes-west-fire",
            BasemapStyle::HimawariIR => "himawari-ir",
            BasemapStyle::HimawariAirMass => "himawari-air-mass",
            BasemapStyle::HimawariVisible => "himawari-visible",
            BasemapStyle::ImergRate => "imerg-rate",
            BasemapStyle::DwdRadarRV => "dwd-radar-rv",
            BasemapStyle::DwdRadarWN => "dwd-radar-wn",
            BasemapStyle::EcccRadarRain => "eccc-radar-rain",
            BasemapStyle::EcccRadarSnow => "eccc-radar-snow",
            BasemapStyle::MapboxStreets => "mapbox-streets",
            BasemapStyle::MapboxSatellite => "mapbox-satellite",
            BasemapStyle::MapboxSatelliteStreets => "mapbox-satellite-streets",
            BasemapStyle::MapboxOutdoors => "mapbox-outdoors",
            BasemapStyle::MapboxDark => "mapbox-dark",
            BasemapStyle::MapboxLight => "mapbox-light",
            BasemapStyle::MapboxNavDay => "mapbox-nav-day",
            BasemapStyle::MapboxNavNight => "mapbox-nav-night",
            BasemapStyle::OsmStandard => "osm",
            BasemapStyle::OpenTopoMap => "opentopo",
            BasemapStyle::CartoPositron => "carto-positron",
            BasemapStyle::CartoDarkMatter => "carto-dark",
            BasemapStyle::CartoVoyager => "carto-voyager",
            BasemapStyle::EsriImagery => "esri-imagery",
            BasemapStyle::EsriStreets => "esri-streets",
            BasemapStyle::EsriTopo => "esri-topo",
            BasemapStyle::UsgsTopo => "usgs-topo",
            BasemapStyle::UsgsImageryTopo => "usgs-imagery-topo",
            BasemapStyle::OsmHot => "osm-hot",
            BasemapStyle::CyclOsm => "cyclosm",
            BasemapStyle::VectorLiberty => "liberty",
            BasemapStyle::VectorBright => "bright",
            BasemapStyle::VectorPositron => "positron",
            BasemapStyle::VectorMidnight => "midnight",
            BasemapStyle::EsriDarkGray => "esri-dark-gray",
            BasemapStyle::EsriLightGray => "esri-light-gray",
            BasemapStyle::EsriNatGeo => "esri-natgeo",
            BasemapStyle::EsriOcean => "esri-ocean",
            BasemapStyle::HybridSatellite => "hybrid-satellite",
            BasemapStyle::Auto => "auto",
            BasemapStyle::CustomXyz => "custom",
            BasemapStyle::MapTilerStreets => "maptiler-streets",
            BasemapStyle::MapTilerSatellite => "maptiler-satellite",
            BasemapStyle::MapTilerOutdoor => "maptiler-outdoor",
            BasemapStyle::MapTilerTopo => "maptiler-topo",
            BasemapStyle::MapTilerBasic => "maptiler-basic",
            BasemapStyle::MapTilerDatavizDark => "maptiler-dataviz-dark",
        }
    }

    /// Resolve a `--basemap` slug (unknown -> None).
    pub fn from_slug(s: &str) -> BasemapStyle {
        Self::ALL
            .into_iter()
            .find(|st| st.slug() == s)
            .unwrap_or(BasemapStyle::None)
    }

    /// Which provider key this style requires.
    pub fn provider_kind(self) -> Provider {
        match self {
            BasemapStyle::MapboxStreets
            | BasemapStyle::MapboxSatellite
            | BasemapStyle::MapboxSatelliteStreets
            | BasemapStyle::MapboxOutdoors
            | BasemapStyle::MapboxDark
            | BasemapStyle::MapboxLight
            | BasemapStyle::MapboxNavDay
            | BasemapStyle::MapboxNavNight => Provider::Mapbox,
            BasemapStyle::MapTilerStreets
            | BasemapStyle::MapTilerSatellite
            | BasemapStyle::MapTilerOutdoor
            | BasemapStyle::MapTilerTopo
            | BasemapStyle::MapTilerBasic
            | BasemapStyle::MapTilerDatavizDark => Provider::MapTiler,
            _ => Provider::Builtin,
        }
    }

    /// Attribution line for the map corner.
    pub fn attribution(self) -> &'static str {
        match self.provider_kind() {
            Provider::Mapbox => "© Mapbox © OpenStreetMap",
            Provider::MapTiler => "© MapTiler © OpenStreetMap",
            Provider::Builtin => match self {
                BasemapStyle::Satellite
                | BasemapStyle::UsgsTopo
                | BasemapStyle::UsgsImageryTopo => "USGS The National Map",
                BasemapStyle::OsmStandard | BasemapStyle::OsmHot | BasemapStyle::CyclOsm => {
                    "© OpenStreetMap contributors"
                }
                BasemapStyle::OpenTopoMap => "© OpenTopoMap (CC-BY-SA) © OpenStreetMap",
                BasemapStyle::CartoPositron
                | BasemapStyle::CartoDarkMatter
                | BasemapStyle::CartoVoyager => "© CARTO © OpenStreetMap",
                BasemapStyle::EsriImagery | BasemapStyle::HybridSatellite => {
                    "© Esri, Maxar, Earthstar Geographics"
                }
                BasemapStyle::EsriStreets | BasemapStyle::EsriTopo => "© Esri © OpenStreetMap",
                BasemapStyle::EsriDarkGray | BasemapStyle::EsriLightGray => {
                    "© Esri © OpenStreetMap, HERE, Garmin"
                }
                BasemapStyle::EsriNatGeo => "© Esri, National Geographic",
                BasemapStyle::EsriOcean => "© Esri, GEBCO, NOAA",
                // The template is the user's; we cannot know whose data it serves.
                BasemapStyle::CustomXyz => "Custom tile source",
                BasemapStyle::DwdRadarRV | BasemapStyle::DwdRadarWN => {
                    "Radar data © Deutscher Wetterdienst (DL-DE/BY-2.0)"
                }
                BasemapStyle::EcccRadarRain | BasemapStyle::EcccRadarSnow => {
                    "Radar data © Environment and Climate Change Canada (Open Government Licence – Canada)"
                }
                BasemapStyle::HimawariIR
                | BasemapStyle::HimawariAirMass
                | BasemapStyle::HimawariVisible => "NASA GIBS · JMA Himawari",
                BasemapStyle::ImergRate => "NASA GIBS · NASA/JAXA GPM IMERG",
                _ if self.goes_layer().is_some() => "NASA GIBS · NOAA GOES",
                _ => "© OpenMapTiles © OpenStreetMap",
            },
        }
    }

    /// For a GIBS-backed style, its layer id + tile-matrix level (each layer serves a fixed max
    /// zoom). `None` for everything else.
    ///
    /// Named for GOES because that is all it held at first; it now carries Himawari and IMERG too,
    /// which reach GIBS through the same WMTS URL and the same DescribeDomains call and so need no
    /// code of their own. Every id and level below is read off the live
    /// `epsg3857/best/1.0.0/WMTSCapabilities.xml`, not guessed — a wrong level 404s at depth.
    pub(crate) fn goes_layer(self) -> Option<(&'static str, u8)> {
        match self {
            BasemapStyle::GoesEast => Some(("GOES-East_ABI_GeoColor", 7)),
            BasemapStyle::GoesWest => Some(("GOES-West_ABI_GeoColor", 7)),
            BasemapStyle::GoesEastIR => Some(("GOES-East_ABI_Band13_Clean_Infrared", 6)),
            BasemapStyle::GoesWestIR => Some(("GOES-West_ABI_Band13_Clean_Infrared", 6)),
            BasemapStyle::GoesEastAirMass => Some(("GOES-East_ABI_Air_Mass", 6)),
            BasemapStyle::GoesWestAirMass => Some(("GOES-West_ABI_Air_Mass", 6)),
            BasemapStyle::GoesEastDust => Some(("GOES-East_ABI_Dust", 7)),
            BasemapStyle::GoesWestDust => Some(("GOES-West_ABI_Dust", 7)),
            BasemapStyle::GoesEastFire => Some(("GOES-East_ABI_FireTemp", 7)),
            BasemapStyle::GoesWestFire => Some(("GOES-West_ABI_FireTemp", 7)),
            BasemapStyle::HimawariIR => Some(("Himawari_AHI_Band13_Clean_Infrared", 6)),
            BasemapStyle::HimawariAirMass => Some(("Himawari_AHI_Air_Mass", 6)),
            BasemapStyle::HimawariVisible => Some(("Himawari_AHI_Band3_Red_Visible_1km", 7)),
            BasemapStyle::ImergRate => Some(("IMERG_Precipitation_Rate_30min", 6)),
            _ => None,
        }
    }

    /// For a WMS-backed style, its `(endpoint, layer, step, lag)`. `None` for everything else.
    ///
    /// A WMS server renders an arbitrary bounding box rather than a fixed tile pyramid, so one
    /// helper ([`wms_url`]) turns an XYZ tile into a GetMap and every such layer is a row here.
    ///
    /// `step` and `lag` are minutes: the publishing interval a `TIME=` must land on, and how far
    /// behind the clock the newest published frame is. Both are per-publisher — DWD runs on 5
    /// minutes and ECCC on 6 — and both are measured against the live servers, not guessed.
    pub(crate) fn wms_layer(self) -> Option<(&'static str, &'static str, i64, i64)> {
        const DWD: &str = "https://maps.dwd.de/geoserver/dwd/wms";
        const GEOMET: &str = "https://geo.weather.gc.ca/geomet";
        match self {
            BasemapStyle::DwdRadarRV => Some((DWD, "Radar_rv_product_1x1km_ger", 5, 10)),
            BasemapStyle::DwdRadarWN => Some((DWD, "Radar_wn-analysis_1x1km_ger", 5, 10)),
            BasemapStyle::EcccRadarRain => Some((GEOMET, "RADAR_1KM_RRAI", 6, 6)),
            BasemapStyle::EcccRadarSnow => Some((GEOMET, "RADAR_1KM_RSNO", 6, 6)),
            _ => None,
        }
    }

    /// Does this style have a time dimension the frame bar can step through?
    pub fn timed(self) -> bool {
        self.goes_layer().is_some() || self.wms_layer().is_some()
    }

    /// Is this style a raster-tile source (as opposed to the vector MVT basemap or None)?
    /// Everything except the vector Dark/Light and None is a raster source.
    pub fn is_raster(self) -> bool {
        !matches!(
            self,
            BasemapStyle::Dark
                | BasemapStyle::Light
                | BasemapStyle::None
                | BasemapStyle::Auto
                | BasemapStyle::VectorLiberty
                | BasemapStyle::VectorBright
                | BasemapStyle::VectorPositron
                | BasemapStyle::VectorMidnight
        )
    }

    /// Max zoom the raster source serves; deeper views upscale rather than fetch 404s. GIBS GOES
    /// layers top out at their matrix level.
    ///
    /// Every value below was checked against the live endpoint over Dallas: one level past the
    /// number here either 404s or returns the provider's fixed "no data" placeholder (OpenTopoMap
    /// hands back the same 4343-byte image at 18, 19 and 20). Guessing high is not free — a 404
    /// leaves a hole until an ancestor tile happens to be resident, which is what made the USGS
    /// satellite basemap blank out on close zoom: it served nothing past 16 but was asked for 18.
    fn max_raster_z(self) -> u8 {
        if let Some((_, level)) = self.goes_layer() {
            return level;
        }
        // The composites are 1-km grids: past z12 the server is upscaling its own pixels, and it
        // is cheaper for us to do that from a tile we already hold.
        if self.wms_layer().is_some() {
            return 12;
        }
        match self {
            // USGS ArcGIS services (imagery and topo alike) stop at 16.
            BasemapStyle::Satellite | BasemapStyle::UsgsTopo | BasemapStyle::UsgsImageryTopo => 16,
            BasemapStyle::OpenTopoMap => 17,
            // Measured over Dallas: the Canvas and NatGeo services hand back a fixed 2521-byte
            // placeholder from 17 up, and the ocean base does the same from 11.
            BasemapStyle::EsriOcean => 10,
            BasemapStyle::EsriDarkGray | BasemapStyle::EsriLightGray | BasemapStyle::EsriNatGeo => {
                16
            }
            // World Imagery serves real tiles through 20 and placeholders at 21.
            BasemapStyle::HybridSatellite => 20,
            // We cannot probe the user's own server, so we trust the max zoom they configured;
            // overzoom covers an overshoot by stretching the deepest tile that did load.
            // ponytail: no validation of the user's max_z beyond the settings clamp.
            BasemapStyle::CustomXyz => 22,
            BasemapStyle::OsmHot => 18,
            BasemapStyle::OsmStandard | BasemapStyle::CyclOsm => 19,
            BasemapStyle::CartoPositron
            | BasemapStyle::CartoDarkMatter
            | BasemapStyle::CartoVoyager => 20,
            BasemapStyle::EsriImagery | BasemapStyle::EsriStreets | BasemapStyle::EsriTopo => 20,
            _ => 20, // Mapbox and MapTiler raster both serve past 20; the vector styles never get here.
        }
    }

    /// Whether this source serves a true double-resolution tile at the same grid position (the
    /// `@2x` suffix). Worth more than the retina zoom bias it replaces: same tile count, twice the
    /// pixels, and the labels are drawn for the higher density instead of being magnified.
    fn has_2x(self) -> bool {
        matches!(
            self,
            BasemapStyle::CartoPositron
                | BasemapStyle::CartoDarkMatter
                | BasemapStyle::CartoVoyager
        )
    }

    /// Whether this style's tiles are 512 px rather than 256. Mapbox and MapTiler both serve the
    /// same tile grid at either size, so one 512 tile replaces the four 256 tiles a high-DPI
    /// screen would otherwise fetch a level deeper — same pixels on screen, a quarter of the
    /// requests. Callers use this to drop the retina zoom bias.
    pub fn tiles_are_512(self) -> bool {
        matches!(self.provider_kind(), Provider::Mapbox | Provider::MapTiler)
    }

    /// Is this style selectable given which provider keys are set and whether the user has
    /// configured a custom tile template?
    pub fn available(self, mapbox_key: bool, maptiler_key: bool, custom: bool) -> bool {
        if self == BasemapStyle::CustomXyz {
            // Nothing to fetch without a template, and the web build cannot fetch an arbitrary
            // host at all: the proxy allowlist is exact-match, and a direct fetch is at the mercy
            // of whatever CORS headers the user's server happens to send.
            return custom && !cfg!(target_arch = "wasm32");
        }
        match self.provider_kind() {
            Provider::Mapbox => mapbox_key,
            Provider::MapTiler => maptiler_key,
            Provider::Builtin => true,
        }
    }

    /// Next *available* style in [`Self::ALL`] (wraps) — the `z`-cycle step.
    pub fn next(self, mapbox_key: bool, maptiler_key: bool, custom: bool) -> BasemapStyle {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        for step in 1..=Self::ALL.len() {
            let cand = Self::ALL[(i + step) % Self::ALL.len()];
            if cand.available(mapbox_key, maptiler_key, custom) {
                return cand;
            }
        }
        self
    }

    /// Which group this style belongs to in the picker.
    pub fn category(self) -> Category {
        // Satellite-derived, but what it shows is precipitation, so it belongs next to the radar
        // composites in the picker rather than next to the imagery.
        if self == BasemapStyle::ImergRate {
            return Category::Weather;
        }
        if self.goes_layer().is_some() {
            return Category::Satellite;
        }
        if self.wms_layer().is_some() {
            return Category::Weather;
        }
        match self {
            BasemapStyle::None | BasemapStyle::Auto | BasemapStyle::CustomXyz => Category::Other,
            BasemapStyle::Dark
            | BasemapStyle::Light
            | BasemapStyle::VectorLiberty
            | BasemapStyle::VectorBright
            | BasemapStyle::VectorPositron
            | BasemapStyle::VectorMidnight => Category::Vector,
            BasemapStyle::Satellite
            | BasemapStyle::EsriImagery
            | BasemapStyle::HybridSatellite
            | BasemapStyle::MapboxSatellite
            | BasemapStyle::MapboxSatelliteStreets
            | BasemapStyle::MapTilerSatellite => Category::Satellite,
            BasemapStyle::OpenTopoMap
            | BasemapStyle::UsgsTopo
            | BasemapStyle::UsgsImageryTopo
            | BasemapStyle::EsriTopo
            | BasemapStyle::MapboxOutdoors
            | BasemapStyle::MapTilerOutdoor
            | BasemapStyle::MapTilerTopo
            | BasemapStyle::EsriOcean => Category::Topo,
            _ => Category::Streets,
        }
    }

    /// URL of the one tile used as this style's picker thumbnail: z6 over the middle of CONUS,
    /// which is the view the app opens on. Fetched through the same per-style disk cache as any
    /// other tile, so opening the picker twice costs nothing.
    pub(crate) fn thumb_url(self, mapbox: &str, maptiler: &str, custom: &str) -> Option<String> {
        self.url(6, 14, 24, false, mapbox, maptiler, custom)
    }

    /// [`Self::Auto`] resolved against the current theme; every other style is itself.
    ///
    /// Call this at the point a style is about to be *rendered or fetched*, not where it is
    /// stored — the stored value has to stay `Auto` or it would stop following the theme.
    pub fn resolve(self, dark_theme: bool) -> BasemapStyle {
        match self {
            BasemapStyle::Auto if dark_theme => BasemapStyle::Dark,
            BasemapStyle::Auto => BasemapStyle::Light,
            other => other,
        }
    }

    /// Which vector palette this style tessellates with, or `None` if it draws no vector geometry
    /// (raster basemaps other than hybrid, and `None`). Labels are drawn over every basemap
    /// regardless — this is about the roads/fills.
    pub fn vector_palette(self) -> Option<crate::basemap_style::Palette> {
        match self {
            BasemapStyle::Dark => Some(crate::basemap_style::Palette::Dark),
            BasemapStyle::Light => Some(crate::basemap_style::Palette::Light),
            BasemapStyle::VectorLiberty => Some(crate::basemap_style::Palette::Liberty),
            BasemapStyle::VectorBright => Some(crate::basemap_style::Palette::Bright),
            BasemapStyle::VectorPositron => Some(crate::basemap_style::Palette::Positron),
            BasemapStyle::VectorMidnight => Some(crate::basemap_style::Palette::Midnight),
            BasemapStyle::HybridSatellite => Some(crate::basemap_style::Palette::HybridOverlay),
            _ => None,
        }
    }

    /// Stable small id for this style, used to key the GPU/fetch caches so panes showing
    /// different basemaps don't overwrite each other's tiles. Index into [`Self::ALL`].
    pub fn key(self) -> u8 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8
    }

    /// Per-style cache subdir so sources don't collide on disk. Keys never appear here.
    fn provider(self, retina: bool, custom: &str) -> String {
        // The user can repoint the custom slot at a different server; hashing the template keeps
        // the old server's tiles from being served for the new one.
        if self == BasemapStyle::CustomXyz {
            return format!("custom-{:08x}", template_hash(custom));
        }
        // 512-px tiles share the tile grid with the 256-px ones they replace, so a cache written
        // before the switch would keep serving blurry 256s from the same paths. Separate dir —
        // and `@2x` tiles get their own for the same reason.
        if self.tiles_are_512() {
            format!("{}-512", self.slug())
        } else if retina && self.has_2x() {
            format!("{}-2x", self.slug())
        } else {
            self.slug().to_string()
        }
    }

    /// Mapbox style-id / MapTiler map-id used in the tile URL.
    fn style_id(self) -> &'static str {
        match self {
            BasemapStyle::MapboxStreets => "streets-v12",
            BasemapStyle::MapboxSatellite => "satellite-v9",
            BasemapStyle::MapboxSatelliteStreets => "satellite-streets-v12",
            BasemapStyle::MapboxOutdoors => "outdoors-v12",
            BasemapStyle::MapboxDark => "dark-v11",
            BasemapStyle::MapboxLight => "light-v11",
            BasemapStyle::MapboxNavDay => "navigation-day-v1",
            BasemapStyle::MapboxNavNight => "navigation-night-v1",
            BasemapStyle::MapTilerStreets => "streets-v2",
            BasemapStyle::MapTilerSatellite => "satellite",
            BasemapStyle::MapTilerOutdoor => "outdoor-v2",
            BasemapStyle::MapTilerTopo => "topo-v2",
            BasemapStyle::MapTilerBasic => "basic-v2",
            BasemapStyle::MapTilerDatavizDark => "dataviz-dark",
            _ => "",
        }
    }

    /// Raster tile URL for `(z, x, y)`. Built-in Dark/Light are the vector MVT basemap and return
    /// `None` here. Provider styles inject the matching key (never logged/cached in a path).
    // Eight arguments because a URL needs all eight: the tile, whether to ask for @2x, and the
    // three user-supplied strings a style might interpolate. A struct here would be one field per
    // argument and one more thing to keep in sync.
    #[allow(clippy::too_many_arguments)]
    fn url(
        self,
        z: u8,
        x: u32,
        y: u32,
        retina: bool,
        mapbox_key: &str,
        maptiler_key: &str,
        custom: &str,
    ) -> Option<String> {
        // `@2x` on the sources that serve it; empty everywhere else, so the URL is unchanged.
        let hi = if retina && self.has_2x() { "@2x" } else { "" };
        match self.provider_kind() {
            Provider::Builtin => match self {
                // ArcGIS MapServer tiles (public). All use `{z}/{y}/{x}` order and serve JPEG.
                BasemapStyle::Satellite => Some(format!(
                    "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::UsgsTopo => Some(format!(
                    "https://basemap.nationalmap.gov/arcgis/rest/services/USGSTopo/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::UsgsImageryTopo => Some(format!(
                    "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryTopo/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::EsriImagery => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::EsriStreets => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::EsriTopo => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{z}/{y}/{x}"
                )),
                // Hybrid is World Imagery underneath; the roads and labels on top come from the
                // vector pipeline, not from a second raster fetch.
                BasemapStyle::HybridSatellite => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"
                )),
                // These four also answer on `services.arcgisonline.com`, but only `server.` is in
                // the proxy allowlist, so the web build 403s on that host. Same tiles either way.
                BasemapStyle::EsriDarkGray => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::EsriLightGray => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Light_Gray_Base/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::EsriNatGeo => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/NatGeo_World_Map/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::EsriOcean => Some(format!(
                    "https://server.arcgisonline.com/ArcGIS/rest/services/Ocean/World_Ocean_Base/MapServer/tile/{z}/{y}/{x}"
                )),
                BasemapStyle::CustomXyz => valid_xyz_template(custom)
                    .then(|| crate::vector_tiles::fill_template(custom, z, x, y)),
                // Standard XYZ `{z}/{x}/{y}.png` slippy tiles. Single subdomain shard where the
                // provider uses them (ponytail: rotate a-c only if throttled).
                BasemapStyle::OsmStandard => {
                    Some(format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png"))
                }
                BasemapStyle::OpenTopoMap => {
                    Some(format!("https://a.tile.opentopomap.org/{z}/{x}/{y}.png"))
                }
                BasemapStyle::CartoPositron => {
                    Some(format!("https://basemaps.cartocdn.com/light_all/{z}/{x}/{y}{hi}.png"))
                }
                BasemapStyle::CartoDarkMatter => {
                    Some(format!("https://basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{hi}.png"))
                }
                BasemapStyle::CartoVoyager => {
                    Some(format!("https://basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}{hi}.png"))
                }
                BasemapStyle::OsmHot => {
                    Some(format!("https://a.tile.openstreetmap.fr/hot/{z}/{x}/{y}.png"))
                }
                BasemapStyle::CyclOsm => Some(format!(
                    "https://a.tile-cyclosm.openstreetmap.fr/cyclosm/{z}/{x}/{y}.png"
                )),
                _ => match self.wms_layer() {
                    // The layer's own default time; the fetch loop appends `TIME=` for a
                    // selected frame, the same way it rewrites the GIBS time slot.
                    Some((base, layer, ..)) => Some(wms_url(base, layer, z, x, y, "")),
                    // NASA GIBS WMTS (web mercator), latest GOES imagery. GIBS uses `{z}/{y}/{x}`.
                    None => self.goes_layer().map(|(layer, level)| {
                        format!(
                            "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{layer}/default/default/GoogleMapsCompatible_Level{level}/{z}/{y}/{x}.png"
                        )
                    }),
                },
            },
            Provider::Mapbox => (!mapbox_key.is_empty()).then(|| {
                format!(
                    "https://api.mapbox.com/styles/v1/mapbox/{}/tiles/512/{z}/{x}/{y}?access_token={mapbox_key}",
                    self.style_id()
                )
            }),
            Provider::MapTiler => (!maptiler_key.is_empty()).then(|| {
                let ext = if self == BasemapStyle::MapTilerSatellite { "jpg" } else { "png" };
                format!(
                    "https://api.maptiler.com/maps/{}/{z}/{x}/{y}.{ext}?key={maptiler_key}",
                    self.style_id()
                )
            }),
        }
    }
}

/// Web-mercator metre bounds of tile `(z, x, y)`, as `(min_x, min_y, max_x, max_y)`.
fn tile_bbox_3857(z: u8, x: u32, y: u32) -> (f64, f64, f64, f64) {
    // Half the side of the square web mercator projects the world into.
    const HALF: f64 = 20_037_508.342_789_244;
    let span = 2.0 * HALF / f64::from(1u32 << z.min(30));
    let min_x = -HALF + f64::from(x) * span;
    let max_y = HALF - f64::from(y) * span;
    (min_x, max_y - span, min_x + span, max_y)
}

/// A WMS 1.3.0 GetMap URL rendering exactly the square that XYZ tile `(z, x, y)` covers.
///
/// `time` is an instant the server's time dimension accepts, or empty for the layer's default.
/// Servers reject an instant that is not on their published step, so the caller picks from
/// [`wms_frames`] rather than rounding here.
fn wms_url(base: &str, layer: &str, z: u8, x: u32, y: u32, time: &str) -> String {
    let (x0, y0, x1, y1) = tile_bbox_3857(z, x, y);
    // EPSG:3857 is an easting/northing CRS, so 1.3.0's axis-order rule leaves BBOX as x,y; the
    // swap that rule is famous for only bites on geographic CRSs like EPSG:4326.
    let t = if time.is_empty() {
        String::new()
    } else {
        format!("&TIME={time}")
    };
    format!(
        "{base}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&CRS=EPSG:3857\
&BBOX={x0:.3},{y0:.3},{x1:.3},{y1:.3}&WIDTH=256&HEIGHT=256&LAYERS={layer}\
&FORMAT=image/png&TRANSPARENT=TRUE{t}"
    )
}

/// Is this a tile-URL template we are willing to fetch?
///
/// Trust boundary: the string comes from the user's settings file, and whatever it points at gets
/// fetched with the app's HTTP client. `https` only — a plain-http template would silently
/// downgrade every tile request — and it has to actually be a slippy template, or every tile
/// would be the same URL hammered a screenful at a time.
pub fn valid_xyz_template(t: &str) -> bool {
    t.starts_with("https://") && t.contains("{z}") && t.contains("{x}") && t.contains("{y}")
}

/// Short stable hash of a custom template, for its cache directory name.
fn template_hash(t: &str) -> u32 {
    // ponytail: FNV-1a, not a crypto hash. This names a cache directory; a collision would serve
    // one custom source's tiles for another, which is why it is 32 bits and not 8.
    let mut h: u32 = 0x811c9dc5;
    for b in t.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// Integer tile ids covering `cam`'s view (zoom clamped to `max_z`) with their world rects.
/// Shared by raster (`max_z` 18) and vector (`max_z` 14) layers.
///
/// `zoom_bias` bumps the fetched tile detail level without changing the covered extent — used to
/// pull sharper raster tiles on high-DPI screens (a 256-px tile stretched over 4× physical pixels
/// looks blurry). The view geometry (`world_per_pixel`) still keys off the camera zoom, so only the
/// tile grid gets finer.
pub fn tile_cover(
    cam: &Camera,
    viewport_px: (f32, f32),
    max_z: u8,
    zoom_bias: f64,
) -> Vec<VisibleTile> {
    let z = (cam.zoom + zoom_bias).round().clamp(2.0, max_z as f64) as u8;
    let n = 1u32 << z;
    let nf = n as f64;
    let wpp = cam.world_per_pixel();
    let half_w = viewport_px.0 as f64 / 2.0 * wpp;
    let half_h = viewport_px.1 as f64 / 2.0 * wpp;
    let (cx, cy) = cam.center;
    let x0 = ((cx - half_w) * nf).floor() as i64;
    let x1 = ((cx + half_w) * nf).ceil() as i64;
    let y0 = (((cy - half_h) * nf).floor() as i64).max(0);
    let y1 = (((cy + half_h) * nf).ceil() as i64).min(n as i64);

    let mut out = Vec::new();
    for ty in y0..y1 {
        for tx in x0..x1 {
            let wrapped_x = tx.rem_euclid(n as i64) as u32;
            let id = (z, wrapped_x, ty as u32);
            // World rect uses the *unwrapped* tx so tiles tile seamlessly across the
            // antimeridian within one view.
            let wx0 = tx as f32 / nf as f32;
            let wy0 = ty as f32 / nf as f32;
            let wx1 = (tx + 1) as f32 / nf as f32;
            let wy1 = (ty + 1) as f32 / nf as f32;
            out.push(VisibleTile {
                id,
                world_min: [wx0, wy0],
                world_max: [wx1, wy1],
            });
        }
    }
    out
}

/// Parse the `<Domain>` time list from a GIBS DescribeDomains XML into sorted instants. The
/// domain is comma-separated `start/end/PT{n}M` (or `PT{n}H`) ranges; each is expanded to its
/// discrete steps. Returns at most `limit` most-recent instants.
pub fn parse_goes_domain(xml: &str, limit: usize) -> Vec<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, Utc};
    let Some(start) = xml.find("<Domain>") else {
        return Vec::new();
    };
    let Some(end) = xml[start..].find("</Domain>") else {
        return Vec::new();
    };
    let body = &xml[start + "<Domain>".len()..start + end];
    let mut out: Vec<DateTime<Utc>> = Vec::new();
    for range in body.split(',') {
        let parts: Vec<&str> = range.trim().split('/').collect();
        let (s, e, period) = match parts.as_slice() {
            [s, e, p] => (*s, *e, *p),
            [s] => (*s, *s, "PT10M"), // a lone instant
            _ => continue,
        };
        let (Some(s), Some(e)) = (parse_domain_instant(s), parse_domain_instant(e)) else {
            continue;
        };
        let step_min = parse_iso_minutes(period).unwrap_or(10).max(1);
        let mut t = s;
        while t <= e {
            out.push(t);
            t += chrono::Duration::minutes(step_min);
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.len() > limit {
        out.drain(0..out.len() - limit);
    }
    out
}

/// One bound of a domain range. GIBS writes most of them as full instants, but a layer whose
/// range opens at midnight is written as a bare date — IMERG's reads `2026-08-29/...T11:30:00Z` —
/// and parsing that as an instant fails, which silently dropped the whole range and left the layer
/// with no frames at all.
fn parse_domain_instant(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, NaiveDate, Utc};
    if let Ok(t) = s.parse::<DateTime<Utc>>() {
        return Some(t);
    }
    Some(s.parse::<NaiveDate>().ok()?.and_hms_opt(0, 0, 0)?.and_utc())
}

/// Minutes in an ISO8601 duration like `PT10M` or `PT1H` (only the forms GIBS uses).
fn parse_iso_minutes(p: &str) -> Option<i64> {
    let p = p.strip_prefix("PT")?;
    if let Some(m) = p.strip_suffix('M') {
        m.parse().ok()
    } else if let Some(h) = p.strip_suffix('H') {
        h.parse::<i64>().ok().map(|h| h * 60)
    } else {
        None
    }
}

/// Which GOES frame times to ask for, given the wall clock and the active pane's radar time.
///
/// Returns the archive hour the window covers (`None` while live) and the window itself. The hour
/// is the refetch key: scrubbing within it reuses the frames already loaded, and only crossing
/// into another hour asks GIBS again.
pub fn goes_window(
    now: chrono::DateTime<chrono::Utc>,
    radar: Option<chrono::DateTime<chrono::Utc>>,
) -> (
    Option<i64>,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
) {
    // Four hours back is well past the point where a pane is replaying rather than lagging.
    let hour = radar
        .filter(|t| now.signed_duration_since(*t) > chrono::Duration::hours(4))
        .map(|t| t.timestamp().div_euclid(3600));
    match hour {
        // Brackets the hour so stepping either way stays inside what was loaded.
        Some(h) => {
            let c = chrono::DateTime::from_timestamp(h * 3600, 0).unwrap_or(now);
            (
                hour,
                c - chrono::Duration::hours(1),
                c + chrono::Duration::hours(2),
            )
        }
        None => (None, now - chrono::Duration::hours(8), now),
    }
}

/// The frame times a WMS composite offers, oldest first, ending at the newest one `anchor` can
/// expect to be published.
///
/// ponytail: the grid is computed, not fetched. Both publishers run a fixed interval — DWD 5
/// minutes, ECCC 6 — and each says so in a GetCapabilities document whose only useful sentence is
/// that; DWD's is 850 KB, so downloading it every time the style changes would cost more than every
/// tile in the loop put together. The lags are measured (DWD served the 16:05 frame at 16:10 and not
/// 16:10; GeoMet served 16:30 by 16:33), and asking for one that has not landed yet costs a blank
/// tile, not an error. A layer that ever publishes irregularly needs the capabilities parse; none of
/// the ones here does.
fn wms_frames(
    anchor: chrono::DateTime<chrono::Utc>,
    limit: usize,
    step_min: i64,
    lag_min: i64,
) -> Vec<chrono::DateTime<chrono::Utc>> {
    let step = chrono::Duration::minutes(step_min);
    let newest = anchor - chrono::Duration::minutes(lag_min);
    // Down to the step boundary: an off-step instant is rejected outright.
    let newest = newest
        - chrono::Duration::seconds(newest.timestamp().rem_euclid(step_min * 60))
        - chrono::Duration::nanoseconds(i64::from(newest.timestamp_subsec_nanos()));
    (0..limit as i64)
        .map(|i| newest - step * (i as i32))
        .rev()
        .collect()
}

/// Fetch the available GOES frame times for `style` between `from` and `to` (best-effort; empty
/// on any failure). Uses the GIBS REST DescribeDomains endpoint.
///
/// The window is a parameter rather than "the last N hours" because the same call serves the live
/// loop and archive replay: GIBS keeps GeoColor about two weeks back and Band 13 several months,
/// so scrubbing the radar into last week's event can ask for that week's satellite frames.
pub async fn fetch_frame_times(
    client: &reqwest::Client,
    style: BasemapStyle,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    limit: usize,
) -> Vec<chrono::DateTime<chrono::Utc>> {
    if let Some((_, _, step, lag)) = style.wms_layer() {
        // `to` is the end of the window the caller wants, which is now for a live pane and the
        // end of the replayed hour for an archived one.
        return wms_frames(to, limit, step, lag);
    }
    let Some((layer, level)) = style.goes_layer() else {
        return Vec::new();
    };
    let from = from.format("%Y-%m-%dT%H:%M:%SZ");
    let to = to.format("%Y-%m-%dT%H:%M:%SZ");
    let url = format!(
        "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/1.0.0/{layer}/default/GoogleMapsCompatible_Level{level}/-180,-90,180,90/{from}--{to}.xml"
    );
    match client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(xml) => parse_goes_domain(&xml, limit),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

struct FetchedTile {
    id: TileId,
    style: u8,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// A picker thumbnail's state.
enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

/// A finished fetch, or the id of one that failed (so it can leave `requested` and be retried).
type TileResult = Result<FetchedTile, crate::render::TileKey>;

/// How many tile fetches may be in flight at once. A screenful is ~28 tiles, and firing all of
/// them at a CDN at once means 28 TLS handshakes competing for the same radio: the first tile
/// lands later than it would have with a queue behind it.
const MAX_INFLIGHT: usize = 6;

/// How long a failed tile is left alone before the next visibility pass retries it.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

pub struct TileManager {
    spawner: crate::rt::Spawner,
    client: reqwest::Client,
    tx: Sender<TileResult>,
    rx: Receiver<TileResult>,
    /// Repaint handle. Tile fetches finish on a worker thread; without this the frame that would
    /// show them waits for whatever wakes egui next — on Android an idle heartbeat up to 250 ms
    /// away, per tile.
    ctx: Option<egui::Context>,
    /// Fetches currently in flight, shared with the tasks so they can decrement on the way out.
    inflight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Tiles whose fetch failed, and when. Retried after [`RETRY_AFTER`].
    failed: std::collections::HashMap<crate::render::TileKey, wxdata::clock::Instant>,
    requested: HashSet<crate::render::TileKey>,
    /// Tiles believed to be live on the GPU, newest-touched first. This mirrors the renderer's
    /// tile map exactly: the CPU side decides what gets evicted and tells the renderer, so the
    /// two can never disagree about whether a tile still exists.
    uploaded: LruCache<crate::render::TileKey, ()>,
    /// Ids evicted by the last `touch_visible`, handed to the renderer to drop.
    evicted: Vec<crate::render::TileKey>,
    /// Tiles counted visible across all panes this frame; drives the eviction high-water mark.
    frame_visible: usize,
    cache_root: Option<std::path::PathBuf>,
    mapbox_key: String,
    maptiler_key: String,
    /// Selected GOES frame time (`None` = latest/`default`). Only affects GOES styles.
    goes_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Ask sources that serve them for `@2x` tiles (high-DPI screen, unmetered link).
    retina: bool,
    /// `{z}/{x}/{y}` template for [`BasemapStyle::CustomXyz`]. Empty until the user sets one.
    custom_template: String,
    /// Deepest zoom the custom source is configured to serve.
    custom_max_z: u8,
    /// Picker thumbnails, keyed by [`BasemapStyle::key`].
    thumbs: std::collections::HashMap<u8, Thumb>,
    thumb_tx: Sender<(u8, Option<FetchedTile>)>,
    thumb_rx: Receiver<(u8, Option<FetchedTile>)>,
}

impl TileManager {
    pub fn new(spawner: crate::rt::Spawner) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (thumb_tx, thumb_rx) = std::sync::mpsc::channel();
        let client =
            crate::platform::http_timeouts(reqwest::Client::builder().user_agent(USER_AGENT))
                .build()
                .expect("build reqwest client");
        let cache_root = crate::paths::cache_dir().map(|d| d.join("tiles"));
        if let Some(root) = cache_root.clone() {
            sweep_later(root, "tile cache", tile_cache_bytes());
        }
        Self {
            spawner,
            client,
            tx,
            rx,
            ctx: None,
            inflight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            failed: std::collections::HashMap::new(),
            requested: HashSet::new(),
            uploaded: LruCache::new(NonZeroUsize::new(RASTER_TILE_CACHE).unwrap()),
            evicted: Vec::new(),
            frame_visible: 0,
            custom_template: String::new(),
            custom_max_z: 19,
            thumbs: std::collections::HashMap::new(),
            thumb_tx,
            thumb_rx,
            cache_root,
            mapbox_key: String::new(),
            maptiler_key: String::new(),
            goes_time: None,
            retina: false,
        }
    }

    /// Select a GOES frame time (`None` = latest). Returns true if it changed (caller clears the
    /// GPU tile cache). Only meaningful for GOES styles.
    pub fn set_goes_time(&mut self, t: Option<chrono::DateTime<chrono::Utc>>) -> bool {
        if self.goes_time == t {
            return false;
        }
        self.goes_time = t;
        self.requested.clear();
        self.uploaded.clear();
        true
    }

    /// Turn `@2x` tiles on or off (high-DPI screen, and not on a metered link). Returns true if it
    /// changed, so the caller can clear the GPU cache: the old tiles are a different size.
    pub fn set_retina(&mut self, retina: bool) -> bool {
        if self.retina == retina {
            return false;
        }
        self.retina = retina;
        self.requested.clear();
        self.uploaded.clear();
        true
    }

    /// Whether `style` is being fetched at double resolution right now — the caller drops its
    /// retina zoom bias when this is true, since the extra pixels are already in the tile.
    pub fn is_retina(&self, style: BasemapStyle) -> bool {
        self.retina && style.has_2x()
    }

    /// Update the provider API keys (from Settings). Clears fetch state if a key changed so the
    /// active provider style refetches. Keys are held in memory only — never written to a path.
    pub fn set_keys(&mut self, mapbox: &str, maptiler: &str) {
        if self.mapbox_key != mapbox || self.maptiler_key != maptiler {
            self.mapbox_key = mapbox.to_string();
            self.maptiler_key = maptiler.to_string();
            self.requested.clear();
            self.uploaded.clear();
        }
    }

    /// This style's picker thumbnail, fetching it the first time it is asked for.
    ///
    /// One z6 tile over the middle of CONUS per style, through the same disk cache as any other
    /// tile. Returns `None` while it is in flight, if it failed, or if the style has no raster
    /// URL — the picker paints a palette swatch in all of those cases, so nothing ever waits on
    /// a network round trip to draw.
    pub fn thumb(
        &mut self,
        style: BasemapStyle,
        ctx: &egui::Context,
    ) -> Option<egui::TextureHandle> {
        while let Ok((key, fetched)) = self.thumb_rx.try_recv() {
            let state = match fetched {
                Some(f) => {
                    let img = egui::ColorImage::from_rgba_unmultiplied(
                        [f.width as usize, f.height as usize],
                        &f.rgba,
                    );
                    Thumb::Ready(ctx.load_texture(
                        format!("basemap-thumb-{key}"),
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                None => Thumb::Failed,
            };
            self.thumbs.insert(key, state);
        }
        let key = style.key();
        match self.thumbs.get(&key) {
            Some(Thumb::Ready(t)) => return Some(t.clone()),
            Some(_) => return None,
            None => {}
        }
        // A cellular link should not spend a screenful of tiles on decoration.
        if crate::platform::is_metered() {
            return None;
        }
        // Small separate budget: the picker opening must not stall the map's own tile fetches.
        // ponytail: flat 4, independent of MAX_INFLIGHT.
        if self
            .thumbs
            .values()
            .filter(|t| matches!(t, Thumb::Loading))
            .count()
            >= 4
        {
            return None;
        }
        let url = style.thumb_url(&self.mapbox_key, &self.maptiler_key, &self.custom_template)?;
        let path = self.cache_root.as_ref().map(|d| {
            d.join(style.provider(false, &self.custom_template))
                .join("default")
                .join("6/14/24")
        });
        self.thumbs.insert(key, Thumb::Loading);
        let client = self.client.clone();
        let tx = self.thumb_tx.clone();
        let ctx2 = self.ctx.clone();
        let blocking = self.spawner.clone();
        self.spawner.spawn(async move {
            let bytes = load_tile_bytes(&client, &url, path.as_deref()).await;
            blocking.spawn_blocking(move || {
                let decoded = bytes
                    .ok()
                    .and_then(|b| image::load_from_memory(&b).ok())
                    .map(|img| {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        FetchedTile {
                            id: (6, 14, 24),
                            style: key,
                            rgba: rgba.into_raw(),
                            width: w,
                            height: h,
                        }
                    });
                let _ = tx.send((key, decoded));
                if let Some(ctx) = ctx2 {
                    ctx.request_repaint();
                }
            });
        });
        None
    }

    /// Point the custom-XYZ slot at a template. Clears the caches on change, like the key setter
    /// above: the same `(z, x, y)` now means a different server's imagery.
    pub fn set_custom_max_z(&mut self, z: u8) {
        // Clamped rather than trusted: `tile_cover` builds a grid of `4^z` tiles, and a typo in
        // the settings file should not turn into an unbounded fetch loop.
        self.custom_max_z = z.clamp(1, 22);
    }

    pub fn set_custom_template(&mut self, template: &str) {
        if self.custom_template != template {
            self.custom_template = template.to_string();
            self.requested.clear();
            self.uploaded.clear();
        }
    }

    /// Integer tile ids covering the current view, and their world-space rects. `zoom_bias`
    /// pulls sharper tiles on high-DPI displays (see [`tile_cover`]).
    pub fn visible(
        &self,
        style: BasemapStyle,
        cam: &Camera,
        viewport_px: (f32, f32),
        zoom_bias: f64,
    ) -> Vec<VisibleTile> {
        tile_cover(cam, viewport_px, self.max_z(style), zoom_bias)
    }

    /// Deepest zoom to fetch for `style` — the measured provider cap, except for the custom slot
    /// where only the user knows.
    fn max_z(&self, style: BasemapStyle) -> u8 {
        if style == BasemapStyle::CustomXyz {
            self.custom_max_z
        } else {
            style.max_raster_z()
        }
    }

    /// Kick off fetches for any visible tiles not yet requested.
    pub fn request_missing(&mut self, style: BasemapStyle, visible: &[VisibleTile]) {
        use std::sync::atomic::Ordering;
        let skey = style.key();
        for v in visible {
            if self.requested.contains(&(skey, v.id)) {
                continue;
            }
            if self
                .failed
                .get(&(skey, v.id))
                .is_some_and(|t| t.elapsed() < RETRY_AFTER)
            {
                continue;
            }
            // Queue the rest for a later pass rather than dumping a whole screenful on the radio.
            if self.inflight.load(Ordering::Relaxed) >= MAX_INFLIGHT {
                break;
            }
            let (z, x, y) = v.id;
            let Some(mut url) = style.url(
                z,
                x,
                y,
                self.retina,
                &self.mapbox_key,
                &self.maptiler_key,
                &self.custom_template,
            ) else {
                continue;
            };
            // GOES frame time: rewrite the `default` time slot in the GIBS URL and tag the cache
            // dir so different frames don't collide. Latest (`None`) keeps `default`.
            let time_tag = match self.goes_time {
                Some(t) if style.timed() => {
                    let iso = t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
                    if style.wms_layer().is_some() {
                        url.push_str(&format!("&TIME={iso}"));
                    } else {
                        url = url.replace(
                            "/default/GoogleMapsCompatible",
                            &format!("/{iso}/GoogleMapsCompatible"),
                        );
                    }
                    t.format("%Y%m%dT%H%M").to_string()
                }
                _ => "default".to_string(),
            };
            self.requested.insert((skey, v.id));
            let path = self.cache_root.as_ref().map(|d| {
                d.join(style.provider(self.retina, &self.custom_template))
                    .join(&time_tag)
                    .join(format!("{z}/{x}/{y}"))
            });
            let client = self.client.clone();
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            let inflight = self.inflight.clone();
            let blocking = self.spawner.clone();
            self.inflight.fetch_add(1, Ordering::Relaxed);
            self.spawner.spawn(async move {
                let bytes = load_tile_bytes(&client, &url, path.as_deref()).await;
                // PNG/JPEG decode is pure CPU and would otherwise run on the async worker, where
                // it stalls every other fetch sharing that thread.
                blocking.spawn_blocking(move || {
                    let decoded = bytes
                        .ok()
                        .and_then(|b| image::load_from_memory(&b).ok())
                        .map(|img| {
                            let rgba = img.to_rgba8();
                            let (w, h) = rgba.dimensions();
                            FetchedTile {
                                id: (z, x, y),
                                style: skey,
                                rgba: rgba.into_raw(),
                                width: w,
                                height: h,
                            }
                        });
                    let _ = tx.send(decoded.ok_or((skey, (z, x, y))));
                    inflight.fetch_sub(1, Ordering::Relaxed);
                    if let Some(ctx) = ctx {
                        ctx.request_repaint();
                    }
                });
            });
        }
    }

    /// Max zoom `style` serves (for the chase-pack depth cap).
    pub fn max_pack_z(&self, style: BasemapStyle) -> u8 {
        self.max_z(style)
    }

    /// Whether `style` produces raster tiles a chase pack can pre-download (has a URL, isn't a
    /// time-tagged layer). Takes the style explicitly so it doesn't depend on which pane the
    /// tile manager last rendered.
    pub fn packable(&self, style: BasemapStyle) -> bool {
        !style.timed()
            && style
                .url(
                    0,
                    0,
                    0,
                    false,
                    &self.mapbox_key,
                    &self.maptiler_key,
                    &self.custom_template,
                )
                .is_some()
    }

    /// Build the `(url, cache_path)` jobs for an offline chase pack of the lon/lat bbox over
    /// `z_lo..=z_hi` in `style`. Empty for URL-less (Dark/Light/None) and time-tagged styles. Cache paths
    /// match the live read-through cache so pre-downloads are transparent.
    #[allow(clippy::too_many_arguments)] // scalar bbox mirrors pack_tile_count's tested signature
    pub fn pack_jobs(
        &self,
        style: BasemapStyle,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        z_lo: u8,
        z_hi: u8,
    ) -> Vec<PackJob> {
        let Some(root) = self.cache_root.as_ref() else {
            return Vec::new();
        };
        if style.timed() {
            return Vec::new();
        }
        let z_hi = z_hi.min(self.max_z(style));
        // Packs are always 1x: a pack is written once and read on whatever device opens it later.
        let dir = root
            .join(style.provider(false, &self.custom_template))
            .join("default");
        pack_tile_ids(min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
            .into_iter()
            .filter_map(|(z, x, y)| {
                let url = style.url(
                    z,
                    x,
                    y,
                    false,
                    &self.mapbox_key,
                    &self.maptiler_key,
                    &self.custom_template,
                )?;
                Some((url, dir.join(format!("{z}/{x}/{y}"))))
            })
            .collect()
    }

    /// Elevation-tile jobs for the same bbox, so a chase pack carries the DEM the beam-blockage
    /// overlay needs when the radio is off. One fixed zoom ([`crate::elevation::DEM_ZOOM`]), so
    /// this adds tens of tiles, not thousands — and it is independent of the basemap style, which
    /// is why it is its own call rather than a branch inside [`Self::pack_jobs`].
    pub fn dem_pack_jobs(
        &self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
    ) -> Vec<PackJob> {
        let z = crate::elevation::zoom();
        let Some(root) = self.cache_root.as_ref() else {
            return Vec::new();
        };
        pack_tile_ids(min_lon, min_lat, max_lon, max_lat, z, z)
            .into_iter()
            .map(|(_, x, y)| {
                (
                    crate::elevation::tile_url(x, y),
                    crate::elevation::tile_path(root, x, y),
                )
            })
            .collect()
    }

    /// Drain finished fetches into upload-ready tiles (each returned exactly once).
    pub fn drain_ready(&mut self) -> Vec<PendingTile> {
        let mut ready = Vec::new();
        while let Ok(t) = self.rx.try_recv() {
            let t = match t {
                Ok(t) => t,
                Err(id) => {
                    // Out of `requested` so the next visibility pass is the retry, and into
                    // `failed` so that pass isn't the very next frame.
                    self.requested.remove(&id);
                    self.failed.insert(id, wxdata::clock::Instant::now());
                    continue;
                }
            };
            let key = (t.style, t.id);
            self.failed.remove(&key);
            // `push` (not `put`) hands back whatever it evicted, so the renderer can free the
            // texture instead of leaking it.
            let evicted = self.uploaded.push(key, ());
            if let Some((id, _)) = evicted.filter(|(id, _)| *id != key) {
                self.requested.remove(&id);
                self.evicted.push(id);
            } else if evicted.is_some() {
                continue; // already resident
            }
            ready.push(PendingTile {
                id: t.id,
                style: t.style,
                rgba: t.rgba,
                width: t.width,
                height: t.height,
            });
        }
        ready
    }

    /// Repaint handle for the fetch tasks — set once at startup.
    pub fn set_ctx(&mut self, ctx: egui::Context) {
        self.ctx = Some(ctx);
    }

    /// Mark this frame's tiles as most-recently-used. Called once per pane, because panes can
    /// show different basemaps and each pane's tiles must survive the others' eviction pass.
    /// Eviction itself runs once per frame in [`Self::evict_excess`], after every pane has been
    /// counted — evicting mid-frame would drop tiles a later pane still needs.
    pub fn promote_visible(&mut self, style: BasemapStyle, visible: &[VisibleTile]) {
        let skey = style.key();
        for v in visible {
            self.uploaded.promote(&(skey, v.id));
        }
        self.frame_visible += visible.len();
    }

    /// Shrink the cache back to its resting size and return what fell out, so the renderer can
    /// free those textures. Run after the last pane's [`Self::promote_visible`].
    pub fn evict_excess(&mut self) -> Vec<crate::render::TileKey> {
        // Grow to fit a wide (or multi-pane) frame: the cap is a resting size, not a per-frame
        // limit. An evicted tile also drops out of `requested`, so revisiting that area re-fetches
        // it (from disk, usually) instead of leaving a black square.
        let want = RASTER_TILE_CACHE.max(self.frame_visible + 16);
        self.frame_visible = 0;
        // Pop down to size FIRST: shrinking an `LruCache` evicts silently, and a silently evicted
        // tile is a texture the renderer never hears about again.
        while self.uploaded.len() > want {
            if let Some((id, _)) = self.uploaded.pop_lru() {
                self.requested.remove(&id);
                self.evicted.push(id);
            }
        }
        if self.uploaded.cap().get() != want {
            self.uploaded.resize(NonZeroUsize::new(want).unwrap());
        }
        while self.uploaded.len() > want {
            if let Some((id, _)) = self.uploaded.pop_lru() {
                self.requested.remove(&id);
                self.evicted.push(id);
            }
        }
        std::mem::take(&mut self.evicted)
    }
}

/// Fetch and decode all `visible` tiles for `style` (used by the headless verify harness,
/// which has no async drain loop). Missing tiles are skipped.
pub async fn fetch_visible(
    client: &reqwest::Client,
    style: BasemapStyle,
    visible: &[VisibleTile],
    mapbox_key: &str,
    maptiler_key: &str,
) -> Vec<PendingTile> {
    let mut out = Vec::new();
    for v in visible {
        let (z, x, y) = v.id;
        let Some(url) = style.url(z, x, y, false, mapbox_key, maptiler_key, "") else {
            continue;
        };
        match load_tile_bytes(client, &url, None).await {
            Ok(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    out.push(PendingTile {
                        id: v.id,
                        style: style.key(),
                        rgba: rgba.into_raw(),
                        width: w,
                        height: h,
                    });
                }
                Err(e) => log::warn!("tile decode {url}: {e}"),
            },
            Err(e) => log::warn!("tile fetch {url}: {e}"),
        }
    }
    out
}

/// Read-through disk cache: return the cached PNG if present, else fetch and store it.
///
/// A corrupt/partial cache file just fails to decode upstream and gets re-fetched next view,
/// so no locking or temp-rename dance is needed. Bounded by the startup sweep (see [`sweep_later`]).
pub(crate) async fn load_tile_bytes(
    client: &reqwest::Client,
    url: &str,
    path: Option<&std::path::Path>,
) -> anyhow::Result<Vec<u8>> {
    // ponytail: sync std::fs in the async task — tiles are ~20KB, not worth the tokio `fs`
    // feature + spawn_blocking hops.
    if let Some(p) = path {
        if let Ok(bytes) = std::fs::read(p) {
            return Ok(bytes);
        }
    }
    // Browser builds cannot fetch most tile hosts directly — they send no CORS header — so the
    // request goes to the page's own `/proxy/{host}/...` instead, which also means one visitor's
    // tile is the next visitor's edge-cache hit. `fetch_url` leaves the keyed providers alone:
    // api.mapbox.com and api.maptiler.com answer CORS themselves, and proxying them would put a
    // user's API key in a shared cache. Native builds get the URL back unchanged.
    let resp = client
        .get(wxdata::net::fetch_url(url))
        .send()
        .await?
        .error_for_status()?;
    let bytes = resp.bytes().await?.to_vec();
    if let Some(p) = path {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(p, &bytes);
    }
    Ok(bytes)
}

/// How many uploaded raster tiles to keep. A 256x256 RGBA tile is ~256 KB on the GPU, so 512 is
/// ~134 MB on a desktop and 128 keeps a phone near 34 MB. In a browser — especially an iframe on
/// Safari, where a Retina raster_bias quadruples the tile count — a desktop budget gets the whole
/// device recycled out from under us, so wasm gets ~50 MB.
const RASTER_TILE_CACHE: usize = if cfg!(target_os = "android") {
    128
} else if cfg!(target_arch = "wasm32") {
    192
} else {
    512
};

/// Largest the on-disk tile cache may get. It grows by ~20 KB a tile and nothing ever removed
/// anything, so a few long sessions of panning could quietly fill a phone.
pub(crate) const DISK_CACHE_BYTES: u64 = if cfg!(target_os = "android") {
    150 * 1024 * 1024
} else {
    500 * 1024 * 1024
};

/// User overrides for the two disk caps, in bytes; 0 means "use the platform default".
///
/// Globals because the sweeps run from three places that construct before — and without — the
/// settings: the raster and vector tile managers, and the startup volume sweep. One `set` at
/// startup beats threading a cap through every constructor.
static TILE_CAP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static VOLUME_CAP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Apply the user's cap settings (in MB, 0 = platform default). Call once, before the managers.
pub(crate) fn set_cache_caps(tile_mb: u32, volume_mb: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    TILE_CAP.store(u64::from(tile_mb) * 1024 * 1024, Relaxed);
    VOLUME_CAP.store(u64::from(volume_mb) * 1024 * 1024, Relaxed);
}

/// Cap for each on-disk tile cache (raster and vector are swept separately, to this same figure).
pub(crate) fn tile_cache_bytes() -> u64 {
    match TILE_CAP.load(std::sync::atomic::Ordering::Relaxed) {
        0 => DISK_CACHE_BYTES,
        n => n,
    }
}

/// Cap for the on-disk radar-volume cache.
pub(crate) fn volume_cache_bytes() -> u64 {
    match VOLUME_CAP.load(std::sync::atomic::Ordering::Relaxed) {
        0 => VOLUME_CACHE_BYTES,
        n => n,
    }
}

/// Trim `root` to `cap` bytes, deleting oldest-touched files first.
///
/// Runs once at startup on its own thread: a cache sweep mid-session would race the fetch tasks
/// writing into it, and a file deleted out from under a read just gets re-fetched anyway. Chase
/// packs live under the tile root and are deliberately not spared — they are re-downloadable, and
/// a pack the user still cares about is a pack they have been looking at recently.
/// How long a cache sweep waits before it starts.
///
/// Four janitor threads used to walk four cache trees while the app was still opening its window,
/// reading its settings and asking for its first tiles — competing for the same disk with the
/// reads a launch is actually waiting on. Nothing here is urgent: these caps are tripwires
/// against a cache that grew for weeks.
#[cfg(not(target_arch = "wasm32"))]
fn janitor_delay() -> std::time::Duration {
    // Tests want the same thread, spawn and drain, not the wait.
    match cfg!(test) {
        true => std::time::Duration::from_millis(50),
        false => std::time::Duration::from_secs(20),
    }
}

/// One sweep job: the directory, what to call it in the log, and its cap.
#[cfg(not(target_arch = "wasm32"))]
struct SweepJob(std::path::PathBuf, &'static str, u64);

#[cfg(not(target_arch = "wasm32"))]
static JANITOR: std::sync::Mutex<(Vec<SweepJob>, bool)> = std::sync::Mutex::new((Vec::new(), false));

/// Queue a cache sweep for the shared janitor: one thread, started once, running every queued
/// sweep in turn after [`janitor_delay`].
///
/// ponytail: a `Vec` and a `bool` rather than a channel — the queue is four entries deep at
/// startup and empty forever after.
pub(crate) fn sweep_later(root: std::path::PathBuf, label: &'static str, cap: u64) {
    // The browser has no cache directory to sweep and no threads to sweep it with; the callers
    // are the same on both targets so the caps have one definition, not two.
    #[cfg(target_arch = "wasm32")]
    let _ = (root, label, cap);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Ok(mut j) = JANITOR.lock() else { return };
        j.0.push(SweepJob(root, label, cap));
        if j.1 {
            return;
        }
        j.1 = true;
        std::thread::spawn(|| {
            std::thread::sleep(janitor_delay());
            loop {
                // The flag is cleared under the same lock the queue is popped from, so a sweep
                // queued after this thread gives up starts a new one rather than being forgotten.
                let job = {
                    let Ok(mut j) = JANITOR.lock() else { return };
                    match j.0.pop() {
                        Some(job) => job,
                        None => {
                            j.1 = false;
                            return;
                        }
                    }
                };
                sweep_cache_dir(&job.0, job.1, job.2);
            }
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sweep_cache_dir(root: &std::path::Path, label: &str, cap: u64) {
    fn walk(
        dir: &std::path::Path,
        out: &mut Vec<(std::time::SystemTime, u64, std::path::PathBuf)>,
    ) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                walk(&e.path(), out);
            } else {
                let t = md.modified().unwrap_or(std::time::UNIX_EPOCH);
                out.push((t, md.len(), e.path()));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    let total: u64 = files.iter().map(|(_, n, _)| *n).sum();
    if total <= cap {
        return;
    }
    files.sort_by_key(|(t, _, _)| *t); // oldest first
    let mut freed = 0u64;
    for (_, n, path) in files {
        if total - freed <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            freed += n;
        }
    }
    log::info!(
        "{label}: {:.0} MB over cap, freed {:.0} MB",
        (total - cap) as f64 / 1e6,
        freed as f64 / 1e6
    );
}

/// Default cap for the on-disk volume cache. An Archive II volume is 3-8 MB, so the desktop cap
/// holds a few hundred — an afternoon of scrubbing one event — and the phone cap a couple of dozen.
/// The Storage tab can override it; see [`volume_cache_bytes`].
pub(crate) const VOLUME_CACHE_BYTES: u64 = if cfg!(target_os = "android") {
    300 * 1024 * 1024
} else {
    2 * 1024 * 1024 * 1024
};

/// Largest the small caches — zone geometry, RAOB soundings, server snapshots — may get between
/// them. Each is a few MB in normal use; the cap is a tripwire for a cache that has started
/// growing without bound, not a budget anyone is meant to run into.
pub(crate) const SMALL_CACHE_BYTES: u64 = 50 * 1024 * 1024;

/// A chase-pack download job: the tile URL and the disk path to cache it at.
pub type PackJob = (String, std::path::PathBuf);

/// Number of tiles a chase-pack download covers over zoom `z_lo..=z_hi` for the lon/lat bbox.
/// Pure — the drawer's Map section calls it each frame for a live size estimate. `≈ tiles × 25 KB`.
pub fn pack_tile_count(
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    z_lo: u8,
    z_hi: u8,
) -> u64 {
    let z_hi = z_hi.min(22); // guard the shifts below — a junk CLI zmax must not overflow 1<<z
    let a = crate::render::mercator::lonlat_to_world(min_lon, min_lat);
    let b = crate::render::mercator::lonlat_to_world(max_lon, max_lat);
    let (wxmin, wxmax) = (a.0.min(b.0), a.0.max(b.0));
    let (wymin, wymax) = (a.1.min(b.1), a.1.max(b.1));
    let mut count = 0u64;
    for z in z_lo..=z_hi {
        let n = 1u64 << z;
        let nf = n as f64;
        let tx0 = (wxmin * nf).floor() as i64;
        let tx1 = (wxmax * nf).ceil() as i64;
        let ty0 = ((wymin * nf).floor() as i64).max(0);
        let ty1 = ((wymax * nf).ceil() as i64).min(n as i64);
        count += (tx1 - tx0).max(0) as u64 * (ty1 - ty0).max(0) as u64;
    }
    count
}

/// The `(z, x, y)` tile ids covering the lon/lat bbox over `z_lo..=z_hi` (x wrapped into range).
/// Shared by the raster and vector chase-pack job builders.
pub(crate) fn pack_tile_ids(
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    z_lo: u8,
    z_hi: u8,
) -> Vec<TileId> {
    let z_hi = z_hi.min(22); // guard 1u32 << z (see pack_tile_count)
    let a = crate::render::mercator::lonlat_to_world(min_lon, min_lat);
    let b = crate::render::mercator::lonlat_to_world(max_lon, max_lat);
    let (wxmin, wxmax) = (a.0.min(b.0), a.0.max(b.0));
    let (wymin, wymax) = (a.1.min(b.1), a.1.max(b.1));
    let mut ids = Vec::new();
    for z in z_lo..=z_hi {
        let n = 1u32 << z;
        let nf = n as f64;
        let tx0 = (wxmin * nf).floor() as i64;
        let tx1 = (wxmax * nf).ceil() as i64;
        let ty0 = ((wymin * nf).floor() as i64).max(0);
        let ty1 = ((wymax * nf).ceil() as i64).min(n as i64);
        for ty in ty0..ty1 {
            for tx in tx0..tx1 {
                ids.push((z, tx.rem_euclid(n as i64) as u32, ty as u32));
            }
        }
    }
    ids
}

/// Download `jobs` into the disk tile cache with 4 fixed workers, reporting each tile's outcome
/// on `tx` as `(ok, downloaded_bytes)` — `bytes == 0` means it was already cached (skipped). Stops
/// early when `cancel` is set. // ponytail: 4 fixed workers pulling a shared queue, no semaphore crate.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_pack_download(
    rt: &Handle,
    jobs: Vec<PackJob>,
    cancel: Arc<AtomicBool>,
    tx: Sender<(bool, u64)>,
) {
    let queue = Arc::new(Mutex::new(jobs.into_iter()));
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT) // browser-ish UA: imagery hosts 403 bare UAs (matches live tile traffic)
        .build()
        .expect("build reqwest client");
    for _ in 0..4 {
        let queue = queue.clone();
        let cancel = cancel.clone();
        let tx = tx.clone();
        let client = client.clone();
        rt.spawn(async move {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let job = queue.lock().unwrap().next();
                let Some((url, path)) = job else { break };
                if path.exists() {
                    let _ = tx.send((true, 0));
                    continue;
                }
                match load_tile_bytes(&client, &url, Some(&path)).await {
                    Ok(bytes) => {
                        let _ = tx.send((true, bytes.len() as u64));
                    }
                    Err(_) => {
                        let _ = tx.send((false, 0));
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped default. A fresh install with no settings file and no API key anywhere lands
    /// here, so it has to be keyless, reachable on the web build, and offered on a phone.
    #[test]
    fn the_default_basemap_ships_usable() {
        let d = BasemapStyle::default();
        assert_eq!(d, BasemapStyle::UsgsImageryTopo);
        assert_eq!(d.slug(), "usgs-imagery-topo");
        // Keyless: `available` says yes with no Mapbox key, no MapTiler key, no custom template.
        assert!(d.available(false, false, false));
        // It draws something. `url` returning None would mean the vector path, which this is not.
        let url = d
            .url(6, 14, 24, false, "", "", "")
            .expect("the default basemap must resolve to a tile URL");
        // The host the browser build fetches through must be one the proxy will pass.
        assert!(url.starts_with("https://basemap.nationalmap.gov/"), "{url}");
        // Reachable from the phone's quick chips, and named short enough to fit one.
        assert!(BasemapStyle::COMMON.contains(&d));
        assert!(d.short_label().len() <= 14, "{}", d.short_label());
    }

    /// Panes render their own basemap, so the caches are keyed by style as well as tile id. Two
    /// styles must never collapse onto the same key — that is what put one pane's imagery under
    /// another pane's radar.
    #[test]
    fn style_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for s in BasemapStyle::ALL {
            assert!(seen.insert(s.key()), "duplicate style key for {s:?}");
        }
        assert_eq!(seen.len(), BasemapStyle::ALL.len());
    }

    #[test]
    fn cache_caps_fall_back_to_the_platform_default_at_zero() {
        assert_eq!(tile_cache_bytes(), DISK_CACHE_BYTES);
        assert_eq!(volume_cache_bytes(), VOLUME_CACHE_BYTES);
        set_cache_caps(64, 128);
        assert_eq!(tile_cache_bytes(), 64 * 1024 * 1024);
        assert_eq!(volume_cache_bytes(), 128 * 1024 * 1024);
        set_cache_caps(0, 0);
        assert_eq!(tile_cache_bytes(), DISK_CACHE_BYTES);
    }

    #[test]
    fn pack_tile_count_covers_expected() {
        // The whole world at z0 is a single tile.
        assert_eq!(pack_tile_count(-179.9, -85.0, 179.9, 85.0, 0, 0), 1);
        // A sub-tile-sized box still needs at least one tile at each level.
        assert_eq!(pack_tile_count(-97.52, 35.30, -97.50, 35.32, 6, 6), 1);
        // Summing a range equals summing its levels.
        let a = pack_tile_count(-97.52, 35.30, -97.50, 35.32, 5, 5);
        let b = pack_tile_count(-97.52, 35.30, -97.50, 35.32, 6, 6);
        assert_eq!(pack_tile_count(-97.52, 35.30, -97.50, 35.32, 5, 6), a + b);
        // id list length matches the count.
        assert_eq!(
            pack_tile_ids(-99.0, 34.0, -96.0, 36.0, 7, 9).len() as u64,
            pack_tile_count(-99.0, 34.0, -96.0, 36.0, 7, 9)
        );
    }

    /// The janitor really does run: one thread, spawned once, sweeping every queued directory
    /// after its delay. Without this the sweeps could stop happening and nothing would say so —
    /// `sweep_cache_dir` is silent when a cache is under its cap.
    #[test]
    fn the_janitor_thread_sweeps_what_is_queued() {
        let root = std::env::temp_dir().join(format!("hookecho-janitor-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let file = root.join("big.bin");
        std::fs::write(&file, vec![0u8; 4096]).expect("write");
        super::sweep_later(root.clone(), "janitor test", 1024);
        for _ in 0..100 {
            if !file.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!file.exists(), "the queued sweep never ran");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_trims_oldest_first() {
        let root = std::env::temp_dir().join(format!("hookecho-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        // Three 1 KB files written in order, one nested, so the walk has to recurse. Filesystem
        // mtime resolution is finer than the write loop, so "a" really is the oldest.
        let files = [root.join("a"), root.join("sub/b"), root.join("c")];
        for p in &files {
            std::fs::write(p, vec![0u8; 1024]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Cap of 2 KB has to evict exactly one file, and it must be the oldest.
        sweep_cache_dir(&root, "test", 2048);
        assert!(!files[0].exists(), "oldest goes first");
        assert!(files[1].exists() && files[2].exists(), "the rest stay");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parses_goes_domain_ranges() {
        let xml = "<Domains><DimensionDomain><ows:Identifier>time</ows:Identifier>\
            <Domain>2026-07-18T22:00:00Z/2026-07-18T22:20:00Z/PT10M,2026-07-18T23:00:00Z/2026-07-18T23:00:00Z/PT10M</Domain>\
            </DimensionDomain></Domains>";
        let times = parse_goes_domain(xml, 100);
        // 22:00, 22:10, 22:20, 23:00 = 4 instants, sorted ascending.
        assert_eq!(times.len(), 4);
        assert_eq!(times.last().unwrap().format("%H:%M").to_string(), "23:00");
        assert_eq!(times[0].format("%H:%M").to_string(), "22:00");
    }

    /// IMERG's range opens on a bare date rather than an instant, and a parse that only accepts
    /// instants drops the whole range — leaving the layer with an empty frame bar instead of a
    /// visible error.
    #[test]
    fn a_domain_range_may_open_on_a_bare_date() {
        let xml = "<Domain>2026-08-29/2026-08-29T01:00:00Z/PT30M</Domain>";
        let times = parse_goes_domain(xml, 100);
        assert_eq!(times.len(), 3, "00:00, 00:30, 01:00");
        assert_eq!(
            times[0].format("%Y-%m-%dT%H:%M").to_string(),
            "2026-08-29T00:00"
        );
    }

    /// The non-GOES GIBS layers ride the GOES bridge unchanged, so what has to hold is that each
    /// resolves to a real layer id and credits the agency that flies the satellite.
    #[test]
    fn the_gibs_table_reaches_past_goes() {
        for (style, layer, level, credit) in [
            (
                BasemapStyle::HimawariIR,
                "Himawari_AHI_Band13_Clean_Infrared",
                6,
                "Himawari",
            ),
            (
                BasemapStyle::HimawariVisible,
                "Himawari_AHI_Band3_Red_Visible_1km",
                7,
                "Himawari",
            ),
            (
                BasemapStyle::ImergRate,
                "IMERG_Precipitation_Rate_30min",
                6,
                "IMERG",
            ),
        ] {
            assert_eq!(style.goes_layer(), Some((layer, level)), "{style:?}");
            assert!(style.timed(), "{style:?} has a time dimension");
            let url = style
                .url(4, 3, 6, false, "", "", "")
                .expect("a GIBS style resolves");
            assert!(url.contains(&format!("/{layer}/default/default/")), "{url}");
            assert!(
                url.contains(&format!("GoogleMapsCompatible_Level{level}/4/6/3")),
                "{url}"
            );
            assert!(
                style.attribution().contains(credit),
                "{style:?}: {}",
                style.attribution()
            );
        }
        // Precipitation sits with the radar composites, not with the imagery.
        assert_eq!(BasemapStyle::ImergRate.category(), Category::Weather);
        assert_eq!(BasemapStyle::HimawariIR.category(), Category::Satellite);
    }

    #[test]
    fn goes_domain_limit_keeps_most_recent() {
        let xml = "<Domain>2026-07-18T00:00:00Z/2026-07-18T01:00:00Z/PT10M</Domain>"; // 7 instants
        let times = parse_goes_domain(xml, 3);
        assert_eq!(times.len(), 3);
        assert_eq!(times.last().unwrap().format("%H:%M").to_string(), "01:00");
    }

    #[test]
    fn slug_roundtrips_for_all_styles() {
        for s in BasemapStyle::ALL {
            assert_eq!(BasemapStyle::from_slug(s.slug()), s, "slug roundtrip {s:?}");
        }
    }

    #[test]
    fn keyless_rasters_have_urls() {
        // The new keyless providers must produce a URL with no API key set.
        let keyless = [
            BasemapStyle::OsmStandard,
            BasemapStyle::OpenTopoMap,
            BasemapStyle::CartoPositron,
            BasemapStyle::CartoDarkMatter,
            BasemapStyle::CartoVoyager,
            BasemapStyle::EsriImagery,
            BasemapStyle::EsriStreets,
            BasemapStyle::EsriTopo,
            BasemapStyle::UsgsTopo,
            BasemapStyle::UsgsImageryTopo,
            BasemapStyle::OsmHot,
            BasemapStyle::CyclOsm,
            BasemapStyle::EsriDarkGray,
            BasemapStyle::EsriLightGray,
            BasemapStyle::EsriNatGeo,
            BasemapStyle::EsriOcean,
            BasemapStyle::HybridSatellite,
        ];
        for s in keyless {
            assert!(s.is_raster(), "{s:?} should be raster");
            assert!(
                s.url(6, 15, 25, false, "", "", "").is_some(),
                "{s:?} should have a keyless URL"
            );
        }
        // ArcGIS services use {z}/{y}/{x}: y before x in the path.
        let esri = BasemapStyle::EsriImagery
            .url(6, 15, 25, false, "", "", "")
            .unwrap();
        assert!(esri.ends_with("/6/25/15"), "Esri y/x order: {esri}");
        // Standard slippy tiles use {z}/{x}/{y}.
        let osm = BasemapStyle::OsmStandard
            .url(6, 15, 25, false, "", "", "")
            .unwrap();
        assert!(osm.ends_with("/6/15/25.png"), "OSM x/y order: {osm}");
        // Mapbox nav styles stay key-gated.
        assert!(BasemapStyle::MapboxNavDay
            .url(6, 15, 25, false, "", "", "")
            .is_none());
        assert!(BasemapStyle::MapboxNavDay
            .url(6, 15, 25, false, "k", "", "")
            .is_some());
    }

    /// Retina asks for the `@2x` tile only where the provider serves one, and those tiles get
    /// their own cache directory so a 1x and a 2x tile never overwrite each other on disk.
    #[test]
    fn retina_only_changes_the_sources_that_serve_2x() {
        let carto = BasemapStyle::CartoDarkMatter;
        assert!(carto
            .url(6, 15, 25, true, "", "", "")
            .unwrap()
            .ends_with("@2x.png"));
        assert!(carto
            .url(6, 15, 25, false, "", "", "")
            .unwrap()
            .ends_with("/25.png"));
        assert_ne!(carto.provider(true, ""), carto.provider(false, ""));
        // No `@2x` upstream: the URL and the cache path are identical either way.
        let osm = BasemapStyle::OsmStandard;
        assert_eq!(
            osm.url(6, 15, 25, true, "", "", ""),
            osm.url(6, 15, 25, false, "", "", "")
        );
        assert_eq!(osm.provider(true, ""), osm.provider(false, ""));
    }

    /// The USGS satellite basemap serves nothing past zoom 16; asking for 17 got a 404 and left a
    /// hole in the map, which is what "blurry, then blank, when you zoom in" turned out to be.
    #[test]
    fn max_zoom_matches_what_the_providers_actually_serve() {
        assert_eq!(BasemapStyle::Satellite.max_raster_z(), 16);
        assert_eq!(BasemapStyle::OpenTopoMap.max_raster_z(), 17);
        assert_eq!(BasemapStyle::OsmStandard.max_raster_z(), 19);
        assert_eq!(BasemapStyle::CartoDarkMatter.max_raster_z(), 20);
        // GOES layers keep their own matrix level, whatever the table above says.
        assert_eq!(
            BasemapStyle::GoesEast.max_raster_z(),
            BasemapStyle::GoesEast.goes_layer().unwrap().1
        );
    }

    /// `Auto` is a marker, not something the tile layer should ever be asked to fetch.
    #[test]
    fn auto_resolves_to_a_real_style_and_nothing_else_moves() {
        assert_eq!(BasemapStyle::Auto.resolve(true), BasemapStyle::Dark);
        assert_eq!(BasemapStyle::Auto.resolve(false), BasemapStyle::Light);
        assert!(!BasemapStyle::Auto.is_raster());
        for s in BasemapStyle::ALL {
            if s != BasemapStyle::Auto {
                assert_eq!(s.resolve(true), s, "{s:?} should not follow the theme");
                assert_eq!(s.resolve(false), s, "{s:?} should not follow the theme");
            }
        }
    }

    /// Hybrid satellite is the one style that is raster *and* draws vector geometry.
    #[test]
    fn only_hybrid_is_both_raster_and_vector() {
        for s in BasemapStyle::ALL {
            let both = s.is_raster() && s.vector_palette().is_some();
            assert_eq!(
                both,
                s == BasemapStyle::HybridSatellite,
                "{s:?} raster+vector should only be hybrid"
            );
        }
        assert_eq!(
            BasemapStyle::HybridSatellite.vector_palette(),
            Some(crate::basemap_style::Palette::HybridOverlay)
        );
    }

    /// The custom template comes out of the settings file, so it is a trust boundary: anything
    /// that is not an https slippy template must not be fetched at all.
    #[test]
    fn custom_template_is_validated_and_keyed_by_content() {
        assert!(valid_xyz_template("https://t.example/{z}/{x}/{y}.png"));
        assert!(!valid_xyz_template("http://t.example/{z}/{x}/{y}.png"));
        assert!(!valid_xyz_template("https://t.example/tiles.png"));
        assert!(!valid_xyz_template(""));
        assert!(!valid_xyz_template("file:///etc/{z}/{x}/{y}"));

        let good = "https://t.example/{z}/{x}/{y}.png";
        assert_eq!(
            BasemapStyle::CustomXyz.url(6, 15, 25, false, "", "", good),
            Some("https://t.example/6/15/25.png".to_string())
        );
        // An invalid template produces no URL rather than a request to something unexpected.
        assert!(BasemapStyle::CustomXyz
            .url(6, 15, 25, false, "", "", "http://t.example/{z}/{x}/{y}")
            .is_none());
        // Repointing the slot must not read the previous server's tiles out of the cache.
        assert_ne!(
            BasemapStyle::CustomXyz.provider(false, good),
            BasemapStyle::CustomXyz.provider(false, "https://other.example/{z}/{x}/{y}.png")
        );
        // And it is only selectable once a template exists.
        assert!(!BasemapStyle::CustomXyz.available(true, true, false));
    }

    /// Every vector look has to be selectable, or it is dead code that still costs a review.
    #[test]
    fn every_palette_has_a_basemap_entry() {
        for pal in crate::basemap_style::Palette::ALL {
            assert!(
                BasemapStyle::ALL
                    .iter()
                    .any(|s| s.vector_palette() == Some(pal)),
                "{pal:?} is not reachable from the basemap list"
            );
        }
    }

    /// The picker draws one section per category. A style in no section is invisible, and a
    /// section with nothing in it is a stray heading.
    #[test]
    fn categories_partition_the_style_list() {
        let mut seen = 0;
        for cat in Category::ALL {
            let n = BasemapStyle::ALL
                .iter()
                .filter(|s| s.category() == cat)
                .count();
            assert!(n > 0, "{cat:?} has no styles");
            seen += n;
        }
        assert_eq!(seen, BasemapStyle::ALL.len());
        // The three that are not a map of anywhere belong together, away from the imagery.
        for s in [
            BasemapStyle::None,
            BasemapStyle::Auto,
            BasemapStyle::CustomXyz,
        ] {
            assert_eq!(s.category(), Category::Other, "{s:?}");
        }
        // Every GIBS imagery product lands under Satellite whatever else the match says. IMERG is
        // the one exception, and deliberately so: it is a precipitation field, not a picture.
        for s in BasemapStyle::ALL {
            if s.goes_layer().is_some() && s != BasemapStyle::ImergRate {
                assert_eq!(s.category(), Category::Satellite, "{s:?}");
            }
        }
    }

    /// The GetMap bbox has to be the exact square the XYZ tile covers, or the composite lands
    /// offset from the ground under it — the failure mode nobody notices until a storm sits a
    /// county to the left of where it is.
    #[test]
    fn a_wms_tile_asks_for_the_square_it_covers() {
        const HALF: f64 = 20_037_508.342_789_244;
        let (x0, y0, x1, y1) = tile_bbox_3857(0, 0, 0);
        assert!((x0 + HALF).abs() < 1e-6 && (y1 - HALF).abs() < 1e-6);
        assert!((x1 - HALF).abs() < 1e-6 && (y0 + HALF).abs() < 1e-6);
        // Tile (1, 1, 0) is the north-east quadrant: east of the meridian, north of the equator.
        let (x0, y0, x1, y1) = tile_bbox_3857(1, 1, 0);
        assert!(x0.abs() < 1e-6 && y0.abs() < 1e-6);
        assert!((x1 - HALF).abs() < 1e-6 && (y1 - HALF).abs() < 1e-6);
        // Neighbours share an edge, so tiles neither overlap nor leave a seam. Within a
        // nanometre: the URL rounds to millimetres and the pixel is a kilometre wide.
        assert!((tile_bbox_3857(5, 16, 10).2 - tile_bbox_3857(5, 17, 10).0).abs() < 1e-6);
        assert!((tile_bbox_3857(5, 16, 10).1 - tile_bbox_3857(5, 16, 11).3).abs() < 1e-6);
    }

    /// The DWD composites are one URL flavour, not a subsystem: the style resolves to a GetMap on
    /// the allowlisted host, with no TIME until a frame is picked.
    #[test]
    fn the_dwd_composite_resolves_to_a_getmap() {
        let url = BasemapStyle::DwdRadarRV
            .url(6, 33, 21, false, "", "", "")
            .expect("a WMS style must resolve to a URL");
        assert!(
            url.starts_with("https://maps.dwd.de/geoserver/dwd/wms?"),
            "{url}"
        );
        for want in [
            "SERVICE=WMS",
            "VERSION=1.3.0",
            "REQUEST=GetMap",
            "CRS=EPSG:3857",
            "LAYERS=Radar_rv_product_1x1km_ger",
            "FORMAT=image/png",
            "TRANSPARENT=TRUE",
        ] {
            assert!(url.contains(want), "{want} missing from {url}");
        }
        // The layer's own default frame until the fetch loop appends one.
        assert!(!url.contains("TIME="), "{url}");
        assert!(BasemapStyle::DwdRadarRV.timed());
        assert_eq!(BasemapStyle::DwdRadarRV.category(), Category::Weather);
        // A time-tagged style has no fixed pyramid to pre-download into a chase pack.
        assert!(BasemapStyle::DwdRadarWN.timed());

        // The Canadian composites ride the same bridge, on their own endpoint and cadence.
        let url = BasemapStyle::EcccRadarSnow
            .url(6, 18, 22, false, "", "", "")
            .expect("a WMS style must resolve to a URL");
        assert!(
            url.starts_with("https://geo.weather.gc.ca/geomet?"),
            "{url}"
        );
        assert!(url.contains("LAYERS=RADAR_1KM_RSNO"), "{url}");
        assert!(!url.contains("TIME="), "{url}");
        assert_eq!(BasemapStyle::EcccRadarRain.category(), Category::Weather);
        assert_eq!(
            BasemapStyle::EcccRadarRain.wms_layer().map(|w| (w.2, w.3)),
            Some((6, 6)),
            "GeoMet publishes on 6 minutes, not DWD's 5"
        );
    }

    /// DWD rejects a TIME that is not on its 5-minute step outright, so every frame the bar can
    /// select has to land on one — and none of them may be newer than the publisher.
    #[test]
    fn wms_frames_land_on_the_publishers_step() {
        // 12:03:17Z, deliberately off-step and off-second.
        let anchor = chrono::DateTime::from_timestamp(1_700_000_597, 0).unwrap();
        let f = wms_frames(anchor, 12, 5, 10);
        assert_eq!(f.len(), 12);
        assert!(f.windows(2).all(|w| w[0] < w[1]), "oldest first");
        for t in &f {
            assert_eq!(t.timestamp() % 300, 0, "{t} is off the 5-minute step");
            assert!(
                *t <= anchor - chrono::Duration::minutes(10),
                "{t} is ahead of the publisher"
            );
        }
        // GeoMet runs on 6 minutes, so the same grid has to move with the publisher, not with a
        // constant baked in for the first one.
        let g = wms_frames(anchor, 12, 6, 6);
        for t in &g {
            assert_eq!(t.timestamp() % 360, 0, "{t} is off the 6-minute step");
            assert!(
                *t <= anchor - chrono::Duration::minutes(6),
                "{t} is ahead of the publisher"
            );
        }
        let last = *f.last().unwrap();
        assert!(
            anchor - last < chrono::Duration::minutes(15),
            "the newest frame is the newest one there is"
        );
        assert_eq!(last - f[f.len() - 2], chrono::Duration::minutes(5));
    }
}

#[cfg(test)]
mod goes_window_tests {
    use super::goes_window;

    #[test]
    fn live_asks_for_the_last_eight_hours_and_an_archive_brackets_its_hour() {
        let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let (hour, from, to) = goes_window(now, Some(now - chrono::Duration::minutes(6)));
        assert_eq!(hour, None, "a lagging live pane is still live");
        assert_eq!(to, now);
        assert_eq!(now - from, chrono::Duration::hours(8));

        let old = now - chrono::Duration::days(3);
        let (hour, from, to) = goes_window(now, Some(old));
        let h = hour.expect("three days back is an archive");
        assert_eq!(h, old.timestamp().div_euclid(3600));
        assert!(
            from <= old && old <= to,
            "the frame we are replaying is covered"
        );
        assert_eq!(to - from, chrono::Duration::hours(3));
        // Scrubbing a few minutes inside the same hour must not change the refetch key.
        let (again, ..) = goes_window(now, Some(old + chrono::Duration::minutes(5)));
        assert_eq!(again, hour);
    }
}
