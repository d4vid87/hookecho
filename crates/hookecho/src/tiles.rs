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

/// A selectable basemap under the radar. Dark/Light are the vector MVT basemap
/// (see [`crate::vector_tiles`]); Satellite is raster USGS imagery. The Mapbox*/MapTiler* styles
/// are provider raster tiles, available only when the matching Settings API key is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BasemapStyle {
    #[default]
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
    UsgsImageryTopo,
    OsmHot,
    CyclOsm,
}

impl BasemapStyle {
    /// Cycle order for the `z` hotkey; provider styles trail the built-ins.
    pub const ALL: [BasemapStyle; 40] = [
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
        BasemapStyle::Dark,
        BasemapStyle::Light,
        BasemapStyle::Satellite,
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
            BasemapStyle::EsriImagery => "Satellite",
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
                BasemapStyle::EsriImagery => "© Esri, Maxar, Earthstar Geographics",
                BasemapStyle::EsriStreets | BasemapStyle::EsriTopo => "© Esri © OpenStreetMap",
                _ if self.goes_layer().is_some() => "NASA GIBS · NOAA GOES",
                _ => "© OpenMapTiles © OpenStreetMap",
            },
        }
    }

    /// For a GOES style, its GIBS layer id + tile-matrix level (each layer serves a fixed max
    /// zoom). `None` for non-GOES styles.
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
            _ => None,
        }
    }

    /// Is this style a raster-tile source (as opposed to the vector MVT basemap or None)?
    /// Everything except the vector Dark/Light and None is a raster source.
    pub fn is_raster(self) -> bool {
        !matches!(
            self,
            BasemapStyle::Dark | BasemapStyle::Light | BasemapStyle::None
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
        match self {
            // USGS ArcGIS services (imagery and topo alike) stop at 16.
            BasemapStyle::Satellite | BasemapStyle::UsgsTopo | BasemapStyle::UsgsImageryTopo => 16,
            BasemapStyle::OpenTopoMap => 17,
            BasemapStyle::OsmHot => 18,
            BasemapStyle::OsmStandard | BasemapStyle::CyclOsm => 19,
            BasemapStyle::CartoPositron | BasemapStyle::CartoDarkMatter | BasemapStyle::CartoVoyager => 20,
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
            BasemapStyle::CartoPositron | BasemapStyle::CartoDarkMatter | BasemapStyle::CartoVoyager
        )
    }

    /// Whether this style's tiles are 512 px rather than 256. Mapbox and MapTiler both serve the
    /// same tile grid at either size, so one 512 tile replaces the four 256 tiles a high-DPI
    /// screen would otherwise fetch a level deeper — same pixels on screen, a quarter of the
    /// requests. Callers use this to drop the retina zoom bias.
    pub fn tiles_are_512(self) -> bool {
        matches!(self.provider_kind(), Provider::Mapbox | Provider::MapTiler)
    }

    /// Is this style selectable given which provider keys are set?
    pub fn available(self, mapbox_key: bool, maptiler_key: bool) -> bool {
        match self.provider_kind() {
            Provider::Mapbox => mapbox_key,
            Provider::MapTiler => maptiler_key,
            Provider::Builtin => true,
        }
    }

    /// Next *available* style in [`Self::ALL`] (wraps) — the `z`-cycle step.
    pub fn next(self, mapbox_key: bool, maptiler_key: bool) -> BasemapStyle {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        for step in 1..=Self::ALL.len() {
            let cand = Self::ALL[(i + step) % Self::ALL.len()];
            if cand.available(mapbox_key, maptiler_key) {
                return cand;
            }
        }
        self
    }

    /// Stable small id for this style, used to key the GPU/fetch caches so panes showing
    /// different basemaps don't overwrite each other's tiles. Index into [`Self::ALL`].
    pub fn key(self) -> u8 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8
    }

    /// Per-style cache subdir so sources don't collide on disk. Keys never appear here.
    fn provider(self, retina: bool) -> String {
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
    fn url(
        self,
        z: u8,
        x: u32,
        y: u32,
        retina: bool,
        mapbox_key: &str,
        maptiler_key: &str,
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
                // NASA GIBS WMTS (web mercator), latest GOES imagery. GIBS uses `{z}/{y}/{x}`.
                _ => self.goes_layer().map(|(layer, level)| {
                    format!(
                        "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{layer}/default/default/GoogleMapsCompatible_Level{level}/{z}/{y}/{x}.png"
                    )
                }),
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
        let (Ok(s), Ok(e)) = (s.parse::<DateTime<Utc>>(), e.parse::<DateTime<Utc>>()) else {
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

/// Fetch the available GOES frame times for `style` over the last `hours` (best-effort; empty on
/// any failure). Uses the GIBS REST DescribeDomains endpoint.
pub async fn fetch_goes_times(
    client: &reqwest::Client,
    style: BasemapStyle,
    hours: i64,
    limit: usize,
) -> Vec<chrono::DateTime<chrono::Utc>> {
    let Some((layer, level)) = style.goes_layer() else {
        return Vec::new();
    };
    let now = chrono::Utc::now();
    let from = (now - chrono::Duration::hours(hours)).format("%Y-%m-%dT%H:%M:%SZ");
    let to = now.format("%Y-%m-%dT%H:%M:%SZ");
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
}

impl TileManager {
    pub fn new(spawner: crate::rt::Spawner) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let client =
            crate::platform::http_timeouts(reqwest::Client::builder().user_agent(USER_AGENT))
                .build()
                .expect("build reqwest client");
        let cache_root = crate::paths::cache_dir().map(|d| d.join("tiles"));
        if let Some(root) = cache_root.clone() {
            std::thread::spawn(move || sweep_tile_cache(&root));
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

    /// Integer tile ids covering the current view, and their world-space rects. `zoom_bias`
    /// pulls sharper tiles on high-DPI displays (see [`tile_cover`]).
    pub fn visible(
        &self,
        style: BasemapStyle,
        cam: &Camera,
        viewport_px: (f32, f32),
        zoom_bias: f64,
    ) -> Vec<VisibleTile> {
        tile_cover(cam, viewport_px, style.max_raster_z(), zoom_bias)
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
            let Some(mut url) =
                style.url(z, x, y, self.retina, &self.mapbox_key, &self.maptiler_key)
            else {
                continue;
            };
            // GOES frame time: rewrite the `default` time slot in the GIBS URL and tag the cache
            // dir so different frames don't collide. Latest (`None`) keeps `default`.
            let time_tag = match (style.goes_layer(), self.goes_time) {
                (Some(_), Some(t)) => {
                    let iso = t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
                    url = url.replace(
                        "/default/GoogleMapsCompatible",
                        &format!("/{iso}/GoogleMapsCompatible"),
                    );
                    t.format("%Y%m%dT%H%M").to_string()
                }
                _ => "default".to_string(),
            };
            self.requested.insert((skey, v.id));
            let path = self.cache_root.as_ref().map(|d| {
                d.join(style.provider(self.retina))
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
        style.max_raster_z()
    }

    /// Whether `style` produces raster tiles a chase pack can pre-download (has a URL, isn't a
    /// time-tagged GOES layer). Takes the style explicitly so it doesn't depend on which pane the
    /// tile manager last rendered.
    pub fn packable(&self, style: BasemapStyle) -> bool {
        style.goes_layer().is_none()
            && style
                .url(0, 0, 0, false, &self.mapbox_key, &self.maptiler_key)
                .is_some()
    }

    /// Build the `(url, cache_path)` jobs for an offline chase pack of the lon/lat bbox over
    /// `z_lo..=z_hi` in `style`. Empty for URL-less (Dark/Light/None) and GOES styles. Cache paths
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
        if style.goes_layer().is_some() {
            return Vec::new();
        }
        let z_hi = z_hi.min(style.max_raster_z());
        // Packs are always 1x: a pack is written once and read on whatever device opens it later.
        let dir = root.join(style.provider(false)).join("default");
        pack_tile_ids(min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
            .into_iter()
            .filter_map(|(z, x, y)| {
                let url = style.url(z, x, y, false, &self.mapbox_key, &self.maptiler_key)?;
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
        let Some(url) = style.url(z, x, y, false, mapbox_key, maptiler_key) else {
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
/// so no locking or temp-rename dance is needed. Bounded by [`sweep_tile_cache`] at startup.
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

/// Trim the on-disk tile cache to [`tile_cache_bytes`], oldest-touched first.
pub(crate) fn sweep_tile_cache(root: &std::path::Path) {
    sweep_cache_dir(root, "tile cache", tile_cache_bytes());
}

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
        ];
        for s in keyless {
            assert!(s.is_raster(), "{s:?} should be raster");
            assert!(
                s.url(6, 15, 25, false, "", "").is_some(),
                "{s:?} should have a keyless URL"
            );
        }
        // ArcGIS services use {z}/{y}/{x}: y before x in the path.
        let esri = BasemapStyle::EsriImagery.url(6, 15, 25, false, "", "").unwrap();
        assert!(esri.ends_with("/6/25/15"), "Esri y/x order: {esri}");
        // Standard slippy tiles use {z}/{x}/{y}.
        let osm = BasemapStyle::OsmStandard.url(6, 15, 25, false, "", "").unwrap();
        assert!(osm.ends_with("/6/15/25.png"), "OSM x/y order: {osm}");
        // Mapbox nav styles stay key-gated.
        assert!(BasemapStyle::MapboxNavDay.url(6, 15, 25, false, "", "").is_none());
        assert!(BasemapStyle::MapboxNavDay.url(6, 15, 25, false, "k", "").is_some());
    }

    /// Retina asks for the `@2x` tile only where the provider serves one, and those tiles get
    /// their own cache directory so a 1x and a 2x tile never overwrite each other on disk.
    #[test]
    fn retina_only_changes_the_sources_that_serve_2x() {
        let carto = BasemapStyle::CartoDarkMatter;
        assert!(carto.url(6, 15, 25, true, "", "").unwrap().ends_with("@2x.png"));
        assert!(carto.url(6, 15, 25, false, "", "").unwrap().ends_with("/25.png"));
        assert_ne!(carto.provider(true), carto.provider(false));
        // No `@2x` upstream: the URL and the cache path are identical either way.
        let osm = BasemapStyle::OsmStandard;
        assert_eq!(osm.url(6, 15, 25, true, "", ""), osm.url(6, 15, 25, false, "", ""));
        assert_eq!(osm.provider(true), osm.provider(false));
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
}
