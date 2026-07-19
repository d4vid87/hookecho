//! Async raster-tile fetching and visible-tile computation for the slippy map.
//!
//! The basemap source is a [`BasemapStyle`] (dark/light/satellite raster, or none). Switching
//! styles clears the pending/uploaded sets so the new source is refetched; the GPU tile cache
//! is cleared in the render layer via the callback's `clear_tiles` flag.

use crate::render::{mercator::Camera, PendingTile, TileId, VisibleTile};
use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender};
use tokio::runtime::Handle;

// Browser-prefixed so imagery hosts (e.g. Esri) that 403 bare library UAs still serve tiles,
// while still identifying the app.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; hookecho/0.0; +github.com/d4vid87/hookecho)";

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
        Self::ALL.into_iter().find(|st| st.slug() == s).unwrap_or(BasemapStyle::None)
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
                BasemapStyle::Satellite | BasemapStyle::UsgsTopo | BasemapStyle::UsgsImageryTopo => {
                    "USGS The National Map"
                }
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
        !matches!(self, BasemapStyle::Dark | BasemapStyle::Light | BasemapStyle::None)
    }

    /// Max zoom the raster source serves; deeper views upscale rather than fetch 404s. GIBS
    /// GOES layers top out at their matrix level; the USGS ArcGIS services cap at 16.
    fn max_raster_z(self) -> u8 {
        if let Some((_, level)) = self.goes_layer() {
            return level;
        }
        match self {
            BasemapStyle::UsgsTopo | BasemapStyle::UsgsImageryTopo => 16,
            _ => 18,
        }
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

    /// Per-style cache subdir so sources don't collide on disk. Keys never appear here.
    fn provider(self) -> &'static str {
        self.slug()
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
    fn url(self, z: u8, x: u32, y: u32, mapbox_key: &str, maptiler_key: &str) -> Option<String> {
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
                    Some(format!("https://basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png"))
                }
                BasemapStyle::CartoDarkMatter => {
                    Some(format!("https://basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png"))
                }
                BasemapStyle::CartoVoyager => {
                    Some(format!("https://basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}.png"))
                }
                BasemapStyle::OsmHot => {
                    Some(format!("https://a.tile.openstreetmap.fr/hot/{z}/{x}/{y}.png"))
                }
                BasemapStyle::CyclOsm => Some(format!(
                    "https://a.tile-cyclosm.openstreetmap.fr/cyclosm/{z}/{x}/{y}.png"
                )),
                // NASA GIBS WMTS (web mercator), latest GOES imagery. GIBS uses `{z}/{y}/{x}`.
                _ if self.goes_layer().is_some() => {
                    let (layer, level) = self.goes_layer().unwrap();
                    Some(format!(
                        "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{layer}/default/default/GoogleMapsCompatible_Level{level}/{z}/{y}/{x}.png"
                    ))
                }
                _ => None,
            },
            Provider::Mapbox => (!mapbox_key.is_empty()).then(|| {
                format!(
                    "https://api.mapbox.com/styles/v1/mapbox/{}/tiles/256/{z}/{x}/{y}?access_token={mapbox_key}",
                    self.style_id()
                )
            }),
            Provider::MapTiler => (!maptiler_key.is_empty()).then(|| {
                let ext = if self == BasemapStyle::MapTilerSatellite { "jpg" } else { "png" };
                format!(
                    "https://api.maptiler.com/maps/{}/256/{z}/{x}/{y}.{ext}?key={maptiler_key}",
                    self.style_id()
                )
            }),
        }
    }
}

/// Integer tile ids covering `cam`'s view (zoom clamped to `max_z`) with their world rects.
/// Shared by raster (`max_z` 18) and vector (`max_z` 14) layers.
pub fn tile_cover(cam: &Camera, viewport_px: (f32, f32), max_z: u8) -> Vec<VisibleTile> {
    let z = cam.zoom.round().clamp(2.0, max_z as f64) as u8;
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
            out.push(VisibleTile { id, world_min: [wx0, wy0], world_max: [wx1, wy1] });
        }
    }
    out
}

/// Parse the `<Domain>` time list from a GIBS DescribeDomains XML into sorted instants. The
/// domain is comma-separated `start/end/PT{n}M` (or `PT{n}H`) ranges; each is expanded to its
/// discrete steps. Returns at most `limit` most-recent instants.
pub fn parse_goes_domain(xml: &str, limit: usize) -> Vec<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, Utc};
    let Some(start) = xml.find("<Domain>") else { return Vec::new() };
    let Some(end) = xml[start..].find("</Domain>") else { return Vec::new() };
    let body = &xml[start + "<Domain>".len()..start + end];
    let mut out: Vec<DateTime<Utc>> = Vec::new();
    for range in body.split(',') {
        let parts: Vec<&str> = range.trim().split('/').collect();
        let (s, e, period) = match parts.as_slice() {
            [s, e, p] => (*s, *e, *p),
            [s] => (*s, *s, "PT10M"), // a lone instant
            _ => continue,
        };
        let (Ok(s), Ok(e)) = (s.parse::<DateTime<Utc>>(), e.parse::<DateTime<Utc>>()) else { continue };
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
    let Some((layer, level)) = style.goes_layer() else { return Vec::new() };
    let now = chrono::Utc::now();
    let from = (now - chrono::Duration::hours(hours)).format("%Y-%m-%dT%H:%M:%SZ");
    let to = now.format("%Y-%m-%dT%H:%M:%SZ");
    let url = format!(
        "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/1.0.0/{layer}/default/GoogleMapsCompatible_Level{level}/-180,-90,180,90/{from}--{to}.xml"
    );
    match client.get(&url).header("User-Agent", USER_AGENT).send().await {
        Ok(resp) => match resp.text().await {
            Ok(xml) => parse_goes_domain(&xml, limit),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

struct FetchedTile {
    id: TileId,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

pub struct TileManager {
    rt: Handle,
    client: reqwest::Client,
    tx: Sender<FetchedTile>,
    rx: Receiver<FetchedTile>,
    requested: HashSet<TileId>,
    uploaded: HashSet<TileId>,
    style: BasemapStyle,
    cache_root: Option<std::path::PathBuf>,
    mapbox_key: String,
    maptiler_key: String,
    /// Selected GOES frame time (`None` = latest/`default`). Only affects GOES styles.
    goes_time: Option<chrono::DateTime<chrono::Utc>>,
}

impl TileManager {
    pub fn new(rt: Handle) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("build reqwest client");
        let cache_root = directories::ProjectDirs::from("", "", "hookecho")
            .map(|d| d.cache_dir().join("tiles"));
        Self {
            rt,
            client,
            tx,
            rx,
            requested: HashSet::new(),
            uploaded: HashSet::new(),
            style: BasemapStyle::Dark,
            cache_root,
            mapbox_key: String::new(),
            maptiler_key: String::new(),
            goes_time: None,
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

    /// Switch basemap source. Returns true if the style changed (caller should clear the GPU
    /// tile cache so stale tiles from the old source aren't shown).
    pub fn set_style(&mut self, style: BasemapStyle) -> bool {
        if self.style == style {
            return false;
        }
        self.style = style;
        self.requested.clear();
        self.uploaded.clear();
        true
    }

    /// Integer tile ids covering the current view, and their world-space rects.
    pub fn visible(&self, cam: &Camera, viewport_px: (f32, f32)) -> Vec<VisibleTile> {
        tile_cover(cam, viewport_px, self.style.max_raster_z())
    }

    /// Kick off fetches for any visible tiles not yet requested.
    pub fn request_missing(&mut self, visible: &[VisibleTile]) {
        for v in visible {
            if self.requested.contains(&v.id) {
                continue;
            }
            let (z, x, y) = v.id;
            let Some(mut url) = self.style.url(z, x, y, &self.mapbox_key, &self.maptiler_key) else { continue };
            // GOES frame time: rewrite the `default` time slot in the GIBS URL and tag the cache
            // dir so different frames don't collide. Latest (`None`) keeps `default`.
            let time_tag = match (self.style.goes_layer(), self.goes_time) {
                (Some(_), Some(t)) => {
                    let iso = t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
                    url = url.replace("/default/GoogleMapsCompatible", &format!("/{iso}/GoogleMapsCompatible"));
                    t.format("%Y%m%dT%H%M").to_string()
                }
                _ => "default".to_string(),
            };
            self.requested.insert(v.id);
            let path = self
                .cache_root
                .as_ref()
                .map(|d| d.join(self.style.provider()).join(&time_tag).join(format!("{z}/{x}/{y}")));
            let client = self.client.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Ok(bytes) = load_tile_bytes(&client, &url, path.as_deref()).await {
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        let _ = tx.send(FetchedTile {
                            id: (z, x, y),
                            rgba: rgba.into_raw(),
                            width: w,
                            height: h,
                        });
                    }
                }
            });
        }
    }

    /// Drain finished fetches into upload-ready tiles (each returned exactly once).
    pub fn drain_ready(&mut self) -> Vec<PendingTile> {
        let mut ready = Vec::new();
        while let Ok(t) = self.rx.try_recv() {
            if self.uploaded.insert(t.id) {
                ready.push(PendingTile {
                    id: t.id,
                    rgba: t.rgba,
                    width: t.width,
                    height: t.height,
                });
            }
        }
        ready
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
        let Some(url) = style.url(z, x, y, mapbox_key, maptiler_key) else { continue };
        match load_tile_bytes(client, &url, None).await {
            Ok(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    out.push(PendingTile { id: v.id, rgba: rgba.into_raw(), width: w, height: h });
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
/// so no locking or temp-rename dance is needed. `// ponytail: unbounded cache (~10-30KB/tile);
/// add a size cap when it ever matters.`
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
    let resp = client.get(url).send().await?.error_for_status()?;
    let bytes = resp.bytes().await?.to_vec();
    if let Some(p) = path {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(p, &bytes);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(s.url(6, 15, 25, "", "").is_some(), "{s:?} should have a keyless URL");
        }
        // ArcGIS services use {z}/{y}/{x}: y before x in the path.
        let esri = BasemapStyle::EsriImagery.url(6, 15, 25, "", "").unwrap();
        assert!(esri.ends_with("/6/25/15"), "Esri y/x order: {esri}");
        // Standard slippy tiles use {z}/{x}/{y}.
        let osm = BasemapStyle::OsmStandard.url(6, 15, 25, "", "").unwrap();
        assert!(osm.ends_with("/6/15/25.png"), "OSM x/y order: {osm}");
        // Mapbox nav styles stay key-gated.
        assert!(BasemapStyle::MapboxNavDay.url(6, 15, 25, "", "").is_none());
        assert!(BasemapStyle::MapboxNavDay.url(6, 15, 25, "k", "").is_some());
    }
}
