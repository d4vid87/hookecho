//! HookEcho application shell: the map view, its floating chrome, and the async data flow.
//!
//! UI code only mutates the active [`MapView`]; a single per-frame sync step turns those
//! mutations into GPU uploads and background fetches, so buttons and hotkeys share one path.

/// Touch-first Android chrome (top bar, bottom dock, slide-up sheets), replacing the desktop
/// drawer / pills / alert dock. Only the chrome differs; the map,
/// windows, and every data path are shared.
mod chrome;
mod mobile;

use crate::colormap::{ColorTable, Palettes};
use crate::hotkeys::{self, BindableAction};
use crate::overlay_build;
#[cfg(not(target_arch = "wasm32"))]
use crate::perf::PerfReadout;
use crate::render::{mercator::Camera, MapCallback, OverlayUpload, RadarUpload, RenderResources};
use crate::settings::Settings;
use crate::tiles::TileManager;
use crate::ui;
use crate::ui::detail_window::Detail;
use crate::view::{MapView, Volume};
use chrono::{DateTime, NaiveDate, Utc};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Runtime;
use wxdata::alerts::{self};
use wxdata::clock::Instant;
use wxdata::level2::{self, BinnedSweep, Identifier, Moment, Scan};
use wxdata::level3::{self, Cell, CellKind};
use wxdata::live;
use wxdata::overlay::{self, GeoFeature};

/// Frames to let a stepped archive volume load before grabbing it for the loop GIF.
const LOOP_SETTLE_FRAMES: u8 = 12;

/// How long with no input at all before the idle heartbeat slows to [`IDLE_QUIET_MS`]. Long
/// enough that it never fires between two deliberate actions — reading a detail window and then
/// reaching for the mouse is not "idle" — and short enough that a window left alone stops costing
/// a core within a couple of seconds.
const QUIET_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// The heartbeat in a window nobody is touching. Two frames a second: the clocks the heartbeat
/// exists for (volume age, countdowns) are minute- and second-resolution, so the visible cost is
/// a reading up to 0.4 s stale. Everything that actually moves asks for its own repaint and is
/// unaffected — see the comment at the use site.
const IDLE_QUIET_MS: u64 = 500;

/// How long any one overlay or field fetch may run before it is abandoned.
///
/// Shorter than the cadences that drive them (120 s for the alert/watch/MD burst, 60 s at the
/// fastest for a gridded field), which is what keeps a stalled feed from stacking a second copy
/// of itself on every tick. Deliberately *longer* than `wxdata::net::FEED_TIMEOUT`, which is the
/// deadline that actually aborts the request: this one only drops our future, and dropping it
/// first would leave the browser's `fetch` running with its connection held.
const OVERLAY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(55);

/// The same, for a Level 2 volume: bigger file, more patience, still finite. A volume fetch that
/// never returns leaves its pane marked loading, and a pane marked loading never polls again.
/// Again longer than the request's own 90 s deadline in the vendored S3 client, so the abort
/// happens before we stop listening for it.
const VOLUME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(100);

/// Loop frames in flight, keyed by volume name, with when each was kicked off.
type PrefetchBook = std::collections::HashMap<String, Instant>;

/// Take the prefetch book, recovering from poisoning.
///
/// A panic elsewhere while this is held must not take the loop down with it: the book is a list
/// of names in flight, not an invariant. Losing track of one of them costs a duplicate download.
fn book(b: &Mutex<PrefetchBook>) -> std::sync::MutexGuard<'_, PrefetchBook> {
    b.lock().unwrap_or_else(|e| e.into_inner())
}

/// Even-odd point-in-ring test on a `[lon, lat]` ring — the click test for watch zones.
fn point_in_ring_ll(ring: &[[f64; 2]], lon: f64, lat: f64) -> bool {
    wxdata::overlay::rings_intersect(
        ring,
        // A tiny square around the click: reuses the one geometry primitive rather than adding a
        // second point-in-polygon implementation here.
        &[
            [lon - 1e-6, lat - 1e-6],
            [lon + 1e-6, lat - 1e-6],
            [lon + 1e-6, lat + 1e-6],
            [lon - 1e-6, lat + 1e-6],
        ],
    )
}

/// The first `http(s)://` URL in a free-text line, if there is one. Spotter reports and chase
/// partners paste stream links into their status text; this is how we find them.
fn first_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(|w| w.trim_end_matches(['.', ',', ')', '"', '\'']).to_string())
}

/// 3D volume grid size: `VOL3D_N` cells across each horizontal axis, `VOL3D_NZ` up. Big enough to
/// resolve a hail core, small enough to resample in about a second.
const VOL3D_N: usize = 192;
const VOL3D_NZ: usize = 48;

/// Squared screen-space hit radius (px²) for a tap/click target of nominal `px` radius. Android
/// finger taps need a fatter target than a mouse cursor, so targets grow ~1.8× there; desktop is
/// unchanged.
fn tap_r2(px: f32) -> f32 {
    let r = if cfg!(target_os = "android") {
        px * 1.8
    } else {
        px
    };
    r * r
}

/// Colour for an EF rating, running the same yellow→red ramp the NWS uses in its own survey maps.
/// Unrateable damage (EFU) draws grey so it can't be mistaken for a weak rating.
fn ef_color(efscale: &str) -> egui::Color32 {
    match wxdata::dat::ef_number(efscale) {
        Some(0) => egui::Color32::from_rgb(120, 200, 120),
        Some(1) => egui::Color32::from_rgb(240, 220, 80),
        Some(2) => egui::Color32::from_rgb(245, 165, 50),
        Some(3) => egui::Color32::from_rgb(240, 100, 50),
        Some(4) => egui::Color32::from_rgb(225, 45, 45),
        Some(_) => egui::Color32::from_rgb(200, 60, 200),
        None => egui::Color32::from_rgb(160, 160, 165),
    }
}

/// The first segment of a geocoder result — "Norman, Cleveland County, Oklahoma, United States"
/// is a fine answer to a query and a terrible name for a pin on a map.
fn short_place_name(s: &str) -> &str {
    s.split(',').next().unwrap_or(s).trim()
}

/// Which severe-weather overlays are shown.
pub struct OverlayFilters {
    pub show_alerts: bool,
    pub alert_cats: [bool; 6],
    /// SPC categorical outlook day (0 = off, else 1–3).
    pub outlook_day: u8,
    /// Day-1 outlook hazard: categorical risk, or a tornado/wind/hail probability grid.
    pub outlook_kind: wxdata::spc::OutlookKind,
    pub show_mds: bool,
    /// SPC tornado and severe thunderstorm watches in effect.
    pub show_watches: bool,
    /// WPC Winter Storm Severity Index day (0 = off, else 1-3).
    pub wssi_day: u8,
    /// WPC Excessive Rainfall Outlook day (0 = off, else 1-3).
    pub ero_day: u8,
    /// Level 3 storm cells (clickable dots: storm tracking, hail, mesocyclone).
    pub show_cells: bool,
    /// SCIT forecast tracks (painter-only; no overlay rebuild).
    pub show_tracks: bool,
    /// Storm arrival-time cones: project cell motion forward + ETA to watched markers.
    pub show_arrival_cones: bool,
    /// Optical-flow nowcast: advect the current reflectivity echo forward by the mean storm motion.
    pub show_nowcast: bool,
    /// Nowcast lead time in minutes (how far ahead to extrapolate).
    pub nowcast_lead_min: u8,
    /// Auto tornado-debris-signature detection (low CC collocated with high reflectivity).
    pub show_tds: bool,
    /// Flag velocity rotation couplets (client-side gate-to-gate azimuthal shear).
    pub show_couplets: bool,
    /// Flag three-body scatter spikes (hail spikes) off the lowest tilt.
    pub show_tbss: bool,
    /// Flag ZDR columns — rain lofted above the freezing level, an updraft proxy.
    pub show_zdr_columns: bool,
}

impl Default for OverlayFilters {
    fn default() -> Self {
        Self {
            show_alerts: true,
            alert_cats: [true; 6],
            outlook_day: 0, // SPC outlook off by default; user opts in from Layer options
            outlook_kind: wxdata::spc::OutlookKind::Categorical,
            wssi_day: 0, // off by default, like the SPC outlook
            ero_day: 0,

            show_mds: true,
            show_watches: true,
            show_cells: true,
            show_tracks: true,
            show_arrival_cones: false,
            show_nowcast: false,
            nowcast_lead_min: 15,
            show_tds: false,
            show_couplets: false,
            show_tbss: false,
            show_zdr_columns: false,
        }
    }
}

/// How many TFR shapes to fetch per refresh.
///
/// The FAA lists ~135 active restrictions and each shape is a separate document, so a first load
/// cannot be one request. Shapes never change once issued, so this only paces that first load: a
/// few refresh cycles and the set is complete and stays complete.
const TFR_BATCH: usize = 25;

/// Background overlay fetch results.
enum OverlayMsg {
    Alerts(Vec<GeoFeature>),
    /// Last run's alert overlay, read from disk off the launch path. Applied only if no live
    /// fetch has landed yet; it seeds the known-warning ids either way, so a restart mid-event
    /// doesn't re-banner and re-speak warnings already on the map.
    AlertSeed(Vec<GeoFeature>),
    /// WPC coded surface analysis (fronts + pressure centers).
    Fronts(wxdata::fronts::SurfaceAnalysis),
    Outlook(u8, Vec<GeoFeature>),
    Mds(Vec<GeoFeature>),
    Watches(Vec<GeoFeature>),
    /// WPC Winter Storm Severity Index polygons for a day.
    Wssi(u8, Vec<GeoFeature>),
    /// WPC Excessive Rainfall Outlook polygons for a day.
    Ero(u8, Vec<GeoFeature>),
    /// mPING crowd precipitation-type reports.
    Mping(Vec<wxdata::mping::Report>),
    /// Pilot reports within the fetched bbox.
    Pireps(Vec<wxdata::aviation::Pirep>),
    /// Hurricane-hunter flight-track observations.
    Recon(Vec<wxdata::recon::HdobOb>),
    /// Storm cells for a specific site (dropped if the active site changed meanwhile).
    Cells(String, Vec<Cell>),
    /// A fetched placefile keyed by its URL.
    Placefile(String, wxdata::placefile::Placefile),
    /// The latest grid for a national field layer (mosaic, rotation, MESH, AzShear, lightning).
    Field(crate::render::FieldLayer, wxdata::mrms::MrmsField),
    /// A model-difference grid plus the two valid times it compared, for the layer's own row.
    ModelDiff(wxdata::mrms::MrmsField, (String, String)),
    /// `(0 °C, −20 °C)` level heights above sea level, in metres, at the active radar.
    FreezingLevels(f64, f64),
    /// Local storm reports: live trailing window (`None`) or an archive bucket (feature CC).
    StormReports(Option<i64>, Vec<wxdata::spc::StormReport>),
    /// Live Spotter Network positions (CONUS-wide; filtered to the active site at draw time).
    Spotters(Vec<wxdata::spotters::Spotter>),
    /// ProbSevere per-storm probability polygons.
    ProbSevere(Vec<GeoFeature>),
    /// An HRRR composite-reflectivity forecast (regridded + run/valid metadata).
    Hrrr(wxdata::hrrr::HrrrForecast),
    /// HRRR wind components for the particle layer.
    ///
    /// Deliberately not an [`OverlayMsg::Field`]: `spawn_overlay` runs `decimated` on every
    /// `Field` message, and `decimated` **max-pools**. That is right for reflectivity and wrong
    /// for a signed vector component — it would bias u and v independently toward positive, i.e.
    /// a phantom northeasterly drift. Boxed because a pair of CONUS grids is ~11 MB and this enum
    /// is moved by value through the channel.
    Wind(Box<crate::wind_draw::WindField>),
    /// Nearest-station observations for a site (or an error string).
    Obs(String, Result<wxdata::obs::StationObs, String>),
    /// VAD wind profile for a site.
    Vwp(String, Vec<wxdata::level3::VwpLevel>),
    /// Archived storm-based warnings for a 5-min UTC bucket (feature W).
    ArchiveWarnings(i64, Vec<GeoFeature>),
    /// Surface observations (METAR station plots) for the requested bbox (feature U).
    Metar(
        Vec<wxdata::metar::SurfaceOb>,
        std::collections::HashMap<String, String>,
    ),
    /// FAA camera sites for the requested bbox.
    Webcams(Vec<wxdata::webcams::CamSite>),
    /// Wildfire perimeters + incident points for the requested bbox.
    Fires(Vec<GeoFeature>, Vec<wxdata::wfigs::FireIncident>),
    /// AirNow AQI observations for the requested bbox.
    Aqi(Vec<wxdata::airnow::AqiOb>),
    /// Live surface stations for the telemetry cards.
    Stations(Vec<wxdata::stations::StationOb>),
    /// The current PPEF electric-field table (ionospheric, mV/m).
    Ppef(wxdata::efield::Ppef),
    /// Highway cameras for the requested bbox.
    DotCams(Vec<wxdata::dotcams::DotCam>),
    /// Newest reading from a configured ground field mill (kV/m).
    Mill(f32),
    /// NWS damage-survey points and surveyed tracks for the requested bbox + storm day.
    Dat(Vec<wxdata::dat::DamagePoint>, Vec<wxdata::dat::DamageTrack>),
    /// A plugin or placefile that failed to load, with why (shown in the manager window).
    PlacefileError(String, String),
    /// A finished multi-radar reflectivity composite: the grid, its contributing sites, and the
    /// oldest contributing scan time.
    Mosaic(
        wxdata::mrms::MrmsField,
        Vec<String>,
        chrono::DateTime<chrono::Utc>,
    ),
    /// River flood gauges (NWPS) for the requested bbox.
    Gauges(Vec<wxdata::river::Gauge>),
    /// HRRR model contour polylines for a kind, plus the forecast valid time.
    Contours(
        ContourKind,
        Vec<wxdata::contour::ContourLine>,
        DateTime<Utc>,
    ),
    /// NHC tropical cyclones: cones + per-storm tracks (feature V).
    Tropical(wxdata::tropical::TropicalData),
    /// County power outages from ODIN.
    Outages(Vec<overlay::GeoFeature>),
    /// Aviation SIGMET/AIRMET hazard polygons (feature GG).
    Aviation(Vec<GeoFeature>),
    /// Newly-fetched TFR shapes keyed by NOTAM id, and how many are still unfetched.
    Tfr(Vec<(String, GeoFeature)>, usize),
}

/// One overlay data source to fetch.
#[derive(Clone)]
enum OverlaySource {
    /// NWS alerts; the `(lat, lon)` list scopes zone-only alert resolution to the active radar and
    /// every saved marker. The bounds are the active pane's viewport, which is what decides
    /// whether European warnings are worth fetching alongside them.
    Alerts(Vec<(f64, f64)>, (f64, f64, f64, f64)),
    Mds,
    /// Tornado and severe thunderstorm watch polygons in effect.
    Watches,
    /// Winter Storm Severity Index for a day (1-3).
    Wssi(u8),
    /// Excessive Rainfall Outlook for a day (1-3).
    Ero(u8),
    /// mPING crowd reports from the last hour, with the user's API key.
    Mping(String),
    /// Pilot reports within a lat/lon bbox `(lat0, lon0, lat1, lon1)`.
    Pireps(f64, f64, f64, f64),
    /// Hurricane-hunter HDOBs from the last few hours.
    Recon,
    Outlook(u8, wxdata::spc::OutlookKind),
    Cells(String),
    Placefile(String),
    /// A national field layer plus the MRMS S3 product path to fetch it from.
    Field(crate::render::FieldLayer, String),
    /// Local storm reports: live (`None`) or a 30-min archive bucket (Unix secs / 1800).
    StormReports(Option<i64>),
    Spotters,
    ProbSevere,
    /// WPC coded surface analysis (fronts + pressure centers).
    Fronts,
    /// HRRR composite-reflectivity forecast for a forecast hour (0..=18).
    Hrrr(u8),
    /// Environment field (CAPE/SRH) at f00 from `model`; `ml` = mixed-layer CAPE, `srh_km` = SRH
    /// depth. RAP makes it an observed analysis rather than an HRRR forecast at hour zero.
    Env(crate::render::FieldLayer, wxdata::hrrr::Model, bool, u8),
    /// HRRR-backed field layer (rotation tracks, smoke) at a forecast hour.
    HrrrLayer(crate::render::FieldLayer, u8),
    /// A global-model field (GFS or ECMWF) at a forecast hour.
    Global(
        crate::render::FieldLayer,
        wxdata::global::GlobalModel,
        wxdata::global::GlobalField,
        u16,
    ),
    /// One model's field minus another's, at a forecast hour. Which two models is implied by the
    /// field (see `fielddiff::DiffField::pair`).
    ModelDiff(crate::fielddiff::DiffField, u16),
    /// Gridded L3 product (DVL/EET) for a site, projected to a lat/lon field (feature X).
    L3Grid(crate::render::FieldLayer, String),
    /// Melting-level and −20 °C heights at `(lon, lat)`, for the derived hail grids.
    FreezingLevels(f64, f64),
    /// NOHRSC observed snowfall analysis over an accumulation window (hours).
    Snow(u16),
    /// Banded snow: the MRMS mosaic cut to elongated echo and masked to snow.
    SnowBands,
    /// Nearest-station observations for `site` at `(lat, lon)`.
    Obs {
        site: String,
        lat: f64,
        lon: f64,
    },
    /// VAD wind profile for `site`.
    Vwp(String),
    /// Archived storm-based warnings valid at a 5-min UTC bucket (Unix seconds, feature W).
    ArchiveWarnings(i64),
    /// Aviation SIGMET/AIRMET polygons (feature GG).
    Aviation,
    /// FAA Temporary Flight Restrictions; carries the NOTAM ids already held, so a refresh only
    /// fetches shapes that are new.
    Tfr(Vec<String>),
    /// Surface observations within a lat/lon bbox `(lat0, lon0, lat1, lon1)` (feature U).
    Metar(f64, f64, f64, f64),
    /// Run an external-process plugin: `(key, command, args, context)`. The key is the synthetic
    /// `plugin:<name>` id it shares with the placefile pipeline it feeds.
    #[cfg(not(target_arch = "wasm32"))]
    Plugin(String, String, Vec<String>, crate::plugins::Context),
    /// Camera sites within a lon/lat bbox `(min_lon, min_lat, max_lon, max_lat)`, plus the user's
    /// Windy API key — empty for FAA-only, which is the keyless default.
    Webcams(f64, f64, f64, f64, String),
    /// Wildfire perimeters + incidents within a lon/lat bbox `(west, south, east, north)`.
    Fires(f64, f64, f64, f64),
    /// AirNow AQI within a lon/lat bbox, plus the user's key (never fetched without one).
    Aqi(f64, f64, f64, f64, String),
    /// Live stations in a lat/lon bbox, plus the view centre the keyed networks are asked around
    /// and the keys themselves (empty = that network stays off).
    Stations {
        bbox: (f64, f64, f64, f64),
        center: (f64, f64),
        tempest: String,
        wu: String,
        synoptic: String,
    },
    /// NOAA's PPEF electric-field table.
    Ppef,
    /// Highway cameras within a lon/lat bbox.
    DotCams(f64, f64, f64, f64),
    /// A user-configured field-mill endpoint.
    Mill(String),
    /// Damage surveys: a lon/lat bbox plus the UTC day whose storms to ask for.
    Dat((f64, f64, f64, f64), chrono::NaiveDate),
    /// Multi-radar mosaic over a named set of sites (chosen from the view before spawning, so the
    /// fetch task needs no camera state).
    Mosaic(Vec<String>),
    /// River flood gauges within a lat/lon bbox `(lat0, lon0, lat1, lon1)`.
    Gauges(f64, f64, f64, f64),
    /// Model contours for a field kind (surface f00, contoured off-thread), from HRRR or the RAP
    /// analysis.
    Contours(ContourKind, wxdata::hrrr::Model),
    /// County power outages by county, from ODIN (DOE/ORNL).
    Outages,
    /// NHC tropical cyclones (feature V).
    /// NHC tropical suite: `(wind-field threshold in kt, include storm surge)`.
    Tropical(Option<u8>, bool),
    /// HRRR wind components for the particle layer, at a level and forecast hour.
    Wind(wxdata::hrrr::WindLevel, u8),
}

impl OverlaySource {
    async fn fetch(self, http: &reqwest::Client) -> anyhow::Result<OverlayMsg> {
        Ok(match self {
            OverlaySource::Alerts(points, bounds) => {
                let mut feats = alerts::fetch_active(http, &points).await?;
                // Europe's warnings come from a different publisher on a different continent, so
                // they are only asked for when the view is actually over one of the countries
                // that publishes them — otherwise this is a no-op with no request at all.
                if !wxdata::meteoalarm::countries_in_view(bounds).is_empty() {
                    match wxdata::meteoalarm::fetch_in_view(http, bounds).await {
                        Ok(eu) => feats.extend(eu),
                        // A European feed being down must not cost the US alerts already fetched.
                        Err(e) => log::warn!("meteoalarm fetch failed ({e})"),
                    }
                }
                // Canada's the same story one border north: gated on the view, and its own
                // failure, so an outage there costs neither the US nor the European alerts.
                if wxdata::eccc::in_view(bounds) {
                    match wxdata::eccc::fetch_in_view(http, bounds).await {
                        Ok(ca) => feats.extend(ca),
                        Err(e) => log::warn!("eccc alerts fetch failed ({e})"),
                    }
                }
                OverlayMsg::Alerts(feats)
            }
            OverlaySource::Mds => {
                OverlayMsg::Mds(wxdata::spc::fetch_mesoscale_discussions(http).await?)
            }
            OverlaySource::Watches => OverlayMsg::Watches(wxdata::spc::fetch_watches(http).await?),
            OverlaySource::Wssi(day) => {
                OverlayMsg::Wssi(day, wxdata::wssi::fetch(http, day).await?)
            }
            OverlaySource::Mping(key) => {
                OverlayMsg::Mping(wxdata::mping::fetch(http, &key, 60).await?)
            }
            OverlaySource::Pireps(lat0, lon0, lat1, lon1) => OverlayMsg::Pireps(
                wxdata::aviation::fetch_pireps(http, lat0, lon0, lat1, lon1).await?,
            ),
            OverlaySource::Ero(day) => OverlayMsg::Ero(day, wxdata::ero::fetch(http, day).await?),
            OverlaySource::Recon => OverlayMsg::Recon(wxdata::recon::fetch(http, 6).await?),
            OverlaySource::Outlook(day, kind) => {
                OverlayMsg::Outlook(day, wxdata::spc::fetch_outlook_kind(http, day, kind).await?)
            }
            OverlaySource::Cells(site) => {
                let cells = level3::fetch_cells(http, &site).await;
                OverlayMsg::Cells(site, cells)
            }
            OverlaySource::Placefile(url) => {
                let pf = wxdata::placefile::fetch(http, &url).await?;
                OverlayMsg::Placefile(url, pf)
            }
            #[cfg(not(target_arch = "wasm32"))]
            OverlaySource::Plugin(key, command, args, pctx) => {
                // A plugin failure is the user's own command misbehaving, so it has to reach the
                // manager window rather than only the log — hence a message either way.
                match crate::plugins::run(&command, &args, &pctx).await {
                    Ok(pf) => OverlayMsg::Placefile(key, pf),
                    Err(e) => OverlayMsg::PlacefileError(key, e.to_string()),
                }
            }
            OverlaySource::Field(layer, product) => {
                OverlayMsg::Field(layer, wxdata::mrms::fetch_latest(http, &product).await?)
            }
            OverlaySource::SnowBands => {
                // Both grids at once: the mask is useless without the echo and vice versa.
                let (mosaic, flags) = futures_util::future::try_join(
                    wxdata::mrms::fetch_latest(http, wxdata::mrms::REFLECTIVITY),
                    wxdata::mrms::fetch_latest(http, wxdata::mrms::PRECIP_TYPE),
                )
                .await?;
                // MRMS PrecipFlag: 3 is snow, 4 is wet snow. Everything else is rain, ice or
                // nothing, and a snow-squall layer that lit up over warm rain would be a liar.
                let bands = wxdata::banding::bands(&mosaic, 20.0, Some((&flags, &[3, 4])))
                    .ok_or_else(|| anyhow::anyhow!("the mosaic came back empty"))?;
                OverlayMsg::Field(crate::render::FieldLayer::SnowBands, bands)
            }
            OverlaySource::Global(layer, model, field, fh) => {
                let fc = wxdata::global::fetch(http, model, field, fh).await?;
                OverlayMsg::Field(layer, fc.field)
            }
            OverlaySource::ModelDiff(field, fh) => {
                use crate::fielddiff::DiffField;
                use wxdata::global::{GlobalField, GlobalModel};
                let (a, b, valid) = match field {
                    DiffField::Global(kind) => {
                        let g: GlobalField = kind.into();
                        // Both cycles at once: they are independent services, and the layer is
                        // useless until both answer anyway.
                        let (gfs, ecmwf) = futures_util::future::try_join(
                            wxdata::global::fetch(http, GlobalModel::Gfs, g, fh),
                            wxdata::global::fetch(http, GlobalModel::Ecmwf, g, fh),
                        )
                        .await?;
                        let valid = (
                            gfs.valid().format("%d %H:%MZ").to_string(),
                            ecmwf.valid().format("%d %H:%MZ").to_string(),
                        );
                        (gfs.field, ecmwf.field, valid)
                    }
                    DiffField::Cape | DiffField::Srh => {
                        let (var, level, min_valid) = match field {
                            DiffField::Srh => ("HLCY", "3000-0 m above ground", f64::NEG_INFINITY),
                            _ => ("CAPE", "surface", 0.0),
                        };
                        let (hrrr, rap) = futures_util::future::try_join(
                            wxdata::hrrr::fetch_field(
                                http,
                                wxdata::hrrr::Model::Hrrr,
                                var,
                                level,
                                0,
                                min_valid,
                            ),
                            wxdata::hrrr::fetch_field(
                                http,
                                wxdata::hrrr::Model::Rap,
                                var,
                                level,
                                0,
                                min_valid,
                            ),
                        )
                        .await?;
                        let valid = (
                            hrrr.run.format("%d %H:%MZ").to_string(),
                            rap.run.format("%d %H:%MZ").to_string(),
                        );
                        (hrrr.field, rap.field, valid)
                    }
                };
                let d = crate::fielddiff::diff(&a, &b)
                    .ok_or_else(|| anyhow::anyhow!("the two models cover nothing in common"))?;
                OverlayMsg::ModelDiff(d, valid)
            }
            OverlaySource::StormReports(bucket) => {
                // Archive bucket: the 6 h of reports ending at the bucket's close; live: last 6 h.
                let reports = match bucket {
                    Some(b) => {
                        let end =
                            chrono::DateTime::from_timestamp((b + 1) * 1800, 0).unwrap_or_default();
                        let start = end - chrono::Duration::hours(6);
                        let fmt = "%Y-%m-%dT%H:%MZ";
                        wxdata::lsr::fetch(
                            http,
                            Some((&start.format(fmt).to_string(), &end.format(fmt).to_string())),
                        )
                        .await?
                    }
                    None => wxdata::lsr::fetch(http, None).await?,
                };
                OverlayMsg::StormReports(bucket, reports)
            }
            OverlaySource::Fronts => OverlayMsg::Fronts(wxdata::fronts::fetch(http).await?),
            OverlaySource::Spotters => {
                OverlayMsg::Spotters(wxdata::spotters::fetch_spotters(http).await?)
            }
            OverlaySource::ProbSevere => {
                OverlayMsg::ProbSevere(wxdata::probsevere::fetch_probsevere(http).await?)
            }
            OverlaySource::Hrrr(fh) => {
                OverlayMsg::Hrrr(wxdata::hrrr::fetch_forecast(http, fh).await?)
            }
            OverlaySource::HrrrLayer(layer, fh) => {
                use crate::render::FieldLayer as FL;
                let fc = match layer {
                    // Rotation tracks read as a swath: the union of every hourly max window from
                    // now through the scrubbed hour, not just that one hour's slice.
                    FL::UpdraftHelicity => {
                        wxdata::hrrr::fetch_field_swath(
                            http,
                            "MXUPHL",
                            "5000-2000 m above ground",
                            fh.max(1),
                            0.0,
                        )
                        .await?
                    }
                    // Accumulated snowfall since the run started, through the scrubbed hour.
                    FL::Snowfall => {
                        wxdata::hrrr::fetch_field(
                            http,
                            wxdata::hrrr::Model::Hrrr,
                            "ASNOW",
                            "surface",
                            fh,
                            0.0,
                        )
                        .await?
                    }
                    // NBM's calibrated probability of thunder over the hour ending at `fh`. The
                    // idx lists the trailing window first, so the plain var+level match already
                    // picks that one over the run-total windows beside it.
                    FL::ThunderProb => {
                        wxdata::hrrr::fetch_field(
                            http,
                            wxdata::hrrr::Model::Nbm,
                            "TSTM",
                            "surface",
                            fh.max(1),
                            0.0,
                        )
                        .await?
                    }
                    _ => {
                        wxdata::hrrr::fetch_field(
                            http,
                            wxdata::hrrr::Model::Hrrr,
                            "MASSDEN",
                            "8 m above ground",
                            fh,
                            0.0,
                        )
                        .await?
                    }
                };
                OverlayMsg::Field(layer, fc.field)
            }
            OverlaySource::Env(layer, model, ml, srh_km) => {
                use crate::render::FieldLayer as FL;
                let (var, level, min_valid) = match layer {
                    FL::Cape if ml => ("CAPE", "90-0 mb above ground".to_string(), 0.0),
                    FL::Cape => ("CAPE", "surface".to_string(), 0.0),
                    FL::Srh => (
                        "HLCY",
                        format!("{}000-0 m above ground", srh_km),
                        f64::NEG_INFINITY,
                    ),
                    _ => ("REFC", "entire atmosphere".to_string(), -30.0),
                };
                let fc = wxdata::hrrr::fetch_field(http, model, var, &level, 0, min_valid).await?;
                OverlayMsg::Field(layer, fc.field)
            }
            OverlaySource::L3Grid(layer, site) => {
                use crate::render::FieldLayer as FL;
                let field = match layer {
                    FL::Vil => wxdata::level3::fetch_dvl(http, &site).await,
                    FL::EchoTops => wxdata::level3::fetch_eet(http, &site).await,
                    FL::Hca => wxdata::level3::fetch_hhc(http, &site).await,
                    _ => None,
                };
                match field {
                    Some(f) => OverlayMsg::Field(layer, f),
                    None => anyhow::bail!("no L3 grid for {site}"),
                }
            }
            OverlaySource::Snow(hours) => OverlayMsg::Field(
                crate::render::FieldLayer::SnowAnalysis,
                wxdata::nohrsc::fetch(http, hours).await?,
            ),
            OverlaySource::FreezingLevels(lon, lat) => {
                // HRRR carries both isotherm heights as analysis fields, so the hail algorithm
                // sources its own thermodynamics instead of asking the user for a freezing level.
                let h0 = wxdata::hrrr::fetch_field(
                    http,
                    wxdata::hrrr::Model::Hrrr,
                    "HGT",
                    "0C isotherm",
                    0,
                    f64::NEG_INFINITY,
                )
                .await?;
                // 253 K is −20.15 °C — the level Witt's hail weighting tops out at.
                let hm20 = wxdata::hrrr::fetch_field(
                    http,
                    wxdata::hrrr::Model::Hrrr,
                    "HGT",
                    "253 K level",
                    0,
                    f64::NEG_INFINITY,
                )
                .await?;
                match (
                    h0.field.sample_bilinear(lon, lat),
                    hm20.field.sample_bilinear(lon, lat),
                ) {
                    (Some(a), Some(b)) => OverlayMsg::FreezingLevels(a as f64, b as f64),
                    _ => anyhow::bail!("no freezing levels at {lon},{lat}"),
                }
            }
            OverlaySource::Obs { site, lat, lon } => {
                let r = wxdata::obs::fetch_nearest(http, lat, lon)
                    .await
                    .map_err(|e| e.to_string());
                OverlayMsg::Obs(site, r)
            }
            OverlaySource::Vwp(site) => {
                let levels = wxdata::level3::fetch_vwp(http, &site).await;
                OverlayMsg::Vwp(site, levels)
            }
            OverlaySource::ArchiveWarnings(bucket) => {
                let ts = chrono::DateTime::from_timestamp(bucket * 300, 0)
                    .unwrap_or_default()
                    .to_rfc3339();
                // An IEM outage caches the bucket empty (self-heals via LRU); log it so a
                // silent "no warnings that day" isn't mistaken for truth.
                let feats = match wxdata::archive_warnings::fetch(http, &ts).await {
                    Ok(f) => f,
                    Err(e) => {
                        log::warn!("archive warnings fetch {ts}: {e} (bucket shown empty)");
                        Vec::new()
                    }
                };
                OverlayMsg::ArchiveWarnings(bucket, feats)
            }
            OverlaySource::Metar(lat0, lon0, lat1, lon1) => {
                let mut obs = wxdata::metar::fetch_bbox(http, lat0, lon0, lat1, lon1).await?;
                // Buoys extend the same layer offshore and over the Great Lakes, where the
                // airport network simply has no stations. A buoy outage must not take the
                // METARs down with it.
                match wxdata::ndbc::fetch_bbox(http, lat0, lon0, lat1, lon1).await {
                    Ok(buoys) => obs.extend(buoys),
                    Err(e) => note_feed_error("Buoy observations", e),
                }
                // Terminal forecasts for the same box, riding along with the obs they belong
                // beside. A TAF outage costs the tooltips their forecast line, nothing more.
                let tafs = match wxdata::metar::fetch_tafs(http, lat0, lon0, lat1, lon1).await {
                    Ok(t) => t,
                    Err(e) => {
                        note_feed_error("Terminal forecasts (TAF)", e);
                        Default::default()
                    }
                };
                OverlayMsg::Metar(obs, tafs)
            }
            OverlaySource::Webcams(min_lon, min_lat, max_lon, max_lat, windy_key) => {
                // Both networks, merged: the FAA is keyless but US-only, Windy covers the rest of
                // the world for anyone who has supplied a key. Nothing for the user to choose.
                let mut sites =
                    wxdata::webcams::fetch_bbox(http, min_lon, min_lat, max_lon, max_lat)
                        .await
                        .unwrap_or_else(|e| {
                            note_feed_error("FAA webcams", e);
                            Vec::new()
                        });
                if !windy_key.is_empty() {
                    // A bad or throttled key must not take the FAA cameras down with it.
                    match wxdata::webcams::fetch_windy_bbox(
                        http, &windy_key, min_lon, min_lat, max_lon, max_lat,
                    )
                    .await
                    {
                        Ok(w) => sites.extend(w),
                        Err(e) => note_feed_error("Windy webcams", e),
                    }
                }
                OverlayMsg::Webcams(sites)
            }
            OverlaySource::Fires(w, s, e, n) => {
                // Two servers; either can be down without taking the other's layer with it.
                let bbox = [w, s, e, n];
                let perims = wxdata::wfigs::fetch_perimeters(http, bbox)
                    .await
                    .unwrap_or_else(|e| {
                        log::warn!("wfigs perimeters: {e}");
                        Vec::new()
                    });
                let incidents = wxdata::wfigs::fetch_incidents(http, bbox)
                    .await
                    .unwrap_or_else(|e| {
                        log::warn!("wfigs incidents: {e}");
                        Vec::new()
                    });
                OverlayMsg::Fires(perims, incidents)
            }
            OverlaySource::Aqi(w, s, e, n, key) => {
                OverlayMsg::Aqi(wxdata::airnow::fetch_bbox(http, &key, [w, s, e, n]).await?)
            }
            OverlaySource::Stations {
                bbox,
                center,
                tempest,
                wu,
                synoptic,
            } => {
                // METARs come first and cost one request; the keyed networks add themselves.
                let metars = wxdata::metar::fetch_bbox(http, bbox.0, bbox.1, bbox.2, bbox.3)
                    .await
                    .unwrap_or_default();
                OverlayMsg::Stations(
                    wxdata::stations::fetch_all(
                        http, &metars, &tempest, &wu, &synoptic, center.0, center.1,
                    )
                    .await,
                )
            }
            OverlaySource::Ppef => OverlayMsg::Ppef(wxdata::efield::fetch_ppef(http).await?),
            OverlaySource::DotCams(min_lon, min_lat, max_lon, max_lat) => OverlayMsg::DotCams(
                wxdata::dotcams::fetch_bbox(http, min_lon, min_lat, max_lon, max_lat).await?,
            ),
            OverlaySource::Mill(url) => {
                let mut r = wxdata::efield::fetch_mill(http, &url).await?;
                r.sort_by_key(|x| x.time);
                OverlayMsg::Mill(r.last().map(|x| x.kv_per_m).unwrap_or(0.0))
            }
            OverlaySource::Dat(bbox, day) => {
                // A survey is filed against the storm's local date, which can be either side of the
                // UTC one for an evening event — ask for the day either side and let the bbox and
                // the map do the rest.
                let start = day
                    .pred_opt()
                    .unwrap_or(day)
                    .and_hms_opt(0, 0, 0)
                    .unwrap_or_default()
                    .and_utc();
                let end = start + chrono::Duration::days(3);
                let (points, tracks) = wxdata::dat::fetch(http, bbox, start, end).await?;
                OverlayMsg::Dat(points, tracks)
            }
            OverlaySource::Mosaic(sites) => {
                let m = wxdata::mosaic::fetch(http, &sites)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("no radar mosaic for {sites:?}"))?;
                OverlayMsg::Mosaic(m.field, m.sites, m.oldest)
            }
            OverlaySource::Gauges(lat0, lon0, lat1, lon1) => {
                OverlayMsg::Gauges(wxdata::river::fetch_bbox(http, lat0, lon0, lat1, lon1).await?)
            }
            OverlaySource::Contours(kind, model) => {
                // Composite parameters (STP/SCP/EHI) combine several same-run HRRR fields.
                if let Some(sk) = kind.severe() {
                    let fc = wxdata::severe::fetch_grid(http, model, sk).await?;
                    let valid = fc.valid();
                    let lines = wxdata::contour::contour_lines(&fc.field, kind.severe_interval());
                    return Ok(OverlayMsg::Contours(kind, lines, valid));
                }
                let (var, level, interval) = kind
                    .params()
                    .ok_or_else(|| anyhow::anyhow!("contour Off"))?;
                let mut fc =
                    wxdata::hrrr::fetch_field(http, model, var, level, 0, f64::NEG_INFINITY)
                        .await?;
                // Convert to display units so the interval is in hPa / °F / etc, then contour off-thread.
                for v in &mut fc.field.values {
                    if v.is_finite() {
                        *v = kind.to_display(*v);
                    }
                }
                let valid = fc.valid();
                OverlayMsg::Contours(
                    kind,
                    wxdata::contour::contour_lines(&fc.field, interval),
                    valid,
                )
            }
            OverlaySource::Outages => OverlayMsg::Outages(wxdata::outages::fetch(http).await?),
            OverlaySource::Tropical(wind_kt, surge) => OverlayMsg::Tropical(
                wxdata::tropical::fetch_active_opts(http, wind_kt, surge).await?,
            ),
            OverlaySource::Wind(level, fh) => {
                let (run, u, v) = wxdata::hrrr::fetch_wind(http, level, fh).await?;
                OverlayMsg::Wind(Box::new(crate::wind_draw::WindField {
                    u,
                    v,
                    level,
                    run,
                    fcst_hour: fh,
                }))
            }
            OverlaySource::Aviation => {
                let mut f = wxdata::aviation::fetch_airsigmet(http).await?;
                // G-AIRMETs ride with the SIGMETs: same layer, same question, and a failure
                // fetching them must not cost the SIGMETs that already arrived.
                match wxdata::aviation::fetch_gairmet(http).await {
                    Ok(g) => f.extend(g),
                    Err(e) => log::warn!("g-airmet: {e}"),
                }
                OverlayMsg::Aviation(f)
            }
            OverlaySource::Tfr(have) => {
                let (new, remaining) = wxdata::tfr::fetch(http, &have, TFR_BATCH).await?;
                OverlayMsg::Tfr(new, remaining)
            }
        })
    }
}

/// One freehand annotation stroke: a lon/lat polyline and the colour it was drawn in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stroke2d {
    pub points: Vec<[f64; 2]>,
    pub color: egui::Color32,
}

/// The four annotation colours, picked to stay legible over both radar and satellite basemaps.
pub(crate) const DRAW_COLORS: [egui::Color32; 4] = [
    egui::Color32::from_rgb(255, 80, 80),
    egui::Color32::from_rgb(255, 215, 60),
    egui::Color32::from_rgb(90, 220, 255),
    egui::Color32::WHITE,
];

/// Append `pt` to the newest stroke, or start one in `color` when `new_stroke`.
pub(crate) fn draw_append(
    strokes: &mut Vec<Stroke2d>,
    pt: [f64; 2],
    color: egui::Color32,
    new_stroke: bool,
) {
    if new_stroke || strokes.is_empty() {
        strokes.push(Stroke2d {
            points: vec![pt],
            color,
        });
        return;
    }
    let last = strokes.last_mut().expect("checked non-empty");
    // Skip points the previous one already covers — a 60 fps drag would otherwise pile up
    // thousands of coincident vertices.
    if last.points.last() != Some(&pt) {
        last.points.push(pt);
    }
}

/// What a left-click on the map does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum MapTool {
    /// Interrogate storm cells / overlay features (the default).
    #[default]
    Interrogate,
    /// Measure great-circle distance/bearing between two clicks.
    Measure,
    /// Drop a location marker at the clicked point.
    Marker,
    /// Draw a two-click line, then reconstruct a vertical cross-section along it.
    CrossSection,
    /// Click a point to pull an HRRR point sounding (Skew-T / hodograph).
    Sounding,
    /// Click to set your position for chase mode (follow-me + nearest-radar handoff).
    Chase,
    /// Click a point for the plain NWS forecast there (7-day + hourly).
    Forecast,
    /// Click a point to list historical tornado tracks near it (SPC climatology).
    Climatology,
    /// Freehand annotation: drag to draw a line on the map (session-only).
    Draw,
    /// Click out a watch zone vertex by vertex; double-click (or Enter) closes and names it.
    AlertZone,
}

/// HRRR model field drawn as contour lines over the radar (surface `f00`). SB-CAPE / 0-3 km SRH
/// are fixed here — `// ponytail: not wired to the env suite's env_cape_ml / env_srh_km toggles.`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum ContourKind {
    #[default]
    Off,
    Mslp,
    T2m,
    Td2m,
    Cape,
    Srh,
    /// Significant Tornado Parameter (composite of several HRRR fields — see `wxdata::severe`).
    Stp,
    /// Supercell Composite Parameter.
    Scp,
    /// Energy-Helicity Index, 0-1 km.
    Ehi,
    /// 700–500 hPa lapse rate (°C/km).
    Lapse700500,
    /// 850–500 hPa lapse rate (°C/km).
    Lapse850500,
    /// Effective bulk wind difference (kt).
    EffShear,
    /// Effective storm-relative helicity (m²/s²).
    EffSrh,
    /// STP in its effective-layer form — the one SPC mesoanalysis draws.
    StpEff,
}

impl ContourKind {
    pub(crate) const ALL: [ContourKind; 14] = [
        ContourKind::Off,
        ContourKind::Mslp,
        ContourKind::T2m,
        ContourKind::Td2m,
        ContourKind::Cape,
        ContourKind::Srh,
        ContourKind::Stp,
        ContourKind::Scp,
        ContourKind::Ehi,
        ContourKind::Lapse700500,
        ContourKind::Lapse850500,
        ContourKind::EffShear,
        ContourKind::EffSrh,
        ContourKind::StpEff,
    ];

    /// The composite parameters, which combine several GRIB fields instead of drawing one.
    pub(crate) fn severe(self) -> Option<wxdata::severe::SevereKind> {
        use wxdata::severe::SevereKind as S;
        Some(match self {
            ContourKind::Stp => S::Stp,
            ContourKind::Scp => S::Scp,
            ContourKind::Ehi => S::Ehi,
            ContourKind::Lapse700500 => S::Lapse700500,
            ContourKind::Lapse850500 => S::Lapse850500,
            ContourKind::EffShear => S::EffShear,
            ContourKind::EffSrh => S::EffSrh,
            ContourKind::StpEff => S::StpEff,
            _ => return None,
        })
    }

    /// Contour interval in display units (composites only; single fields carry theirs in `params`).
    pub(crate) fn severe_interval(self) -> f32 {
        match self {
            ContourKind::Stp | ContourKind::StpEff => 0.5,
            ContourKind::Scp => 2.0,
            // °C/km: 0.5 resolves the 7-8 °C/km band steep-lapse-rate plumes live in.
            ContourKind::Lapse700500 | ContourKind::Lapse850500 => 0.5,
            ContourKind::EffShear => 10.0, // kt
            ContourKind::EffSrh => 100.0,  // m²/s²
            _ => 1.0,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            ContourKind::Off => "Off",
            ContourKind::Mslp => "MSLP",
            ContourKind::T2m => "2 m temp",
            ContourKind::Td2m => "2 m dewpoint",
            ContourKind::Cape => "SB-CAPE",
            ContourKind::Srh => "0-3 km SRH",
            ContourKind::Stp => "STP (fixed)",
            ContourKind::Scp => "SCP",
            ContourKind::Ehi => "EHI 0-1 km",
            ContourKind::Lapse700500 => "700-500 lapse",
            ContourKind::Lapse850500 => "850-500 lapse",
            ContourKind::EffShear => "Eff. bulk shear",
            ContourKind::EffSrh => "Eff. SRH",
            ContourKind::StpEff => "STP (effective)",
        }
    }

    /// Parse a headless CLI token (`mslp|t2m|td2m|cape|srh`) into a kind.
    pub(crate) fn from_token(s: &str) -> Option<ContourKind> {
        Some(match s {
            "mslp" => ContourKind::Mslp,
            "t2m" => ContourKind::T2m,
            "td2m" => ContourKind::Td2m,
            "cape" => ContourKind::Cape,
            "srh" => ContourKind::Srh,
            "stp" => ContourKind::Stp,
            "scp" => ContourKind::Scp,
            "ehi" => ContourKind::Ehi,
            "lapse700" => ContourKind::Lapse700500,
            "lapse850" => ContourKind::Lapse850500,
            "ebwd" => ContourKind::EffShear,
            "esrh" => ContourKind::EffSrh,
            "stpeff" => ContourKind::StpEff,
            _ => return None,
        })
    }

    /// GRIB `(var, level, contour interval)` in display units, or `None` for `Off`.
    pub(crate) fn params(self) -> Option<(&'static str, &'static str, f32)> {
        match self {
            ContourKind::Off => None,
            ContourKind::Mslp => Some(("MSLMA", "mean sea level", 2.0)), // hPa
            ContourKind::T2m => Some(("TMP", "2 m above ground", 5.0)),  // °F
            ContourKind::Td2m => Some(("DPT", "2 m above ground", 5.0)), // °F
            ContourKind::Cape => Some(("CAPE", "surface", 500.0)),       // J/kg
            ContourKind::Srh => Some(("HLCY", "3000-0 m above ground", 100.0)), // m²/s²
            // Composites are built from several fields — see `severe()` / `severe_interval()`.
            ContourKind::Stp
            | ContourKind::Scp
            | ContourKind::Ehi
            | ContourKind::Lapse700500
            | ContourKind::Lapse850500
            | ContourKind::EffShear
            | ContourKind::EffSrh
            | ContourKind::StpEff => None,
        }
    }

    /// Convert a raw GRIB value to the display unit the interval is expressed in.
    pub(crate) fn to_display(self, raw: f32) -> f32 {
        match self {
            ContourKind::Mslp => raw / 100.0, // Pa → hPa
            ContourKind::T2m | ContourKind::Td2m => raw * 9.0 / 5.0 - 459.67, // K → °F
            _ => raw,                         // CAPE / SRH as-is
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            ContourKind::Mslp => egui::Color32::from_rgb(235, 235, 235),
            ContourKind::T2m => egui::Color32::from_rgb(240, 120, 60),
            ContourKind::Td2m => egui::Color32::from_rgb(90, 200, 120),
            ContourKind::Cape => egui::Color32::from_rgb(240, 160, 40),
            ContourKind::Srh => egui::Color32::from_rgb(190, 110, 230),
            ContourKind::Stp => egui::Color32::from_rgb(230, 60, 90),
            ContourKind::Scp => egui::Color32::from_rgb(250, 120, 50),
            ContourKind::Ehi => egui::Color32::from_rgb(150, 110, 235),
            ContourKind::Lapse700500 => egui::Color32::from_rgb(240, 200, 90),
            ContourKind::Lapse850500 => egui::Color32::from_rgb(210, 170, 70),
            ContourKind::EffShear => egui::Color32::from_rgb(120, 190, 250),
            ContourKind::EffSrh => egui::Color32::from_rgb(200, 130, 240),
            ContourKind::StpEff => egui::Color32::from_rgb(250, 70, 110),
            ContourKind::Off => egui::Color32::WHITE,
        }
    }
}

/// A boolean overlay/panel toggle addressable by name, so the layers panel and command palette
/// can flip any of them without a match arm per surface (see [`HookEchoApp::overlay_flag`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum OverlayToggle {
    AlertPanel,
    StormReports,
    Spotters,
    RadarSites,
    Metar,
    Webcams,
    Fires,
    Aqi,
    Stations,
    Dat,
    Gauges,
    Tropical,
    /// County power outages (ODIN).
    Outages,
    ProbSevere,
    Aviation,
    Tfr,
    RangeRings,
    Sensors,
    Hodo,
    Cells,
    Tracks,
    ArrivalCones,
    Nowcast,
    Tds,
    Couplets,
    Tbss,
    ZdrColumns,
    Alerts,
    Mds,
    Mping,
    Pireps,
    Recon,
    Fronts,
    /// SPC tornado and severe thunderstorm watches in effect.
    Watches,
    /// Cell tracks computed here from reflectivity, for sites with no Level 3 SCIT product.
    LocalTracks,
    GlmLightning,
    /// Ground strikes republished onto the user's own MQTT broker (see `strikes_topic`).
    Strikes,
    Wind,
    LinkCameras,
    /// The always-on-top mini-loop window (desktop only).
    MiniLoop,
    /// Beam-vs-terrain blockage shading for the displayed tilt (chase mode).
    Blockage,
}

/// What a computed blockage raster was built for. Any change here (site, tilt, or a pan/zoom past
/// the quantization) makes the resident raster stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockageKey {
    site: String,
    /// Tilt elevation in millidegrees (an integer so the key can be compared).
    tilt_mdeg: i32,
    /// The world-space rect, in units of 1e-7 world (~2 m at the equator).
    world: [i64; 4],
}

impl OverlayToggle {
    /// Every toggle, for the persistence sweep. A new variant belongs here too, or it silently
    /// stops being remembered across restarts.
    pub(crate) const ALL: [OverlayToggle; 41] = [
        Self::AlertPanel,
        Self::StormReports,
        Self::Spotters,
        Self::RadarSites,
        Self::Metar,
        Self::Webcams,
        Self::Fires,
        Self::Aqi,
        Self::Stations,
        Self::Dat,
        Self::Gauges,
        Self::Tropical,
        Self::Outages,
        Self::ProbSevere,
        Self::Aviation,
        Self::Tfr,
        Self::RangeRings,
        Self::Sensors,
        Self::Hodo,
        Self::Cells,
        Self::Tracks,
        Self::ArrivalCones,
        Self::Nowcast,
        Self::Tds,
        Self::Couplets,
        Self::Tbss,
        Self::ZdrColumns,
        Self::Alerts,
        Self::Mds,
        Self::Mping,
        Self::Pireps,
        Self::Recon,
        Self::Fronts,
        Self::Watches,
        Self::LocalTracks,
        Self::GlmLightning,
        Self::Strikes,
        Self::Wind,
        Self::LinkCameras,
        Self::MiniLoop,
        Self::Blockage,
    ];

    /// Toggles that describe this session's window arrangement rather than a layer: camera
    /// linking is about the panes on screen right now, and the mini loop is a window. Neither is
    /// persisted or captured into a workspace.
    pub(crate) fn session_only(self) -> bool {
        matches!(self, Self::LinkCameras | Self::MiniLoop)
    }

    /// Stable name used in the settings file. Persisted as a string, not as the enum: an unknown
    /// name written by a newer build has to be skippable, and a failed `Settings` parse takes the
    /// whole file down with it.
    pub(crate) fn slug(self) -> String {
        // The variant name, which is also the serde name — one list of names, not two.
        format!("{self:?}")
    }

    pub(crate) fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.slug() == s)
    }
}

/// A floating window the palette can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum AppWindow {
    Site,
    Settings,
    Markers,
    Placefiles,
    Palettes,
    Events,
    /// Replay a recorded drive against the archive.
    ChaseReplay,
    Digest,
    Afd,
    Cappi,
    Volume3d,
    StormTable,
    /// Shortcuts, vocabulary, the tour, and what changed — one searchable page.
    #[serde(alias = "Glossary")]
    Help,
    /// The user's own alert rules.
    AlertRules,
    /// Warning verification lab (IEM Cow): how the office's warnings scored on an event day.
    Verify,
    Climatology,
    LayerManager,
    /// First-run setup: which radar to open to.
    #[serde(alias = "Wizard")]
    Setup,
    /// The spotlight tour of the live chrome.
    Tour,
    About,
}

/// One thing the user can do, addressable from any surface (layers panel, command palette,
/// mobile quick-layers sheet). The single registry keeps those surfaces in sync for free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum PaletteAction {
    /// Select a radar moment; the bool is the storm-relative flag (velocity only).
    SetMoment(Moment, bool),
    /// Four panes, one product, four distinct tilts, cameras linked.
    AllTilts,
    ToggleField(crate::render::FieldLayer),
    ToggleOverlay(OverlayToggle),
    SetContours(ContourKind),
    Tool(MapTool),
    OpenWindow(AppWindow),
    SetPanes(usize),
    CycleBasemap,
    ToggleMute,
    /// Show/hide the docked timeline bar under the map (desktop).
    /// Show/hide the docked sidebar on the left (desktop).
    TogglePanel,
    Reload,
    InstantReplay,
    GoLive,
    /// Hand the current view off to windy.com in the browser.
    OpenInWindy,
    /// Copy a `hookecho://goto/…` link to this view (site, center, zoom, archive time).
    CopyViewLink,
    /// Open Help at the glossary entry that explains a label's abbreviation. An index into
    /// `ui::glossary::ENTRIES` rather than the term itself, so the action stays `Copy`.
    Explain(usize),
    /// Snapshot the current pane layout as a new workspace.
    SaveWorkspace,
    /// Restore the saved workspace at this index (an index, not the workspace itself, so the enum
    /// stays `Copy` and the palette rows stay cheap).
    ApplyWorkspace(usize),
}

/// A placefile label/marker the egui painter draws over the map.
pub(crate) struct PlaceLabel {
    pub color: egui::Color32,
    pub pos: [f64; 2],
    /// `Object:` anchor: when set, `pos` is a pixel offset from this point rather than a position.
    pub anchor: Option<[f64; 2]>,
    pub hover: String,
    pub kind: PlaceLabelKind,
}

/// What a [`PlaceLabel`] draws.
pub(crate) enum PlaceLabelKind {
    Text(String),
    /// An icon with no usable sheet (none declared, or the image hasn't loaded): ring + dot.
    Marker,
    /// One cell of a loaded icon sheet, rotated `angle` degrees clockwise.
    Sprite {
        tex: egui::TextureId,
        uv: egui::Rect,
        size: egui::Vec2,
        hot: egui::Vec2,
        angle: f32,
    },
}

/// How loud a [`Toast`] is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Error,
}

/// Auxiliary-feed failures waiting to be told to the user.
///
/// A global rather than a channel: these happen in half a dozen spawned tasks across three
/// different message enums, and none of them owns a sender. The app drains it once a frame.
///
/// ponytail: unbounded and process-wide, which is fine for something drained every frame and
/// gated to one toast per feed. A sender per task is the upgrade if it ever needs ordering
/// against the other messages.
static FEED_ERRORS: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());

/// How many undrained feed errors are kept. More than fits on screen, far less than a failing
/// network produces over an afternoon.
const MAX_FEED_ERRORS: usize = 64;

/// Log an auxiliary feed's failure and queue it for the user.
///
/// "Auxiliary" means a feed whose failure leaves the rest of its layer standing — buoys inside
/// the station plot, Windy inside the webcams. Those used to fail into the log alone, which is
/// indistinguishable from the feed simply having nothing to show.
pub(crate) fn note_feed_error(feed: &'static str, err: impl std::fmt::Display) {
    log::warn!("{feed}: {err}");
    if let Ok(mut q) = FEED_ERRORS.lock() {
        // Bounded because the drain is per frame and frames are not guaranteed: a hidden browser
        // tab does not draw, and a network that is failing produces one of these per feed per
        // refresh. The newest are the ones worth showing when the frames come back.
        if q.len() >= MAX_FEED_ERRORS {
            q.remove(0);
        }
        q.push((feed, err.to_string()));
    }
}

/// A short-lived note about something the user just did.
pub(crate) struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

/// A registry row: what it's called, where it lives, what it does, and (for toggles) its state.
#[derive(Clone)]
pub(crate) struct PaletteEntry {
    pub label: String,
    pub category: &'static str,
    pub action: PaletteAction,
    pub on: Option<bool>,
    /// One line of plain English. Jargon labels ("AzShear (0–2 km)") mean nothing on their own.
    pub desc: &'static str,
    /// Shown before the "Show all" expander. Everything else is one click further in — never gone.
    pub common: bool,
    /// The key bound to this action, if any — drawn as a chip on the row so the shortcut is
    /// learnable from the place you already click.
    pub key: Option<String>,
}

/// Refresh cadence (seconds) for a national field layer's product.
fn field_refresh_secs(layer: crate::render::FieldLayer) -> u64 {
    use crate::render::FieldLayer as FL;
    match layer {
        FL::Lightning | FL::AzShear => 60,
        FL::Mrms | FL::Mesh | FL::Rotation | FL::Hrrr | FL::Mosaic => 120,
        // QPE accumulations update on a ~2-minute MRMS cadence.
        // The rate product lands every 2 minutes; the accumulations move far more slowly.
        FL::PrecipRate => 120,
        FL::Qpe1h | FL::Qpe24h => 120,
        // MRMS precip type / flash-flood ARI on the ~2-min cadence; L3 grids on the 120 s L3 cadence.
        FL::PrecipType | FL::FlashFlood | FL::Vil | FL::EchoTops | FL::Hca => 120,
        // Bands are cut from the ~2-min mosaic, so they are as fresh as it is.
        FL::SnowBands => 120,
        FL::UpdraftHelicity => 600,
        // Snowfall accumulates over a whole model run; it moves as slowly as the run does.
        FL::Snowfall => 600,
        // The analysis is reissued four times a day; half an hour is plenty.
        FL::SnowAnalysis => 1800,
        // Global cycles are six hours apart and take hours to post. Half an hour is generous.
        FL::GlobalMslp
        | FL::GlobalHeight500
        | FL::GlobalTemp2m
        | FL::GlobalDewpoint2m
        | FL::GlobalWind10m
        | FL::GlobalPrecip
        // Two global cycles behind it, so the same half hour.
        | FL::ModelDiff => 1800,
        FL::Smoke => 900,
        // NBM posts hourly; the blend moves no faster than that.
        FL::ThunderProb => 900,
        // An accumulation moves slower than the grid it accumulates, whatever the window.
        FL::HailSwath => 300,
        // Environment (HRRR CAPE/SRH) refreshes slowly — 15 min.
        FL::Cape | FL::Srh => 900,
        // Derived products cost no network: they recompute when the volume does, not on a clock.
        FL::CompositeLocal
        | FL::VilLocal
        | FL::VilDensity
        | FL::EtopLocal
        | FL::HailMehs
        | FL::HailPosh => 60,
        // Gridded from the GLM feed the app already polls every 20 s; regridding is local work.
        FL::GlmFed => 60,
    }
}

/// The ZDR-column cache: the volume it was computed for, its columns, and the bright band the
/// same pass found.
/// A place the proximity alerts watch: a saved marker, or wherever the GPS says you are.
struct WatchedPoint {
    /// The marker's stable id; cooldowns key on this, never on the name.
    id: String,
    name: String,
    lon: f64,
    lat: f64,
    radius_mi: f64,
}

/// Cooldown key for the follow-GPS pseudo-marker, which has no entry in the settings.
const GPS_POINT_ID: &str = "gps";

/// A volume key with the detector thresholds folded in, so moving a slider recomputes instead of
/// handing back the answer to the previous question.
type TunedKey = ((usize, String, usize), u64);

type ZdrCache = (
    TunedKey,
    Vec<wxdata::dualpol::ZdrColumnHit>,
    Option<wxdata::dualpol::BrightBand>,
);

/// Per-field-layer UI + fetch state (toggle, pending upload, refresh clock).
#[derive(Default)]
pub(crate) struct FieldState {
    pub pending: Option<crate::render::MrmsUpload>,
    pub last_fetch: Option<Instant>,
    /// Since when no pane has drawn this layer. Its GPU texture (up to 8192 px of R8) is freed
    /// after [`FIELD_EVICT`]; before this, thirty-five layers could stay resident until exit.
    pub off_since: Option<Instant>,
}

/// How long a field layer stays uploaded after the last pane turns it off. Long enough that
/// toggling a layer to compare it against another doesn't re-fetch, short enough that an
/// afternoon of browsing doesn't end with every layer's texture still on the GPU.
const FIELD_EVICT: std::time::Duration = std::time::Duration::from_secs(300);

/// What a settings-sync worker reports back. Everything network lives on the runtime; the app
/// thread only ever applies the outcome.
enum SyncMsg {
    /// Fresh tokens (from finishing a sign-in, or from a refresh mid-sync).
    Signed(crate::cloud::Tokens),
    /// The remote settings blob, to merge over the local one.
    Pulled {
        body: String,
        modified: String,
    },
    /// Our settings are now the remote ones, as of this `modifiedTime`.
    Pushed {
        modified: String,
        hash: u64,
    },
    /// Both sides had edits. We took the remote copy; say so rather than pretend.
    Conflict,
    UpToDate,
    Error(String),
}

/// Where a pending screenshot is delivered: saved to a file, or copied to the clipboard, or
/// captured as one frame of a loop-GIF export.
enum ShotDest {
    File(std::path::PathBuf),
    Clipboard,
    Loop,
    /// Pushed to the user's ntfy topic as an attachment, captioned with this title.
    Push(String),
    /// Written silently for the Android home-screen widget to pick up.
    Widget(std::path::PathBuf),
}

/// In-progress loop export (GIF or MP4): steps the active timeline, grabbing one screenshot per
/// frame.
struct LoopExport {
    dest: std::path::PathBuf,
    format: crate::loopexport::LoopFormat,
    frames: Vec<image::RgbaImage>,
    /// Slots still to capture (counts down as frames are grabbed).
    remaining: usize,
    /// Frames to let the newly-stepped radar settle/load before grabbing.
    settle: u8,
    /// A screenshot has been requested; waiting for its event.
    capturing: bool,
    /// Playback speed the scrubber was set to when the export started — the exported clip plays
    /// at the speed the user was watching, instead of a hardcoded 5 fps.
    fps: f32,
}

/// A placefile the app has fetched and is tracking (mirrors a `PlacefileConfig` by URL).
/// What the memoised placefile labels depend on: (placefile item/enabled/icon fingerprint,
/// minute, view range in nmi).
type PlaceLabelKey = (usize, i64, i32);

struct LoadedPlacefile {
    /// A URL, or the synthetic `plugin:<name>` key for a plugin-produced overlay.
    url: String,
    enabled: bool,
    pf: wxdata::placefile::Placefile,
    last_fetch: Option<Instant>,
    loaded: bool,
    /// Why the last load failed, if it did.
    error: Option<String>,
}

/// A background fetch result routed back to a specific view.
/// Loop frames a phone keeps decoded at once. Each volume is tens of MB; a longer loop than this
/// pushes the process into the range Android kills.
#[cfg(target_os = "android")]
const ANDROID_LOOP_WINDOW: usize = 6;
#[cfg(not(target_os = "android"))]
const ANDROID_LOOP_WINDOW: usize = 6;

/// Frame downloads allowed in flight at once, on top of the head poll and the frame being shown.
///
/// A phone (or a browser tab) on a cell radio gets nothing from a deeper queue: the frames arrive
/// in the same total time and each one lands later than it would have with a queue behind it.
const MAX_PREFETCH_INFLIGHT: usize = if cfg!(target_os = "android") || cfg!(target_arch = "wasm32")
{
    2
} else {
    4
};

/// Which frames to pull in around the playhead, nearest first.
///
/// Playing only ever moves forward, so it looks ahead. Scrubbing can go either way and usually
/// reverses, so it takes one behind as well — that one is what makes dragging the timeline back a
/// frame feel instant instead of costing a fresh download.
fn prefetch_offsets(playing: bool) -> &'static [isize] {
    if playing {
        &[1, 2, 3]
    } else {
        &[1, -1, 2, -2]
    }
}

/// Loop frames the browser build keeps decoded at once — "the last fifteen minutes", which at a
/// severe-weather VCP is four volumes. A wasm heap is 32-bit and a decoded volume is tens of MB,
/// so this is a memory budget as much as a time window.
const WEB_LOOP_WINDOW: usize = 4;

enum DataMsg {
    Volume {
        view: usize,
        site: String,
        name: String,
        time: DateTime<Utc>,
        scan: Scan,
    },
    /// A live sweep-boundary update (merged full volume) from the chunk streamer.
    Live {
        view: usize,
        site: String,
        name: String,
        time: DateTime<Utc>,
        /// Already shared with the streaming task's running volume (see `live::Update`).
        scan: Arc<Scan>,
        changed: Vec<f32>,
    },
    /// The live stream for `view` ended (error or clean exit); polling resumes.
    LiveEnded {
        view: usize,
        site: String,
        /// Stream generation — a stale end must not clear a newer stream's handle.
        gen: u64,
    },
    /// The archive volume listing for a site+date (timeline frames).
    Frames {
        view: usize,
        site: String,
        date: NaiveDate,
        frames: Vec<Identifier>,
    },
    UpToDate {
        view: usize,
        site: String,
    },
    /// A loop frame fetched ahead of the playhead. Goes into the scan cache and nowhere else —
    /// showing it would jump the display forward.
    Prefetched {
        view: usize,
        site: String,
        name: String,
        scan: Scan,
    },
    Error {
        view: usize,
        site: String,
        err: String,
    },
}

impl DataMsg {
    fn view(&self) -> usize {
        match self {
            DataMsg::Volume { view, .. }
            | DataMsg::Live { view, .. }
            | DataMsg::LiveEnded { view, .. }
            | DataMsg::Frames { view, .. }
            | DataMsg::UpToDate { view, .. }
            | DataMsg::Prefetched { view, .. }
            | DataMsg::Error { view, .. } => *view,
        }
    }
    fn site(&self) -> &str {
        match self {
            DataMsg::Volume { site, .. }
            | DataMsg::Live { site, .. }
            | DataMsg::LiveEnded { site, .. }
            | DataMsg::Frames { site, .. }
            | DataMsg::UpToDate { site, .. }
            | DataMsg::Prefetched { site, .. }
            | DataMsg::Error { site, .. } => site,
        }
    }
}

/// What is currently uploaded to the GPU, so we only re-bin/re-upload on a real change.
/// The trailing option is the storm-motion (east, north) m/s for storm-relative velocity.
///
/// The palette generation is deliberately *not* in here — it rides alongside in `pane_lut`, so
/// a color-table change re-bakes the 3 KB LUT without re-binning or re-uploading the sweep.
type ShownKey = (
    String,
    Moment,
    usize,
    Option<f32>,
    bool,
    Option<(u32, u32)>,
    bool,
    // Precipitation-tint generation: `None` when the tint is off, else the grid revision, so a
    // new precipitation-type grid or toggling the tint rebuilds the image.
    Option<u32>,
);

/// An in-progress offline chase-pack download: the worker outcome channel, a cancel flag the
/// workers poll, and running tallies for the Map ▸ offline-pack progress bar.
struct ChasePack {
    rx: Receiver<(bool, u64)>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    total: u64,
    done: u64,
    errors: u64,
    bytes: u64,
}

/// Build a windy.com permalink for a map position.
///
/// Grammar, from Windy's own URL-parameters documentation and matching the share links their
/// satellite view produces: `https://www.windy.com/?{overlay},{lat},{lon},{zoom}`. `lat,lon,zoom`
/// are required and must appear in that order; the overlay is optional and goes first. Two rules
/// worth keeping: **latitude comes before longitude** (the opposite of this codebase's own
/// `(lon, lat)` convention, which is exactly how that gets written backwards), and coordinates
/// must carry a decimal part or Windy ignores them.
///
/// Windy's zoom tops out at 18 and its overlay names are its own — `radar`, `satellite` and `cape`
/// all resolve, alongside the documented `wind`, `temp`, `rain` and friends.
fn windy_url(overlay: &str, lon: f64, lat: f64, zoom: f64) -> String {
    let z = zoom.round().clamp(3.0, 18.0) as u32;
    format!("https://www.windy.com/?{overlay},{lat:.3},{lon:.3},{z}")
}

/// `?embed` in the query string: this build is a chromeless pane inside another page's iframe.
/// Read once at startup — nobody flips it at runtime.
#[cfg(target_arch = "wasm32")]
fn is_embed() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|s| s.trim_start_matches('?').split('&').any(|p| p == "embed"))
}

#[cfg(not(target_arch = "wasm32"))]
fn is_embed() -> bool {
    false
}

/// URL scheme for a shared view. One parser serves all three ways a view arrives: the
/// `HOOKECHO_GOTO` env var, the `goto.txt` the Android notification tap writes (which uses the
/// site-less `,lon,lat,zoom` form), and a tapped `hookecho://goto/…` link.
const GOTO_SCHEME: &str = "hookecho://goto/";

/// A parsed deep link. `moment`/`tilt` stay `None` unless the link named them, so a link that
/// only says where to look leaves the product the viewer already had.
struct Goto {
    site: String,
    lon: f64,
    lat: f64,
    zoom: f64,
    time: Option<DateTime<Utc>>,
    moment: Option<Moment>,
    tilt: Option<usize>,
    basemap: Option<String>,
    /// Outer `None`: the link said nothing about the threshold, so the viewer keeps its own.
    /// Inner `None`: the link said `thr:off`, which turns the threshold off on purpose.
    threshold: Option<Option<f32>>,
    srv: bool,
}

/// Parse `[hookecho://goto/]SITE[,lon,lat,zoom][,extra…]`, where each extra is an RFC3339 time, a
/// moment code (`VEL`), a tilt index, a basemap (`bm:dark`), a threshold (`thr:25` / `thr:off`)
/// or the literal `srv` — sniffed by shape, so their order does not matter. A bare `SITE` flies to the site itself; the site may be
/// empty when lon/lat are given.
fn parse_goto(v: &str) -> Option<Goto> {
    let v = v.trim().strip_prefix(GOTO_SCHEME).unwrap_or(v.trim());
    // Chat clients and mail readers hand the link back percent-encoded, commas and the time's
    // colons included, so decode the whole thing before splitting. No field here can legitimately
    // hold a comma, which is what makes decoding first the safe direction. Anything that isn't a
    // valid escape survives as typed, so a stray `%` doesn't lose the link.
    let decoded = percent_encoding::percent_decode_str(v).decode_utf8_lossy();
    let p: Vec<String> = decoded.split(',').map(|s| s.trim().to_string()).collect();
    let mut g = if p.len() == 1 {
        let s = wxdata::sites::site_by_id(&p[0])?;
        // Same zoom a cold start picks for the default site.
        Goto {
            site: p[0].to_ascii_uppercase(),
            lon: s.longitude as f64,
            lat: s.latitude as f64,
            zoom: 8.0,
            time: None,
            moment: None,
            tilt: None,
            basemap: None,
            threshold: None,
            srv: false,
        }
    } else {
        let (Some(site), Some(Ok(lon)), Some(Ok(lat)), Some(Ok(zoom))) = (
            p.first(),
            p.get(1).map(|s| s.parse()),
            p.get(2).map(|s| s.parse()),
            p.get(3).map(|s| s.parse()),
        ) else {
            return None;
        };
        Goto {
            site: site.to_string(),
            lon,
            lat,
            zoom,
            time: None,
            moment: None,
            tilt: None,
            basemap: None,
            threshold: None,
            srv: false,
        }
    };
    for s in p.iter().skip(4).filter(|s| !s.is_empty()) {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
            g.time = Some(t.with_timezone(&Utc));
        } else if let Some(m) = Moment::from_code(s) {
            g.moment = Some(m);
        } else if let Ok(i) = s.parse::<usize>() {
            g.tilt = Some(i);
        } else if let Some(slug) = s.strip_prefix("bm:") {
            g.basemap = Some(slug.to_string());
        } else if let Some(t) = s.strip_prefix("thr:") {
            // The value is in the moment's own unit, which is what the slider stores: dBZ for
            // reflectivity, m/s for velocity. Not the display unit — a link must mean the same
            // thing whichever Units the recipient has set.
            g.threshold = if t.eq_ignore_ascii_case("off") {
                Some(None)
            } else if let Ok(v) = t.parse::<f32>() {
                Some(Some(v))
            } else {
                log::warn!("goto: want thr:<number> or thr:off, got {s:?}");
                None
            };
        } else if s.eq_ignore_ascii_case("srv") {
            g.srv = true;
        } else {
            log::warn!("goto: ignoring unrecognized field {s:?}");
        }
    }
    Some(g)
}

/// The shareable link for a view. Native gets the `hookecho://` scheme the OS has registered; the
/// browser build gets its own origin with the state in the fragment, which never leaves the client
/// — no server, cache or worker ever sees where someone is looking.
fn goto_link(g: &Goto) -> String {
    let site = &g.site;
    let (lon, lat, zoom) = (g.lon, g.lat, g.zoom);
    let t = g
        .time
        .map(|t| format!(",{}", t.to_rfc3339()))
        .unwrap_or_default();
    // Defaults stay out of the link: a field that isn't there leaves the recipient's own alone,
    // which is the whole contract of the trailing fields.
    let m = match g.moment {
        Some(m) if m != Moment::Reflectivity => format!(",{}", m.short_name()),
        _ => String::new(),
    };
    let z = match g.tilt {
        Some(i) if i != 0 => format!(",{i}"),
        _ => String::new(),
    };
    // Only an active threshold travels. Sharing "no threshold" as `thr:off` would override the
    // recipient's own setting with a default nobody chose.
    let thr = g
        .threshold
        .flatten()
        .map(|v| format!(",thr:{v}"))
        .unwrap_or_default();
    let body = format!("{site},{lon:.4},{lat:.4},{zoom:.1}{t}{m}{z}{thr}");
    #[cfg(target_arch = "wasm32")]
    {
        let origin = web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default();
        format!("{origin}/#goto={body}")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("{GOTO_SCHEME}{body}")
    }
}

/// The credit line a radar network's licence asks for, or `None` for one that asks for nothing
/// (NOAA's WSR-88Ds and TDWRs are public domain). One arm per source, added as sources are.
///
/// ponytail: a function, not a NOTICE file or a licence registry. Attribution belongs on the map
/// beside the data it covers; if a distribution ever needs a bundled notice, generate it from here.
fn data_attribution(site_id: &str) -> Option<&'static str> {
    match wxdata::sites::network(site_id) {
        wxdata::sites::Network::Dwd => Some("Radar data © Deutscher Wetterdienst (DL-DE/BY-2.0)"),
        // ORD publishes every member's data under one licence, and the credit EUMETNET asks for
        // names the network rather than the twenty national services behind it.
        wxdata::sites::Network::Opera => {
            Some("Radar data © EUMETNET OPERA / OpenRadarData (CC BY 4.0)")
        }
        _ => None,
    }
}

/// How a lightning flash looks at `age_secs`: bright white-hot when it just happened, fading to a
/// dim orange ember by the end of the window. The brightness IS the recency cue — a map of
/// same-colored dots says where lightning has been, not where it is now.
/// Most strikes kept at once. A continent-wide topic on an active night runs tens of thousands
/// an hour, and the painter walks the whole deque every frame.
const STRIKE_CAP: usize = 20_000;

/// How long a strike stays on the map. Same window as the GLM feed, so the two layers age alike.
const STRIKE_WINDOW_SECS: i64 = 900;

/// Cyan-white for a fresh strike fading to deep blue, deliberately nothing like [`glm_style`]'s
/// white-to-orange: with both layers on, colour is the only thing telling optical flashes from
/// ground strikes.
fn strike_style(age_secs: f32) -> (egui::Color32, f32) {
    let t = (age_secs / STRIKE_WINDOW_SECS as f32).clamp(0.0, 1.0);
    let r = 3.4 - 1.4 * t;
    let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
    (
        egui::Color32::from_rgba_unmultiplied(
            lerp(225.0, 40.0),
            lerp(250.0, 90.0),
            lerp(255.0, 220.0),
            lerp(255.0, 70.0),
        ),
        r,
    )
}

fn glm_style(age_secs: f32) -> (egui::Color32, f32) {
    const WINDOW: f32 = 900.0; // 15 minutes, matching the feed
    let t = (age_secs / WINDOW).clamp(0.0, 1.0);
    // Newest flashes get a slightly bigger dot so a live storm reads at a glance.
    let r = 3.4 - 1.4 * t;
    let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
    let alpha = lerp(255.0, 70.0);
    (
        egui::Color32::from_rgba_unmultiplied(
            lerp(255.0, 235.0),
            lerp(250.0, 140.0),
            lerp(210.0, 40.0),
            alpha,
        ),
        r,
    )
}

/// The first `want` indices of `elevations` at distinct angles (0.1\u{b0} tolerance), lowest
/// first. SAILS/MRLE repeat the lowest cut mid-volume, so a naive `0..4` yields duplicates.
fn distinct_tilts(elevations: &[f32], want: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(want);
    for (i, &a) in elevations.iter().enumerate() {
        if out.len() == want {
            break;
        }
        if !out.iter().any(|&j| (elevations[j] - a).abs() < 0.1) {
            out.push(i);
        }
    }
    out
}

/// How many buttons the right-edge control column shows — the badge lane stacks below them.
const CONTROL_BUTTONS: usize = 6;

pub struct HookEchoApp {
    /// The native runtime, kept alive for as long as the app is. Work is spawned through
    /// `spawner`, which is the same thing natively and the browser's event loop on the web.
    #[cfg(not(target_arch = "wasm32"))]
    _rt: Runtime,
    spawner: crate::rt::Spawner,
    tiles: TileManager,
    vtiles: crate::vector_tiles::VectorTileManager,
    /// One shared label-occupancy pass for the whole frame — see [`crate::labelplace`].
    labels: crate::labelplace::Placer,
    settings: Settings,
    saved: Settings,
    views: Vec<MapView>,
    active: usize,
    msg_rx: Receiver<DataMsg>,
    msg_tx: Sender<DataMsg>,
    /// Geocode results for the marker window's address search: `(label, lat, lon)` or an error.
    /// About window + the once-per-session release check.
    about_open: bool,
    update_state: ui::about_window::UpdateState,
    /// Update chip dismissed this session (see `chrome::chips::update_chip`).
    update_chip_hidden: bool,
    update_tx: Sender<ui::about_window::UpdateState>,
    update_rx: Receiver<ui::about_window::UpdateState>,
    geocode_tx: Sender<Result<(String, f64, f64), String>>,
    geocode_rx: Receiver<Result<(String, f64, f64), String>>,
    /// `(lon, lat)` from the hosting edge's own geo-IP (browser build only), used once at boot to
    /// open on the nearest radar. Never fed on native, where the saved view is the answer.
    #[cfg(target_arch = "wasm32")]
    ipgeo_tx: Sender<(f64, f64)>,
    ipgeo_rx: Receiver<(f64, f64)>,
    /// In-progress offline chase-pack tile download (basemap pre-cache for the current view).
    chasepack: Option<ChasePack>,
    /// Per-pane "what's uploaded" key, so each pane re-bins/re-uploads only on a real change.
    pane_shown: std::collections::HashMap<usize, ShownKey>,
    /// Palette generation currently baked into each pane's LUT (see [`ShownKey`]).
    pane_lut: std::collections::HashMap<usize, u64>,
    /// Last `(theme, system_dark, density, accent)` handed to `theme::apply`.
    theme_applied: Option<(
        crate::settings::Theme,
        bool,
        crate::ui::m3::Density,
        Option<[u8; 3]>,
    )>,
    /// When the settings tree was last diffed against the saved copy.
    settings_checked: Option<Instant>,
    /// Frame counter, only used to invalidate within-frame memos.
    frame_nr: u64,
    palette_cache: Option<(u64, std::sync::Arc<[PaletteEntry]>)>,
    /// Visible city/town labels, keyed by the visible tile ids and the label-set generation.
    #[allow(clippy::type_complexity)]
    vlabel_cache: Option<(
        (Vec<crate::render::TileId>, u64),
        std::sync::Arc<[crate::vector_tiles::PlaceLabel]>,
    )>,
    /// Last result of each per-volume detector, keyed by what it depends on (see `volume_key`).
    #[allow(clippy::type_complexity)]
    nowcast_cache: Option<(
        ((usize, String, usize), usize, u8, Option<(u32, u32)>, u64),
        Vec<(f64, f64, egui::Color32)>,
    )>,
    tds_cache: Option<((usize, String, usize), Vec<wxdata::tds::TdsHit>)>,
    /// Same shape as `tds_cache`, for the hail-spike detector.
    tbss_cache: Option<(TunedKey, Vec<wxdata::dualpol::TbssHit>)>,
    /// ZDR columns, plus the bright band read off the same volume's CC — both cost a full pass
    /// over every tilt, so they share one cache and are computed together.
    zdr_cache: Option<ZdrCache>,
    couplet_cache: Option<((usize, String, usize), Vec<wxdata::rotation::CoupletHit>)>,
    /// Cells found per decoded volume, so a track built over a dozen frames flood-fills each
    /// sweep once rather than once a frame.
    celltrack_cache: LruCache<String, Vec<wxdata::celltrack::Blob>>,
    /// The tracks themselves, with the frame list they were built from.
    tracks_cache: Option<((usize, String, usize), Vec<wxdata::celltrack::Track>)>,
    show_local_tracks: bool,
    site_dialog: Option<ui::site_dialog::SiteDialog>,
    firstrun: ui::firstrun::FirstRun,
    /// The optional spotlight tour, and where the chrome drew the things it points at this frame.
    tour: ui::tour::Tour,
    tour_anchors: ui::tour::TourAnchors,
    settings_window: ui::settings_window::SettingsWindow,
    /// Active color tables (one per moment); reloaded when the palette settings change.
    palettes: Palettes,
    /// Live chunk stream for the active view: (view index, site, the generation it was spawned
    /// at). Cancellation is a counter bump rather than a task abort, because the browser has no
    /// abort — `spawn_local` hands back nothing to hold. The stream reads the counter before
    /// every chunk fetch and ends itself when it no longer recognizes its own generation.
    live_stream: Option<(usize, String, u64)>,
    /// Bumped to cancel whatever stream is running. Shared with the spawned task.
    live_gen: Arc<std::sync::atomic::AtomicU64>,
    last_stream_attempt: Option<Instant>,
    /// Decoded-volume LRU keyed by AWS object name, so scrubbing back and forth on the
    /// timeline doesn't re-download. ~10 volumes; each ~a few MB.
    scan_cache: LruCache<String, Arc<Scan>>,
    /// Loop frames being fetched ahead of the playhead, with when they were kicked off.
    ///
    /// Shared with the fetch tasks so each one gives its slot back when it ends, however it ends.
    /// The book used to be main-thread-only and expire on a timer, which meant a download slower
    /// than the timer lost its entry while still running — and the next tick started it again,
    /// and the one after that, without bound. The age-out survives as a backstop for a task that
    /// is genuinely gone.
    prefetching: Arc<Mutex<PrefetchBook>>,
    /// Browser only: the opening loop has not started yet. Cleared the moment it does, or when a
    /// deep link says the visitor asked for one specific moment rather than "show me now".
    autoplay_pending: bool,
    /// When this process started drawing — used to bound how long boot work may be deferred.
    boot_at: Instant,
    // --- Overlays (severe-weather layers; geographic, shared across views) ---
    http: reqwest::Client,
    overlay_rx: Receiver<OverlayMsg>,
    overlay_tx: Sender<OverlayMsg>,
    filters: OverlayFilters,
    alert_features: Vec<GeoFeature>,
    /// Archived storm-based warnings (feature W) keyed by 5-min UTC bucket (ts/300); shown while
    /// the active pane is scrubbed off-live.
    arch_warns: LruCache<i64, Vec<GeoFeature>>,
    /// The 5-min bucket currently being fetched (dedupes in-flight requests).
    arch_warn_inflight: Option<i64>,
    /// The bucket whose warnings are currently substituted into the overlay set (None = live).
    arch_warn_shown: Option<i64>,
    /// Archived local storm reports (feature CC) keyed by 30-min UTC bucket (ts/1800); shown while
    /// the active pane is scrubbed off-live (each bucket = the 6 h of reports ending there).
    arch_lsr: LruCache<i64, Vec<wxdata::spc::StormReport>>,
    arch_lsr_inflight: Option<i64>,
    arch_lsr_shown: Option<i64>,
    /// One slot per SPC outlook day, 1..=8 (Days 4–8 are the experimental probability layer).
    outlook_features: [Vec<GeoFeature>; 8],
    md_features: Vec<GeoFeature>,
    /// Watch boxes in effect, one feature per county row the service returns.
    watch_features: Vec<GeoFeature>,
    /// Winter Storm Severity Index polygons for the selected day.
    wssi_features: Vec<GeoFeature>,
    /// Excessive Rainfall Outlook polygons for the selected day.
    ero_features: Vec<GeoFeature>,
    /// Crowd precipitation-type reports (mPING), and their fetch clock.
    show_mping: bool,
    mping_reports: Vec<wxdata::mping::Report>,
    mping_last_fetch: Option<Instant>,
    /// Hurricane-hunter flight-track observations: toggle, obs, fetch clock.
    show_recon: bool,
    recon: Vec<wxdata::recon::HdobOb>,
    recon_last_fetch: Option<Instant>,
    /// Tropical wind-field threshold (34/50/64 kt), or `None` for no wind field, and whether
    /// to draw the potential storm-surge flooding polygons.
    tropical_wind_kt: Option<u8>,
    tropical_surge: bool,
    /// Pilot reports: toggle, current reports, fetch clock (rides the METAR view bbox).
    show_pireps: bool,
    pireps: Vec<wxdata::aviation::Pirep>,
    pirep_last_fetch: Option<Instant>,
    /// ProbSevere storm-probability polygons + badges (toggle + refresh clock).
    show_probsevere: bool,
    probsevere: Vec<GeoFeature>,
    probsevere_last_fetch: Option<Instant>,
    /// The currently-displayed, filtered feature set (hit-tested + tessellated).
    overlays: Vec<GeoFeature>,
    overlay_gen: u64,
    built_gen: u64,
    built_zoom_bucket: i32,
    built_theme: crate::settings::Theme,
    pending_overlay: Option<OverlayUpload>,
    overlay_ready: bool,
    overlay_last_fetch: Option<Instant>,
    detail: Option<Detail>,
    /// Open "Storm {id} Attributes" window (a clicked storm cell).
    cell_popup: Option<Cell>,
    /// Which of `settings.markers` the tapped-marker popup is editing.
    // ponytail: index identity — markers have no id, and their names aren't unique ("Marker 3"
    // comes back after a delete). A bounds check closes the popup if the list shrinks under it.
    marker_popup: Option<usize>,
    /// Which global model the global layers read, and how far into its run.
    global_model: wxdata::global::GlobalModel,
    global_fcst_hour: u16,
    /// The (model, hour) each global layer was last fetched for, so a change refetches at once.
    global_layer_key:
        std::collections::HashMap<crate::render::FieldLayer, (wxdata::global::GlobalModel, u16)>,
    /// What the difference layer differences, and the two valid times its last fetch compared —
    /// the pair rarely shares a cycle, and a difference between two instants has to say so.
    diff_field: crate::fielddiff::DiffField,
    diff_valid: Option<(String, String)>,
    /// When `goto.txt` was last looked for — see the poll in `update`.
    goto_poll: Option<Instant>,
    /// The last difference grid, kept on the CPU after upload so the cursor can read a number off
    /// it. A diverging color says "the models disagree here"; only a value says by how much.
    diff_grid: Option<wxdata::mrms::MrmsField>,
    /// The field the difference layer was last fetched for, so a change refetches at once.
    diff_key: Option<(crate::fielddiff::DiffField, u16)>,
    /// Where the open sounding was taken, so a forecast-hour change can refetch the same point.
    sounding_at: Option<(f64, f64)>,
    /// Vertices clicked so far with the watch-zone tool, `[lon, lat]`. Empty when not drawing.
    zone_pts: Vec<[f64; 2]>,
    /// A finished ring waiting for the user to name it.
    zone_naming: Option<(Vec<[f64; 2]>, String)>,
    /// Which of `settings.alert_polygons` the tapped-zone popup is editing.
    zone_popup: Option<usize>,
    /// A spotter dot tapped this frame, opened after the radar pane's borrows end.
    pending_spotter: Option<wxdata::spotters::Spotter>,
    /// The one open live-video window, if any (a marker's or a chase partner's stream).
    // ponytail: one at a time; a Vec of players when someone wants a wall of streams.
    video_player: Option<ui::video_window::VideoPlayer>,
    cells_window: ui::cells_window::CellsWindow,
    help_hub: ui::help_hub::HelpHub,
    rules_window: ui::rules_window::RulesWindow,
    /// The one slide-over surface every browsable tool page renders into.
    drawer: ui::drawer::Drawer,
    /// Anchors for the cards that answer a click on the map.
    popovers: ui::popover::Popovers,
    /// Warning verification lab and its in-flight query.
    verify_window: ui::verify_window::VerifyWindow,
    verify_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::verify::Verification, String>>>,
    /// Which moment the cross-section slices (session state, not persisted).
    xsection_moment: Moment,
    /// Storm-follow camera: the `(site, last-snapshot cell, since)` the active pane is tracking.
    /// Each new volume recenters on this cell; a manual pan or site change cancels it.
    follow_cell: Option<(String, Cell, Instant)>,
    /// Transient "follow ended" note `(text, shown-at)`; renders in the follow badge slot ~5 s.
    follow_notice: Option<(String, Instant)>,
    /// Open "Active Warnings" window (clicked warning/watch polygons).
    warning_popup: Option<ui::warning_window::WarningPopup>,
    /// Newest pane error and the time it appeared, for the auto-hiding bottom-center chip.
    error_chip: Option<(String, f64)>,
    /// Search text in the mobile navigation drawer's registry list.
    /// Forecast hour each HRRR-backed field layer was last fetched for, so scrubbing the tail
    /// refetches instead of showing a stale hour until the cadence expires.
    hrrr_layer_hour: std::collections::HashMap<crate::render::FieldLayer, u8>,
    /// Level 3 clickable storm cells for `cells_site` (the active site when last fetched).
    storm_cells: Vec<Cell>,
    cells_site: Option<String>,
    /// Per-cell-id trend history (VIL/top/dBZ across volumes); cleared when the site changes.
    cell_trends: std::collections::HashMap<String, Vec<ui::cell_window::CellSample>>,
    /// Last `ui_scale` pushed to egui, to tell slider changes apart from keyboard zoom.
    ui_scale_applied: f32,
    /// Android: whether we've asked for the soft keyboard (tracks egui's wants_keyboard_input).
    ime_shown: bool,
    /// Android: clipboard text read via JNI, queued for injection as an egui Paste event.
    pending_paste: Option<String>,
    /// Android: the text field that was focused when Paste was tapped. Tapping the button steals
    /// focus, so we re-focus this the frame the Paste event is delivered (else it lands nowhere).
    paste_target: Option<egui::Id>,
    /// Loaded placefile overlays (reconciled from `settings.placefiles` by URL).
    placefiles: Vec<LoadedPlacefile>,
    /// [`App::placefile_labels`] memoised, keyed by [`PlaceLabelKey`].
    placefile_label_cache: Option<(PlaceLabelKey, std::sync::Arc<[PlaceLabel]>)>,
    placefile_window: ui::placefile_window::PlacefileWindow,
    /// Last map viewport size (px), used to estimate the view range for placefile thresholds.
    last_viewport: (f32, f32),
    /// Active left-click map tool.
    tool: MapTool,
    /// Measure-tool clicked endpoints in `[lon, lat]` (max 2).
    measure: Vec<[f64; 2]>,
    /// Freehand annotation strokes, in lon/lat so they stick to the ground through pan and zoom.
    /// Session-only by design: this is for pointing at a storm on a stream, not a saved document.
    strokes: Vec<Stroke2d>,
    /// The colour the next stroke gets.
    draw_color: egui::Color32,
    marker_window: ui::marker_window::MarkerWindow,
    event_window: ui::event_window::EventWindow,
    chase_replay: ui::chase_replay::ChaseReplay,
    palette_editor: ui::palette_editor::PaletteEditor,
    digest_window: ui::digest_window::DigestWindow,
    digest_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    sounding_window: ui::sounding_window::SoundingWindow,
    sounding_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::sounding::Sounding, String>>>,
    /// The observed RAOB fetched alongside the HRRR profile, for the same click.
    raob_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::sounding::Sounding, String>>>,
    /// Last spoken storm-position update: when, and the distance in whole miles it reported.
    spoke_pos: Option<(Instant, i32)>,
    /// Detections seen recently, for compound rules to ask "and was there also…". Trimmed to the
    /// compound window every pass, so it stays a handful of entries.
    recent_hits: Vec<(
        crate::settings::RuleTrigger,
        crate::rules::Detection,
        Instant,
    )>,
    /// Chase mode: follow a position, auto-switching the active pane to the nearest radar.
    chase_mode: bool,
    chase_pos: Option<(f64, f64)>,
    /// Breadcrumb track of this session's fixes (see `settings.chase_log`).
    chase_track: crate::chaselog::Track,
    /// The site last warmed ahead of a chase handoff, and when — so a fix every two seconds does
    /// not queue a download every two seconds.
    warmed_site: Option<(String, Instant)>,
    /// Tornado climatology: the loaded SPC track database (lazy), a pending async load, the last
    /// query result + its center, a window-open flag, and a query queued while the CSV loads.
    climo_tracks: Option<std::sync::Arc<Vec<wxdata::torclimo::TornadoTrack>>>,
    climo_rx:
        Option<std::sync::mpsc::Receiver<Result<Vec<wxdata::torclimo::TornadoTrack>, String>>>,
    climo_hits: Vec<wxdata::torclimo::TornadoTrack>,
    climo_center: Option<(f64, f64)>,
    climo_open: bool,
    climo_loading: bool,
    climo_error: Option<String>,
    climo_pending_query: Option<(f64, f64)>,
    /// Warning history for the same clicked point (IEM VTEC by-point), fetched alongside the
    /// tornado tracks. `Some(rx)` while the request is in flight.
    climo_warn: Option<wxdata::archive_warnings::PointSummary>,
    climo_warn_rx:
        Option<std::sync::mpsc::Receiver<Result<wxdata::archive_warnings::PointSummary, String>>>,
    chase_applied: Option<(f64, f64)>,
    /// Live position stream from gpsd, when the user has connected it.
    gps_rx: Option<std::sync::mpsc::Receiver<(f64, f64)>>,
    /// Google Drive settings sync: the saved tokens, the last-agreed state, an in-flight
    /// sign-in's code pair, the line shown in the Sync tab, worker replies, and the poll clock.
    sync_tokens: Option<crate::cloud::Tokens>,
    sync_state: crate::cloud::SyncState,
    sync_login: Option<crate::cloud::Pending>,
    sync_status: String,
    sync_rx: Option<std::sync::mpsc::Receiver<SyncMsg>>,
    sync_checked: Option<wxdata::clock::Instant>,
    /// Position sharing (LAN broadcast + optional relay), started on first use.
    share: Option<crate::share::Share>,
    /// Everyone else's last known position, keyed by their device id.
    peers: std::collections::HashMap<String, crate::share::Peer>,
    /// When we last put our own fix on the wire (both transports share the cadence).
    share_sent: Option<wxdata::clock::Instant>,
    /// GOES satellite frame times (for the sub-hourly scrub), the style they were fetched for,
    /// and the selected index (`None` = latest).
    goes_times: Vec<chrono::DateTime<chrono::Utc>>,
    goes_times_style: Option<crate::tiles::BasemapStyle>,
    goes_time_idx: Option<usize>,
    /// Keep the GOES frame on the active pane's radar clock rather than on a hand-picked frame.
    goes_follow_radar: bool,
    goes_times_rx: Option<std::sync::mpsc::Receiver<Vec<chrono::DateTime<chrono::Utc>>>>,
    /// The archive hour the loaded frame times cover (`None` = the live window ending now).
    /// Scrubbing far enough back to cross into another hour refetches; staying inside one does
    /// not, which is what keeps an archive loop from asking GIBS a question per frame.
    goes_hour: Option<i64>,
    /// The previous flash-extent grid, kept only so the lightning jump has something to subtract.
    glm_fed_prev: Option<wxdata::mrms::MrmsField>,
    /// When the Android widget snapshot was last written.
    widget_shot_at: Option<Instant>,
    /// A warning that wants a radar picture pushed after it (see `settings.ntfy_snapshot`).
    snapshot_push: Option<String>,
    /// Per-location cooldown for the rotation-near-a-watched-place alert.
    rotation_alerted: std::collections::HashMap<String, Instant>,
    /// Volume start time of the last new-scan chime (see `scan_chime`).
    last_chime: Option<chrono::DateTime<Utc>>,
    /// Pushes held back by quiet hours, replayed as one summary when the window ends.
    ///
    /// A `Mutex` because `notify_alert` takes `&self` (ten call sites); a lock beats threading
    /// `&mut` through all of them.
    /// Held across a restart through `Settings::quiet_pending`, written on exit: a quiet window
    /// that spans a relaunch still owes its catch-up.
    quiet_queue: std::sync::Mutex<Vec<(String, String)>>,
    /// Outbreak rollup state. A `Mutex` for the same reason `quiet_queue` is one: `notify_alert`
    /// takes `&self`.
    rollup: std::sync::Mutex<crate::alert_rollup::Rollup>,
    /// Whether the last frame was inside quiet hours, so the end of the window is an edge.
    was_quiet: bool,
    /// Where a requested screenshot should go once the image event arrives.
    screenshot_pending: Option<ShotDest>,
    /// A capture waiting on the share-card footer to be on screen: the destination, and how many
    /// more frames to draw the footer before asking for the image (see `share_card_footer`).
    share_card: Option<(ShotDest, u8)>,
    loop_export: Option<LoopExport>,
    /// When true, all panes share the active pane's camera.
    link_cameras: bool,
    /// The always-on-top mini-loop window is open (desktop only; see `mini_loop_viewport`).
    mini_loop: bool,
    /// The mini loop's own camera while it is open; `None` until it borrows the pane's.
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    mini_cam: Option<crate::render::mercator::Camera>,
    /// The report left by a previous run that panicked, shown once until dismissed.
    crash_report: Option<String>,
    /// National gridded field layers (MRMS mosaic, rotation, MESH, AzShear, lightning), each with
    /// its own toggle + pending GPU upload + refresh throttle. Keyed by [`crate::render::FieldLayer`].
    fields: std::collections::HashMap<crate::render::FieldLayer, FieldState>,
    /// Selected rotation-track accumulation window (minutes): 30, 60, or 120.
    rotation_minutes: u16,
    /// Selected hail-swath accumulation window (minutes); see [`wxdata::mrms::hail_swath`].
    hail_minutes: u16,
    /// Environment suite (HRRR CAPE/SRH): CAPE uses the mixed-layer (90-0 mb) parcel when true,
    /// else surface-based; SRH depth in km (1 = 0-1 km, 3 = 0-3 km). Changing either clears the
    /// layer's last_fetch so the next frame refetches.
    env_cape_ml: bool,
    env_srh_km: u8,
    /// Where the environment fields and contours come from: the HRRR forecast, or the RAP f00
    /// analysis (13 km, observation-assimilated — "mesoanalysis"). Changing it refetches both.
    env_model: wxdata::hrrr::Model,
    /// The site the L3 gridded products (DVL/EET) were last fetched for (feature X); refetch on
    /// site change.
    l3grid_site: Option<String>,
    /// What the locally derived products (VIL/VILD/echo tops) were last computed from:
    /// `(volume name, echo-top threshold, enabled-layer mask, melting level)`. Any of them moving
    /// recomputes.
    derived_key: Option<(String, u32, u8, i32)>,
    /// `(site, 0 °C height, −20 °C height)` above sea level in metres, for the hail grids, plus
    /// the clock that refreshes them on the environment cadence.
    freezing: Option<(String, f64, f64)>,
    freezing_last_fetch: Option<Instant>,
    /// Accumulation window (hours) for the observed snowfall analysis, and the one last fetched.
    snow_hours: u16,
    snow_fetched: Option<u16>,
    /// Surface obs (METAR station plots, feature U): toggle, current obs, fetch clock + bbox.
    show_metar: bool,
    metars: Vec<wxdata::metar::SurfaceOb>,
    /// Raw TAF text by ICAO, for the station tooltips (empty where a station files none).
    tafs: std::collections::HashMap<String, String>,
    metar_last_fetch: Option<Instant>,
    /// The `(lat0, lon0, lat1, lon1)` bbox the current `metars` were fetched for.
    metar_bounds: Option<(f64, f64, f64, f64)>,
    /// River flood gauges (NWPS): toggle, current gauges, fetch clock + bbox (mirrors METAR).
    show_gauges: bool,
    gauges: Vec<wxdata::river::Gauge>,
    gauge_last_fetch: Option<Instant>,
    gauge_bounds: Option<(f64, f64, f64, f64)>,
    /// HRRR model contours: selected field, current polylines, valid time, fetch clock, and the
    /// kind the current lines were fetched for (drives refetch-on-change).
    contour_kind: ContourKind,
    contours: Vec<wxdata::contour::ContourLine>,
    contour_valid: Option<DateTime<Utc>>,
    contour_last_fetch: Option<Instant>,
    contour_fetched_kind: Option<(ContourKind, wxdata::hrrr::Model)>,
    /// NHC tropical suite (feature V): toggle, fetched data, refresh clock. On by default like
    /// the other severe layers — an active hurricane is not something to have to go and enable.
    show_tropical: bool,
    tropical: Option<wxdata::tropical::TropicalData>,
    tropical_last_fetch: Option<Instant>,
    /// County power outages (ODIN): on by default like the tropical suite — it draws nothing
    /// until a county is significantly dark, so a quiet day costs one fetch and no pixels.
    show_outages: bool,
    outage_features: Vec<overlay::GeoFeature>,
    outages_last_fetch: Option<Instant>,
    /// CAPPI slice window (feature AA): toggle, selected altitude (km), rendered texture, and the
    /// key `(volume name, altitude bits)` the texture was built for (re-slice on change).
    show_cappi: bool,
    cappi_alt_km: f32,
    cappi_tex: Option<egui::TextureHandle>,
    cappi_key: Option<(String, u32)>,
    /// HRRR "future radar": selected forecast hour, last-fetched hour, run/valid times, clock.
    hrrr_fcst_hour: u8,
    hrrr_fetched_hour: Option<u8>,
    hrrr_run: Option<DateTime<Utc>>,
    hrrr_valid: Option<DateTime<Utc>>,
    hrrr_last_fetch: Option<Instant>,
    /// True while the HRRR layer is being driven by a forecast-tail scrub (vs. the manual toggle).
    hrrr_by_timeline: bool,
    /// Tray-menu command channel (Linux StatusNotifier); `None` if no tray host is available.
    tray_rx: std::sync::mpsc::Receiver<crate::tray::TrayCmd>,
    /// Last state pushed to the tray, so an unchanged frame sends nothing.
    tray_state: crate::tray::TrayState,
    /// True once a StatusNotifier host has taken the tray item; registration is async, so this
    /// can flip after the first frames.
    tray_present: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set by the tray "Quit" item so the close-to-tray handler lets the window actually close.
    really_quit: bool,
    /// Local storm-report markers (live IEM LSR feed, trailing 6 h) + toggle + refresh clock.
    show_storm_reports: bool,
    storm_reports: Vec<wxdata::spc::StormReport>,
    reports_last_fetch: Option<Instant>,
    /// Aviation SIGMET/AIRMET overlay (feature GG): toggle, features, refresh clock.
    show_aviation: bool,
    aviation_features: Vec<GeoFeature>,
    aviation_last_fetch: Option<Instant>,
    /// FAA Temporary Flight Restrictions: toggle, shapes by NOTAM id, refresh clock, and how
    /// many shapes are still unfetched (the first load comes in batches).
    /// MRMS surface precipitation classes for the reflectivity tint, kept whether or not the
    /// precipitation-type layer itself is shown. Behind an `Arc` so a pane can take a cheap
    /// handle to it while the volume it is drawing is mutably borrowed.
    precip_flag_grid: Option<std::sync::Arc<PrecipGrid>>,
    /// Bumped whenever `precip_flag_grid` is replaced, so a pane knows its upload is stale.
    precip_flag_gen: u32,
    show_tfr: bool,
    tfr_features: std::collections::HashMap<String, GeoFeature>,
    tfr_last_fetch: Option<Instant>,
    tfr_pending: usize,
    /// Area Forecast Discussion window (feature DD): open flag, fetched text, in-flight receiver.
    afd_open: bool,
    afd: Option<wxdata::afd::Afd>,
    afd_error: Option<String>,
    afd_busy: bool,
    afd_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::afd::Afd, String>>>,
    /// NHC public advisory / forecast discussion reader.
    tropical_window: ui::tropical_window::TropicalWindow,
    tropical_text_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::tropical::Advisory, String>>>,
    /// Range rings + azimuth spokes around the active site (feature HH).
    show_range_rings: bool,
    /// Draw all NEXRAD radar sites on the map; clicking one switches the pane to that radar.
    show_radar_sites: bool,
    /// Beam-vs-terrain blockage shading: the resident raster, what it was built for, and the
    /// world rect it covers. Built off-thread (it fetches DEM tiles), so it arrives on a channel.
    show_blockage: bool,
    blockage_tex: Option<(BlockageKey, egui::TextureHandle, [f64; 4])>,
    /// Key of the build in flight, and when it started — one at a time, and never more often than
    /// every 600 ms, so a continuous pan doesn't queue a raster per frame.
    blockage_pending: Option<(BlockageKey, Instant)>,
    blockage_rx: Receiver<(BlockageKey, [f64; 4], egui::ColorImage)>,
    blockage_tx: Sender<(BlockageKey, [f64; 4], egui::ColorImage)>,
    /// Layers panel (floating, searchable layer picker): open flag + its search text.
    /// Viewport minus the docked bars, refreshed each frame — floating `Area`s constrain to this
    /// instead of `content_rect`, which egui measures before panels take their bite.
    chrome_rect: egui::Rect,
    layers_query: String,
    /// Ctrl+K command palette: open flag, query, and the highlighted row.
    /// Set by Ctrl+K so the drawer grabs the search field on the frame it opens.
    /// Is the floating left panel showing? Runtime state, not a setting: the map is the app,
    /// and a panel you left open yesterday shouldn't cover it today.
    panel_open: bool,
    /// Is the background picker slid out beside the control column?
    basemap_open: bool,
    sidebar_focus_search: bool,
    /// The `?` keyboard cheat sheet is up.
    show_cheatsheet: bool,
    /// The Hotkeys settings tab is waiting for the next keypress to bind; the global hotkey table
    /// stands down while it is.
    capture_key: bool,
    /// Top search pill: the place query and a transient "flew to …" status.
    place_query: String,
    place_status: Option<(String, Instant)>,
    /// After a search flies somewhere, the offer to keep it: `(name, lat, lon, when)`. Searching is
    /// how you look around, so dropping a pin every time would litter the map — this asks first.
    save_offer: Option<(String, f64, f64, Instant)>,
    /// True while the in-flight geocode came from the search pill (navigate only) rather than
    /// the marker window (which adds a marker).
    geocode_nav: bool,
    /// Layer manager window (per-placefile enable/order/opacity).
    layer_window_open: bool,
    /// Placefile icon-sheet textures by URL. `None` = fetch in flight or failed (negative-cached
    /// so a broken sheet isn't retried every frame), same idiom as `marker_icon_tex`.
    pf_icon_tex: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    pf_icon_rx: Receiver<(String, egui::ColorImage)>,
    pf_icon_tx: Sender<(String, egui::ColorImage)>,
    /// Android: hide all floating chrome to view the whole radar (toggled by the eye button).
    mobile_chrome_hidden: bool,
    /// Touch: when and where the last tap landed, so a drag that starts right on top of it is
    /// read as the second half of a double-tap-drag zoom rather than a pan.
    last_tap: Option<(f64, egui::Pos2)>,
    /// Touch: the anchor of a double-tap-drag zoom in progress (`None` = a drag pans).
    tap_zoom: Option<egui::Pos2>,
    /// Android: rects the mobile chrome covers this frame. Two-finger gestures are read straight
    /// off the raw input, which has no idea egui drew a sheet over the map, so the pane input
    /// block checks the gesture center against these.
    mobile_occlusion: Vec<egui::Rect>,
    /// When the last two-finger gesture ended. Lifting one finger of a pinch leaves the other
    /// one down, which egui immediately reads as a click and a fresh drag — an interrogate popup
    /// and a jump for what was only the end of a zoom. A short cooldown eats both.
    last_gesture_end: Option<wxdata::clock::Instant>,
    /// Spotter Network positions + toggle + refresh clock (filtered to active site at draw).
    show_spotters: bool,
    /// FAA WeatherCams: the toggle, the sites in view, and the bbox//time they were fetched for.
    show_webcams: bool,
    webcams: Vec<wxdata::webcams::CamSite>,
    webcam_bounds: Option<(f64, f64, f64, f64)>,
    webcam_last_fetch: Option<Instant>,
    /// WFIGS wildfires: perimeters (tessellated with the other overlay polygons), incident points,
    /// and the bbox/clock they were fetched for.
    show_fires: bool,
    fire_perims: Vec<GeoFeature>,
    fire_incidents: Vec<wxdata::wfigs::FireIncident>,
    fire_bounds: Option<(f64, f64, f64, f64)>,
    fire_last_fetch: Option<Instant>,
    /// AirNow AQI dots: toggle, the obs in view, and the bbox/clock they were fetched for. Needs
    /// a user key; without one the layer never fetches.
    show_aqi: bool,
    aqi: Vec<wxdata::airnow::AqiOb>,
    aqi_bounds: Option<(f64, f64, f64, f64)>,
    aqi_last_fetch: Option<Instant>,
    /// Live station cards: the toggle, the layer state, and the poll clocks behind it.
    show_stations: bool,
    stations: crate::stationlayer::Layer,
    station_last_poll: Option<Instant>,
    ppef_last_fetch: Option<Instant>,
    dotcam_bounds: Option<(f64, f64, f64, f64)>,
    /// NOAA Weather Radio: the running player (dropping it stops playback) and the relay picked
    /// in the drawer.
    #[cfg(not(target_arch = "wasm32"))]
    nwr: Option<crate::nwr::Player>,
    nwr_pick: String,
    /// NWS damage surveys: the toggle, the last result, and the `(bbox, day)` it was fetched for
    /// (surveys never change, so the key alone decides when to refetch).
    show_dat: bool,
    dat_points: Vec<wxdata::dat::DamagePoint>,
    dat_tracks: Vec<wxdata::dat::DamageTrack>,
    dat_key: Option<((f64, f64, f64, f64), chrono::NaiveDate)>,
    /// Multi-radar mosaic: which sites the last composite used, the oldest scan in it, the view it
    /// was built for — panning off the composite refetches instead of leaving a stale picture.
    mosaic_sites: Vec<String>,
    mosaic_oldest: Option<chrono::DateTime<chrono::Utc>>,
    mosaic_bounds: Option<(f64, f64, f64, f64)>,
    spotters: Vec<wxdata::spotters::Spotter>,
    spotters_last_fetch: Option<Instant>,
    /// Sensor dashboard: open flag, latest fetch (Ok/Err), the site it's for, and a refresh clock.
    show_sensors: bool,
    sensor_data: Option<Result<wxdata::obs::StationObs, String>>,
    sensor_site: Option<String>,
    sensor_last_fetch: Option<Instant>,
    /// VAD hodograph: open flag, latest profile, its site, and a refresh clock.
    show_hodo: bool,
    hodo_data: Vec<wxdata::level3::VwpLevel>,
    /// Profiles collected this session, oldest→newest. The radar only ever publishes the newest
    /// one, so a time-height view has to be accumulated live.
    hodo_history:
        std::collections::VecDeque<(chrono::DateTime<Utc>, Vec<wxdata::level3::VwpLevel>)>,
    hodo_tab: ui::hodograph_window::Tab,
    /// Tap-for-forecast: window state, the tapped point, the in-flight fetch, and a short cache
    /// keyed by rounded lat/lon.
    forecast_open: bool,
    forecast_at: Option<(f64, f64)>,
    forecast_state: ui::forecast_window::State,
    #[allow(clippy::type_complexity)]
    forecast_rx: Option<(
        (i32, i32),
        std::sync::mpsc::Receiver<Result<wxdata::forecast::PointForecast, String>>,
    )>,
    forecast_cache:
        std::collections::HashMap<(i32, i32), (Instant, wxdata::forecast::PointForecast)>,
    /// Current conditions for the same tapped point, fetched beside the forecast and cached the
    /// same way. Separate from the Obs overlay, which is radar-site-scoped and only live when that
    /// overlay is on.
    #[allow(clippy::type_complexity)]
    forecast_obs_rx: Option<(
        (i32, i32),
        std::sync::mpsc::Receiver<(String, wxdata::obs::Observation)>,
    )>,
    forecast_obs_cache:
        std::collections::HashMap<(i32, i32), (Instant, String, wxdata::obs::Observation)>,
    /// Rain-arrival alerting: per-point persistence/cooldown state, plus the current ETAs for the
    /// on-map chip.
    rain_detector: crate::rain_arrival::Detector,
    /// Volume the rain check last ran against, so it runs per scan (what the detector's
    /// persistence and cooldown are written for) instead of per frame.
    rain_key: Option<(usize, String, usize)>,
    /// When the flash-extent grid was last built. Its own clock, because a rule can want the
    /// grid while the layer that used to own the cadence is off.
    glm_fed_last: Option<Instant>,
    /// Same per-volume guard for the user's scan rules: evaluated once per volume, not per frame.
    rules_key: Option<(usize, String, usize)>,
    /// When each rule last fired, keyed `"{rule id}:{place}"` — a rule watching two places gets
    /// to speak about both.
    rules_fired: std::collections::HashMap<String, Instant>,
    rain_eta: Vec<(String, f32)>,
    /// Minute-by-minute rain over the forecast point, and the (volume, point, motion) key it was
    /// computed for — the walk samples the whole sweep, so it runs once per volume, not per frame.
    minute_profile: Option<Vec<Option<f32>>>,
    minute_key: Option<String>,
    /// WPC surface analysis overlay: fronts + pressure centers, refreshed a few times an hour.
    show_fronts: bool,
    fronts: Option<wxdata::fronts::SurfaceAnalysis>,
    fronts_last_fetch: Option<Instant>,
    /// GOES satellite lightning: the rolling flash window and its poll clock. The feed lives
    /// behind a mutex because the poll runs on the tokio runtime while the painter reads it.
    show_glm: bool,
    glm: std::sync::Arc<std::sync::Mutex<wxdata::glm::GlmFeed>>,
    glm_last_poll: Option<Instant>,
    glm_polling: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Ground strikes as they arrive over MQTT, oldest first. Nothing fills this unless the user
    /// points `strikes_topic` at a broker that carries them — the app never talks to a strike
    /// network itself.
    show_strikes: bool,
    strikes: std::collections::VecDeque<(f64, f64, chrono::DateTime<chrono::Utc>)>,
    /// Animated wind particles. The grids are shared; the particle sets are per pane, because each
    /// pane has its own camera. Nothing here persists to settings — neither do fronts or GLM.
    show_wind: bool,
    wind: Option<crate::wind_draw::WindField>,
    wind_level: wxdata::hrrr::WindLevel,
    wind_particles: std::collections::HashMap<usize, crate::wind_draw::Particles>,
    /// Whether the particles are advected on the GPU. `HOOKECHO_CPU_WIND=1` forces the CPU mesh,
    /// which is also what runs if the GPU layer ever fails to build.
    wind_on_gpu: bool,
    /// The wind field the GPU copy was uploaded from, so a frame that has not changed is not
    /// re-warped and re-uploaded.
    wind_uploaded: Option<(chrono::DateTime<chrono::Utc>, u8, wxdata::hrrr::WindLevel)>,
    /// What the current grids are of, so a level or forecast-hour change refetches at once.
    wind_fetched: Option<(wxdata::hrrr::WindLevel, u8)>,
    wind_last_fetch: Option<Instant>,
    /// When the in-flight fetch started, or `None` if none is. One at a time: 10 m u+v is 4.5 MB
    /// an hour, and a fast scrub across the forecast tail would otherwise queue ~82 MB of GRIB
    /// behind itself. A timestamp rather than a flag because `spawn_overlay` drops fetch errors
    /// into the log — a plain flag would never be cleared on failure and would wedge the layer.
    wind_inflight: Option<Instant>,
    /// Previous frame's instant, and the clamped timestep derived from it. Computed once per
    /// frame so every pane advects by the same amount.
    wind_last_frame: Option<Instant>,
    wind_dt: f32,
    hodo_site: Option<String>,
    hodo_last_fetch: Option<Instant>,
    /// Streamer/OBS mode: hide all chrome (drawer/pills/docks), leaving only the map.
    obs_mode: bool,
    /// `?embed` in the browser build: chromeless map inside someone else's iframe (WeatherDesk).
    /// Hides chrome like OBS mode, and idles at one frame a minute until the visitor touches it —
    /// an embedded radar repainting at 10 fps costs the host page a whole core.
    embed: bool,
    /// Set by the first interaction with an embedded map: from then on it repaints normally.
    embed_live: bool,
    /// When the user last did anything — the input the idle heartbeat listens for. Also what
    /// "a gesture is in progress" is read from.
    last_input: Instant,
    /// A pointer or finger is down this frame. Read once at the top of `ui`, because the overlay
    /// tessellation asks it before any pane has drawn.
    gesture_live: bool,
    /// Perf readout state: whether `HOOKECHO_PERF=1` asked for the window, the frame count and
    /// mark the frames-per-minute number is derived from, and the last idle interval requested.
    #[cfg(not(target_arch = "wasm32"))]
    perf: PerfReadout,
    /// Last pane state posted to the parent frame, so only real changes cross the boundary.
    #[cfg(target_arch = "wasm32")]
    last_posted: Option<crate::workspace::PaneSnap>,
    /// Auto-tour: cycle the camera through active-warning centroids while in OBS mode.
    obs_tour: bool,
    obs_tour_last: Option<Instant>,
    obs_tour_idx: usize,
    /// Warning dedupe keys already seen (VTEC event keys), so a new warning is detected on
    /// arrival and a continuation of one already announced is not.
    known_warning_ids: std::collections::HashSet<String>,
    /// False until the first alert fetch seeds `known_warning_ids` (avoids alerting on startup).
    warnings_seeded: bool,
    /// Per-location cooldown clock for the lightning-proximity alarm (re-alert after it goes quiet).
    lightning_alerted: std::collections::HashMap<String, Instant>,
    /// True while a TDS is currently detected, so the alert fires on the rising edge only.
    tds_active: bool,
    /// True while a rotation couplet is currently detected (rising-edge alarm latch).
    rot_active: bool,
    /// Active new-warning banners (event, area, first-seen time); expire after a while.
    warning_banners: Vec<(String, String, Instant)>,
    /// Transient results of things the user just did (export saved, encode failed). The third
    /// lane, distinct from the warning banners (weather) and the error chip (radar feed).
    toasts: Vec<Toast>,
    /// Auxiliary feeds already reported to the user; see [`HookEchoApp::drain_feed_errors`].
    feed_errors_told: std::collections::HashMap<&'static str, Instant>,
    /// Right-dock active-alerts panel toggle.
    show_alert_panel: bool,
    /// Cross-section tool: clicked endpoints `[lon,lat]` (max 2), the built section + its texture.
    xsection_pts: Vec<[f64; 2]>,
    xsection: Option<wxdata::xsection::CrossSection>,
    xsection_tex: Option<egui::TextureHandle>,
    /// Lazily-loaded textures for uploaded marker icons, keyed by filename. `None` = load failed
    /// (negative-cached so a missing/corrupt file isn't retried every frame).
    marker_icon_tex: ui::marker_window::IconTextures,
    /// 3D raymarch view: open flag, orbit camera (az/el degrees + distance), and a pending
    /// volume upload (taken by the first paint after a rebuild).
    show_3d: bool,
    vol3d: ui::volume3d_window::Volume3dState,
    /// Which volume the built grid belongs to, so reopening the window doesn't rebuild it.
    vol3d_key: Option<(String, usize)>,
    /// In-flight build (the resample runs off the UI thread).
    #[allow(clippy::type_complexity)]
    vol3d_rx: Option<std::sync::mpsc::Receiver<(crate::render3d::Volume3dUpload, (f32, f32))>>,
    /// The built volume's dBZ span, which the window's threshold slider works in.
    vol3d_range: (f32, f32),
    vol3d_pending: Option<crate::render3d::Volume3dUpload>,
    /// GPU 2D texture-size cap (device limit), used to clamp field-grid decimation on mobile GPUs.
    max_texture_dim: u32,
    /// Whether this device can hold the 3D texture the raymarch window needs. See its assignment
    /// in `new` — it is a property of the adapter that turned up, not of the platform.
    volume3d_supported: bool,
}

/// Split `r` into `n` pane rects: 1 full, 2 side-by-side, 3–4 in a 2×2 grid.
fn pane_rects(r: egui::Rect, n: usize) -> Vec<egui::Rect> {
    let gap = 2.0;
    match n {
        0 | 1 => vec![r],
        2 => {
            // Stack top/bottom when the viewport is taller than wide (portrait phone), else split
            // left/right (wide desktop). Gives the horizontal split the user wants on mobile.
            if r.height() >= r.width() {
                let h = (r.height() - gap) / 2.0;
                vec![
                    egui::Rect::from_min_size(r.min, egui::vec2(r.width(), h)),
                    egui::Rect::from_min_size(
                        egui::pos2(r.min.x, r.min.y + h + gap),
                        egui::vec2(r.width(), h),
                    ),
                ]
            } else {
                let w = (r.width() - gap) / 2.0;
                vec![
                    egui::Rect::from_min_size(r.min, egui::vec2(w, r.height())),
                    egui::Rect::from_min_size(
                        egui::pos2(r.min.x + w + gap, r.min.y),
                        egui::vec2(w, r.height()),
                    ),
                ]
            }
        }
        _ => {
            let w = (r.width() - gap) / 2.0;
            let h = (r.height() - gap) / 2.0;
            let mut v = Vec::new();
            for row in 0..2 {
                for col in 0..2 {
                    let x = r.min.x + (w + gap) * col as f32;
                    let y = r.min.y + (h + gap) * row as f32;
                    v.push(egui::Rect::from_min_size(
                        egui::pos2(x, y),
                        egui::vec2(w, h),
                    ));
                }
            }
            v.truncate(n.clamp(1, 4));
            v
        }
    }
}

impl HookEchoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Inter in front, Phosphor's icon glyphs behind it (the mobile chrome draws line icons
        // egui's default face has none of), and on native egui's own faces behind both as the
        // fallback for anything Inter's subset dropped. The browser build starts without those
        // fallbacks and fetches them a moment later — see `crate::fonts`.
        cc.egui_ctx.set_fonts(crate::fonts::base());
        #[cfg(target_arch = "wasm32")]
        crate::fonts::spawn_load(cc.egui_ctx.clone());

        #[cfg(not(target_arch = "wasm32"))]
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        #[cfg(not(target_arch = "wasm32"))]
        let spawner = crate::rt::Spawner::new(rt.handle().clone());
        #[cfg(target_arch = "wasm32")]
        let spawner = crate::rt::Spawner::new();

        let render_state = cc.wgpu_render_state.as_ref().expect("wgpu backend");
        // Which GPU actually got picked. One line, at startup, because every performance report
        // is unreadable without it — "the map is choppy" means one thing on a discrete adapter
        // and another on llvmpipe, and nothing in the app said which one was running.
        {
            let info = render_state.adapter.get_info();
            log::info!(
                "gpu: {} ({:?}, {:?}) driver {}",
                info.name,
                info.device_type,
                info.backend,
                info.driver
            );
        }
        // Device loss on wasm is unrecoverable from inside the app: WebGPU (Safari 26+) loses
        // devices silently — black canvas, `webglcontextlost` can never fire — and on the WebGL
        // fallback (WebKitGTK) wgpu marks the device lost for gles errors that never surface as
        // a JS context-loss event either (seen on NVIDIA + WebKitGTK: dies at first paint, the
        // page-level handler never fires). Reload is the only recovery on every backend.
        //
        // The throttle lives in the URL, not sessionStorage — WebKit blocks storage in
        // third-party iframes (the WeatherDesk embed), and a throttle that fails open there is a
        // reload loop (the Ubuntu lockup). One reload per navigation: flag already present means
        // this navigation was the retry, so stay on the dead canvas. index.src.html strips the
        // flag after 60s of healthy running, earning a future retry.
        #[cfg(target_arch = "wasm32")]
        render_state.device.set_device_lost_callback(|reason, msg| {
            // `Dropped`/`ReplacedCallback` are clean teardown, not failure.
            if !matches!(reason, wgpu::DeviceLostReason::Unknown) {
                return;
            }
            log::error!("wgpu device lost: {msg}");
            let Some(win) = web_sys::window() else { return };
            let search = win.location().search().unwrap_or_default();
            if search.contains("relaunched") {
                return;
            }
            let sep = if search.is_empty() { "?" } else { "&" };
            // Setting `search` navigates; the spec keeps the fragment, so `#goto=…` survives.
            let _ = win
                .location()
                .set_search(&format!("{search}{sep}relaunched"));
        });
        // The GPU's 2D texture-size cap: desktop/Adreno do 16384, but many mobile GPUs cap at
        // 4096. Field grids (MRMS rotation/AzShear reach 14000 px) are decimated to fit this.
        let max_texture_dim = render_state.device.limits().max_texture_dimension_2d;
        // The raymarched volume is one `VOL3D_N` cubed 3D texture, and not every backend can hold
        // one. Asked once, from the device's own limit, rather than from the target: WebGPU can do
        // this and the WebGL2 fallback cannot, and which of the two a browser gives you is a
        // runtime fact, not a compile-time one. A desktop GL driver old enough to say no gets the
        // same honest answer instead of an empty window.
        let volume3d_supported =
            render_state.device.limits().max_texture_dimension_3d as usize >= VOL3D_N;
        {
            // Shaders and pipelines are compiled here, synchronously, before the first paint —
            // the suspected dominant term in a cold launch. Timed so the guess is a number.
            #[cfg(not(target_arch = "wasm32"))]
            let pipelines_at = std::time::Instant::now();
            let mut w = render_state.renderer.write();
            w.callback_resources.insert(RenderResources::new(
                &render_state.device,
                render_state.target_format,
            ));
            // The 3D volume pipeline is NOT compiled here — see `Volume3dCallback::prepare`.
            // Most sessions never open that window, and it was paying for it at every launch.
            w.callback_resources
                .insert(crate::render3d::Volume3dFormat(render_state.target_format));
            #[cfg(not(target_arch = "wasm32"))]
            log::info!(
                "perf: pipelines compiled in {} ms",
                pipelines_at.elapsed().as_millis()
            );
        }

        // Registering with the StatusNotifier host is a blocking D-Bus round trip; started here
        // so it overlaps the rest of construction instead of sitting in front of the first frame.
        let (tray_rx_init, tray_present_init) = crate::tray::spawn();

        let mut settings = Settings::load();
        // Three arrangements worth having before you have built any of your own. Once only: the
        // flag is what makes deleting them stick.
        if settings.workspaces.is_empty() && !settings.seeded_workspaces {
            settings.workspaces = crate::workspace::starters();
            settings.seeded_workspaces = true;
            settings.save();
        }
        // Sample terrain at the resolution this user packs at, so a hi-res pack is actually read.
        crate::elevation::set_hires(settings.pack_hires_dem);
        // A decoded volume is tens of MB, so the phone's cache is sized to the loop window it can
        // actually afford (see ANDROID_LOOP_WINDOW) plus the head and the frame in flight —
        // enough that a loop stops re-downloading itself on every wrap, without the ~900 MB RSS
        // that holding a full desktop-sized window cost.
        // The browser gets the same treatment for the same reason, only harder: a wasm heap is
        // 32-bit, so thirty decoded volumes is not a large cache there, it is an out-of-memory.
        let scan_cache_cap = if cfg!(target_os = "android") {
            ANDROID_LOOP_WINDOW + 2
        } else if cfg!(target_arch = "wasm32") {
            WEB_LOOP_WINDOW + 4
        } else {
            30
        };
        // The alert overlay from the last run, minus anything that has expired since. Also seeds
        // the known-warning ids, so a restart during an event doesn't re-banner and re-speak
        // every warning already on the map.
        let seeded_alerts: Vec<GeoFeature> = Vec::new();
        let known_warning_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        // Zone geometry (county and forecast-zone shapes) never changes, so it outlives the run.
        if let Some(dir) = crate::paths::cache_dir() {
            wxdata::alerts::set_zone_cache_dir(dir);
        }
        // Whatever the last run's quiet hours were still holding when it closed.
        let quiet_pending = settings.quiet_pending.clone();
        // The user's cap overrides, before anything that sweeps or reports against them.
        crate::tiles::set_cache_caps(settings.tile_disk_cache_mb, settings.volume_cache_mb);
        // Archived volumes are kept on disk forever within a cap; same startup sweep the tile
        // caches get, for the same reason (mid-session deletion would race the fetch tasks).
        if let Some(root) = crate::paths::cache_dir().map(|d| d.join("volumes")) {
            crate::tiles::sweep_later(root, "volume cache", crate::tiles::volume_cache_bytes());
        }
        // The small caches had no sweep at all: zone geometry and archived RAOB soundings grew
        // for the life of the install. They are small enough that the cap is a tripwire.
        if let Some(dir) = crate::paths::cache_dir() {
            for (sub, label) in [
                ("zones", "zone cache"),
                ("raob", "RAOB cache"),
                ("snapshots", "snapshot cache"),
                ("pficons", "placefile icon cache"),
            ] {
                crate::tiles::sweep_later(dir.join(sub), label, crate::tiles::SMALL_CACHE_BYTES);
            }
        }
        let mut tiles = TileManager::new(spawner.clone());
        let mut vtiles = crate::vector_tiles::VectorTileManager::new(spawner.clone());
        // Tile workers wake the UI the moment a tile is ready; without this a finished tile waits
        // for the next repaint the app happens to want.
        tiles.set_ctx(cc.egui_ctx.clone());
        vtiles.set_ctx(cc.egui_ctx.clone());
        // One small JSON fetch up front: a chase pack can ask for street tiles while a raster
        // basemap is showing, and without the template `pack_jobs` would return nothing.
        // Not on the web, where there are no chase packs and this is one more request racing the
        // radar on the critical path — `request_missing` calls it anyway if vector tiles are used.
        #[cfg(not(target_arch = "wasm32"))]
        vtiles.ensure_template();
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        let (overlay_tx, overlay_rx) = std::sync::mpsc::channel();
        // Loaded off the launch path: the snapshot is a few MB of JSON, and parsing it before
        // the first paint bought nothing — it is applied as `OverlayMsg::AlertSeed`, and dropped
        // if the live fetch has already landed by then.
        {
            let tx = overlay_tx.clone();
            spawner.spawn(async move {
                let feats = wxdata::task::blocking(crate::alert_snapshot::load)
                    .await
                    .unwrap_or_default();
                if !feats.is_empty() {
                    let _ = tx.send(OverlayMsg::AlertSeed(feats));
                }
            });
        }
        let (update_tx, update_rx) = std::sync::mpsc::channel();
        let (geocode_tx, geocode_rx) = std::sync::mpsc::channel();
        let (ipgeo_tx, ipgeo_rx) = std::sync::mpsc::channel::<(f64, f64)>();
        #[cfg(not(target_arch = "wasm32"))]
        drop(ipgeo_tx); // native never asks; the receiver just stays empty
        let (pf_icon_tx, pf_icon_rx) = std::sync::mpsc::channel();
        let (blockage_tx, blockage_rx) = std::sync::mpsc::channel();
        // Every app-level fetch (alerts, overlays, placefiles, radar index) goes through this one.
        // A hung request with no timeout leaves whatever it was loading stuck loading forever.
        let http = crate::platform::http_timeouts(reqwest::Client::builder())
            .build()
            .unwrap_or_default();

        // Open on the saved startup view if set (and its site still resolves), else where the app
        // was last looking, else the default site.
        let resume = settings.start_view.as_ref().or(settings.last_view.as_ref());
        // Nothing to resume means the default site is a guess, not a choice — the browser build
        // improves on it below with the edge's geo-IP.
        #[cfg(target_arch = "wasm32")]
        let opened_on_default =
            !matches!(resume, Some(sv) if wxdata::sites::site_by_id(&sv.site).is_some());
        let (start, camera) = match resume {
            Some(sv) if wxdata::sites::site_by_id(&sv.site).is_some() => (
                sv.site.clone(),
                Camera {
                    center: (sv.x, sv.y),
                    zoom: sv.zoom,
                },
            ),
            _ => {
                let s = settings.default_site.clone();
                let cam = wxdata::sites::site_by_id(&s)
                    .map(|site| Camera::at_lonlat(site.longitude as f64, site.latitude as f64, 8.0))
                    .unwrap_or_else(|| Camera::at_lonlat(-97.28, 35.33, 8.0));
                (s, cam)
            }
        };
        let mut view = MapView::new(Some(start.clone()), camera);
        view.smooth = settings.smooth_radar;
        // Restore the persisted basemap (empty slug = keep the default; from_slug("") = None).
        if !settings.basemap.is_empty() {
            view.basemap = crate::tiles::BasemapStyle::from_slug(&settings.basemap);
        }
        let settings_setup_done = settings.setup_done;

        let mut app = Self {
            vtiles,
            labels: crate::labelplace::Placer::default(),
            spawner,
            #[cfg(not(target_arch = "wasm32"))]
            _rt: rt,
            tiles,
            saved: settings.clone(),
            settings,
            views: vec![view],
            active: 0,
            msg_rx,
            msg_tx,
            about_open: false,
            update_state: ui::about_window::UpdateState::Idle,
            update_chip_hidden: false,
            update_tx,
            update_rx,
            geocode_tx,
            geocode_rx,
            #[cfg(target_arch = "wasm32")]
            ipgeo_tx,
            ipgeo_rx,
            chasepack: None,
            pane_shown: std::collections::HashMap::new(),
            pane_lut: std::collections::HashMap::new(),
            theme_applied: None,
            settings_checked: None,
            frame_nr: 0,
            palette_cache: None,
            vlabel_cache: None,
            nowcast_cache: None,
            tds_cache: None,
            tbss_cache: None,
            zdr_cache: None,
            couplet_cache: None,
            celltrack_cache: LruCache::new(NonZeroUsize::new(48).unwrap()),
            tracks_cache: None,
            show_local_tracks: false,
            site_dialog: None,
            firstrun: {
                let mut w = ui::firstrun::FirstRun::default();
                // Never in an embed: the host page already chose the site, and its storage is
                // partitioned, so "first run" would be every run — a setup dialog over someone
                // else's dashboard panel, forever.
                if !settings_setup_done && !is_embed() {
                    w.start();
                }
                w
            },
            tour: Default::default(),
            tour_anchors: Default::default(),
            settings_window: Default::default(),
            palettes: Palettes::default(),
            live_stream: None,
            live_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_stream_attempt: None,
            // DVR: retain a deep buffer of decoded volumes so instant replay serves recent frames
            // from RAM without re-downloading (~30 volumes ≈ 2.5 h at a 5-min cadence).
            // Phones can't hold a 2.5 h DVR buffer of decoded volumes — each is tens of MB and
            // Android kills the process long before the LRU fills.
            // On Android the cap used to be 6 against a 10-frame loop window, so every wrap of the
            // loop missed on every frame and re-downloaded the whole thing, forever. It has to
            // hold the window plus the head and the frame being fetched.
            prefetching: Arc::new(Mutex::new(PrefetchBook::new())),
            autoplay_pending: cfg!(target_arch = "wasm32"),
            boot_at: Instant::now(),
            scan_cache: LruCache::new(NonZeroUsize::new(scan_cache_cap).unwrap()),
            http,
            overlay_rx,
            overlay_tx,
            filters: OverlayFilters::default(),
            // Seeded from the last run so a restart mid-outbreak draws the warnings that are
            // already on the ground, and doesn't re-banner them as new (see `alert_snapshot`).
            alert_features: seeded_alerts,
            arch_warns: LruCache::new(NonZeroUsize::new(50).unwrap()),
            arch_warn_inflight: None,
            arch_warn_shown: None,
            arch_lsr: LruCache::new(NonZeroUsize::new(50).unwrap()),
            arch_lsr_inflight: None,
            arch_lsr_shown: None,
            outlook_features: std::array::from_fn(|_| Vec::new()),
            md_features: Vec::new(),
            watch_features: Vec::new(),
            wssi_features: Vec::new(),
            ero_features: Vec::new(),
            show_mping: false,
            mping_reports: Vec::new(),
            mping_last_fetch: None,
            show_recon: false,
            recon: Vec::new(),
            recon_last_fetch: None,
            tropical_wind_kt: None,
            tropical_surge: false,
            show_pireps: false,
            pireps: Vec::new(),
            pirep_last_fetch: None,
            show_probsevere: false,
            probsevere: Vec::new(),
            probsevere_last_fetch: None,
            overlays: Vec::new(),
            overlay_gen: 0,
            built_gen: u64::MAX,
            built_zoom_bucket: i32::MIN,
            built_theme: crate::settings::Theme::Dark,
            pending_overlay: None,
            overlay_ready: false,
            overlay_last_fetch: None,
            detail: None,
            cell_popup: None,
            marker_popup: None,
            global_model: wxdata::global::GlobalModel::default(),
            global_fcst_hour: 0,
            global_layer_key: std::collections::HashMap::new(),
            diff_field: crate::fielddiff::DiffField::default(),
            diff_valid: None,
            diff_grid: None,
            goto_poll: None,
            diff_key: None,
            sounding_at: None,
            zone_pts: Vec::new(),
            zone_naming: None,
            zone_popup: None,
            pending_spotter: None,
            video_player: None,
            cells_window: Default::default(),
            help_hub: Default::default(),
            rules_window: Default::default(),
            drawer: Default::default(),
            popovers: Default::default(),
            verify_window: Default::default(),
            verify_rx: None,
            xsection_moment: Moment::Reflectivity,
            follow_cell: None,
            follow_notice: None,
            warning_popup: None,
            error_chip: None,
            hrrr_layer_hour: std::collections::HashMap::new(),
            storm_cells: Vec::new(),
            ui_scale_applied: -1.0,
            ime_shown: false,
            pending_paste: None,
            paste_target: None,
            placefiles: Vec::new(),
            placefile_label_cache: None,
            placefile_window: Default::default(),
            last_viewport: (1000.0, 800.0),
            tool: MapTool::default(),
            measure: Vec::new(),
            strokes: Vec::new(),
            draw_color: DRAW_COLORS[0],
            marker_window: Default::default(),
            event_window: Default::default(),
            chase_replay: Default::default(),
            palette_editor: Default::default(),
            digest_window: Default::default(),
            digest_rx: None,
            sounding_window: Default::default(),
            sounding_rx: None,
            raob_rx: None,
            chase_mode: false,
            spoke_pos: None,
            recent_hits: Vec::new(),
            chase_pos: None,
            chase_track: crate::chaselog::Track::default(),
            warmed_site: None,
            climo_tracks: None,
            climo_rx: None,
            climo_hits: Vec::new(),
            climo_center: None,
            climo_open: false,
            climo_loading: false,
            climo_error: None,
            climo_pending_query: None,
            climo_warn: None,
            climo_warn_rx: None,
            chase_applied: None,
            gps_rx: None,
            sync_tokens: crate::cloud::Tokens::load(),
            sync_state: crate::cloud::SyncState::load(),
            sync_login: None,
            sync_status: String::new(),
            sync_rx: None,
            sync_checked: None,
            share: None,
            peers: std::collections::HashMap::new(),
            share_sent: None,
            goes_times: Vec::new(),
            goes_times_style: None,
            goes_time_idx: None,
            goes_follow_radar: true,
            goes_times_rx: None,
            goes_hour: None,
            glm_fed_prev: None,
            widget_shot_at: None,
            snapshot_push: None,
            rotation_alerted: std::collections::HashMap::new(),
            last_chime: None,
            quiet_queue: std::sync::Mutex::new(quiet_pending),
            rollup: std::sync::Mutex::default(),
            was_quiet: false,
            screenshot_pending: None,
            share_card: None,
            loop_export: None,
            link_cameras: false,
            mini_loop: false,
            #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
            mini_cam: None,
            #[cfg(not(target_arch = "wasm32"))]
            crash_report: crate::crash::take_report(),
            #[cfg(target_arch = "wasm32")]
            crash_report: None,
            cells_site: None,
            cell_trends: std::collections::HashMap::new(),
            fields: crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .map(|&l| (l, FieldState::default()))
                .collect(),
            rotation_minutes: 30,
            hail_minutes: 1440,
            env_cape_ml: false,
            env_srh_km: 3,
            l3grid_site: None,
            derived_key: None,
            snow_hours: 24,
            snow_fetched: None,
            freezing: None,
            freezing_last_fetch: None,
            show_metar: false,
            metars: Vec::new(),
            tafs: Default::default(),
            metar_last_fetch: None,
            metar_bounds: None,
            show_gauges: false,
            gauges: Vec::new(),
            gauge_last_fetch: None,
            gauge_bounds: None,
            contour_kind: ContourKind::Off,
            contours: Vec::new(),
            contour_valid: None,
            contour_last_fetch: None,
            contour_fetched_kind: None,
            env_model: wxdata::hrrr::Model::Hrrr,
            show_tropical: true,
            tropical: None,
            tropical_last_fetch: None,
            show_outages: true,
            outage_features: Vec::new(),
            outages_last_fetch: None,
            show_cappi: false,
            cappi_alt_km: 3.0,
            cappi_tex: None,
            cappi_key: None,
            hrrr_fcst_hour: 1,
            hrrr_fetched_hour: None,
            hrrr_run: None,
            hrrr_valid: None,
            hrrr_last_fetch: None,
            hrrr_by_timeline: false,
            tray_rx: tray_rx_init,
            tray_state: crate::tray::TrayState::default(),
            tray_present: tray_present_init,
            really_quit: false,
            show_storm_reports: false,
            storm_reports: Vec::new(),
            reports_last_fetch: None,
            show_aviation: false,
            aviation_features: Vec::new(),
            aviation_last_fetch: None,
            precip_flag_grid: None,
            precip_flag_gen: 0,
            show_tfr: false,
            tfr_features: std::collections::HashMap::new(),
            tfr_last_fetch: None,
            tfr_pending: 0,
            afd_open: false,
            afd: None,
            afd_error: None,
            afd_busy: false,
            afd_rx: None,
            tropical_window: ui::tropical_window::TropicalWindow::default(),
            tropical_text_rx: None,
            show_range_rings: false,
            show_radar_sites: true,
            show_blockage: false,
            blockage_tex: None,
            blockage_pending: None,
            blockage_rx,
            blockage_tx,
            // Map-first by default on both platforms: the floating chrome covers the common paths,
            // and the full toolbox is one "Advanced" tap away.
            chrome_rect: egui::Rect::EVERYTHING,
            layers_query: String::new(),
            panel_open: false,
            basemap_open: false,
            sidebar_focus_search: false,
            show_cheatsheet: false,
            capture_key: false,
            place_query: String::new(),
            place_status: None,
            save_offer: None,
            geocode_nav: false,
            layer_window_open: false,
            pf_icon_tex: std::collections::HashMap::new(),
            pf_icon_rx,
            pf_icon_tx,
            mobile_chrome_hidden: false,
            last_tap: None,
            tap_zoom: None,
            mobile_occlusion: Vec::new(),
            last_gesture_end: None,
            show_spotters: false,
            show_webcams: false,
            webcams: Vec::new(),
            show_fires: false,
            fire_perims: Vec::new(),
            fire_incidents: Vec::new(),
            fire_bounds: None,
            fire_last_fetch: None,
            show_aqi: false,
            aqi: Vec::new(),
            aqi_bounds: None,
            aqi_last_fetch: None,
            webcam_bounds: None,
            webcam_last_fetch: None,
            show_stations: false,
            stations: Default::default(),
            station_last_poll: None,
            ppef_last_fetch: None,
            dotcam_bounds: None,
            #[cfg(not(target_arch = "wasm32"))]
            nwr: None,
            nwr_pick: String::new(),
            show_dat: false,
            dat_points: Vec::new(),
            dat_tracks: Vec::new(),
            dat_key: None,
            mosaic_sites: Vec::new(),
            mosaic_oldest: None,
            mosaic_bounds: None,
            spotters: Vec::new(),
            spotters_last_fetch: None,
            show_sensors: false,
            sensor_data: None,
            sensor_site: None,
            sensor_last_fetch: None,
            show_hodo: false,
            hodo_data: Vec::new(),
            hodo_history: std::collections::VecDeque::new(),
            hodo_tab: Default::default(),
            forecast_open: false,
            forecast_at: None,
            forecast_state: ui::forecast_window::State::Loading,
            forecast_rx: None,
            forecast_cache: std::collections::HashMap::new(),
            forecast_obs_rx: None,
            forecast_obs_cache: std::collections::HashMap::new(),
            minute_profile: None,
            minute_key: None,
            rain_detector: Default::default(),
            rain_key: None,
            glm_fed_last: None,
            rules_key: None,
            rules_fired: std::collections::HashMap::new(),
            rain_eta: Vec::new(),
            show_fronts: false,
            fronts: None,
            fronts_last_fetch: None,
            show_glm: false,
            show_strikes: false,
            strikes: std::collections::VecDeque::new(),
            glm: std::sync::Arc::new(std::sync::Mutex::new(wxdata::glm::GlmFeed::new(15))),
            glm_last_poll: None,
            glm_polling: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            show_wind: false,
            wind: None,
            wind_level: wxdata::hrrr::WindLevel::Surface,
            wind_particles: std::collections::HashMap::new(),
            wind_on_gpu: std::env::var("HOOKECHO_CPU_WIND").is_err(),
            wind_uploaded: None,
            wind_fetched: None,
            wind_last_fetch: None,
            wind_inflight: None,
            wind_last_frame: None,
            wind_dt: 0.0,
            hodo_site: None,
            hodo_last_fetch: None,
            obs_mode: false,
            embed: is_embed(),
            embed_live: false,
            last_input: Instant::now(),
            gesture_live: false,
            #[cfg(not(target_arch = "wasm32"))]
            perf: PerfReadout::new(),
            #[cfg(target_arch = "wasm32")]
            last_posted: None,
            obs_tour: false,
            obs_tour_last: None,
            obs_tour_idx: 0,
            known_warning_ids,
            warnings_seeded: false,
            lightning_alerted: std::collections::HashMap::new(),
            tds_active: false,
            rot_active: false,
            warning_banners: Vec::new(),
            toasts: Vec::new(),
            feed_errors_told: std::collections::HashMap::new(),
            show_alert_panel: false,
            xsection_pts: Vec::new(),
            xsection: None,
            xsection_tex: None,
            marker_icon_tex: Default::default(),
            show_3d: false,
            vol3d: Default::default(),
            vol3d_key: None,
            vol3d_rx: None,
            vol3d_range: (-30.0, 80.0),
            vol3d_pending: None,
            max_texture_dim,
            volume3d_supported,
        };
        // Restore the overlays from last time, assigning rather than only ever switching on: the
        // additive version could never turn a default-on layer off, so unchecking one lasted until
        // the next restart and then came back. `None` is "no run has recorded this yet", where the
        // built-in defaults still stand; a recorded list is the whole truth about every layer.
        //
        // Unknown names (an older build reading a newer file) are skipped rather than treated as
        // an error.
        if let Some(saved) = app.settings.overlays_on.clone() {
            let restore: Vec<OverlayToggle> = saved
                .iter()
                .filter_map(|s| OverlayToggle::from_slug(s))
                .collect();
            for t in OverlayToggle::ALL {
                if t.session_only() {
                    continue;
                }
                *app.overlay_flag(t) = restore.contains(&t);
            }
            // Whatever the outcome, the overlay set now differs from the one the constructor built,
            // so the derived features have to be rebuilt from it once.
            app.rebuild_overlays();
        }
        app.palettes.reload(&app.settings.palette_paths());
        app.apply_goto_env();
        app.drain_goto_file();
        #[cfg(target_arch = "wasm32")]
        app.apply_goto_hash();
        #[cfg(target_arch = "wasm32")]
        if opened_on_default {
            app.locate_by_ip(&cc.egui_ctx.clone());
        }
        crate::platform::set_background_alerts(app.settings.background_alerts);
        crate::platform::set_battery_saver(app.settings.battery_saver);
        // The broker is publish-only and reconnects on its own, so it starts here and is never
        // stopped; changing the setting takes a restart, same as the tray.
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        crate::mqtt::spawn(&app.settings, true);
        // Point the speech path at Piper before anything can speak.
        #[cfg(not(target_arch = "wasm32"))]
        crate::speech::set_piper(&app.settings.piper_path, &app.settings.piper_voice);
        // Not on the web: this is a burst of six fetches, and on the one thread a browser gives
        // us they queue ahead of the radar the visitor actually came for. The periodic refresh in
        // `update` picks them up a moment later, once there is radar on screen.
        #[cfg(not(target_arch = "wasm32"))]
        app.fetch_overlays(&cc.egui_ctx.clone());
        app
    }

    /// `HOOKECHO_GOTO=SITE,lon,lat,zoom[,RFC3339]` opens straight onto a view, archive time and
    /// all — the same deep link the Event Library uses, minus the clicking.
    ///
    /// This exists for the screenshot harness (`scripts/shots/`): staging a historic storm by
    /// driving the UI means hunting for a window's row coordinates, which breaks the moment the
    /// layout moves. Companion to the headless `HOOKECHO_CAM`/`HOOKECHO_BASEMAP` knobs.
    fn apply_goto_env(&mut self) {
        let Ok(v) = std::env::var("HOOKECHO_GOTO") else {
            return;
        };
        self.apply_goto(&v);
    }

    /// Consume a `goto.txt` dropped in the storage base by the Android notification tap
    /// (`MainActivity`), which is how a background alert deep-links into the storm it fired on.
    /// The file is deleted as it's read so the jump happens once, not on every resume.
    fn drain_goto_file(&mut self) {
        let Some(path) = crate::paths::goto_file() else {
            return;
        };
        let Ok(v) = std::fs::read_to_string(&path) else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        self.apply_goto(v.trim());
    }

    /// `SITE[,lon,lat,zoom][,extra…]`, with or without the `hookecho://goto/` prefix.
    fn apply_goto(&mut self, v: &str) {
        let Some(g) = parse_goto(v) else {
            log::warn!("HOOKECHO_GOTO: want SITE[,lon,lat,zoom[,RFC3339|product|tilt]], got {v:?}");
            return;
        };
        self.goto_view(&g.site, g.lon, g.lat, g.zoom, g.time);
        let view = &mut self.views[self.active];
        if let Some(m) = g.moment {
            view.moment = m;
        }
        if let Some(t) = g.tilt {
            view.tilt = t;
        }
        if let Some(slug) = &g.basemap {
            view.basemap = crate::tiles::BasemapStyle::from_slug(slug);
        }
        // After the moment, so `VEL,thr:15` thresholds velocity and not whatever was showing.
        if let Some(t) = g.threshold {
            let mi = view.moment.index();
            view.threshold_enabled[mi] = t.is_some();
            if t.is_some() {
                view.thresholds[mi] = t;
            }
        }
        if g.srv {
            view.srv = true;
        }
    }

    /// The browser's own deep link: `https://…/#goto=KTLX,-97.3,35.3,9`. Read once at boot — a
    /// fragment change afterwards is someone editing the URL bar, not a share being opened.
    /// Percent-escapes are decoded by `parse_goto`, so a fragment that came back from a chat
    /// client with its commas and colons escaped still opens.
    #[cfg(target_arch = "wasm32")]
    fn apply_goto_hash(&mut self) {
        let Some(h) = web_sys::window().and_then(|w| w.location().hash().ok()) else {
            return;
        };
        if let Some(v) = h.strip_prefix("#goto=") {
            self.apply_goto(v);
            // A shared link points at one moment on purpose. Auto-playing away from it would
            // throw away the thing the sender was pointing at.
            self.autoplay_pending = false;
        }
    }

    /// Ask our own origin where the visitor is (`/geo.json`, answered by the Pages Worker
    /// or the Cloudflare Worker from the request's geo-IP) and open on the nearest radar site.
    ///
    /// Same-origin, so no proxy and no CORS; the reply is `[lon, lat]` or `null` when the edge has
    /// no fix. A deep link wins — someone who opened a share link asked for that site, not this one.
    #[cfg(target_arch = "wasm32")]
    fn locate_by_ip(&mut self, ctx: &egui::Context) {
        let Some(win) = web_sys::window() else {
            return;
        };
        if win.location().hash().is_ok_and(|h| h.starts_with("#goto=")) {
            return;
        }
        let Ok(origin) = win.location().origin() else {
            return;
        };
        let http = self.http.clone();
        let tx = self.ipgeo_tx.clone();
        let ctx = ctx.clone();
        self.spawner.spawn(async move {
            // index.html starts this fetch before the wasm has even downloaded, so by now it has
            // almost always landed: the opening site costs no round trip. Falling back to our own
            // request keeps the app working when the page didn't (an embed, a local build).
            let body = match Self::page_geo().await {
                Some(b) => b,
                None => {
                    let Ok(resp) = http.get(format!("{origin}/geo.json")).send().await else {
                        return;
                    };
                    let Ok(b) = resp.text().await else {
                        return;
                    };
                    b
                }
            };
            if let Ok(Some(pos)) = serde_json::from_str::<Option<(f64, f64)>>(&body) {
                let _ = tx.send(pos);
                ctx.request_repaint();
            }
        });
    }

    /// The body of the `/geo.json` fetch `web/index.html` started at page load, if it started one.
    #[cfg(target_arch = "wasm32")]
    async fn page_geo() -> Option<String> {
        use wasm_bindgen::JsCast as _;
        let promise: js_sys::Promise = js_sys::Reflect::get(&js_sys::global(), &"__geo".into())
            .ok()?
            .dyn_into()
            .ok()?;
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .ok()?
            .as_string()
    }

    /// Post the active pane's state to the embedding page (`?embed`), tagged so the host can tell
    /// it apart from every other frame shouting into the same window. `"*"` as the target origin:
    /// this is view state, nothing secret, and the host has to origin-check us regardless.
    #[cfg(target_arch = "wasm32")]
    fn post_state_to_parent(&mut self) {
        let snap = crate::workspace::PaneSnap::capture(&self.views[self.active]);
        if self.last_posted.as_ref() == Some(&snap) {
            return;
        }
        let Some(win) = web_sys::window() else {
            return;
        };
        // No parent (or we are the top frame): nobody to tell.
        let Ok(parent) = win.parent() else { return };
        let Some(parent) = parent.filter(|p| p != &win) else {
            return;
        };
        let Ok(mut v) = serde_json::to_value(&snap) else {
            return;
        };
        if let Some(o) = v.as_object_mut() {
            o.insert("hookecho".into(), serde_json::json!(1));
            // The product goes out as its share-link code ("REF"), not the serde variant name:
            // the host hands this straight back to us in a `#goto`, and `Moment::from_code` only
            // knows the codes. Round-tripping the variant name would silently drop the product.
            o.insert("moment".into(), serde_json::json!(snap.moment.short_name()));
        }
        if parent
            .post_message(&wasm_bindgen::JsValue::from_str(&v.to_string()), "*")
            .is_ok()
        {
            self.last_posted = Some(snap);
        }
    }

    /// Volume poll cadence, doubled on a metered link. A phone on mobile data pulls a multi-MB
    /// volume every interval; halving that rate costs at most a couple of minutes of latency on
    /// the live head, which the chunk stream covers anyway when it is running.
    fn poll_interval_secs(&self) -> u64 {
        let base = self.settings.poll_interval_secs;
        let base = if crate::platform::is_metered() {
            base * 2
        } else {
            base
        };
        // Battery saver stacks with metering: both are "spend less", and a chaser who has turned
        // both on has said so twice.
        if self.settings.battery_saver {
            base * 2
        } else {
            base
        }
    }

    /// Largest edge a national field grid may keep. Bounded by what the GPU will accept, and
    /// then hard-capped: at 8192 a single f32 grid is ~268 MB of RAM before it is ever indexed,
    /// which neither a phone nor a browser tab should be asked to hold — and the Adreno 750
    /// reports 16384, so the device limit alone never bit. 4096 is still finer than either
    /// screen can show.
    fn field_texture_cap(&self) -> usize {
        let ceiling = if cfg!(target_os = "android") || cfg!(target_arch = "wasm32") {
            // The browser is in the phone's bracket here, not the desktop's: an 8192 f32 grid is
            // ~268 MB staged before it is ever indexed, and a wasm heap only grows.
            4096
        } else {
            8192
        };
        (self.max_texture_dim as usize).min(ceiling)
    }

    /// Spawn background fetches for all overlay sources (alerts, SPC outlooks, MDs).
    /// Spawn a background overlay fetch, routing the result to `overlay_rx`.
    /// Which moments the active pane's volume carries. All true when nothing is loaded, so an
    /// empty pane still offers the full product list.
    fn available_moments(&self) -> [bool; Moment::ALL.len()] {
        // The pane's remembered union, not this instant's volume: a half-arrived live volume
        // carries fewer moments than the radar sends, and the rows must not blink.
        self.views[self.active].moments()
    }

    /// Keep the beam-blockage raster in step with the camera, the site, and the displayed tilt.
    ///
    /// Building one fetches DEM tiles, so it runs as a task and lands on `blockage_rx`. The old
    /// raster keeps painting (stretched to its own world rect) until the new one arrives, which is
    /// what makes a pan look continuous instead of blinking.
    fn update_blockage(&mut self, ctx: &egui::Context) {
        while let Ok((key, world, image)) = self.blockage_rx.try_recv() {
            let tex = ctx.load_texture("blockage", image, egui::TextureOptions::LINEAR);
            self.blockage_tex = Some((key, tex, world));
            self.blockage_pending = None;
        }
        if !self.show_blockage {
            self.blockage_tex = None;
            self.blockage_pending = None;
            return;
        }
        let view = &self.views[self.active];
        let (Some(site), Some(vol)) = (
            view.site.as_deref().and_then(wxdata::sites::site_by_id),
            view.volume.as_ref(),
        ) else {
            return;
        };
        let Some(tilt_deg) = vol.elevations.get(view.tilt).copied() else {
            return;
        };
        let cam = &view.camera;
        let vp = self.last_viewport;
        let (wx0, wy0) = cam.screen_to_world((0.0, 0.0), vp);
        let (wx1, wy1) = cam.screen_to_world((vp.0, vp.1), vp);
        let world = [wx0, wy0, wx1, wy1];
        let key = BlockageKey {
            site: site.id.to_string(),
            tilt_mdeg: (tilt_deg * 1000.0) as i32,
            world: world.map(|w| (w * 1e7) as i64),
        };
        if self.blockage_tex.as_ref().is_some_and(|(k, ..)| *k == key)
            || self.blockage_pending.as_ref().is_some_and(|(k, at)| {
                *k == key || at.elapsed() < std::time::Duration::from_millis(600)
            })
        {
            return;
        }
        let beam = crate::elevation::BeamSite {
            lon: site.longitude as f64,
            lat: site.latitude as f64,
            // The site registry is the elevation source; the tower table adds the antenna.
            ground_m: site.elevation_meters as f64,
            tower_m: wxdata::towers::tower_m(site.id),
            tilt_deg: tilt_deg as f64,
        };
        self.blockage_pending = Some((key.clone(), Instant::now()));
        let http = self.http.clone();
        let tx = self.blockage_tx.clone();
        let ctx = ctx.clone();
        self.spawner.spawn(async move {
            let image = crate::elevation::blockage_image(&http, beam, world).await;
            let _ = tx.send((key, world, image));
            ctx.request_repaint();
        });
    }

    /// Recompute the locally derived products (VIL, VIL density, echo tops) when the active pane's
    /// volume, the echo-top threshold, or the set of enabled derived layers changed.
    ///
    /// Unlike every other field layer this costs no network — the volume is already decoded here —
    /// so it has no cadence: it recomputes exactly when its inputs move, which is what makes it
    /// work in archive replay and on each live tilt.
    fn recompute_derived(&mut self, ctx: &egui::Context) {
        use crate::render::FieldLayer as FL;
        const LAYERS: [FL; 6] = [
            FL::CompositeLocal,
            FL::VilLocal,
            FL::VilDensity,
            FL::EtopLocal,
            FL::HailMehs,
            FL::HailPosh,
        ];
        /// Bit positions in the mask for the two hail grids.
        const HAIL_BITS: u8 = 0b11000;
        let mask = LAYERS
            .iter()
            .enumerate()
            .fold(0u8, |m, (i, l)| m | u8::from(self.field_wanted(*l)) << i);
        if mask == 0 {
            self.derived_key = None;
            return;
        }
        let site = self.views[self.active].site.clone();
        // The hail algorithm needs the melting level, and the only source for it is the current
        // model analysis — so, like the live mosaic, the hail grids are live-only rather than
        // quietly applying today's freezing level to a storm from 2021.
        let live = self.views[self.active].timeline.following;
        let mask = if live { mask } else { mask & !HAIL_BITS };
        if mask == 0 {
            self.derived_key = None;
            return;
        }
        // Freezing levels are only worth a request when a hail grid is actually on.
        let levels = match (mask & HAIL_BITS != 0, &self.freezing, &site) {
            (true, Some((s, h0, hm20)), Some(cur)) if s == cur => Some((*h0, *hm20)),
            _ => None,
        };
        if mask & HAIL_BITS != 0 && levels.is_none() {
            self.fetch_freezing_levels(ctx);
        }
        // Beam heights are above the radar; the model heights are above sea level.
        let radar_m = site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
            .map_or(0.0, |s| s.elevation_meters as f64);
        let Some(vol) = self.views[self.active].volume.as_mut() else {
            return;
        };
        let key = (
            vol.name.clone(),
            self.settings.etop_dbz.to_bits(),
            mask,
            levels.map_or(0, |(h0, _)| h0 as i32),
        );
        if self.derived_key.as_ref() == Some(&key) {
            return;
        }
        // Binning is cached on the volume; the integral is the expensive half and runs off-thread.
        let sweeps = vol.reflectivity_tilts();
        if sweeps.len() < 2 {
            return;
        }
        let opts = wxdata::derived::DerivedOpts {
            etop_dbz: self.settings.etop_dbz,
            time: vol.time,
            ..Default::default()
        };
        self.derived_key = Some(key);
        let tx = self.overlay_tx.clone();
        let cap = self.field_texture_cap();
        let ctx = ctx.clone();
        self.spawner.spawn_blocking(move || {
            let mut out: Vec<(FL, wxdata::mrms::MrmsField)> = Vec::new();
            if mask & !HAIL_BITS != 0 {
                if let Some(d) = wxdata::derived::derive(&sweeps, &opts) {
                    out.extend([
                        (FL::CompositeLocal, d.composite),
                        (FL::VilLocal, d.vil),
                        (FL::VilDensity, d.vild),
                        (FL::EtopLocal, d.etop),
                    ]);
                }
            }
            if let Some((h0, hm20)) = levels.filter(|_| mask & HAIL_BITS != 0) {
                if let Some(h) = wxdata::derived::hail(&sweeps, h0 - radar_m, hm20 - radar_m, &opts)
                {
                    out.extend([(FL::HailMehs, h.mehs), (FL::HailPosh, h.posh)]);
                }
            }
            for (layer, f) in out {
                let bit = LAYERS.iter().position(|l| *l == layer).unwrap_or(0);
                if mask & (1 << bit) != 0 {
                    let _ = tx.send(OverlayMsg::Field(layer, f.decimated(cap)));
                }
            }
            ctx.request_repaint();
        });
    }

    /// Refresh the melting-level heights the hail grids need, on the environment cadence.
    fn fetch_freezing_levels(&mut self, ctx: &egui::Context) {
        let Some(site) = self.views[self.active]
            .site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
        else {
            return;
        };
        if self
            .freezing_last_fetch
            .is_some_and(|t| t.elapsed().as_secs() < 900)
        {
            return;
        }
        self.freezing_last_fetch = Some(Instant::now());
        self.spawn_overlay(
            ctx,
            OverlaySource::FreezingLevels(site.longitude as f64, site.latitude as f64),
        );
    }

    fn spawn_overlay(&self, ctx: &egui::Context, source: OverlaySource) {
        let http = self.http.clone();
        let tx = self.overlay_tx.clone();
        let ctx = ctx.clone();
        let cap = self.field_texture_cap();
        self.spawner.spawn(async move {
            // Deliberately shorter than the 120 s refresh that drives this: a fetch that cannot
            // outlive its own cadence cannot stack. Before, a feed the network swallowed left a
            // task alive forever and the next tick started another one on top of it.
            match wxdata::task::timeout(OVERLAY_TIMEOUT, source.fetch(&http))
                .await
                .unwrap_or_else(Err)
            {
                Ok(msg) => {
                    // Max-pool oversized grids here, on the fetch task: MRMS rotation tracks and
                    // AzShear arrive 14000x7000, and doing this on the UI thread stalled a frame
                    // for the whole pool.
                    let msg = match msg {
                        OverlayMsg::Field(layer, f) => OverlayMsg::Field(layer, f.decimated(cap)),
                        other => other,
                    };
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                }
                Err(e) => note_feed_error("Overlay", e),
            }
        });
    }

    /// Hazard kind for the current outlook day: probabilistic layers exist only for Day 1;
    /// Days 2–3 always fetch the categorical risk.
    fn outlook_kind_for_day(&self) -> wxdata::spc::OutlookKind {
        if self.filters.outlook_day == 1 {
            self.filters.outlook_kind
        } else {
            wxdata::spc::OutlookKind::Categorical
        }
    }

    fn fetch_overlays(&mut self, ctx: &egui::Context) {
        self.overlay_last_fetch = Some(Instant::now());
        // Scope zone-only alert resolution (heat, advisories) to the active radar and to every
        // saved marker, so an advisory at the far edge of a wide zone resolves for the places
        // people actually care about, not just the radar — see `alerts::fetch_active`.
        let mut points: Vec<(f64, f64)> = self.views[self.active]
            .site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
            .map(|s| (s.latitude as f64, s.longitude as f64))
            .into_iter()
            .collect();
        points.extend(self.settings.markers.iter().map(|m| (m.lat, m.lon)));
        self.spawn_overlay(ctx, OverlaySource::Alerts(points, self.view_bounds()));
        self.spawn_overlay(ctx, OverlaySource::Mds);
        self.spawn_overlay(ctx, OverlaySource::Watches);
        if (1..=3).contains(&self.filters.wssi_day) {
            self.spawn_overlay(ctx, OverlaySource::Wssi(self.filters.wssi_day));
        }
        if (1..=3).contains(&self.filters.ero_day) {
            self.spawn_overlay(ctx, OverlaySource::Ero(self.filters.ero_day));
        }
        // Only fetch the SPC outlook the user has selected (off = day 0 fetches nothing).
        if (1..=8).contains(&self.filters.outlook_day) {
            self.spawn_overlay(
                ctx,
                OverlaySource::Outlook(self.filters.outlook_day, self.outlook_kind_for_day()),
            );
        }
        // Storm cells for the active view's site (Level 3 products are per-site). Terminal and
        // DWD radars don't publish the storm-cell algorithms under their four-letter id.
        if let Some(site) = self.views[self.active]
            .site
            .clone()
            .filter(|s| wxdata::sites::is_nexrad(s))
        {
            self.spawn_overlay(ctx, OverlaySource::Cells(site));
        }
    }

    /// Reconcile loaded placefiles with `settings.placefiles`: fetch new/enabled URLs, drop
    /// removed ones, mirror the enabled flag, and refetch on each file's `RefreshSeconds`.
    fn sync_placefiles(&mut self, ctx: &egui::Context) {
        // Plugins ride the same pipeline as placefiles — they produce the same format — keyed by
        // a synthetic `plugin:<name>` instead of a URL.
        let plugin_keys: Vec<(String, bool)> = self
            .settings
            .plugins
            .iter()
            .map(|p| (format!("plugin:{}", p.name), p.enabled))
            .collect();
        // Drop entries no longer configured.
        let before = self.placefiles.len();
        self.placefiles.retain(|lp| {
            self.settings.placefiles.iter().any(|c| c.url == lp.url)
                || plugin_keys.iter().any(|(k, _)| *k == lp.url)
        });
        let mut changed = self.placefiles.len() != before;
        for (key, enabled) in &plugin_keys {
            match self.placefiles.iter_mut().find(|lp| lp.url == *key) {
                Some(lp) => {
                    if lp.enabled != *enabled {
                        lp.enabled = *enabled;
                        changed = true;
                    }
                }
                None => {
                    changed = true;
                    self.placefiles.push(LoadedPlacefile {
                        url: key.clone(),
                        enabled: *enabled,
                        pf: Default::default(),
                        last_fetch: None,
                        loaded: false,
                        error: None,
                    });
                }
            }
        }
        for cfg in &self.settings.placefiles {
            match self.placefiles.iter_mut().find(|lp| lp.url == cfg.url) {
                Some(lp) => {
                    if lp.enabled != cfg.enabled {
                        lp.enabled = cfg.enabled;
                        changed = true;
                    }
                }
                None => {
                    changed = true;
                    self.placefiles.push(LoadedPlacefile {
                        url: cfg.url.clone(),
                        enabled: cfg.enabled,
                        pf: Default::default(),
                        last_fetch: None,
                        loaded: false,
                        error: None,
                    });
                }
            }
        }
        // Fetch never-loaded and refresh stale (min 15s cadence).
        let mut to_fetch = Vec::new();
        for lp in &self.placefiles {
            if !lp.enabled {
                continue;
            }
            // A plugin's cadence is the user's setting, not the placefile's own RefreshSeconds:
            // a plugin sampling something live should be asked again on a schedule they control.
            let plugin_secs = self
                .settings
                .plugins
                .iter()
                .find(|p| lp.url == format!("plugin:{}", p.name))
                .map(|p| p.refresh_secs);
            let stale = match (lp.last_fetch, plugin_secs) {
                (None, _) => true,
                // A failed plugin retries on its cadence rather than every frame.
                (Some(t), Some(secs)) => t.elapsed().as_secs() >= secs.max(5) as u64,
                (Some(t), None) => {
                    lp.loaded
                        && lp.pf.refresh_secs > 0
                        && t.elapsed().as_secs() >= lp.pf.refresh_secs.max(15) as u64
                }
            };
            if stale {
                to_fetch.push(lp.url.clone());
            }
        }
        for url in to_fetch {
            if let Some(lp) = self.placefiles.iter_mut().find(|lp| lp.url == url) {
                lp.last_fetch = Some(Instant::now());
            }
            let source = match self
                .settings
                .plugins
                .iter()
                .find(|p| url == format!("plugin:{}", p.name))
            {
                #[cfg(not(target_arch = "wasm32"))]
                Some(p) => OverlaySource::Plugin(
                    url.clone(),
                    p.command.clone(),
                    p.args.clone(),
                    self.plugin_context(),
                ),
                #[cfg(target_arch = "wasm32")]
                Some(_) => OverlaySource::Placefile(url),
                None => OverlaySource::Placefile(url),
            };
            self.spawn_overlay(ctx, source);
        }
        if changed {
            self.overlay_gen = self.overlay_gen.wrapping_add(1);
        }
    }

    /// What the active pane is showing, for a plugin to answer about. Archive-aware: scrubbing
    /// back gives the plugin the historic instant, not now.
    #[cfg(not(target_arch = "wasm32"))]
    fn plugin_context(&self) -> crate::plugins::Context {
        let v = &self.views[self.active];
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        crate::plugins::Context {
            site: v.site.clone().unwrap_or_default(),
            bbox: (min_lon, min_lat, max_lon, max_lat),
            time: v
                .timeline
                .current()
                .and_then(|id| id.date_time())
                .unwrap_or_else(chrono::Utc::now),
            product: v.moment.short_name().to_string(),
        }
    }

    /// Vector-mean storm motion (`dir_deg`, `speed_kt`) over the current SCIT storm cells that
    /// carry a movement, or `None` if none do. Averages u/v so directions wrap correctly.
    fn scit_mean_motion(&self) -> Option<(f32, f32)> {
        let (mut u, mut v, mut n) = (0.0f32, 0.0f32, 0u32);
        for c in &self.storm_cells {
            if let (Some(dir), Some(spd)) = (c.mvt_deg, c.mvt_kt) {
                let r = dir.to_radians();
                u += spd * r.sin();
                v += spd * r.cos();
                n += 1;
            }
        }
        if n == 0 {
            return None;
        }
        let (u, v) = (u / n as f32, v / n as f32);
        let dir = u.atan2(v).to_degrees().rem_euclid(360.0);
        Some((dir, (u * u + v * v).sqrt()))
    }

    /// Ask GitHub for the newest tagged release, once per session.
    ///
    /// The list endpoint, not `/releases/latest` — see [`ui::about_window::pick_latest_tag`] for
    /// why. Thirty is more releases than this project has, and one page keeps it to one request
    /// against the 60/hour unauthenticated budget.
    fn check_for_update(&mut self, ctx: &egui::Context) {
        if self.update_state != ui::about_window::UpdateState::Idle {
            return;
        }
        self.update_state = ui::about_window::UpdateState::Checking;
        let http = self.http.clone();
        let tx = self.update_tx.clone();
        let ctx2 = ctx.clone();
        self.spawner.spawn(async move {
            let url = "https://api.github.com/repos/d4vid87/hookecho/releases?per_page=30";
            // GitHub rejects requests without a User-Agent.
            let body = async {
                let text = http
                    .get(url)
                    .header("User-Agent", "hookecho")
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .text()
                    .await
                    .ok()?;
                Some(text)
            }
            .await;
            // `None` means the request itself failed; a body with no version tag in it is a
            // different answer, and the two must not collapse into one message.
            let state = match body {
                Some(body) => match ui::about_window::pick_latest_tag(&body) {
                    Some(tag) => ui::about_window::compare(&tag),
                    None => ui::about_window::UpdateState::NoRelease,
                },
                None => ui::about_window::UpdateState::Failed,
            };
            let _ = tx.send(state);
            ctx2.request_repaint();
        });
    }

    /// Height of pane `idx`'s beam centre above the radar, in feet, over the point `ll`
    /// (`[lon, lat]`). `None` when the pane has no site or no loaded tilt.
    ///
    /// Ground range is close enough to slant range for the shallow tilts this is read at, and the
    /// 4/3-earth model is the same one the cross-section draws with
    /// ([`wxdata::xsection::beam_height_km`]), so the two agree.
    fn beam_height_ft(&self, idx: usize, ll: [f64; 2]) -> Option<f64> {
        let v = &self.views[idx];
        let site = wxdata::sites::site_by_id(v.site.as_deref()?)?;
        let elev = *v.volume.as_ref()?.elevations.get(v.tilt)? as f64;
        let (km, _) = crate::geo::great_circle([site.longitude as f64, site.latitude as f64], ll);
        Some(wxdata::xsection::beam_height_km(km, elev) * 3280.84)
    }

    /// Every moment this volume carries, at the gate under `(lon, lat)`, plus where that gate
    /// is: azimuth and range from the radar, and how high the beam is there.
    ///
    /// The height is the part people forget. A 60 dBZ reading 90 km out on the 0.5 degree cut is
    /// 1.5 km up, so it is not what is reaching the ground, and a velocity couplet is only a
    /// low-level couplet if the beam is low. Showing the number without the height invites the
    /// wrong reading of it.
    ///
    /// Returns `None` when the pointer is off the sweep entirely, so hovering empty map says
    /// nothing rather than flashing an empty tooltip.
    fn gate_readout(&mut self, idx: usize, lon: f64, lat: f64) -> Option<String> {
        let tilt = self.views[idx].tilt;
        let active = self.views[idx].moment;
        let dealias = self.settings.dealias_velocity;
        let carried = self.views[idx].volume.as_ref()?.moments;
        let vol = self.views[idx].volume.as_mut()?;

        // The active moment leads — it is the one being looked at — then the rest in the usual
        // product order, skipping any this volume does not carry.
        let order = std::iter::once(active)
            .chain(Moment::ALL.into_iter().filter(|m| *m != active))
            .filter(|m| carried[m.index()]);

        let mut lines: Vec<String> = Vec::new();
        let mut where_at: Option<String> = None;
        for m in order {
            // Velocity is read dealiased when the display is dealiased, or the number under the
            // cursor would disagree with the colour over it.
            let want_dealias = dealias && m == Moment::Velocity;
            let Ok(sweep) = vol.binned(m, tilt, want_dealias) else {
                continue;
            };
            let Some(g) = sweep.sample_at(lon, lat) else {
                continue;
            };
            if where_at.is_none() {
                where_at = Some(format!(
                    "{:.0}\u{b0} at {:.0} km \u{b7} beam {:.0} ft",
                    g.azimuth_deg,
                    g.range_km,
                    sweep.beam_height_ft(g.range_km)
                ));
            }
            let name = crate::products::info(m).short;
            let units = m.units();
            let value = match (g.value, g.folded) {
                (Some(v), _) => {
                    let precision = if m == Moment::CorrelationCoefficient {
                        3
                    } else {
                        1
                    };
                    if units.is_empty() {
                        format!("{v:.*}", precision)
                    } else {
                        format!("{v:.*} {units}", precision)
                    }
                }
                // "Range folded" and "nothing here" look identical on the map and mean opposite
                // things, so the readout is where the difference gets said out loud.
                (None, true) => "range folded".to_string(),
                (None, false) => "\u{2014}".to_string(),
            };
            let marker = if m == active { "\u{25b8} " } else { "  " };
            lines.push(format!("{marker}{name:<4}{value}"));
        }
        let where_at = where_at?;
        Some(format!("{where_at}\n{}", lines.join("\n")))
    }

    /// Chime when a new volume lands on the live pane you are watching — the "look up" cue for
    /// someone doing something else while a storm is on.
    ///
    /// Only the active pane, only while following the head, and only for a volume newer than the
    /// last one chimed for: a scrub, a backfilled frame, or the same volume growing another chunk
    /// is not a new scan. The first volume after a site switch or a cold start is swallowed, since
    /// that one is the user's own doing.
    fn scan_chime(&mut self, view: usize, time: chrono::DateTime<Utc>) {
        if !self.settings.scan_chime || view != self.active || !self.views[view].timeline.following
        {
            return;
        }
        let first = self.last_chime.is_none();
        if self.last_chime.is_some_and(|t| t >= time) {
            return;
        }
        self.last_chime = Some(time);
        if !first {
            self.play_alert(&self.settings.scan_sound.clone());
        }
    }

    /// Every alert sound goes through here, so one mute switch covers all of them (and any that
    /// get added later) instead of a guard per call site.
    fn play_alert(&self, sound: &crate::settings::AlertSound) {
        if self.settings.mute_alerts || self.in_quiet_hours() {
            return;
        }
        crate::audio::play(sound, self.settings.alert_volume);
        crate::platform::haptic(crate::platform::Haptic::Alert);
    }

    /// A sound quiet hours does not silence: the escalated warning tiers and the two detections
    /// that mean a tornado may be on the ground. `mute_alerts` still wins — that switch is the
    /// user saying so about right now, where quiet hours is a standing preference.
    fn play_alert_urgent(&self, sound: &crate::settings::AlertSound) {
        if self.settings.mute_alerts {
            return;
        }
        crate::audio::play(sound, self.settings.alert_volume);
        crate::platform::haptic(crate::platform::Haptic::Alert);
    }

    /// Everywhere the proximity alerts watch: the saved markers, plus your own live position when
    /// "alert where I am" is on and there is a fix. Returns `(name, lon, lat, radius_mi)`.
    ///
    /// The GPS entry is deliberately not a real marker — a marker that moves would drag its way
    /// through the saved list, and it must vanish the moment the fix or the setting does.
    fn watched_points(&self) -> Vec<WatchedPoint> {
        let mut out: Vec<WatchedPoint> = self
            .settings
            .markers
            .iter()
            .map(|m| WatchedPoint {
                id: m.id.clone(),
                name: m.name.clone(),
                lon: m.lon,
                lat: m.lat,
                radius_mi: m.alert_radius_mi,
            })
            .collect();
        if self.settings.alert_follow_gps {
            if let Some((lon, lat)) = self.chase_pos {
                out.push(WatchedPoint {
                    id: GPS_POINT_ID.to_string(),
                    name: "my location".to_string(),
                    lon,
                    lat,
                    radius_mi: crate::settings::default_alert_radius_mi(),
                });
            }
        }
        out
    }

    /// Is the local clock inside the user's quiet-hours window?
    fn in_quiet_hours(&self) -> bool {
        use chrono::Timelike;
        self.settings.in_quiet_hours(chrono::Local::now().hour())
    }

    /// Surface auxiliary-feed failures queued by [`note_feed_error`], at most once per feed per
    /// half hour.
    ///
    /// Rate-limited rather than silenced: a feed that is down stays down, and a toast every
    /// refresh would be the nag this app does not do — but told-once-forever also swallowed a
    /// genuine second outage hours after the feed had recovered. The log keeps every occurrence.
    fn drain_feed_errors(&mut self) {
        let queued: Vec<(&'static str, String)> = match FEED_ERRORS.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => return,
        };
        for (feed, err) in queued {
            let due = self
                .feed_errors_told
                .get(feed)
                .is_none_or(|t| t.elapsed().as_secs() >= 1800);
            if due {
                self.feed_errors_told.insert(feed, Instant::now());
                self.toast(ToastKind::Error, format!("{feed} unavailable — {err}"));
            }
        }
    }

    /// Detect warning-tier alerts whose id we haven't seen, raising a banner + audible cue for
    /// each new one. The first fetch only seeds the known set (no alert on already-active warnings).
    fn detect_new_warnings(&mut self, feats: &[GeoFeature]) {
        let metric = self.metric();
        let mut alerted = false;
        let mut max_esc = 0u8; // highest escalation among newly-seen warnings this pass
        // Collected, not spoken here: the tone has to play first, and it plays once for the whole
        // pass rather than once per warning.
        let mut to_speak: Vec<String> = Vec::new();
                               // Only banner warnings within the selected radar's coverage — a warning covering a saved
                               // location still banners + pushes regardless (that's a watched place, not the viewed site).
        let site_box = self.active_site_bounds(250.0);
        for f in feats {
            if f.kind != overlay::FeatureKind::Warning {
                continue;
            }
            let Some(a) = &f.alert else { continue };
            // Mark every warning seen so it can't re-banner later, but only alert on genuinely new
            // ones after the first (seeding) pass. Keyed by VTEC event, not message id — an office
            // re-issues a continuation of the same warning every few minutes with a fresh id, and
            // deduping on that is why the same tornado warning announced itself over and over.
            if self.known_warning_ids.insert(a.dedupe_key()) && self.warnings_seeded {
                let esc = wxdata::alerts::escalation(a);
                let urgent = esc >= 2;
                // The polygon's middle, computed once: the rule pass below uses it as the
                // warning's stand-in detection, and the spoken line uses it to say which way the
                // warning lies from a watched place.
                let centroid: Option<[f64; 2]> = f.rings.first().filter(|r| !r.is_empty()).map(|ring| {
                    let n = ring.len() as f64;
                    let (x, y) = ring
                        .iter()
                        .fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
                    [x / n, y / n]
                });
                // Severity floor: below the tier the user set, the warning still banners and
                // still joins the alert list — it just doesn't push, speak or make noise.
                let notify_ok = esc >= self.settings.alert_min_escalation;
                // A watched location always alerts + pushes: inside the polygon, or within that
                // marker's radius of it. Home first, then the closest — a warning that clips two
                // saved places should name the one you sleep in.
                // Owned, not borrowed: the rule pass below needs `&mut self` while this is
                // still in hand.
                let hit: Option<(String, f64, Option<f32>)> = self
                    .settings
                    .markers
                    .iter()
                    .filter_map(|m| {
                        let km = f.distance_km(m.lon, m.lat);
                        (km <= m.alert_radius_mi * crate::geo::KM_PER_MILE).then(|| {
                            // Distance to the nearest edge, bearing to the middle: the edge is
                            // what "how far" means for a polygon, and the middle is what "which
                            // way" means. Mixing them is fine at the scale of one sentence.
                            let bearing = centroid
                                .map(|c| crate::geo::great_circle([m.lon, m.lat], c).1 as f32);
                            (m.name.clone(), m.home, km, bearing)
                        })
                    })
                    .min_by(|(_, ha, ka, _), (_, hb, kb, _)| {
                        hb.cmp(ha)
                            .then(ka.partial_cmp(kb).unwrap_or(std::cmp::Ordering::Equal))
                    })
                    .map(|(name, _, km, bearing)| (name, km, bearing));
                // A drawn watch zone the warning polygon touches. Independent of the markers: a
                // zone is an area you care about, not a point with a radius around it.
                let zone: Option<String> = self
                    .settings
                    .alert_polygons
                    .iter()
                    .find(|z| {
                        f.rings
                            .first()
                            .is_some_and(|outer| wxdata::overlay::rings_intersect(outer, &z.ring))
                    })
                    .map(|z| z.name.clone());
                if let (Some(z), true) = (zone.as_deref(), notify_ok) {
                    self.notify_alert(
                        &format!("⚠ {} — {z}", a.event),
                        if a.headline.is_empty() {
                            &a.area
                        } else {
                            &a.headline
                        },
                        urgent,
                    );
                }
                // User rules that watch warnings. The id-dedupe above is their cooldown: a
                // warning is announced once, however long it stands.
                for rule in self.settings.alert_rules.clone() {
                    if !rule.enabled || !crate::rules::warning_matches(&rule, &a.event) {
                        continue;
                    }
                    // Anywhere: the warning polygon is somewhere on this radar. A place: the
                    // polygon has to reach it, using the same distance the built-in alert uses.
                    let reaches = match &rule.place {
                        crate::settings::RulePlace::Anywhere => true,
                        crate::settings::RulePlace::Marker { id } => self
                            .settings
                            .markers
                            .iter()
                            .find(|m| &m.id == id)
                            .is_some_and(|m| {
                                f.distance_km(m.lon, m.lat)
                                    <= m.alert_radius_mi * crate::geo::KM_PER_MILE
                            }),
                        crate::settings::RulePlace::Zone { name } => self
                            .settings
                            .alert_polygons
                            .iter()
                            .find(|z| &z.name == name)
                            .is_some_and(|z| {
                                f.rings.first().is_some_and(|outer| {
                                    wxdata::overlay::rings_intersect(outer, &z.ring)
                                })
                            }),
                    };
                    if reaches {
                        // The warning's own centroid stands in for a detection, so a warning rule
                        // can carry extra conditions like every other rule ("a tornado warning,
                        // and also rotation within 20 km").
                        let hit = centroid
                            .map(|c| crate::rules::Detection::at(c[0], c[1]))
                            .unwrap_or(crate::rules::Detection::at(0.0, 0.0));
                        if crate::rules::compound_ok(&rule, &hit, &self.recent_for_rules()) {
                            self.fire_rule_named(&rule, &hit, Some(a.event.clone()));
                        }
                    }
                }
                let zone_name = zone;
                // `area` is banner text and `relation` is speech: "5 mi from Home" reads as
                // "five em eye", and "covers Home" reads as a verb the sentence already had.
                let (label, area, relation) = match hit {
                    Some((name, km, bearing)) => {
                        // Watched location covered → push to the phone (opt-in ntfy topic).
                        if notify_ok {
                            self.notify_alert(
                                &format!("⚠ {} — {}", a.event, name),
                                if a.headline.is_empty() {
                                    &a.area
                                } else {
                                    &a.headline
                                },
                                urgent,
                            );
                        }
                        let where_ = if km <= 0.05 {
                            format!("covers {name}")
                        } else {
                            format!("{} from {name}", crate::geo::fmt_distance(km, metric, 0))
                        };
                        (
                            format!("⚠ {}", a.event),
                            where_,
                            wxdata::spoken::relation(&name, km, bearing, metric),
                        )
                    }
                    // A zone hit banners on its own terms, wherever the radar happens to be
                    // pointed — that is the whole point of drawing one.
                    None if zone_name.is_some() => {
                        let zone = zone_name.expect("checked Some");
                        (
                            format!("⚠ {}", a.event),
                            format!("touches {zone}"),
                            format!("touching {zone}"),
                        )
                    }
                    None => {
                        // No watched location: banner only if it's near the selected radar.
                        if site_box.is_none_or(|bx| !feature_in_box(f, bx)) {
                            continue;
                        }
                        // Nowhere of the user's own to relate it to; the counties carry it.
                        (a.event.clone(), a.area.clone(), String::new())
                    }
                };
                if notify_ok {
                    max_esc = max_esc.max(esc);
                }
                // Queued, not spoken: the tone leads, and the whole pass is announced together
                // below so two warnings in one fetch cannot talk over each other. Chasing is an
                // eyes-on-the-road activity, and a warning you have to read is one you read late.
                if self.settings.speak_warnings && notify_ok {
                    let until = a
                        .expires
                        .map(|t| {
                            crate::timefmt::fmt_clock(
                                t,
                                self.settings.tz_for(self.views[self.active].site.as_deref()),
                                false,
                            )
                        })
                        .unwrap_or_default();
                    // Hazard, then where it sits against a place you know, then the counties, the
                    // towns in its path and what to do — see `wxdata::spoken`.
                    to_speak.push(wxdata::spoken::warning_script(a, &relation, &until));
                }
                if notify_ok && self.settings.ntfy_snapshot {
                    // Newest wins: one picture per pass, of whatever last warned.
                    self.snapshot_push = Some(format!("{label} — {area}"));
                }
                self.banner(label, area);
                alerted |= notify_ok;
            }
        }
        self.warnings_seeded = true;
        if alerted {
            print!("\x07"); // free terminal bell alongside the chime
            use std::io::Write;
            let _ = std::io::stdout().flush();
            // Escalated (Tornado Emergency / PDS / destructive) warnings use the emergency sound
            // and go past quiet hours — which is now true of the words as well as the tone. The
            // voice used to ignore quiet hours entirely, so a 3 a.m. warning too minor to chime
            // for still read itself out in the dark.
            let urgent = max_esc >= 2;
            if !self.settings.mute_alerts && (urgent || !self.in_quiet_hours()) {
                let tone = self.settings.alert_sound.then(|| {
                    (
                        if urgent {
                            self.settings.emergency_sound.clone()
                        } else {
                            self.settings.warn_sound.clone()
                        },
                        self.settings.alert_volume,
                    )
                });
                if tone.is_some() {
                    crate::platform::haptic(crate::platform::Haptic::Alert);
                }
                // The voice tracks the same slider the tones do; Piper's output has no level of
                // its own, so without this the words arrived louder than the tone.
                crate::speech::set_volume(self.settings.alert_volume);
                // One announcement for the whole pass: tone, then every new warning in turn.
                crate::speech::announce(tone, to_speak);
            }
        }
    }

    /// Run the user's scan rules over one volume's detections.
    ///
    /// Once per volume, not per frame: the same reason `check_rain_arrival` keys on the volume.
    /// Called with every scan detector's hits already computed — including detectors whose layer
    /// is off, which is what makes a rule independent of what happens to be drawn.
    fn evaluate_scan_rules(
        &mut self,
        idx: usize,
        tds: &[wxdata::tds::TdsHit],
        tbss: &[wxdata::dualpol::TbssHit],
        zdr: &[wxdata::dualpol::ZdrColumnHit],
        couplets: &[wxdata::rotation::CoupletHit],
    ) {
        use crate::rules::Detection;
        use crate::settings::RuleTrigger as T;
        if self.settings.alert_rules.iter().all(|r| !r.enabled) {
            return;
        }
        let key = self.volume_key(idx);
        if self.rules_key.as_ref() == Some(&key) {
            return;
        }
        self.rules_key = Some(key);
        // Strengths in the units the rule is written in: knots for rotation, and nothing for the
        // signatures that are their own answer.
        let hits = |t: &T| -> Vec<Detection> {
            match t {
                T::Tds => tds.iter().map(|h| Detection::at(h.lon, h.lat)).collect(),
                T::Tbss => tbss.iter().map(|h| Detection::at(h.lon, h.lat)).collect(),
                T::ZdrColumn => zdr.iter().map(|h| Detection::at(h.lon, h.lat)).collect(),
                T::Rotation => couplets
                    .iter()
                    .map(|h| Detection::with_strength(h.lon, h.lat, h.vrot_ms as f64 * 1.943_844))
                    .collect(),
                _ => Vec::new(),
            }
        };
        // Every scan detector's hits are remembered, not only the armed ones: a compound rule
        // asks about a trigger no rule is armed on all the time ("rotation and also a TDS").
        for t in [T::Tds, T::Tbss, T::ZdrColumn, T::Rotation] {
            let h = hits(&t);
            self.note_hits(&t, &h);
        }
        let recent = self.recent_for_rules();
        for rule in self.settings.alert_rules.clone() {
            if !rule.enabled || !rule.trigger.is_scan() {
                continue;
            }
            // The closest qualifying detection is the one worth naming.
            let hit = hits(&rule.trigger)
                .into_iter()
                .filter(|h| crate::rules::matches(&rule, h, &self.settings))
                .min_by(|a, b| {
                    let d = |h: &Detection| self.distance_from_radar_km(idx, h.lon, h.lat);
                    d(a).total_cmp(&d(b))
                });
            let Some(hit) = hit else { continue };
            if !crate::rules::compound_ok(&rule, &hit, &recent) {
                continue;
            }
            self.fire_rule(&rule, &hit);
        }
    }

    /// Run the rules on `trigger` over a freshly built lightning grid (density, or its rate of
    /// rise).
    ///
    /// The grid's own cell is the detection: a cell whose flash count clears the rule's threshold
    /// is somewhere worth knowing about. Cooldowns are per rule and place, as everywhere else.
    fn evaluate_grid_rules(
        &mut self,
        trigger: crate::settings::RuleTrigger,
        field: &wxdata::mrms::MrmsField,
    ) {
        use crate::rules::Detection;
        let rules: Vec<crate::settings::AlertRule> = self
            .settings
            .alert_rules
            .iter()
            .filter(|r| r.enabled && r.trigger == trigger)
            .cloned()
            .collect();
        if rules.is_empty() || field.nx == 0 || field.ny == 0 {
            return;
        }
        let (dx, dy) = (
            (field.lon_east - field.lon_west) / field.nx as f64,
            (field.lat_north - field.lat_south) / field.ny as f64,
        );
        for rule in rules {
            // The busiest qualifying cell — a rule about lightning wants the worst of it.
            let best = field
                .values
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_finite() && **v > 0.0)
                .map(|(i, v)| {
                    let (x, y) = (i % field.nx, i / field.nx);
                    Detection::with_strength(
                        field.lon_west + (x as f64 + 0.5) * dx,
                        field.lat_north - (y as f64 + 0.5) * dy,
                        *v as f64,
                    )
                })
                .filter(|h| crate::rules::matches(&rule, h, &self.settings))
                .max_by(|a, b| {
                    a.strength
                        .unwrap_or(0.0)
                        .total_cmp(&b.strength.unwrap_or(0.0))
                });
            if let Some(hit) = best {
                self.note_hits(&trigger, &[hit]);
                if crate::rules::compound_ok(&rule, &hit, &self.recent_for_rules()) {
                    self.fire_rule(&rule, &hit);
                }
            }
        }
    }

    /// Run the ProbSevere rules over the freshly fetched storm probabilities.
    fn evaluate_probsevere_rules(&mut self, feats: &[GeoFeature]) {
        use crate::rules::Detection;
        let rules: Vec<crate::settings::AlertRule> = self
            .settings
            .alert_rules
            .iter()
            .filter(|r| r.enabled && r.trigger == crate::settings::RuleTrigger::ProbSevere)
            .cloned()
            .collect();
        if rules.is_empty() {
            return;
        }
        // One detection per storm: its centroid, carrying the Severe percentage.
        let storms: Vec<Detection> = feats
            .iter()
            .filter_map(|f| {
                let pct = crate::rules::probsevere_percent(&f.detail)?;
                let ring = f.rings.first()?;
                let n = ring.len().max(1) as f64;
                let (lon, lat) = ring
                    .iter()
                    .fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
                Some(Detection::with_strength(lon / n, lat / n, pct))
            })
            .collect();
        for rule in rules {
            let worst = storms
                .iter()
                .filter(|h| crate::rules::matches(&rule, h, &self.settings))
                .max_by(|a, b| {
                    a.strength
                        .unwrap_or(0.0)
                        .total_cmp(&b.strength.unwrap_or(0.0))
                })
                .copied();
            if let Some(hit) = worst {
                self.note_hits(&crate::settings::RuleTrigger::ProbSevere, &[hit]);
                if crate::rules::compound_ok(&rule, &hit, &self.recent_for_rules()) {
                    self.fire_rule(&rule, &hit);
                }
            }
        }
    }

    /// Kilometres from the pane's radar to a point — how "closest detection" is judged.
    fn distance_from_radar_km(&self, idx: usize, lon: f64, lat: f64) -> f64 {
        let Some(site) = self.views[idx]
            .site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
        else {
            return 0.0;
        };
        crate::geo::great_circle([site.longitude as f64, site.latitude as f64], [lon, lat]).0
    }

    /// Kick off a backtest of one rule against an archive day, on the shared runtime.
    ///
    /// The site is whichever radar the active pane is on: a rule about a place is only replayable
    /// against a radar that can see it, and the pane the user is looking at is the best guess
    /// anyone can make without asking.
    fn start_backtest(&mut self, rule_idx: usize, day: chrono::NaiveDate) {
        let Some(rule) = self.settings.alert_rules.get(rule_idx).cloned() else {
            return;
        };
        let Some(site) = self.views[self.active].site.clone() else {
            return;
        };
        let shared: crate::backtest::Shared = Default::default();
        self.rules_window.backtest = Some(shared.clone());
        let settings = self.settings.clone();
        self.spawner
            .spawn(crate::backtest::run(site, day, rule, settings, shared));
    }

    /// Remember detections so a compound rule can ask about them next pass, and forget anything
    /// older than the compound window.
    fn note_hits(
        &mut self,
        trigger: &crate::settings::RuleTrigger,
        hits: &[crate::rules::Detection],
    ) {
        let window = std::time::Duration::from_secs_f64(crate::rules::COMPOUND_WINDOW_MIN * 60.0);
        self.recent_hits.retain(|(_, _, t)| t.elapsed() < window);
        for h in hits {
            self.recent_hits.push((trigger.clone(), *h, Instant::now()));
        }
    }

    /// The recent detections in the shape `rules::compound_ok` wants.
    fn recent_for_rules(&self) -> Vec<crate::rules::RecentHit> {
        self.recent_hits
            .iter()
            .map(|(trigger, hit, t)| crate::rules::RecentHit {
                trigger: trigger.clone(),
                hit: *hit,
                age_min: t.elapsed().as_secs_f64() / 60.0,
            })
            .collect()
    }

    /// Deliver a rule's alert, unless its cooldown for this place is still running.
    ///
    /// Keyed by rule *and* place, so a rule watching two zones can speak about each of them.
    fn fire_rule(&mut self, rule: &crate::settings::AlertRule, hit: &crate::rules::Detection) {
        self.fire_rule_named(rule, hit, None);
    }

    /// [`Self::fire_rule`], with the body spelled out. Warning rules name the event they matched,
    /// which the trigger label alone does not carry.
    fn fire_rule_named(
        &mut self,
        rule: &crate::settings::AlertRule,
        hit: &crate::rules::Detection,
        detail: Option<String>,
    ) {
        let place = crate::rules::place_label(&rule.place, &self.settings);
        let key = format!("{}:{place}", rule.id);
        let cooldown = std::time::Duration::from_secs(u64::from(rule.cooldown_min) * 60);
        if self
            .rules_fired
            .get(&key)
            .is_some_and(|t| t.elapsed() < cooldown)
        {
            return;
        }
        self.rules_fired.insert(key, Instant::now());
        let title = format!("\u{25c9} {}", rule.title());
        let body = match (detail, hit.strength) {
            (Some(d), _) => format!("{d} at {place}"),
            (None, Some(v)) => format!("{} \u{2014} {v:.0} at {place}", rule.trigger.label()),
            (None, None) => format!("{} at {place}", rule.trigger.label()),
        };
        if rule.snapshot {
            // Same one-picture-per-pass rule the warning snapshot follows: newest wins.
            self.snapshot_push = Some(format!("{title} — {place}"));
        }
        self.notify_alert(&title, &body, rule.urgent);
        if let Some(sound) = rule.sound.clone() {
            if self.settings.alert_sound && !self.settings.mute_alerts {
                self.play_alert(&sound);
            }
        }
        self.banner(title, body);
    }

    /// Chime + push when cloud-to-ground lightning density exceeds a small threshold within ~15 km
    /// of any saved location. Debounced per location (re-alerts only after ≥10 min of quiet) so a
    /// persistent storm doesn't spam. No-op unless the opt-in alarm is enabled and locations exist.
    fn check_lightning_proximity(&mut self, field: &wxdata::mrms::MrmsField) {
        if !self.settings.lightning_alarm || self.settings.markers.is_empty() {
            return;
        }
        const RADIUS_KM: f64 = 15.0;
        const DENSITY_MIN: f32 = 0.05; // strikes/km²/min — any recent CG activity nearby
        const COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);
        let mut fired = false;
        // Collected first: the alert calls below need `&mut self`.
        let near: Vec<(String, String)> = self
            .watched_points()
            .into_iter()
            .filter(|p| field.max_within_km(p.lon, p.lat, RADIUS_KM) >= DENSITY_MIN)
            .map(|p| (p.id, p.name))
            .collect();
        for (id, name) in near {
            let recent = self
                .lightning_alerted
                .get(&id)
                .is_some_and(|t| t.elapsed() < COOLDOWN);
            if recent {
                continue;
            }
            self.lightning_alerted.insert(id, Instant::now());
            self.notify_alert(
                &format!("⚡ Lightning near {name}"),
                &format!("Cloud-to-ground strikes within {RADIUS_KM:.0} km of {name}"),
                false,
            );
            self.banner(
                format!("⚡ Lightning near {name}"),
                format!("within {RADIUS_KM:.0} km"),
            );
            fired = true;
        }
        if fired && self.settings.alert_sound {
            self.play_alert(&self.settings.lightning_sound.clone());
        }
    }

    /// Per-minute rain over `at` for the next hour, advected off the current volume. Live only —
    /// an advection off an archived scan describes a time that already happened. Cached per
    /// (volume, point, storm motion); recomputing the 61-point walk every frame is wasted work.
    fn minute_profile(&mut self, at: (f64, f64)) -> Option<&[Option<f32>]> {
        let idx = self.active;
        if !self.views[idx].timeline.following {
            self.minute_key = None;
            self.minute_profile = None;
            return None;
        }
        let (dir, kt) = self.scit_mean_motion()?;
        let vol = self.views[idx]
            .timeline
            .current()
            .map(|id| id.name().to_string())
            .unwrap_or_default();
        let key = format!("{vol}|{:.4},{:.4}|{dir:.0},{kt:.0}", at.0, at.1);
        if self.minute_key.as_deref() != Some(key.as_str()) {
            self.minute_key = Some(key);
            self.minute_profile = self.compute_minute_profile(at, dir, kt);
        }
        self.minute_profile.as_deref()
    }

    fn compute_minute_profile(
        &mut self,
        at: (f64, f64),
        dir: f32,
        kt: f32,
    ) -> Option<Vec<Option<f32>>> {
        let idx = self.active;
        let tilt = self.views[idx].tilt;
        let sweep = self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Reflectivity, tilt, false).ok())
            .cloned()?;
        let sample = refl_sampler(&sweep);
        crate::rain_arrival::upstream_profile(sample, [at.0, at.1], dir as f64, kt as f64, 60)
    }

    /// Check whether echo is heading for any watched point (saved markers + your chase position)
    /// and alert once per approach. Live data only — an ETA off an archived scan is meaningless.
    fn check_rain_arrival(&mut self) {
        use crate::rain_arrival::{upstream_eta, Verdict};
        if !self.settings.rain_alerts {
            self.rain_eta.clear();
            return;
        }
        let Some((dir, kt)) = self.scit_mean_motion() else {
            return;
        };
        let idx = self.active;
        if !self.views[idx].timeline.following {
            return;
        }
        // Once per volume, like compute_tds/compute_couplets: called per frame, the detector's
        // "2 consecutive scans" persistence collapses to ~33 ms and its 30-minute cooldown gets
        // re-armed by any momentary ETA gap, which is what stacks duplicate banners.
        let key = self.volume_key(idx);
        if self.rain_key.as_ref() == Some(&key) {
            return;
        }
        // Watched points: every saved marker, plus where you are if chase mode knows. Tracked by
        // id — the detector's per-place persistence must not follow a rename or a reused name.
        let mut points: Vec<(String, String, [f64; 2])> = self
            .settings
            .markers
            .iter()
            .map(|m| (m.id.clone(), m.name.clone(), [m.lon, m.lat]))
            .collect();
        if let Some((lon, lat)) = self.chase_pos {
            points.push((
                GPS_POINT_ID.to_string(),
                "your location".to_string(),
                [lon, lat],
            ));
        }
        if points.is_empty() {
            return;
        }
        let ids: Vec<String> = points.iter().map(|(id, ..)| id.clone()).collect();
        self.rain_detector.retain(&ids);

        let tilt = self.views[idx].tilt;
        let Some(sweep) = self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Reflectivity, tilt, false).ok())
            .cloned()
        else {
            return;
        };
        // Only now — a failed decode should retry next frame, not skip the volume.
        self.rain_key = Some(key);
        let sample = refl_sampler(&sweep);

        let mut fired = false;
        self.rain_eta.clear();
        for (id, name, at) in &points {
            let eta = upstream_eta(
                &sample,
                *at,
                dir as f64,
                kt as f64,
                crate::rain_arrival::MAX_MIN,
            );
            if let Some(min) = eta {
                self.rain_eta.push((name.clone(), min));
            }
            if let Verdict::Fire(min) = self.rain_detector.update(id, eta) {
                self.notify_alert(
                    &format!("\u{1f327} Rain reaching {name}"),
                    &format!("About {min:.0} minutes out"),
                    false,
                );
                self.banner(
                    format!("\u{1f327} Rain reaching {name}"),
                    format!("~{min:.0} min"),
                );
                fired = true;
            }
        }
        if fired && self.settings.alert_sound {
            self.play_alert(&self.settings.rain_sound.clone());
        }
    }

    /// Drive the HRRR "future radar" layer from the active pane's timeline: scrubbing into the
    /// forecast tail enables HRRR at that forecast hour (and suppresses the observed radar for the
    /// scrubbed pane, done at draw time); scrubbing back to observed frames turns it off again.
    fn sync_forecast_scrub(&mut self) {
        use crate::render::FieldLayer as FL;
        match self.views[self.active].timeline.forecast_hour() {
            Some(h) => {
                self.hrrr_fcst_hour = h;
                self.views[self.active].fields_on.insert(FL::Hrrr);
                self.hrrr_by_timeline = true;
            }
            None => {
                if self.hrrr_by_timeline {
                    self.views[self.active].fields_on.remove(&FL::Hrrr);
                    self.hrrr_by_timeline = false;
                }
            }
        }
    }

    /// Build the 3D reflectivity volume from the active pane and open the raymarch window.
    fn build_volume3d(&mut self) {
        if !self.volume3d_supported {
            self.toast(
                ToastKind::Error,
                "3D needs WebGPU — this browser fell back to WebGL, which has no 3D textures",
            );
            return;
        }
        self.show_3d = true;
        let Some(vol) = self.views[self.active].volume.as_mut() else {
            return;
        };
        // Rebuild once per volume, not once per open: resampling 192x192x48 is a second of CPU.
        let key = (vol.name.clone(), VOL3D_N);
        if self.vol3d_key.as_ref() == Some(&key) || self.vol3d_rx.is_some() {
            return;
        }
        let sweeps = vol.reflectivity_tilts();
        if sweeps.is_empty() {
            return;
        }
        self.vol3d_key = Some(key);
        let table = crate::colormap::effective_table(
            &self.palettes,
            Moment::Reflectivity,
            self.settings.theme,
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.vol3d_rx = Some(rx);
        self.spawner.spawn(async move {
            // Off the UI thread: this is pure CPU on tens of MB and would drop a second of frames.
            let built = wxdata::task::blocking(move || {
                let v3 = wxdata::volume3d::build(&sweeps, VOL3D_N, VOL3D_NZ, 150.0, 18.0)?;
                let lut =
                    crate::colormap::bake_lut(&table, (v3.value_min, v3.value_max), None).to_vec();
                Some((
                    crate::render3d::Volume3dUpload {
                        data: v3.data,
                        n: v3.n as u32,
                        nz: v3.nz as u32,
                        lut,
                    },
                    (v3.value_min, v3.value_max),
                ))
            })
            .await
            .ok()
            .flatten();
            if let Some(b) = built {
                let _ = tx.send(b);
            }
        });
    }

    /// Take a finished 3D volume, if the worker has one.
    fn drain_volume3d(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.vol3d_rx else { return };
        match rx.try_recv() {
            Ok((upload, range)) => {
                self.vol3d_pending = Some(upload);
                self.vol3d_range = range;
                self.vol3d_rx = None;
                ctx.request_repaint();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            // The worker gave up (no sweeps survived the resample); allow another attempt.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.vol3d_rx = None;
                self.vol3d_key = None;
            }
        }
    }

    /// Re-slice the active pane's cached volume into a CAPPI at `cappi_alt_km` when the key
    /// (volume name + altitude) changed, and refresh the window texture (feature AA).
    fn update_cappi(&mut self, ctx: &egui::Context) {
        const HALF_KM: f32 = 150.0;
        const N: usize = 256;
        let Some(name) = self.views[self.active]
            .volume
            .as_ref()
            .map(|v| v.name.clone())
        else {
            self.cappi_tex = None;
            self.cappi_key = None;
            return;
        };
        let key = (name, self.cappi_alt_km.to_bits());
        if self.cappi_key.as_ref() == Some(&key) {
            return;
        }
        let Some(vol) = self.views[self.active].volume.as_mut() else {
            return;
        };
        let sweeps = vol.reflectivity_tilts();
        if sweeps.is_empty() {
            return;
        }
        let Some(c) = wxdata::volume3d::cappi(&sweeps, self.cappi_alt_km, N, HALF_KM) else {
            return;
        };
        let hc_table = crate::colormap::effective_table(
            &self.palettes,
            Moment::Reflectivity,
            self.settings.theme,
        );
        let img = ui::cappi_window::to_image(&c, &hc_table);
        self.cappi_tex = Some(ctx.load_texture("cappi", img, egui::TextureOptions::NEAREST));
        self.cappi_key = Some(key);
    }

    /// Reconstruct a vertical reflectivity cross-section along the two clicked endpoints from
    /// pane `idx`'s volume, upload it as a texture, and open the cross-section window.
    /// If the click landed on a radar-site ring (and not on a storm report/cell, which take
    /// precedence), switch pane `idx` to that site and return true. `sync_pane` reacts to the
    /// changed site — no extra plumbing here.
    fn try_pick_site(
        &mut self,
        idx: usize,
        pos: egui::Pos2,
        cam: crate::render::mercator::Camera,
        prect: egui::Rect,
        vp: (f32, f32),
    ) -> bool {
        let to_screen_hit = |lon: f64, lat: f64| {
            let w = crate::render::mercator::lonlat_to_world(lon, lat);
            let (sx, sy) = cam.world_to_screen(w, vp);
            let (dx, dy) = (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
            dx * dx + dy * dy
        };
        // Storm features win: bail if a report or cell dot sits under the cursor.
        let near_storm = (self.show_storm_reports
            && self
                .active_storm_reports()
                .iter()
                .any(|r| to_screen_hit(r.lon, r.lat) <= tap_r2(12.0)))
            || (self.cells_site.as_deref() == self.views[idx].site.as_deref()
                && self
                    .active_storm_cells()
                    .iter()
                    .any(|c| to_screen_hit(c.lon, c.lat) <= tap_r2(14.0)));
        if near_storm {
            return false;
        }
        let hit = wxdata::sites::all()
            .filter(|s| to_screen_hit(s.longitude as f64, s.latitude as f64) <= tap_r2(12.0))
            .min_by(|a, b| {
                to_screen_hit(a.longitude as f64, a.latitude as f64)
                    .total_cmp(&to_screen_hit(b.longitude as f64, b.latitude as f64))
            });
        match hit {
            Some(s) if self.views[idx].site.as_deref() != Some(s.id) => {
                self.views[idx].site = Some(s.id.to_string());
                self.cell_popup = None;
                self.warning_popup = None;
                self.detail = None;
                true
            }
            _ => false,
        }
    }

    /// Load any marker icon files not yet in the texture cache (negative-cached on failure).
    fn load_marker_icons(&mut self, ctx: &egui::Context) {
        let Some(dir) = crate::settings::Settings::marker_icons_dir() else {
            return;
        };
        for m in &self.settings.markers {
            let Some(name) = &m.icon else { continue };
            if self.marker_icon_tex.contains_key(name) {
                continue;
            }
            let tex = std::fs::read(dir.join(name))
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|img| {
                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    ctx.load_texture(format!("marker-{name}"), ci, egui::TextureOptions::LINEAR)
                });
            if tex.is_none() {
                log::warn!("marker icon load failed: {name}");
            }
            self.marker_icon_tex.insert(name.clone(), tex);
        }
    }

    fn build_xsection(&mut self, idx: usize, ctx: &egui::Context) {
        let (a, b) = (self.xsection_pts[0], self.xsection_pts[1]);
        let Some(vol) = self.views[idx].volume.as_mut() else {
            return;
        };
        let moment = self.xsection_moment;
        let sweeps = vol.moment_tilts(moment); // owned → the &mut vol borrow ends here
        if sweeps.is_empty() {
            return;
        }
        let Some(xs) = wxdata::xsection::build(&sweeps, (a[0], a[1]), (b[0], b[1]), 300, 120, 18.0)
        else {
            return;
        };
        let hc_table =
            crate::colormap::effective_table(&self.palettes, moment, self.settings.theme);
        let img = ui::xsection_window::to_image(&xs, &hc_table);
        self.xsection_tex = Some(ctx.load_texture("xsection", img, egui::TextureOptions::LINEAR));
        self.xsection = Some(xs);
    }

    /// Detail card for a tapped Spotter Network dot. Their report text sometimes carries a
    /// stream link; when it does, the card offers it as a link (which is also the Watch path).
    fn open_spotter(&mut self, sp: &wxdata::spotters::Spotter) {
        let body = format!(
            "{}\n{}",
            crate::timefmt::fmt_date_clock(sp.time, self.active_tz()),
            sp.status
        );
        let link = first_url(&sp.status).map(|u| ("▶ Watch stream".to_string(), u));
        self.cell_popup = None;
        self.detail = Some(Detail {
            title: sp.name.clone(),
            body,
            color: [0, 200, 80, 255],
            image: None,
            link,
        });
    }

    /// Open a stream URL: direct HLS/MJPEG plays in the in-app player, everything else (YouTube,
    /// Twitch, a station's watch page) goes to the system browser.
    // ponytail: no yt-dlp sidecar — the browser already plays those better than we would.
    fn watch_stream(&mut self, title: String, url: String) {
        if url.is_empty() {
            return;
        }
        if ui::video_window::playable_in_app(&url) {
            self.video_player = Some(ui::video_window::VideoPlayer::start(
                title,
                url,
                &self.spawner,
            ));
        } else if let Err(e) = crate::platform::open_url(&url) {
            log::warn!("open stream URL failed: {e}");
        }
    }

    /// Deliver an alert to every configured channel: ntfy.sh push plus Discord / Slack / Matrix
    /// webhooks. Each is a no-op when its settings field is blank.
    /// Best-effort on the shared tokio runtime; failures are logged, never fatal.
    fn notify_alert(&self, title: &str, body: &str, urgent: bool) {
        // Quiet hours hold everything back except the escalated tier, which is the one worth
        // waking up for. Banners and the alert list are untouched — this gates what leaves the
        // machine and what makes noise, not what the app knows.
        if !urgent && self.in_quiet_hours() {
            log::debug!("quiet hours: holding push {title:?}");
            if let Ok(mut q) = self.quiet_queue.lock() {
                if q.len() < QUIET_QUEUE_MAX {
                    q.push((title.to_string(), body.to_string()));
                }
            }
            return;
        }
        let http = self.http.clone();
        let (mut title, mut body) = (title.to_string(), body.to_string());

        // Outbreak mode: past the threshold, one rolling summary goes out instead of one push per
        // warning. Escalated alerts are exempt — those are the ones worth a buzz each.
        // ponytail: the summary is a fresh notification each refresh, not an in-place replace;
        // desktop replace-by-tag and the Android fixed notification id can land with the rest of
        // the Android delivery stack.
        if !urgent && self.settings.alert_rollup_threshold > 0 {
            let window =
                std::time::Duration::from_secs(self.settings.alert_rollup_window_min.max(1) * 60);
            let decision = self.rollup.lock().map(|mut r| {
                r.offer(
                    Instant::now(),
                    &title,
                    self.settings.alert_rollup_threshold,
                    window,
                )
            });
            match decision {
                Ok(crate::alert_rollup::Decision::Hold) => return,
                Ok(crate::alert_rollup::Decision::Rollup(text)) => {
                    title = "Multiple alerts".to_string();
                    body = text;
                }
                _ => {}
            }
        }
        let (title, body) = (title, body);

        if self.settings.desktop_notify {
            crate::notify::desktop(&title, &body);
        }

        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        crate::mqtt::publish_alert(&self.settings, &title, &body, urgent);

        let topic = self.settings.ntfy_topic.trim().to_string();
        if !topic.is_empty() {
            let (http, title, body) = (http.clone(), title.clone(), body.clone());
            let priority = if urgent { "urgent" } else { "high" };
            self.spawner.spawn(async move {
                crate::notify::send_retrying("ntfy push", || {
                    http.post(format!("https://ntfy.sh/{topic}"))
                        .header("Title", title.clone())
                        .header("Priority", priority)
                        .header("Tags", "warning,cloud_with_lightning")
                        .body(body.clone())
                })
                .await;
            });
        }

        // Chat webhooks: same shape (POST JSON), so one closure covers Discord and Slack.
        let mut posts: Vec<(&'static str, String, String, Option<String>)> = Vec::new();
        let discord = self.settings.discord_webhook.trim();
        if !discord.is_empty() {
            posts.push((
                "discord",
                discord.to_string(),
                crate::notify::discord_body(&title, &body),
                None,
            ));
        }
        let slack = self.settings.slack_webhook.trim();
        if !slack.is_empty() {
            posts.push((
                "slack",
                slack.to_string(),
                crate::notify::slack_body(&title, &body),
                None,
            ));
        }
        for (what, url, payload, _) in posts {
            let http = http.clone();
            self.spawner.spawn(async move {
                crate::notify::send_retrying(&format!("{what} webhook"), || {
                    http.post(url.clone())
                        .header("Content-Type", "application/json")
                        .body(payload.clone())
                })
                .await;
            });
        }

        // Matrix wants an authenticated PUT with a transaction id.
        let (hs, room, token) = (
            self.settings.matrix_homeserver.trim().to_string(),
            self.settings.matrix_room.trim().to_string(),
            self.settings.matrix_token.trim().to_string(),
        );
        if !hs.is_empty() && !room.is_empty() && !token.is_empty() {
            let txn = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let url = crate::notify::matrix_url(&hs, &room, txn);
            let payload = crate::notify::matrix_body(&title, &body);
            self.spawner.spawn(async move {
                crate::notify::send_retrying("matrix webhook", || {
                    http.put(url.clone())
                        .bearer_auth(token.clone())
                        .header("Content-Type", "application/json")
                        .body(payload.clone())
                })
                .await;
            });
        }
    }

    /// Fetch the Piper voice model into the data directory.
    ///
    /// The model and the `.onnx.json` beside it are both required — Piper reads its sample rate
    /// and phoneme table from the JSON — so a download that gets one and not the other is a
    /// failure, not a half-success. Written to a `.part` file and renamed, so an interrupted
    /// download cannot leave a truncated model that Piper would then crash on.
    ///
    // ponytail: no pinned hash. The download is https from Piper's own voice repository and both
    // files are validated (the JSON parses, the model is the size of a model); pinning a digest
    // means pinning a version, and I could not verify one offline to pin.
    #[cfg(not(target_arch = "wasm32"))]
    fn download_voice(&self, id: String) {
        let Some(path) = crate::speech::voice_path(&id) else {
            crate::speech::set_voice_status(false, "no data directory to download into");
            return;
        };
        let Some(url) = crate::speech::voice_url(&id) else {
            crate::speech::set_voice_status(false, "that is not a Piper voice id");
            return;
        };
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let cfg_url = format!("{url}.json");
            let cfg_path = path.with_extension("onnx.json");
            if let Some(dir) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    crate::speech::set_voice_status(
                        false,
                        format!("could not create {dir:?}: {e}"),
                    );
                    return;
                }
            }
            crate::speech::set_voice_status(true, "downloading voice (~60 MB)…");
            let get = |url: String| {
                let http = http.clone();
                async move {
                    http.get(url)
                        .send()
                        .await?
                        .error_for_status()?
                        .bytes()
                        .await
                }
            };
            let cfg = match get(cfg_url).await {
                Ok(b) => b,
                Err(e) => {
                    crate::speech::set_voice_status(false, format!("voice config failed: {e}"));
                    return;
                }
            };
            if serde_json::from_slice::<serde_json::Value>(&cfg).is_err() {
                crate::speech::set_voice_status(false, "voice config was not JSON; refusing it");
                return;
            }
            let model = match get(url).await {
                Ok(b) => b,
                Err(e) => {
                    crate::speech::set_voice_status(false, format!("voice download failed: {e}"));
                    return;
                }
            };
            // A medium-quality Piper voice is tens of megabytes; anything tiny is an error page
            // that happened to arrive with a 200.
            if model.len() < 1_000_000 {
                crate::speech::set_voice_status(false, "download was too small to be a voice");
                return;
            }
            let part = path.with_extension("part");
            let wrote = std::fs::write(&part, &model)
                .and_then(|()| std::fs::rename(&part, &path))
                .and_then(|()| std::fs::write(&cfg_path, &cfg));
            match wrote {
                Ok(()) => crate::speech::set_voice_status(false, "voice ready"),
                Err(e) => {
                    crate::speech::set_voice_status(false, format!("writing the voice failed: {e}"))
                }
            }
        });
    }

    /// Sub-hourly frame scrub bar: shown when the active basemap has a time dimension (GOES
    /// imagery, a WMS radar composite) and its frame times are loaded. Steps through the recent
    /// frames; "Latest" pins to the newest.
    fn goes_time_bar(&mut self, ctx: &egui::Context) {
        let active_is_timed = self.views[self.active].basemap.timed();
        if !active_is_timed || self.goes_times.is_empty() {
            return;
        }
        // While following, the readout is whichever frame the radar clock picked.
        let followed = self.goes_follow_radar.then(|| {
            self.views[self.active]
                .volume
                .as_ref()
                .map(|v| v.time)
                .and_then(|t| nearest_goes(&self.goes_times, t))
        });
        egui::Area::new(egui::Id::new("goes_time_bar"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -34.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let n = self.goes_times.len();
                        // Effective index (None = latest = n-1).
                        let cur = match followed.flatten() {
                            Some(t) => self
                                .goes_times
                                .iter()
                                .position(|x| *x == t)
                                .unwrap_or(n - 1),
                            None => self.goes_time_idx.unwrap_or(n - 1),
                        };
                        ui.label("🛰 GOES:");
                        if ui
                            .selectable_label(self.goes_follow_radar, "⟲ radar time")
                            .on_hover_text(
                                "Keep the satellite on the radar's clock — scrub the timeline \
                                 and the imagery follows",
                            )
                            .clicked()
                        {
                            self.goes_follow_radar = !self.goes_follow_radar;
                            self.goes_time_idx = None;
                        }
                        if ui
                            .add_enabled(
                                cur > 0,
                                egui::Button::new(egui_phosphor::regular::CARET_LEFT),
                            )
                            .clicked()
                        {
                            self.goes_follow_radar = false;
                            self.goes_time_idx = Some(cur.saturating_sub(1));
                        }
                        let label = crate::timefmt::fmt_clock(
                            self.goes_times[cur],
                            self.active_tz(),
                            false,
                        );
                        ui.monospace(label);
                        if ui
                            .add_enabled(
                                cur + 1 < n,
                                egui::Button::new(egui_phosphor::regular::CARET_RIGHT),
                            )
                            .clicked()
                        {
                            self.goes_follow_radar = false;
                            let ni = cur + 1;
                            self.goes_time_idx = if ni >= n - 1 { None } else { Some(ni) };
                        }
                        if ui
                            .add_enabled(
                                self.goes_time_idx.is_some() || self.goes_follow_radar,
                                egui::Button::new("Latest"),
                            )
                            .clicked()
                        {
                            self.goes_follow_radar = false;
                            self.goes_time_idx = None;
                        }
                    });
                });
            });
    }

    /// Chase-mode follow-me: when the tracked position changes, hand the active pane off to the
    /// nearest NEXRAD site and recenter on the position. Applied once per position change.
    fn apply_chase(&mut self) {
        // Drain any live gpsd fixes into the tracked position (newest wins).
        if let Some(rx) = &self.gps_rx {
            let mut latest = None;
            while let Ok(pos) = rx.try_recv() {
                latest = Some(pos);
            }
            if let Some((lon, lat)) = latest {
                self.chase_pos = Some((lon, lat));
                // The chase log records every fix the app receives, whether or not follow-me is
                // driving the camera: the track is what you did, not what the map did.
                if self.settings.chase_log {
                    self.chase_track.push(lon, lat, crate::share::now());
                }
            }
        }
        if !self.chase_mode {
            self.chase_applied = None;
            return;
        }
        let Some((lon, lat)) = self.chase_pos else {
            return;
        };
        if self.chase_applied == Some((lon, lat)) {
            return;
        }
        if let Some(site) = crate::geo::nearest_site_id(lon, lat) {
            let zoom = self.views[self.active].camera.zoom.max(8.0);
            self.goto_view(&site, lon, lat, zoom, None);
        }
        self.chase_applied = Some((lon, lat));
        self.warm_next_site();
    }

    /// Pull the next site's newest volume into the cache before the handoff needs it.
    ///
    /// Only ever one download ahead, and only once per site: this runs off a GPS fix, which on a
    /// moving car is every couple of seconds. Needs the chase log switched on — the prediction
    /// reads the breadcrumb track, and a single fix has no direction in it.
    fn warm_next_site(&mut self) {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(120);
        if !self.settings.chase_log {
            return;
        }
        let current = self.views[self.active].site.clone();
        let Some(site) = crate::chase::next_site(
            &self.chase_track,
            current.as_deref(),
            crate::chase::LOOKAHEAD_MIN,
        ) else {
            return;
        };
        if self
            .warmed_site
            .as_ref()
            .is_some_and(|(s, at)| *s == site && at.elapsed() < EVERY)
        {
            return;
        }
        self.warmed_site = Some((site.clone(), Instant::now()));
        log::debug!("chase: warming {site} ahead of the handoff");
        self.spawner.spawn(crate::chase::warm(site));
    }

    /// Start the Google sign-in: bind a loopback port, send the browser to Google, and wait for
    /// the redirect to come back with a code.
    fn sync_sign_in(&mut self) {
        let id = self.settings.sync_client_id.trim().to_string();
        if id.is_empty() {
            self.sync_status = "Add your OAuth client id first (see docs/sync.md)".into();
            return;
        }
        match crate::cloud::start_login(&id) {
            Ok(pending) => {
                self.sync_status = match crate::platform::open_url(&pending.url) {
                    Ok(()) => "Finish in the browser window that just opened…".into(),
                    // No browser we could launch — the URL is on screen with a Copy button.
                    Err(e) => format!("Open the sign-in link below ({e})"),
                };
                self.sync_login = Some(pending);
            }
            Err(e) => self.sync_status = format!("Sign-in failed: {e}"),
        }
    }

    /// Watch the loopback listener for the redirect, then swap the code for tokens.
    fn poll_login(&mut self) {
        let Some(code) = self.sync_login.as_ref().and_then(|p| p.rx.try_recv().ok()) else {
            return;
        };
        let Some(pending) = self.sync_login.take() else {
            return;
        };
        let code = match code {
            Ok(c) => c,
            Err(e) => {
                self.sync_status = format!("Sign-in failed: {e}");
                return;
            }
        };
        let (id, secret) = (
            self.settings.sync_client_id.trim().to_string(),
            self.settings.sync_client_secret.trim().to_string(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.sync_rx = Some(rx);
        self.sync_status = "Finishing sign-in…".into();
        self.spawner.spawn(async move {
            let msg = match crate::cloud::exchange(
                &id,
                &secret,
                &code,
                &pending.verifier,
                &pending.redirect,
            )
            .await
            {
                Ok(t) => {
                    t.save();
                    SyncMsg::Signed(t)
                }
                Err(e) => SyncMsg::Error(e),
            };
            let _ = tx.send(msg);
        });
    }

    /// Forget the tokens (and the bookkeeping, so a later sign-in starts clean). The Drive copy
    /// is left alone — signing out of a laptop should not wipe the phone's settings.
    fn sync_sign_out(&mut self) {
        crate::cloud::Tokens::forget();
        self.sync_tokens = None;
        self.sync_login = None;
        self.sync_state = crate::cloud::SyncState::default();
        self.sync_state.save();
        self.sync_status = "Signed out".into();
    }

    /// One sync pass: refresh the token, look at what Drive has, and push or pull accordingly.
    fn sync_now(&mut self) {
        let Some(tokens) = self.sync_tokens.clone() else {
            self.sync_status = "Not signed in".into();
            return;
        };
        let local = match serde_json::to_value(&self.settings) {
            Ok(v) => v,
            Err(e) => {
                self.sync_status = format!("Sync failed: {e}");
                return;
            }
        };
        let share = crate::cloud::shareable(&local);
        let hash = crate::cloud::hash(&share);
        let body = serde_json::to_string_pretty(&share).unwrap_or_default();
        let st = self.sync_state.clone();
        let (id, secret) = (
            self.settings.sync_client_id.trim().to_string(),
            self.settings.sync_client_secret.trim().to_string(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.sync_rx = Some(rx);
        self.sync_checked = Some(wxdata::clock::Instant::now());
        self.sync_status = "Syncing…".into();
        self.spawner.spawn(async move {
            let mut tokens = tokens;
            let access = match crate::cloud::access_token(&id, &secret, &mut tokens).await {
                Ok(a) => a,
                Err(e) => {
                    let _ = tx.send(SyncMsg::Error(e));
                    return;
                }
            };
            let _ = tx.send(SyncMsg::Signed(tokens));
            let remote = match crate::cloud::fetch(&access).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(SyncMsg::Error(e));
                    return;
                }
            };
            let local_changed = hash != st.local_hash;
            let remote_changed = remote
                .as_ref()
                .is_some_and(|r| r.modified != st.remote_modified);
            let action = crate::cloud::decide(local_changed, remote_changed, remote.is_some());
            let msg = match (action, remote) {
                (crate::cloud::Action::Push, r) => {
                    match crate::cloud::push(&access, r.as_ref().map(|r| r.id.as_str()), body).await
                    {
                        Ok(modified) => SyncMsg::Pushed { modified, hash },
                        Err(e) => SyncMsg::Error(e),
                    }
                }
                (crate::cloud::Action::Pull, Some(r)) => SyncMsg::Pulled {
                    body: r.body,
                    modified: r.modified,
                },
                (crate::cloud::Action::Conflict, Some(r)) => {
                    let _ = tx.send(SyncMsg::Conflict);
                    SyncMsg::Pulled {
                        body: r.body,
                        modified: r.modified,
                    }
                }
                _ => SyncMsg::UpToDate,
            };
            let _ = tx.send(msg);
        });
    }

    /// How often a signed-in, sync-enabled app checks Drive on its own.
    const SYNC_SECS: u64 = 300;

    /// Apply whatever the sync worker sent, and start a pass when one is due.
    fn poll_sync(&mut self) {
        self.poll_login();
        while let Some(msg) = self.sync_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match msg {
                SyncMsg::Signed(t) => {
                    let first = self.sync_tokens.is_none();
                    self.sync_tokens = Some(t);
                    if first {
                        self.sync_login = None;
                        self.settings.sync_enabled = true;
                        self.sync_status = "Signed in".into();
                        self.sync_now();
                    }
                }
                SyncMsg::Pulled { body, modified } => match self.apply_synced(&body) {
                    Ok(hash) => {
                        self.sync_state = crate::cloud::SyncState {
                            remote_modified: modified,
                            local_hash: hash,
                            last_sync: crate::share::now(),
                        };
                        self.sync_state.save();
                        if self.sync_status != "Kept the synced copy (both sides had edits)" {
                            self.sync_status = "Settings pulled from Drive".into();
                        }
                    }
                    Err(e) => self.sync_status = format!("Sync failed: {e}"),
                },
                SyncMsg::Pushed { modified, hash } => {
                    self.sync_state = crate::cloud::SyncState {
                        remote_modified: modified,
                        local_hash: hash,
                        last_sync: crate::share::now(),
                    };
                    self.sync_state.save();
                    self.sync_status = "Settings pushed to Drive".into();
                }
                SyncMsg::Conflict => {
                    self.sync_status = "Kept the synced copy (both sides had edits)".into();
                }
                SyncMsg::UpToDate => {
                    self.sync_state.last_sync = crate::share::now();
                    self.sync_state.save();
                    self.sync_status = "Up to date".into();
                }
                SyncMsg::Error(e) => self.sync_status = format!("Sync failed: {e}"),
            }
        }
        // Periodic pass, plus the one at startup (`sync_checked` starts unset).
        if self.settings.sync_enabled
            && self.sync_tokens.is_some()
            && self
                .sync_checked
                .is_none_or(|t| t.elapsed().as_secs() >= Self::SYNC_SECS)
        {
            self.sync_now();
        }
    }

    /// Replace the settings with the synced ones, keeping this machine's own fields, and persist.
    /// Returns the hash the sync bookkeeping should record.
    fn apply_synced(&mut self, body: &str) -> Result<u64, String> {
        let local = serde_json::to_value(&self.settings).map_err(|e| e.to_string())?;
        let merged = crate::cloud::merge_in(&local, body)?;
        let settings: crate::settings::Settings =
            serde_json::from_value(merged).map_err(|e| e.to_string())?;
        self.settings = settings;
        self.settings.save();
        let hash = crate::cloud::hash(&crate::cloud::shareable(
            &serde_json::to_value(&self.settings).map_err(|e| e.to_string())?,
        ));
        Ok(hash)
    }

    /// How often our own fix goes out on both transports. Fast enough to follow a chase vehicle,
    /// slow enough to be free on a metered connection.
    const SHARE_SECS: u64 = 10;

    /// Push our position out and pull everyone else's in. Sharing runs whenever the setting is on
    /// — receiving works even with no fix of our own, which is the desktop-at-home half of it.
    fn sync_share(&mut self, ctx: &egui::Context) {
        if !self.settings.share_position {
            if self.share.is_some() {
                self.share = None;
                self.peers.clear();
            }
            return;
        }
        let share = self.share.get_or_insert_with(crate::share::Share::start);
        if share.drain(&mut self.peers) {
            ctx.request_repaint();
        }
        let due = self
            .share_sent
            .is_none_or(|t| t.elapsed().as_secs() >= Self::SHARE_SECS);
        let Some((lon, lat)) = self.chase_pos.filter(|_| due) else {
            return;
        };
        self.share_sent = Some(wxdata::clock::Instant::now());
        let name = if self.settings.share_name.is_empty() {
            "me"
        } else {
            &self.settings.share_name
        };
        let me = share.me(name, lon, lat, &self.settings.share_video_url);
        share.broadcast(&me);
        let relay = self.settings.share_relay.clone();
        if relay.is_empty() {
            return;
        }
        // Relay round trip: hand it our fix, take back the list. One request pair per tick, and
        // failures are logged rather than surfaced — a dead relay must not break chase mode.
        let tx = share.sender();
        let id = share.id.clone();
        self.spawner.spawn(async move {
            let client = reqwest::Client::new();
            let body = match serde_json::to_string(&me) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("share encode failed: {e}");
                    return;
                }
            };
            if let Err(e) = client
                .post(&relay)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
            {
                log::warn!("share relay post failed: {e}");
                return;
            }
            match client.get(&relay).send().await {
                Ok(r) => match r.text().await.map_err(|e| e.to_string()).and_then(|t| {
                    serde_json::from_str::<Vec<crate::share::Peer>>(&t).map_err(|e| e.to_string())
                }) {
                    Ok(list) => {
                        for p in list.into_iter().filter(|p| p.id != id) {
                            let _ = tx.send(p);
                        }
                    }
                    Err(e) => log::warn!("share relay list unreadable: {e}"),
                },
                Err(e) => log::warn!("share relay get failed: {e}"),
            }
        });
    }

    /// Pull an HRRR point sounding at `(lon, lat)`, shown in the Skew-T window when it arrives.
    /// Fetch the NWS point forecast for a tapped spot. Results are cached per ~0.05° cell for
    /// 15 minutes — the grid only updates hourly, and re-tapping the same neighborhood shouldn't
    /// re-hit the API.
    fn fetch_point_forecast(&mut self, lon: f64, lat: f64) {
        let key = ((lat * 20.0).round() as i32, (lon * 20.0).round() as i32);
        self.forecast_at = Some((lon, lat));
        self.forecast_open = true;
        self.fetch_point_obs(key, lon, lat);
        if let Some((when, f)) = self.forecast_cache.get(&key) {
            if when.elapsed().as_secs() < 900 {
                self.forecast_state = ui::forecast_window::State::Ready(Box::new(f.clone()));
                return;
            }
        }
        self.forecast_state = ui::forecast_window::State::Loading;
        let (tx, rx) = std::sync::mpsc::channel();
        self.forecast_rx = Some((key, rx));
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = wxdata::forecast::fetch(&http, lat, lon)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Current conditions for the forecast point, on the same cache cell and TTL as the forecast.
    /// A failure (offshore, no station, API down) simply sends nothing — the window drops the
    /// "Now" line rather than showing an error for a decoration.
    fn fetch_point_obs(&mut self, key: (i32, i32), lon: f64, lat: f64) {
        if let Some((when, ..)) = self.forecast_obs_cache.get(&key) {
            if when.elapsed().as_secs() < 900 {
                return;
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.forecast_obs_rx = Some((key, rx));
        let http = self.http.clone();
        self.spawner.spawn(async move {
            match wxdata::obs::fetch_nearest(&http, lat, lon).await {
                Ok(s) => {
                    if let Some(o) = s.obs.first() {
                        let _ = tx.send((s.station_id, o.clone()));
                    }
                }
                Err(e) => log::debug!("point obs unavailable: {e}"),
            }
        });
    }

    /// Open the verification lab, prefilled from what's on screen: the office whose warnings are
    /// in view, and the archive day the timeline is parked on. Typing either again is a fallback,
    /// not the normal path.
    fn open_verify(&mut self) {
        if self.verify_window.wfo.is_empty() {
            // Archived warnings carry their issuing office in `area`; a live pane has none, and
            // the field stays blank rather than guessing.
            // The office whose warning actually covers the camera, not merely the first one in
            // the national archive fetch — otherwise a KTLX view opens scored against St. Louis.
            let (clon, clat) = {
                let c = self.views[self.active].camera.center;
                crate::render::mercator::world_to_lonlat(c.0, c.1)
            };
            self.verify_window.wfo = self
                .arch_warns
                .iter()
                .flat_map(|(_, feats)| feats.iter())
                .filter(|f| {
                    f.bbox().is_some_and(|(w, s, e, n)| {
                        clon >= w && clon <= e && clat >= s && clat <= n
                    })
                })
                .find_map(|f| f.alert.as_ref().map(|a| a.area.clone()))
                .unwrap_or_default();
        }
        if self.verify_window.day.is_empty() {
            self.verify_window.day = self.views[self.active]
                .timeline
                .date
                .format("%Y-%m-%d")
                .to_string();
        }
        self.verify_window.open = true;
        if self.verify_window.data.is_none() && !self.verify_window.wfo.is_empty() {
            self.fetch_verify();
        }
    }

    fn fetch_verify(&mut self) {
        let wfo = self.verify_window.wfo.trim().to_ascii_uppercase();
        let Ok(day) = chrono::NaiveDate::parse_from_str(self.verify_window.day.trim(), "%Y-%m-%d")
        else {
            self.verify_window.error = Some("Day must look like 2013-05-20".into());
            return;
        };
        if wfo.is_empty() {
            self.verify_window.error = Some("Enter a WFO, e.g. OUN".into());
            return;
        }
        let start = day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
        let end = start + chrono::Duration::days(1);
        self.verify_window.busy = true;
        self.verify_window.error = None;
        let (tx, rx) = std::sync::mpsc::channel();
        self.verify_rx = Some(rx);
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = wxdata::verify::fetch(&http, &wfo, start, end)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    fn fetch_sounding(&mut self, lon: f64, lat: f64) {
        self.sounding_window.fh = 0;
        self.sounding_at = Some((lon, lat));
        self.refetch_sounding();
        self.fetch_raob(lon, lat);
    }

    /// Re-pull the sounding at the remembered point for the window's current forecast hour. The
    /// observed ascent is left alone — a radiosonde has no forecast hours.
    fn refetch_sounding(&mut self) {
        let Some((lon, lat)) = self.sounding_at else {
            return;
        };
        let fh = self.sounding_window.fh;
        let (tx, rx) = std::sync::mpsc::channel();
        self.sounding_rx = Some(rx);
        self.sounding_window.open = true;
        self.sounding_window.busy = true;
        self.sounding_window.sounding = None;
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = wxdata::sounding::fetch_at(&http, lon, lat, fh)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// The observed ascent to draw beside the model profile: the nearest radiosonde station, at
    /// the synoptic time before whatever instant the active pane is showing (so an archive scrub
    /// gets that day's sounding, not today's).
    fn fetch_raob(&mut self, lon: f64, lat: f64) {
        self.sounding_window.observed = None;
        self.sounding_window.observed_error = None;
        self.sounding_window.observed_station.clear();
        self.raob_rx = None;
        let Some(station) = wxdata::raob::nearest_station(lon, lat) else {
            return;
        };
        let when = self.views[self.active]
            .timeline
            .current()
            .and_then(|id| id.date_time())
            .unwrap_or_else(chrono::Utc::now);
        self.sounding_window.observed_station = format!(
            "{} ({}) {}",
            station.name,
            station.id,
            wxdata::raob::synoptic_before(when).format("%d %b %HZ")
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.raob_rx = Some(rx);
        let http = self.http.clone();
        let cache = crate::paths::cache_dir();
        self.spawner.spawn(async move {
            let res = wxdata::raob::fetch(&http, station, when, cache)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Optical-flow nowcast: advect every strong reflectivity gate of the active pane forward by
    /// the mean SCIT storm motion to the configured lead time. Returns advected `(lon, lat, color)`
    /// points for the painter. Coarse (subsampled gates) — a first-order extrapolation, not a model.
    /// Cached wrapper: these three detectors each bin (and clone) a full sweep and then walk it,
    /// but their inputs only change when a new volume arrives — running them every frame while
    /// their layer was toggled on burned that cost 4-60 times a second for an identical answer.
    fn compute_nowcast(&mut self, idx: usize) -> Vec<(f64, f64, egui::Color32)> {
        let key = (
            self.volume_key(idx),
            self.views[idx].tilt,
            self.filters.nowcast_lead_min,
            self.scit_mean_motion()
                .map(|(d, k)| (d.to_bits(), k.to_bits())),
            self.palettes.gen,
        );
        if let Some((k, v)) = &self.nowcast_cache {
            if *k == key {
                return v.clone();
            }
        }
        let out = self.compute_nowcast_uncached(idx);
        self.nowcast_cache = Some((key, out.clone()));
        out
    }

    /// The volume identity a detector's result depends on: pane, volume name, and how many sweeps
    /// have merged into it (a live volume keeps the same name as it fills out).
    fn volume_key(&self, idx: usize) -> (usize, String, usize) {
        let v = self.views[idx].volume.as_ref();
        (
            idx,
            v.map(|v| v.name.clone()).unwrap_or_default(),
            v.map(|v| v.scan.sweeps().len()).unwrap_or(0),
        )
    }

    /// [`Self::volume_key`] plus a fingerprint of the detector thresholds, for the caches whose
    /// answer changes when the user moves a slider.
    fn tuned_key(&self, idx: usize) -> TunedKey {
        use std::hash::{Hash, Hasher};
        let d = &self.settings.detectors;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        d.tbss_core_dbz.to_bits().hash(&mut h);
        d.zdr_min_db.to_bits().hash(&mut h);
        d.zdr_min_depth_km.to_bits().hash(&mut h);
        (self.volume_key(idx), h.finish())
    }

    fn compute_nowcast_uncached(&mut self, idx: usize) -> Vec<(f64, f64, egui::Color32)> {
        let Some((dir, kt)) = self.scit_mean_motion() else {
            return Vec::new();
        };
        if kt <= 1.0 {
            return Vec::new();
        }
        let lead_km = kt as f64 * 1.852 * (self.filters.nowcast_lead_min as f64 / 60.0);
        let tilt = self.views[idx].tilt;
        let sweep = match self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Reflectivity, tilt, false).ok())
        {
            Some(s) => s.clone(),
            None => return Vec::new(),
        };
        // Confidence in a pure advection falls off with lead: it moves the echo that exists and
        // cannot grow, decay or turn it. Fading the points says so without a disclaimer nobody
        // reads.
        let alpha = (150.0 * nowcast_confidence(self.filters.nowcast_lead_min)) as u8;
        let table = self.palettes.table(Moment::Reflectivity);
        let span = (sweep.value_max - sweep.value_min).max(1e-3);
        let radar = [sweep.radar_lon as f64, sweep.radar_lat as f64];
        let mut out = Vec::new();
        for az in (0..sweep.az_bins).step_by(4) {
            let az_deg = az as f64 * 360.0 / sweep.az_bins as f64;
            for gate in (0..sweep.gate_count).step_by(6) {
                let vidx = sweep.data[az * sweep.gate_count + gate];
                if vidx < 2 {
                    continue;
                }
                let dbz = sweep.value_min + (vidx as f32 - 2.0) / 253.0 * span;
                if dbz < 30.0 {
                    continue; // only advect meaningful echo
                }
                let range_km = (sweep.first_gate_km + gate as f32 * sweep.gate_interval_km) as f64;
                let gate_ll = crate::geo::destination_point(radar, az_deg, range_km);
                let adv = crate::geo::destination_point(gate_ll, dir as f64, lead_km);
                let c = table.sample(dbz).unwrap_or([120, 120, 120, 255]);
                out.push((
                    adv[0],
                    adv[1],
                    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], alpha),
                ));
            }
        }
        out
    }

    /// Hail spikes (TBSS) for the active pane's lowest tilt, cached per volume like the TDS
    /// detector. Display-only: a spike is context about hail, not a reason to make noise.
    fn compute_tbss(&mut self, idx: usize) -> Vec<wxdata::dualpol::TbssHit> {
        let key = self.tuned_key(idx);
        let core_dbz = self.settings.detectors.tbss_core_dbz;
        if let Some((k, v)) = &self.tbss_cache {
            if *k == key {
                return v.clone();
            }
        }
        let z = self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Reflectivity, 0, false).ok())
            .cloned();
        let cc = self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::CorrelationCoefficient, 0, false).ok())
            .cloned();
        let out = match (z, cc) {
            (Some(z), Some(cc)) => wxdata::dualpol::tbss(&z, &cc, core_dbz, 20.0, 0.8, 4.0, 150.0),
            _ => Vec::new(),
        };
        self.tbss_cache = Some((key, out.clone()));
        out
    }

    /// ZDR columns and the bright band, both from a full pass over the active pane's tilts.
    ///
    /// The freezing level comes from the same model analysis the hail grids use, so this needs a
    /// live pane with that fetch already done; without it there is nothing to be "above" and the
    /// answer is empty.
    fn compute_zdr_columns(
        &mut self,
        idx: usize,
        ctx: &egui::Context,
    ) -> Vec<wxdata::dualpol::ZdrColumnHit> {
        let key = self.tuned_key(idx);
        let (min_zdr, min_depth) = (
            self.settings.detectors.zdr_min_db,
            self.settings.detectors.zdr_min_depth_km,
        );
        if let Some((k, v, _)) = &self.zdr_cache {
            if *k == key {
                return v.clone();
            }
        }
        // Model heights are above sea level; beam heights are above the radar.
        let site = self.views[idx].site.clone();
        let radar_km = site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
            .map_or(0.0, |s| s.elevation_meters as f64 / 1000.0);
        let h0_km = match (&self.freezing, &site) {
            (Some((s, h0, _)), Some(cur)) if s == cur => *h0 / 1000.0 - radar_km,
            _ => {
                self.fetch_freezing_levels(ctx);
                return Vec::new();
            }
        };
        let Some(vol) = self.views[idx].volume.as_mut() else {
            return Vec::new();
        };
        let zdr = vol.moment_tilts(Moment::DifferentialReflectivity);
        let z = vol.moment_tilts(Moment::Reflectivity);
        let cc = vol.moment_tilts(Moment::CorrelationCoefficient);
        let hits = wxdata::dualpol::zdr_columns(&zdr, &z, h0_km, min_zdr, min_depth, 40.0, 100.0);
        // The mid tilts are the ones that cut the melting layer at a range where the beam is
        // still narrow enough to mean something.
        let mid = |v: &[wxdata::level2::BinnedSweep]| -> Vec<wxdata::level2::BinnedSweep> {
            v.iter()
                .filter(|s| (2.0..=10.0).contains(&s.elevation_deg))
                .cloned()
                .collect()
        };
        let bb = wxdata::dualpol::bright_band(&mid(&cc), &mid(&z), 6.0);
        self.zdr_cache = Some((key, hits.clone(), bb));
        hits
    }

    /// Auto TDS detection for the active pane's lowest tilt: bin reflectivity + CC and flag debris
    /// signatures (low CC in high Z). Fires a chime + banner on the rising edge of a new detection.
    fn compute_tds(&mut self, idx: usize) -> Vec<wxdata::tds::TdsHit> {
        let key = self.volume_key(idx);
        if let Some((k, v)) = &self.tds_cache {
            if *k == key {
                return v.clone();
            }
        }
        let out = self.compute_tds_uncached(idx);
        self.tds_cache = Some((key, out.clone()));
        out
    }

    fn compute_tds_uncached(&mut self, idx: usize) -> Vec<wxdata::tds::TdsHit> {
        // Lowest tilt carries the near-ground debris; dual-pol CC must be present.
        let z = match self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Reflectivity, 0, false).ok())
        {
            Some(s) => s.clone(),
            None => return Vec::new(),
        };
        let Some(cc) = self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::CorrelationCoefficient, 0, false).ok())
            .cloned()
        else {
            return Vec::new();
        };
        let hits = wxdata::tds::detect(&z, &cc, 0.80, 40.0, 150.0, 4);
        // Rising-edge alert.
        let now_active = !hits.is_empty();
        if now_active && !self.tds_active {
            print!("\x07");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            self.banner(
                "⚠ TDS detected".to_string(),
                format!("{} debris signature(s) — possible tornado", hits.len()),
            );
            self.notify_alert(
                "⚠ Tornado Debris Signature",
                "Low CC + high reflectivity detected on radar",
                true,
            );
            if self.settings.alert_sound {
                self.play_alert_urgent(&self.settings.tds_sound.clone());
            }
        }
        self.tds_active = now_active;
        hits
    }

    /// Client-side rotation detection for the active pane's lowest tilt: bin the dealiased
    /// velocity sweep and flag gate-to-gate couplets. Fires a chime + banner on the rising edge,
    /// like the TDS detector (they're complementary: rotation aloft precedes debris at the ground).
    fn compute_couplets(&mut self, idx: usize) -> Vec<wxdata::rotation::CoupletHit> {
        let key = self.volume_key(idx);
        if let Some((k, v)) = &self.couplet_cache {
            if *k == key {
                return v.clone();
            }
        }
        let out = self.compute_couplets_uncached(idx);
        self.couplet_cache = Some((key, out.clone()));
        out
    }

    /// Packs saved in this browser, refreshed in the background whenever one is written.
    #[cfg(target_arch = "wasm32")]
    fn packs(&self) -> Vec<crate::webcache::Pack> {
        crate::webcache::known_packs()
    }

    /// The last thing the pack machinery has to say — a progress line or an error.
    #[cfg(target_arch = "wasm32")]
    fn pack_status(&self) -> Option<String> {
        crate::webcache::status()
    }

    /// Save the active timeline's archived frames into an offline pack.
    ///
    /// The live head is skipped on purpose: the newest object can still be uploading, and half a
    /// volume kept forever is worse than one frame missing from a saved loop.
    #[cfg(target_arch = "wasm32")]
    fn save_offline_pack(&mut self, ctx: &egui::Context) {
        let tl = &self.views[self.active].timeline;
        let ids: Vec<_> = tl.frames.iter().take(tl.playhead + 1).cloned().collect();
        let site = self.views[self.active].site.clone().unwrap_or_default();
        let date = tl.date.format("%Y-%m-%d").to_string();
        let ctx = ctx.clone();
        self.spawner.spawn(async move {
            crate::webcache::save_timeline(site, date, ids).await;
            ctx.request_repaint();
        });
    }

    /// Point the active pane at a saved pack: its site and day, and its frames, without listing
    /// anything over the network. Playback then reads each volume back out of IndexedDB.
    #[cfg(target_arch = "wasm32")]
    fn load_offline_pack(&mut self, pack: &crate::webcache::Pack) {
        let Ok(date) = chrono::NaiveDate::parse_from_str(&pack.date, "%Y-%m-%d") else {
            return;
        };
        self.views[self.active].site = Some(pack.site.clone());
        let tl = &mut self.views[self.active].timeline;
        tl.date = date;
        tl.following = false;
        tl.listing = false;
        tl.frames = pack
            .volumes
            .iter()
            .map(|n| Identifier::new(n.clone()))
            .collect();
        tl.playhead = 0;
        tl.playing = true;
        tl.loop_enabled = true;
    }

    /// Cell tracks for the active pane, computed here from reflectivity rather than read from a
    /// Level 3 storm-cell table — the point of the layer is the sites that have no such table.
    ///
    /// Built by folding [`wxdata::celltrack::associate`] over the timeline frames still in the
    /// decode cache, so it needs no extra downloads and silently produces nothing on a fresh boot
    /// with one volume in hand.
    fn compute_local_tracks(&mut self) -> Vec<wxdata::celltrack::Track> {
        let key = self.volume_key(self.active);
        if let Some((k, v)) = &self.tracks_cache {
            if *k == key {
                return v.clone();
            }
        }
        let frames: Vec<_> = self.views[self.active]
            .timeline
            .frames
            .iter()
            .take(self.views[self.active].timeline.playhead + 1)
            .filter_map(|id| id.date_time().map(|t| (id.name().to_string(), t)))
            .filter(|(name, _)| self.scan_cache.contains(name))
            .collect();
        let mut tracks: Vec<wxdata::celltrack::Track> = Vec::new();
        for (name, at) in frames {
            let cells = match self.celltrack_cache.get(&name) {
                Some(c) => c.clone(),
                None => {
                    let Some(scan) = self.scan_cache.get(&name).map(Arc::clone) else {
                        continue;
                    };
                    // Lowest tilt: the cell a chaser is driving toward is the one at the ground.
                    let cells = match level2::bin_scan(&scan, Moment::Reflectivity, 0) {
                        Ok(sweep) => wxdata::celltrack::find_cells(&sweep, 45.0),
                        Err(e) => {
                            log::debug!("celltrack: skipping {name}: {e}");
                            Vec::new()
                        }
                    };
                    self.celltrack_cache.put(name.clone(), cells.clone());
                    cells
                }
            };
            tracks = wxdata::celltrack::associate(&tracks, &cells, at, 30.0);
        }
        // A track whose last point is older than the newest frame stopped being seen; drop it
        // rather than leave a stale arrow pointing at empty sky.
        if let Some(newest) = tracks
            .iter()
            .filter_map(|t| t.points.last())
            .map(|p| p.2)
            .max()
        {
            tracks
                .retain(|t| t.points.last().is_some_and(|p| p.2 == newest) && t.points.len() >= 2);
        }
        self.tracks_cache = Some((key, tracks.clone()));
        tracks
    }

    fn compute_couplets_uncached(&mut self, idx: usize) -> Vec<wxdata::rotation::CoupletHit> {
        // Lowest tilt = closest to the ground; dealiased so folded gates don't fake huge shear.
        let vel = match self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Velocity, 0, true).ok())
        {
            Some(s) => s.clone(),
            None => return Vec::new(),
        };
        // 25 m/s gate-to-gate is the legacy weak-TVS criterion; 15-150 km is the usable range band
        // (nearer, clutter fakes couplets; farther, the beam is too high and too coarsely sampled).
        let hits = wxdata::rotation::detect(&vel, 25.0, 15.0, 150.0, 3);
        let now_active = !hits.is_empty();
        if now_active && !self.rot_active {
            let h = hits[0]; // sorted strongest-first
            let site = self.views[idx].site.clone().unwrap_or_default();
            let kt = h.vrot_ms * 1.943_844;
            let (km, bearing) = crate::geo::great_circle(
                [vel.radar_lon as f64, vel.radar_lat as f64],
                [h.lon, h.lat],
            );
            let where_ = format!("{:.0} km {} of {site}", km, cardinal(bearing));
            self.banner(
                "⟳ Rotation detected".to_string(),
                format!("{kt:.0} kt couplet — {where_}"),
            );
            self.notify_alert(
                "⟳ Rotation couplet",
                &format!("{kt:.0} kt rotational velocity — {where_}"),
                true,
            );
            if self.settings.alert_sound {
                self.play_alert_urgent(&self.settings.rotation_sound.clone());
            }
        }
        self.rot_active = now_active;
        self.rotation_near_you(&hits);
        hits
    }

    /// "There is rotation near a place you care about" — the detection above fires once for the
    /// whole radar, which tells you a couplet exists somewhere in a 150 km circle. This one names
    /// the place and the distance, and it re-fires as a storm works down a line, so it is the
    /// alert worth pushing to a phone.
    ///
    /// Same shape as the lightning alarm: a per-location cooldown, so a couplet that persists over
    /// six volumes is one alert, not six.
    fn rotation_near_you(&mut self, hits: &[wxdata::rotation::CoupletHit]) {
        let metric = self.metric();
        const COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);
        if hits.is_empty() {
            return;
        }
        // Strongest couplet within each watched radius, if any.
        let near: Vec<(String, String, f64, f32)> = self
            .watched_points()
            .into_iter()
            .filter_map(|p| {
                let radius_km = p.radius_mi * crate::geo::KM_PER_MILE;
                hits.iter()
                    .filter_map(|h| {
                        let (km, _) = crate::geo::great_circle([p.lon, p.lat], [h.lon, h.lat]);
                        (km <= radius_km).then_some((km, h.vrot_ms))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(km, vrot)| (p.id, p.name, km, vrot))
            })
            .collect();
        let mut fired = false;
        for (id, name, km, vrot_ms) in near {
            if self
                .rotation_alerted
                .get(&id)
                .is_some_and(|t| t.elapsed() < COOLDOWN)
            {
                continue;
            }
            self.rotation_alerted.insert(id, Instant::now());
            let kt = vrot_ms as f64 * 1.943_844;
            let away = crate::geo::fmt_distance(km, metric, 0);
            self.banner(
                format!("\u{21bb} Rotation near {name}"),
                format!("{kt:.0} kt couplet, {away} away"),
            );
            self.notify_alert(
                &format!("\u{21bb} Rotation near {name}"),
                &format!("{kt:.0} kt rotational velocity, {away} from {name}"),
                true,
            );
            fired = true;
        }
        if fired && self.settings.alert_sound {
            self.play_alert_urgent(&self.settings.rotation_sound.clone());
        }
    }

    /// DVR instant replay: jump the active timeline to the earliest frame still buffered in the
    /// decode cache and loop-play from there, so the recent session replays instantly from RAM.
    fn instant_replay(&mut self) {
        let start = {
            let tl = &self.views[self.active].timeline;
            if tl.frames.is_empty() {
                return;
            }
            tl.frames
                .iter()
                .position(|id| self.scan_cache.contains(&id.name().to_string()))
                .unwrap_or(0)
        };
        let tl = &mut self.views[self.active].timeline;
        tl.following = false;
        tl.playhead = start;
        tl.playing = true;
        tl.loop_enabled = true;
    }

    /// Count of the active timeline's frames currently held in the decode cache (DVR depth).
    fn dvr_depth(&self) -> usize {
        self.views[self.active]
            .timeline
            .frames
            .iter()
            .filter(|id| self.scan_cache.contains(&id.name().to_string()))
            .count()
    }

    /// Tornado-climatology query at `(lon, lat)`: if the SPC track database is loaded, list nearby
    /// historical tornadoes; otherwise start the (cached) async load and queue the query.
    fn query_climatology(&mut self, lon: f64, lat: f64) {
        const RADIUS_KM: f64 = 40.0; // ~25 mi
        self.climo_open = true;
        self.climo_error = None;
        self.query_warning_history(lon, lat);
        if let Some(tracks) = self.climo_tracks.clone() {
            self.climo_hits = wxdata::torclimo::near(&tracks, lon, lat, RADIUS_KM);
            self.climo_center = Some((lon, lat));
            return;
        }
        self.climo_center = Some((lon, lat));
        self.climo_pending_query = Some((lon, lat));
        self.load_climatology();
    }

    /// Fetch the archived NWS warning history for `(lon, lat)`. Cheap (one JSON request), so it runs
    /// per click rather than being cached; the IEM archive starts in 1986.
    fn query_warning_history(&mut self, lon: f64, lat: f64) {
        self.climo_warn = None;
        let (tx, rx) = std::sync::mpsc::channel();
        self.climo_warn_rx = Some(rx);
        let http = self.http.clone();
        let edate = chrono::Utc::now().format("%Y-%m-%d").to_string();
        self.spawner.spawn(async move {
            let res =
                wxdata::archive_warnings::fetch_point_events(&http, lon, lat, "1986-01-01", &edate)
                    .await
                    .map(|e| wxdata::archive_warnings::summarize(&e))
                    .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Kick off the one-time tornado-database load: read the on-disk cache if present, else download
    /// the SPC CSV and cache it. Idempotent while a load is already in flight.
    fn load_climatology(&mut self) {
        if self.climo_loading || self.climo_tracks.is_some() {
            return;
        }
        self.climo_loading = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.climo_rx = Some(rx);
        let http = self.http.clone();
        let cache = crate::paths::cache_dir().map(|d| d.join("torclimo_1950-2022.csv"));
        self.spawner.spawn(async move {
            let res = load_or_fetch_climo(&http, cache)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Build a plain-language briefing of the in-view weather. The templated summary shows
    /// instantly; if an Anthropic key is set, Claude rewrites it in the background.
    fn generate_digest(&mut self) {
        let bounds = self.view_bounds();
        let overlaps = |f: &GeoFeature| {
            let Some((w, s, e, n)) = f.bbox() else {
                return false;
            };
            !(e < bounds.0 || w > bounds.2 || n < bounds.1 || s > bounds.3)
        };
        let alerts: Vec<crate::digest::AlertLine> = self
            .alert_features
            .iter()
            .filter(|f| overlaps(f))
            .filter_map(|f| f.alert.as_ref())
            .map(|a| crate::digest::AlertLine {
                event: a.event.clone(),
                area: a.area.clone(),
            })
            .collect();
        let mut reports = [0usize; 3]; // tornado, wind, hail
        for r in self.active_storm_reports() {
            use wxdata::spc::ReportKind::*;
            match r.kind {
                Tornado => reports[0] += 1,
                Wind => reports[1] += 1,
                Hail => reports[2] += 1,
                Flood | Other => {}
            }
        }
        let templated = crate::digest::templated(&alerts, reports);
        self.digest_window.text = templated.clone();
        self.digest_window.enhanced = false;

        // Optional Claude enhancement.
        let key = self.settings.anthropic_key.trim().to_string();
        if key.is_empty() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.digest_rx = Some(rx);
        self.digest_window.busy = true;
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = crate::digest::claude(&http, &key, &templated)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Lon/lat bounds `(min_lon, min_lat, max_lon, max_lat)` of the active pane's viewport.
    fn view_bounds(&self) -> (f64, f64, f64, f64) {
        use crate::render::mercator::world_to_lonlat;
        let cam = &self.views[self.active].camera;
        let vp = self.last_viewport;
        let (wx0, wy0) = cam.screen_to_world((0.0, 0.0), vp);
        let (wx1, wy1) = cam.screen_to_world((vp.0, vp.1), vp);
        let (lon0, lat0) = world_to_lonlat(wx0, wy0);
        let (lon1, lat1) = world_to_lonlat(wx1, wy1);
        (
            lon0.min(lon1),
            lat0.min(lat1),
            lon0.max(lon1),
            lat0.max(lat1),
        )
    }

    /// Fixed chase-pack zoom span for the current view: `z_lo = floor(zoom)`, four levels deeper,
    /// both capped to the active basemap's max (zooming past the style's deepest level packs that
    /// deepest level instead of an empty range).
    fn chasepack_zoom(&self) -> (u8, u8) {
        use crate::tiles::BasemapStyle;
        let style = self.views[self.active].basemap.resolve(true);
        let z_lo = (self.views[self.active].camera.zoom.floor() as i64).clamp(2, 18) as u8;
        let max_z = if style.is_raster() {
            self.tiles.max_pack_z(style)
        } else if matches!(style, BasemapStyle::Dark | BasemapStyle::Light) {
            self.vtiles.max_pack_z()
        } else {
            z_lo
        };
        let z_lo = z_lo.min(max_z);
        (z_lo, (z_lo + 4).min(max_z))
    }

    /// Per-frame chase-pack estimate + progress [`map_rows`](Self::map_rows) renders.
    fn chasepack_ui(&self) -> ui::layer_options::ChasePackUi {
        use crate::tiles::BasemapStyle;
        // Dark and Light pack the same vector tiles (the `.pbf` cache is palette-agnostic), so
        // resolving `Auto` either way gives the same pack.
        let style = self.views[self.active].basemap.resolve(true);
        let packable = if style.is_raster() {
            self.tiles.packable(style)
        } else if matches!(style, BasemapStyle::Dark | BasemapStyle::Light) {
            self.vtiles.packable()
        } else {
            false
        };
        let (z_lo, z_hi) = self.chasepack_zoom();
        let tiles = if packable {
            let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
            let mut n =
                crate::tiles::pack_tile_count(min_lon, min_lat, max_lon, max_lat, z_lo, z_hi);
            // The extras the pack quietly adds, so the estimate matches what downloads: terrain
            // at whichever DEM zoom is set, and streets beside raster imagery.
            let dem_z = if self.settings.pack_hires_dem {
                crate::elevation::DEM_ZOOM_HIRES
            } else {
                crate::elevation::DEM_ZOOM
            };
            n += crate::tiles::pack_tile_count(min_lon, min_lat, max_lon, max_lat, dem_z, dem_z);
            if style.is_raster() && self.settings.pack_include_vector {
                let vz_hi = z_hi.min(self.vtiles.max_pack_z());
                n += crate::tiles::pack_tile_count(
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    z_lo.min(vz_hi),
                    vz_hi,
                );
            }
            if !style.is_raster() && self.settings.pack_include_satellite {
                let sz_hi = z_hi.min(self.tiles.max_pack_z(BasemapStyle::HybridSatellite));
                n += crate::tiles::pack_tile_count(
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    z_lo.min(sz_hi),
                    sz_hi,
                );
            }
            n
        } else {
            0
        };
        // ponytail: 25 KB/tile average across raster + vector; only used for the "≈ MB" hint.
        let mb = tiles as f64 * 25_000.0 / 1e6;
        let progress = self
            .chasepack
            .as_ref()
            .map(|p| (p.done, p.total, p.errors, p.bytes as f64 / 1e6));
        ui::layer_options::ChasePackUi {
            tiles,
            mb,
            packable,
            z_lo,
            z_hi,
            progress,
        }
    }

    /// Kick off an offline chase-pack download of the current view's basemap tiles (4 workers).
    fn start_chasepack(&mut self) {
        use crate::tiles::BasemapStyle;
        if self.chasepack.is_some() {
            return;
        }
        let style = self.views[self.active].basemap.resolve(true);
        let (z_lo, z_hi) = self.chasepack_zoom();
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        // The DEM resolution is a per-session choice; make sure the pack fetches what the sampler
        // will later read.
        crate::elevation::set_hires(self.settings.pack_hires_dem);
        let mut jobs = if style.is_raster() {
            self.tiles
                .pack_jobs(style, min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
        } else if matches!(style, BasemapStyle::Dark | BasemapStyle::Light) {
            self.vtiles
                .pack_jobs(min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
        } else {
            Vec::new()
        };
        // Streets alongside the imagery: a raster pack used to be either/or, which left an
        // offline chaser with satellite pictures and no road names. Vector tiles cap at their own
        // max zoom, so this asks for what exists rather than the raster range.
        if style.is_raster() && self.settings.pack_include_vector {
            let vz_hi = z_hi.min(self.vtiles.max_pack_z());
            let vz_lo = z_lo.min(vz_hi);
            jobs.extend(
                self.vtiles
                    .pack_jobs(min_lon, min_lat, max_lon, max_lat, vz_lo, vz_hi),
            );
        }
        // And imagery alongside the streets, for the packs that went the other way round.
        if !style.is_raster() && self.settings.pack_include_satellite {
            let sat = BasemapStyle::HybridSatellite;
            let sz_hi = z_hi.min(self.tiles.max_pack_z(sat));
            jobs.extend(self.tiles.pack_jobs(
                sat,
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                z_lo.min(sz_hi),
                sz_hi,
            ));
        }
        // The DEM rides along with every pack, whatever the basemap: offline chase mode wants the
        // blockage overlay as much as it wants the map under it.
        jobs.extend(self.tiles.dem_pack_jobs(min_lon, min_lat, max_lon, max_lat));
        if jobs.is_empty() {
            return;
        }
        let total = jobs.len() as u64;
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        #[cfg(not(target_arch = "wasm32"))]
        crate::tiles::start_pack_download(self._rt.handle(), jobs, cancel.clone(), tx);
        self.chasepack = Some(ChasePack {
            rx,
            cancel,
            total,
            done: 0,
            errors: 0,
            bytes: 0,
        });
    }

    /// Act on the signals the chrome raised this frame (drawer sections, mobile sheets, pills).
    fn apply_ui_actions(&mut self, actions: ui::layer_options::UiActions, ctx: &egui::Context) {
        if let Some(a) = actions.palette {
            self.apply_palette(a, ctx);
        }
        if actions.open_site_dialog && self.site_dialog.is_none() {
            self.site_dialog = Some(Default::default());
        }
        if actions.reload {
            self.trigger_reload(ctx);
        }
        if actions.instant_replay {
            self.instant_replay();
        }
        if actions.download_chasepack {
            self.start_chasepack();
        }
        if actions.cancel_chasepack {
            if let Some(p) = &self.chasepack {
                p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.chasepack = None;
        }
        if actions.outlook_kind_changed && self.filters.outlook_day == 1 {
            // Hazard switched: drop the stale Day-1 features so the empty-check refetches it.
            self.outlook_features[0].clear();
        }
        if actions.ero_day_changed {
            self.ero_features.clear();
            if (1..=3).contains(&self.filters.ero_day) {
                self.spawn_overlay(ctx, OverlaySource::Ero(self.filters.ero_day));
            }
        }
        if actions.wssi_day_changed {
            // Day switched: the shown polygons belong to the old day until the new ones land.
            self.wssi_features.clear();
            if (1..=3).contains(&self.filters.wssi_day) {
                self.spawn_overlay(ctx, OverlaySource::Wssi(self.filters.wssi_day));
            }
        }
        if actions.overlays_changed {
            // Selecting an outlook day/kind that hasn't been fetched yet pulls it on demand.
            let day = self.filters.outlook_day;
            if (1..=8).contains(&day) && self.outlook_features[(day - 1) as usize].is_empty() {
                self.spawn_overlay(
                    ctx,
                    OverlaySource::Outlook(day, self.outlook_kind_for_day()),
                );
            }
            self.rebuild_overlays();
        }
        if actions.srv_from_cells {
            if let Some((dir, spd)) = self.scit_mean_motion() {
                let v = &mut self.views[self.active];
                v.storm_dir_deg = dir;
                v.storm_speed_kt = spd;
                v.srv = true;
            }
        }
    }

    /// Basemap style, smoothing, the startup view and the offline chase pack — the map knobs you
    /// set once and forget. They used to be the toolbox's "Map" section; they now sit under the
    /// drawer's App group (and the mobile drawer's Advanced group), which is the only other place
    /// per-map state is edited.
    fn map_rows(&mut self, ui: &mut egui::Ui, actions: &mut ui::layer_options::UiActions) {
        use crate::settings::StartView;
        let chasepack = self.chasepack_ui();
        let (mb_key, mt_key) = (
            !self.settings.mapbox_key.is_empty(),
            !self.settings.maptiler_key.is_empty(),
        );
        let current = self.views[self.active].basemap;
        // A named button that opens the grid, rather than the grid inline: the drawer column is
        // narrow, and fifty cards in it would push everything else off the panel.
        let mut picked = None;
        ui.menu_button(format!("Background: {}", current.label()), |ui| {
            ui.set_min_width(460.0);
            egui::ScrollArea::vertical()
                .max_height(460.0)
                .show(ui, |ui| {
                    picked = ui::basemap_picker::grid(ui, &mut self.tiles, current, &self.settings);
                });
            if picked.is_some() {
                ui.close();
            }
        });
        if let Some(s) = picked {
            self.views[self.active].basemap = s;
            self.settings.basemap = s.slug().to_string(); // persist across restarts
        }
        let (view, settings) = (&mut self.views[self.active], &mut self.settings);
        ui.weak(if mb_key && mt_key {
            "Z cycles backgrounds"
        } else {
            "Z cycles backgrounds · more styles with Mapbox/MapTiler keys in Settings"
        });
        // One taste setting, not a per-pane one: flip it and every pane follows, and it persists.
        let mut smooth = settings.smooth_radar;
        let smooth_changed = ui
            .checkbox(&mut smooth, "Smooth radar data")
            .on_hover_text("Interpolate between gates instead of drawing hard gate squares")
            .changed();
        if smooth_changed {
            settings.smooth_radar = smooth;
        }

        // A download in flight stays above the disclosure — progress you can't find reads as a hang.
        if let Some((done, total, errors, mb)) = chasepack.progress {
            ui.separator();
            let frac = if total > 0 {
                done as f32 / total as f32
            } else {
                1.0
            };
            ui.add(egui::ProgressBar::new(frac).text(format!("{done}/{total} tiles · {mb:.0} MB")));
            if errors > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 120, 60),
                    format!("{errors} failed"),
                );
            }
            if ui.button("Cancel download").clicked() {
                actions.cancel_chasepack = true;
            }
            return;
        }

        ui.collapsing("Startup & offline", |ui| {
            // Startup view: remember this site + camera as the launch position.
            if ui
                .button("Save as startup view")
                .on_hover_text("Open here (site + map position) on next launch")
                .clicked()
            {
                if let Some(site) = &view.site {
                    settings.start_view = Some(StartView {
                        site: site.clone(),
                        x: view.camera.center.0,
                        y: view.camera.center.1,
                        zoom: view.camera.zoom,
                    });
                }
            }
            if let Some(site) = settings.start_view.as_ref().map(|sv| sv.site.clone()) {
                let mut clear = false;
                ui.horizontal(|ui| {
                    ui.weak(format!("Starts at {site}"));
                    clear = ui.small_button("Clear").clicked();
                });
                if clear {
                    settings.start_view = None;
                }
            }

            // Offline chase pack: pre-cache this view's basemap tiles so it renders with no signal.
            ui.separator();
            if !chasepack.packable {
                ui.weak("Offline pack: pick a raster or vector basemap");
            } else {
                ui.weak(format!(
                    "Offline pack: {} tiles ≈ {:.0} MB (z{}–{}, current view)",
                    chasepack.tiles, chasepack.mb, chasepack.z_lo, chasepack.z_hi
                ));
                ui.checkbox(&mut settings.pack_include_vector, "Include streets")
                    .on_hover_text(
                        "Pack vector street tiles beside raster imagery, so road names still \
                         render offline",
                    );
                ui.checkbox(&mut settings.pack_include_satellite, "Include satellite")
                    .on_hover_text(
                        "Pack satellite imagery beside the vector streets, so terrain still \
                         renders offline",
                    );
                ui.checkbox(&mut settings.pack_hires_dem, "High-detail terrain")
                    .on_hover_text(
                        "z12 terrain (~40 m/px) instead of z10 — sixteen times the tiles",
                    );
                let too_big = chasepack.mb > 2000.0;
                if ui
                    .add_enabled(!too_big, egui::Button::new("⬇ Download offline pack"))
                    .on_hover_text(
                        "Cache this view's basemap tiles to disk for offline use in the field",
                    )
                    .clicked()
                {
                    actions.download_chasepack = true;
                }
                if too_big {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 90, 90),
                        "Too large (>2 GB) — zoom in or narrow the view",
                    );
                }
            }
        });
        if smooth_changed {
            for v in &mut self.views {
                v.smooth = smooth;
            }
        }
    }

    /// Whether distances should read in kilometres: they should everywhere the US networks do not
    /// reach, which the active pane's radar tells us for free. No setting, because a setting here
    /// is one more thing to get wrong — someone watching Hamburg wants kilometres, and someone
    /// watching Oklahoma wants miles, and the pane already knows which one they are looking at.
    ///
    /// A pane with no site loaded reads as the US, which is what every path assumed before there
    /// was more than one network.
    fn metric(&self) -> bool {
        self.metric_in(self.active)
    }

    /// The same question for a specific pane, which is what a per-pane drawing (the measure tool)
    /// wants: split panes can show Oklahoma beside Bavaria.
    fn metric_in(&self, idx: usize) -> bool {
        !matches!(
            self.views[idx]
                .site
                .as_deref()
                .map_or(wxdata::sites::Network::Nexrad, wxdata::sites::network),
            wxdata::sites::Network::Nexrad | wxdata::sites::Network::Tdwr
        )
    }

    /// Lon/lat box `±radius_km` around the active pane's radar site (its coverage area), or `None`
    /// when no site is selected. Used to scope new-warning banners to the viewed radar.
    fn active_site_bounds(&self, radius_km: f64) -> Option<(f64, f64, f64, f64)> {
        let site = self.views[self.active].site.as_deref()?;
        let s = wxdata::sites::site_by_id(site)?;
        let (lat, lon) = (s.latitude as f64, s.longitude as f64);
        let dlat = radius_km / 111.0;
        let dlon = radius_km / (111.0 * lat.to_radians().cos().abs().max(0.01));
        Some((lon - dlon, lat - dlat, lon + dlon, lat + dlat))
    }

    /// Open the warning popup on the alert with `id` (from the alerts panel), showing its bulletin.
    /// Floating Layers panel (desktop): a right-edge glass card holding the searchable registry.
    /// Android hosts the same body in the quick-layers sheet (see `app::mobile`).
    /// Active alerts in the current view — the bell's badge count and its urgency colour.
    fn alert_badge(&mut self) -> (usize, u8) {
        let bounds = self.view_bounds();
        let rows = crate::ui::alert_panel::rows_in_view(self.active_alert_features(), bounds);
        let max_esc = rows.iter().map(|r| r.esc).max().unwrap_or(0);
        (rows.len(), max_esc)
    }

    /// Thin right-edge colorbar: the active pane's moment scale, docked so it never covers the map.
    /// Docked timeline scrubber (desktop): transport + scrub + live badge in a full-width bar
    /// under the map. The date picker, loop and speed are one right-click away on the
    /// LIVE/ARCHIVE badge — the transport is the 95% case and gets the pixels.
    /// [`Settings::tz_for`] for the active pane — the zone for chrome that isn't per-pane.
    pub(crate) fn active_tz(&self) -> Option<wxdata::tz::Tz> {
        self.settings
            .tz_for(self.views[self.active].site.as_deref())
    }

    /// Sidebar header: the site, what you're looking at, its tilt, and the per-product knobs.
    ///
    /// The product list itself is the tree's Radar category (with a plain-English blurb per row);
    /// this section owns everything about the *current* product — the tilt strip and the expert
    /// options that used to hide in the toolbox. All of it writes the same fields the hotkeys do.
    fn product_section(&mut self, ui: &mut egui::Ui, actions: &mut ui::layer_options::UiActions) {
        use crate::ui::style;
        /// Height reserved for the tilt row whether or not a volume is loaded.
        const TILT_ROW_H: f32 = 24.0;
        let (moment, srv, tilt) = {
            let v = &self.views[self.active];
            (v.moment, v.srv, v.tilt)
        };
        let elevations = self.views[self.active]
            .volume
            .as_ref()
            .map(|v| v.elevations.clone())
            .unwrap_or_default();
        let site = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "Pick a site".to_string());

        let pick: Option<(wxdata::level2::Moment, bool)> = None;
        let mut pick_tilt: Option<usize> = None;
        // Expert knobs for the product you're on, edited through locals so the popup closure
        // doesn't need `self`. They used to live in the toolbox's Product ▸ Options disclosure.
        let mut srv_from_cells = false;
        let mut dealias = self.settings.dealias_velocity;
        let mut precip_tint = self.settings.precip_tint;
        let mut srv_on = srv;
        let mi = moment.index();
        let (mut dir_deg, mut speed_kt) = {
            let v = &self.views[self.active];
            (v.storm_dir_deg, v.storm_speed_kt)
        };
        let mut thr_on = self.views[self.active].threshold_enabled[mi];
        let mut thr = self.views[self.active].thresholds[mi];
        let (vmin, vmax) = moment.value_range();
        let (unit_factor, unit_label) = display_units(moment, &self.settings);
        let head_rect = ui
            .horizontal(|ui| {
                if ui
                    .button(format!("{}  {site}", egui_phosphor::regular::BROADCAST))
                    .on_hover_text("Choose the radar site")
                    .clicked()
                {
                    actions.open_site_dialog = true;
                }
                ui.label(
                    egui::RichText::new(crate::products::name(moment, srv))
                        .size(style::FONT_BASE)
                        .strong(),
                );
            })
            .response
            .rect;
        // Always one line, always present: a wrapping row reflowed between 9- and 14-tilt VCPs and
        // vanished entirely between volumes, which slid the whole layer tree below it up and down.
        // A fixed-height horizontal scroll keeps the sidebar still and scrolls the extra tilts.
        let tilt_rect = ui
            .scope(|ui| {
                ui.set_height(TILT_ROW_H);
                egui::ScrollArea::horizontal()
                    .id_salt("tilt_row")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Tilt")
                                    .size(style::FONT_SM)
                                    .color(egui::Color32::from_gray(150)),
                            )
                            .on_hover_text("How high above the ground the beam is looking");
                            if elevations.is_empty() {
                                ui.weak("\u{2014}");
                            }
                            for (i, angle) in elevations.iter().enumerate() {
                                if ui
                                    .selectable_label(i == tilt, format!("{angle:.1}\u{b0}"))
                                    .clicked()
                                {
                                    pick_tilt = Some(i);
                                }
                            }
                        });
                    });
            })
            .response
            .rect;
        self.tour_anchors.product = Some(head_rect.union(tilt_rect));
        egui::CollapsingHeader::new("Product options")
            .default_open(false)
            .show(ui, |ui| {
                if moment == wxdata::level2::Moment::Reflectivity {
                    ui.checkbox(&mut precip_tint, "Tint by precipitation type")
                        .on_hover_text(
                            "Colour the echo blue where it is falling as snow and pink where \
                             it is freezing rain or sleet, from the MRMS surface type. \
                             Reflectivity alone cannot tell them apart.",
                        );
                }
                if moment == wxdata::level2::Moment::Velocity {
                    ui.checkbox(&mut dealias, "Dealias")
                        .on_hover_text("Unfold aliased velocity (region-based dealiasing)");
                    ui.checkbox(&mut srv_on, "Storm-relative");
                    if srv_on {
                        ui.horizontal(|ui| {
                            ui.label("Motion:");
                            ui.add(
                                egui::DragValue::new(&mut dir_deg)
                                    .range(0.0..=359.0)
                                    .suffix("\u{b0}"),
                            );
                            ui.add(
                                egui::DragValue::new(&mut speed_kt)
                                    .range(0.0..=150.0)
                                    .suffix(" kt"),
                            );
                        });
                        if ui
                            .button("From storm cells")
                            .on_hover_text(
                                "Set motion to the SCIT storm-cell mean (needs L3 storm cells)",
                            )
                            .clicked()
                        {
                            srv_from_cells = true;
                        }
                    }
                }
                // Threshold for the active moment. The slider value stays internal (m/s for
                // velocity); display honors the Units setting.
                let f = unit_factor as f64;
                ui.horizontal(|ui| {
                    ui.checkbox(&mut thr_on, "Threshold").on_hover_text(
                        "Hide everything below a value \u{2014} cuts light rain out of the picture",
                    );
                    if thr_on {
                        let t = thr.get_or_insert((vmin + vmax) * 0.5);
                        ui.add(
                            egui::Slider::new(t, vmin..=vmax)
                                .custom_formatter(move |v, _| format!("{:.0}", v * f))
                                .custom_parser(move |s| s.parse::<f64>().ok().map(|x| x / f))
                                .suffix(unit_label),
                        );
                    }
                });
            });

        if let Some(i) = pick_tilt {
            self.views[self.active].tilt = i;
        }
        if self.settings.precip_tint != precip_tint {
            self.settings.precip_tint = precip_tint;
            self.settings.save();
        }
        self.settings.dealias_velocity = dealias;
        // A product row was clicked this frame: it already set the moment and SRV flag, so the
        // knob write-back must not put the pre-click values back.
        if pick.is_none() {
            let v = &mut self.views[self.active];
            v.srv = srv_on;
            v.storm_dir_deg = dir_deg;
            v.storm_speed_kt = speed_kt;
            v.threshold_enabled[mi] = thr_on;
            v.thresholds[mi] = thr;
        }
        if srv_from_cells {
            if let Some((dir, spd)) = self.scit_mean_motion() {
                let v = &mut self.views[self.active];
                v.storm_dir_deg = dir;
                v.storm_speed_kt = spd;
                v.srv = true;
            }
        }
    }

    /// Chase HUD: the storm-relative numbers a chaser in motion actually needs — where the storm
    /// is, how close it will come and when, and which way to drive to get off its path. Display
    /// only; the arrival cones and NWS warnings already own the alarms.
    fn chase_hud(&mut self, ctx: &egui::Context) {
        let metric = self.metric();
        if !self.chase_mode {
            return;
        }
        let Some((lon, lat)) = self.chase_pos else {
            return;
        };
        let me = [lon, lat];
        // Prefer the cell the camera is following, else the nearest tracked cell within 300 km.
        let cell = match &self.follow_cell {
            Some((_, c, _)) => Some(c.clone()),
            None => nearest_cell(self.active_storm_cells(), lon, lat, 300.0).cloned(),
        };
        let Some(c) = cell else { return };
        let (km, bearing) = crate::geo::great_circle(me, [c.lon, c.lat]);
        let dir = c.mvt_deg.unwrap_or(0.0) as f64;
        let kt = c.mvt_kt.unwrap_or(0.0) as f64;
        let (close_km, close_min) =
            crate::geo::closest_approach([c.lon, c.lat], dir, kt, me, 120.0);
        let escape = crate::geo::escape_bearing([c.lon, c.lat], dir, me);
        // Spoken position updates: only while chasing, only when the picture has actually
        // changed, and never more than once a minute — a voice that repeats itself is a voice the
        // user turns off. Reported in whole miles, so a storm parked in place says nothing.
        if self.settings.speak_position && !self.settings.mute_alerts {
            let whole = if metric {
                km.round() as i32
            } else {
                (km / crate::geo::KM_PER_MILE).round() as i32
            };
            let fresh = self.spoke_pos.is_none_or(|(t, m)| {
                m != whole && t.elapsed() >= std::time::Duration::from_secs(60)
            });
            if fresh {
                self.spoke_pos = Some((Instant::now(), whole));
                crate::speech::speak(&wxdata::spoken::position_script(
                    // "Cell O7", not the bare id — a synthesizer reading "O7" alone is a noise.
                    &if c.id.is_empty() {
                        c.title.clone()
                    } else {
                        format!("Cell {}", c.id)
                    },
                    bearing as f32,
                    km,
                    c.mvt_deg,
                    metric,
                ));
            }
        }
        // Urgent when the storm will be on top of you soon. The threshold stays in kilometres
        // whatever the card reads in: five miles of warning is five miles of warning in Bavaria.
        let urgent = close_km < 8.05 && close_min < 20.0;
        let accent = crate::theme::accent(self.settings.theme);
        let red = egui::Color32::from_rgb(230, 70, 70);
        let inset_bottom = (ctx.viewport_rect().bottom() - ctx.content_rect().bottom()).max(0.0);
        // Android floats the bottom card at the same edge; sit above it.
        let dy = if cfg!(target_os = "android") {
            -(inset_bottom + 190.0)
        } else {
            // Above the product pill, which owns the bottom-left corner.
            crate::ui::style::LANE_BOTTOM_CHASE
        };
        egui::Area::new(egui::Id::new("chase_hud"))
            .constrain_to(self.chrome_rect)
            // Pure readout — never take input, or it kills pinch over its corner of the map.
            .interactable(false)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(14.0, dy))
            .show(ctx, |ui| {
                let frame = mobile::glass(ui, 244).stroke(egui::Stroke::new(
                    if urgent { 2.0 } else { 1.0 },
                    if urgent {
                        red
                    } else {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 22)
                    },
                ));
                frame.show(ui, |ui| {
                    ui.set_width(226.0);
                    let head = if c.id.is_empty() {
                        "Storm".to_string()
                    } else {
                        format!("Storm {}", c.id)
                    };
                    ui.label(
                        egui::RichText::new(head)
                            .size(14.0)
                            .strong()
                            .color(if urgent { red } else { accent }),
                    );
                    let row = |ui: &mut egui::Ui, k: &str, v: String| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(k)
                                    .size(12.0)
                                    .color(egui::Color32::from_gray(160)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(v)
                                            .size(13.0)
                                            .strong()
                                            .color(egui::Color32::from_gray(235)),
                                    );
                                },
                            );
                        });
                    };
                    row(
                        ui,
                        "Now",
                        format!(
                            "{} {} ({:.0}°)",
                            crate::geo::fmt_distance(km, metric, 1),
                            cardinal(bearing),
                            bearing
                        ),
                    );
                    if kt > 1.0 {
                        row(
                            ui,
                            "Closest",
                            format!(
                                "{} in {close_min:.0} min",
                                crate::geo::fmt_distance(close_km, metric, 1)
                            ),
                        );
                        row(
                            ui,
                            "Escape",
                            format!("{} ({:.0}°)", cardinal(escape), escape),
                        );
                        row(ui, "Motion", format!("{} at {kt:.0} kt", cardinal(dir)));
                    } else {
                        row(ui, "Motion", "stationary".into());
                    }
                    if let Some(site) = crate::geo::nearest_site_id(lon, lat) {
                        row(ui, "Radar", site);
                    }
                });
            });
        if urgent {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// The boolean behind an [`OverlayToggle`]. One place to resolve a named toggle so the
    /// layers panel, command palette, and mobile sheet never drift apart.
    /// Does any pane want this field layer? The grids are fetched once and shared, so one pane
    /// asking is enough to keep the download alive.
    fn field_wanted(&self, layer: crate::render::FieldLayer) -> bool {
        self.views.iter().any(|v| v.fields_on.contains(&layer))
    }

    /// Turn a field layer on or off in the active pane.
    fn set_field(&mut self, layer: crate::render::FieldLayer, on: bool) {
        let set = &mut self.views[self.active].fields_on;
        if on {
            set.insert(layer);
        } else {
            set.remove(&layer);
        }
    }

    fn overlay_flag(&mut self, t: OverlayToggle) -> &mut bool {
        use OverlayToggle as T;
        match t {
            T::AlertPanel => &mut self.show_alert_panel,
            T::StormReports => &mut self.show_storm_reports,
            T::Spotters => &mut self.show_spotters,
            T::RadarSites => &mut self.show_radar_sites,
            T::Metar => &mut self.show_metar,
            T::Webcams => &mut self.show_webcams,
            T::Fires => &mut self.show_fires,
            T::Aqi => &mut self.show_aqi,
            T::Stations => &mut self.show_stations,
            T::Dat => &mut self.show_dat,
            T::Gauges => &mut self.show_gauges,
            T::Tropical => &mut self.show_tropical,
            T::Outages => &mut self.show_outages,
            T::ProbSevere => &mut self.show_probsevere,
            T::Aviation => &mut self.show_aviation,
            T::Tfr => &mut self.show_tfr,
            T::RangeRings => &mut self.show_range_rings,
            T::Fronts => &mut self.show_fronts,
            T::GlmLightning => &mut self.show_glm,
            T::Strikes => &mut self.show_strikes,
            T::Wind => &mut self.show_wind,
            T::Sensors => &mut self.show_sensors,
            T::Hodo => &mut self.show_hodo,
            T::Cells => &mut self.filters.show_cells,
            T::Tracks => &mut self.filters.show_tracks,
            T::ArrivalCones => &mut self.filters.show_arrival_cones,
            T::Nowcast => &mut self.filters.show_nowcast,
            T::Tds => &mut self.filters.show_tds,
            T::Tbss => &mut self.filters.show_tbss,
            T::ZdrColumns => &mut self.filters.show_zdr_columns,
            T::Couplets => &mut self.filters.show_couplets,
            T::Alerts => &mut self.filters.show_alerts,
            T::Mds => &mut self.filters.show_mds,
            T::Watches => &mut self.filters.show_watches,
            T::LocalTracks => &mut self.show_local_tracks,
            T::Mping => &mut self.show_mping,
            T::Pireps => &mut self.show_pireps,
            T::Recon => &mut self.show_recon,
            T::LinkCameras => &mut self.link_cameras,
            T::MiniLoop => &mut self.mini_loop,
            T::Blockage => &mut self.show_blockage,
        }
    }

    /// Run one registry action. Every surface (drawer, pills, mobile sheets) routes through it.
    pub(crate) fn apply_palette(&mut self, action: PaletteAction, ctx: &egui::Context) {
        use AppWindow as W;
        match action {
            PaletteAction::SetMoment(m, srv) => {
                let v = &mut self.views[self.active];
                v.moment = m;
                if m == Moment::Velocity {
                    v.srv = srv;
                }
            }
            PaletteAction::ToggleField(layer) => {
                // The active pane's choice, not the app's: that is what makes two panes able to
                // show two fields.
                let on = self.views[self.active].fields_on.contains(&layer);
                self.set_field(layer, !on);
            }
            PaletteAction::ToggleOverlay(t) => {
                let f = self.overlay_flag(t);
                *f = !*f;
                // These feed the assembled feature set rather than a painter flag.
                use OverlayToggle as T;
                if matches!(
                    t,
                    T::Tropical
                        | T::Outages
                        | T::ProbSevere
                        | T::Aviation
                        | T::Tfr
                        | T::Alerts
                        | T::Mds
                        | T::Fires
                ) {
                    self.rebuild_overlays();
                }
            }
            PaletteAction::SetContours(k) => self.contour_kind = k,
            // Tapping the armed tool disarms it. Interrogate is the resting state, so "off" means
            // back to it — without this the row read ON with no way to turn it off.
            PaletteAction::Tool(t) => {
                self.tool = if self.tool == t {
                    MapTool::Interrogate
                } else {
                    t
                }
            }
            PaletteAction::SetPanes(n) => {
                self.set_pane_count(n);
                if n > 1 {
                    self.hint(
                        "panes",
                        "Each pane keeps its own radar, product and tilt \u{2014} turn \
                         on Link pane cameras to pan them together",
                    );
                }
            }
            PaletteAction::AllTilts => self.apply_all_tilts(),
            PaletteAction::CycleBasemap => {
                let (mb, mt) = (
                    !self.settings.mapbox_key.is_empty(),
                    !self.settings.maptiler_key.is_empty(),
                );
                let next = self.views[self.active].basemap.next(
                    mb,
                    mt,
                    crate::tiles::valid_xyz_template(&self.settings.custom_tile_url),
                );
                self.set_basemap(next);
            }
            PaletteAction::ToggleMute => self.apply_action(BindableAction::ToggleMute, ctx),
            PaletteAction::Explain(i) => self.help_hub.explain(i),
            PaletteAction::TogglePanel => self.panel_open = !self.panel_open,
            PaletteAction::Reload => self.trigger_reload(ctx),
            PaletteAction::InstantReplay => self.instant_replay(),
            PaletteAction::GoLive => self.views[self.active].timeline.go_head(),
            PaletteAction::CopyViewLink => {
                let v = &self.views[self.active];
                let c = v.camera.center;
                let (lon, lat) = crate::render::mercator::world_to_lonlat(c.0, c.1);
                // A live view shares as live; a scrubbed one carries its timestamp, so the link
                // lands on the frame the sender was looking at.
                let time = (!v.timeline.following)
                    .then(|| v.timeline.current().and_then(|id| id.date_time()))
                    .flatten();
                let link = goto_link(&Goto {
                    site: v.site.clone().unwrap_or_default(),
                    lon,
                    lat,
                    zoom: v.camera.zoom,
                    time,
                    moment: Some(v.moment),
                    tilt: Some(v.tilt),
                    basemap: None,
                    threshold: v.threshold_enabled[v.moment.index()]
                        .then(|| v.thresholds[v.moment.index()]),
                    srv: false,
                });
                // A phone or a tablet has a share sheet, and pasting into a chat is what this is
                // for; the clipboard is the fallback for everything that does not.
                if !crate::platform::share_link("HookEcho", &link) {
                    ctx.copy_text(link.clone());
                    self.banner("Link copied".to_string(), link);
                }
            }
            PaletteAction::SaveWorkspace => {
                let ws = self.capture_workspace();
                let name = ws.name.clone();
                self.settings.workspaces.push(ws);
                self.settings.save();
                // ponytail: auto-named, renamed in Settings. A naming dialog mid-storm is the
                // last thing anyone wants.
                self.toast(
                    ToastKind::Success,
                    format!("Saved \u{2014} rename \"{name}\" in Settings"),
                );
            }
            PaletteAction::ApplyWorkspace(i) => {
                if let Some(ws) = self.settings.workspaces.get(i).cloned() {
                    self.apply_workspace(&ws, ctx);
                    self.toast(ToastKind::Info, format!("Workspace: {}", ws.name));
                }
            }
            PaletteAction::OpenInWindy => {
                let v = &self.views[self.active];
                let c = v.camera.center;
                let (lon, lat) = crate::render::mercator::world_to_lonlat(c.0, c.1);
                // Pick the layer the user deliberately turned on, not the one that is always
                // there: radar is the default state, so leading with it would make every other
                // branch dead code and always land people on the same Windy page.
                let overlay = if self.show_wind {
                    "wind"
                } else if self.field_wanted(crate::render::FieldLayer::Cape) {
                    "cape"
                } else if v.basemap.slug().starts_with("goes") {
                    "satellite"
                } else if v.show_radar && v.volume.is_some() {
                    "radar"
                } else {
                    "wind"
                };
                let url = windy_url(overlay, lon, lat, v.camera.zoom);
                if let Err(e) = crate::platform::open_url(&url) {
                    log::warn!("could not open {url}: {e}");
                }
            }
            PaletteAction::OpenWindow(w) => match w {
                W::Site => {
                    if self.site_dialog.is_none() {
                        self.site_dialog = Some(Default::default());
                    }
                }
                W::Settings => self.settings_window.open = true,
                W::Markers => self.marker_window.open = true,
                W::Placefiles => self.placefile_window.open = true,
                W::Palettes => self.palette_editor.open = true,
                W::Events => self.event_window.open = true,
                W::ChaseReplay => self.chase_replay.open = true,
                W::Digest => {
                    self.digest_window.open = true;
                    self.generate_digest();
                }
                W::Afd => {
                    self.afd_open = true;
                    self.fetch_afd();
                }
                W::Cappi => {
                    self.show_cappi = true;
                    self.cappi_key = None; // force a re-slice on open
                }
                W::StormTable => self.cells_window.toggle(),
                W::Help => self.help_hub.toggle(),
                W::AlertRules => self.rules_window.toggle(),
                W::Verify => self.open_verify(),
                W::Volume3d => self.build_volume3d(),
                W::Climatology => {
                    self.climo_open = true;
                    self.load_climatology();
                }
                W::LayerManager => self.layer_window_open = true,
                W::Setup => self.firstrun.start(),
                W::Tour => self.tour.start(),
                W::About => {
                    self.about_open = true;
                    self.check_for_update(ctx);
                }
            },
        }
    }

    fn open_alert_popup(&mut self, id: &str) {
        let mut seen = std::collections::HashSet::new();
        let cards: Vec<ui::warning_window::WarnCard> = self
            .active_alert_features()
            .iter()
            .filter_map(|f| f.alert.as_ref().map(|a| (a, f.stroke)))
            .filter(|(a, _)| a.id == id && seen.insert(a.id.clone()))
            .map(|(a, color)| ui::warning_window::WarnCard {
                info: a.clone(),
                color,
            })
            .collect();
        if !cards.is_empty() {
            self.detail = None;
            self.warning_popup = Some(ui::warning_window::WarningPopup {
                cards,
                selected: Some(0),
            });
        }
    }

    fn poll_overlays(&mut self) {
        crate::prof_scope!("poll_overlays");
        let mut changed = false;
        while let Ok(msg) = self.overlay_rx.try_recv() {
            match msg {
                OverlayMsg::AlertSeed(f) => {
                    for id in f
                        .iter()
                        .filter_map(|f| f.alert.as_ref().map(|a| a.dedupe_key()))
                    {
                        self.known_warning_ids.insert(id);
                    }
                    if self.alert_features.is_empty() {
                        self.alert_features = f;
                    }
                }
                OverlayMsg::Alerts(f) => {
                    self.detect_new_warnings(&f);
                    crate::alert_snapshot::save(&f);
                    self.alert_features = f;
                }
                OverlayMsg::Mds(f) => self.md_features = f,
                OverlayMsg::Watches(f) => self.watch_features = f,
                OverlayMsg::Mping(r) => self.mping_reports = r,
                OverlayMsg::Pireps(p) => self.pireps = p,
                OverlayMsg::Recon(o) => self.recon = o,
                OverlayMsg::Ero(day, f) => {
                    if day == self.filters.ero_day {
                        self.ero_features = f;
                    }
                }
                OverlayMsg::Wssi(day, f) => {
                    // A day change in flight must not overwrite the day now selected.
                    if day == self.filters.wssi_day {
                        self.wssi_features = f;
                    }
                }
                OverlayMsg::Outlook(day, f) => {
                    if (1..=3).contains(&day) {
                        self.outlook_features[(day - 1) as usize] = f;
                    }
                }
                OverlayMsg::Cells(site, cells) => {
                    // Keep only if still the active site.
                    if self.views[self.active].site.as_deref() == Some(site.as_str()) {
                        // Reset trend history on a site change; append this volume's samples.
                        if self.cells_site.as_deref() != Some(site.as_str()) {
                            self.cell_trends.clear();
                        }
                        for c in &cells {
                            if c.id.is_empty() {
                                continue;
                            }
                            let hist = self.cell_trends.entry(c.id.clone()).or_default();
                            let sample = ui::cell_window::CellSample {
                                vil: c.vil,
                                top: c.top_kft,
                                dbz: c.max_dbz,
                            };
                            // Skip a duplicate of the last sample (same volume re-fetched).
                            if hist.last().is_none_or(|s| {
                                (s.vil, s.top, s.dbz) != (sample.vil, sample.top, sample.dbz)
                            }) {
                                hist.push(sample);
                                if hist.len() > 40 {
                                    hist.remove(0);
                                }
                            }
                        }
                        // Cell ids churn every volume, and the map only ever grew — an entry
                        // per cell the radar has ever named, for as long as the site is the same.
                        // Keep the ones this volume still has.
                        self.cell_trends
                            .retain(|id, _| cells.iter().any(|c| &c.id == id));
                        self.storm_cells = cells;
                        self.cells_site = Some(site);
                        self.update_follow();
                    }
                }
                OverlayMsg::Placefile(url, pf) => {
                    if let Some(lp) = self.placefiles.iter_mut().find(|lp| lp.url == url) {
                        lp.pf = pf;
                        lp.loaded = true;
                        lp.error = None;
                        lp.last_fetch = Some(Instant::now());
                        self.overlay_gen = self.overlay_gen.wrapping_add(1);
                    }
                }
                OverlayMsg::PlacefileError(url, err) => {
                    log::warn!("{url}: {err}");
                    if let Some(lp) = self.placefiles.iter_mut().find(|lp| lp.url == url) {
                        lp.error = Some(err);
                        lp.last_fetch = Some(Instant::now());
                    }
                }
                OverlayMsg::Field(layer, field) => {
                    if layer == crate::render::FieldLayer::Lightning {
                        self.check_lightning_proximity(&field);
                    }
                    // The precipitation-type grid is kept, not just uploaded: the radar tint
                    // reads it per fragment, and the layer it belongs to may well be hidden.
                    if layer == crate::render::FieldLayer::PrecipType {
                        self.precip_flag_grid = Some(std::sync::Arc::new(PrecipGrid::new(&field)));
                        self.precip_flag_gen = self.precip_flag_gen.wrapping_add(1);
                    }
                    let upload = self.field_upload(layer, &field);
                    if let Some(s) = self.fields.get_mut(&layer) {
                        s.pending = Some(upload);
                    }
                }
                OverlayMsg::ModelDiff(field, valid) => {
                    let layer = crate::render::FieldLayer::ModelDiff;
                    let (range, deadband) = self.diff_field.range();
                    let scale = self.diff_field.input_scale();
                    let upload = field_index_upload(
                        &field,
                        |v| crate::fielddiff::diff_index(v * scale, range),
                        crate::fielddiff::diverging_lut(range, deadband),
                    );
                    if let Some(s) = self.fields.get_mut(&layer) {
                        s.pending = Some(upload);
                    }
                    self.diff_valid = Some(valid);
                    self.diff_grid = Some(field);
                }
                OverlayMsg::StormReports(bucket, reports) => match bucket {
                    None => self.storm_reports = reports,
                    Some(b) => {
                        self.arch_lsr.put(b, reports);
                        if self.arch_lsr_inflight == Some(b) {
                            self.arch_lsr_inflight = None;
                        }
                    }
                },
                OverlayMsg::Aviation(f) => self.aviation_features = f,
                OverlayMsg::Tfr(new, remaining) => {
                    self.tfr_features.extend(new);
                    self.tfr_pending = remaining;
                }
                OverlayMsg::Spotters(spotters) => self.spotters = spotters,
                OverlayMsg::Fronts(a) => self.fronts = Some(a),
                OverlayMsg::FreezingLevels(h0, hm20) => {
                    if let Some(site) = self.views[self.active].site.clone() {
                        self.freezing = Some((site, h0, hm20));
                    }
                }
                OverlayMsg::ProbSevere(f) => {
                    self.evaluate_probsevere_rules(&f);
                    self.probsevere = f;
                }
                OverlayMsg::Hrrr(fc) => {
                    use crate::render::FieldLayer;
                    let upload = self.field_upload(FieldLayer::Hrrr, &fc.field);
                    if let Some(s) = self.fields.get_mut(&FieldLayer::Hrrr) {
                        s.pending = Some(upload);
                    }
                    self.hrrr_run = Some(fc.run);
                    self.hrrr_valid = Some(fc.valid());
                }
                OverlayMsg::Obs(site, res) => {
                    // Keep only if still the active site.
                    if self.views[self.active].site.as_deref() == Some(site.as_str()) {
                        self.sensor_data = Some(res);
                        self.sensor_site = Some(site);
                    }
                }
                OverlayMsg::Vwp(site, levels) => {
                    if self.views[self.active].site.as_deref() == Some(site.as_str()) {
                        // A site change starts a new time series; mixing radars on one axis would
                        // be nonsense.
                        if self.hodo_site.as_deref() != Some(site.as_str()) {
                            self.hodo_history.clear();
                        }
                        // The product carries no timestamp, so identical profiles mean "same scan
                        // refetched" — dedupe on content rather than stamping duplicates.
                        let dup = self
                            .hodo_history
                            .back()
                            .is_some_and(|(_, prev)| *prev == levels);
                        if !dup && !levels.is_empty() {
                            self.hodo_history.push_back((Utc::now(), levels.clone()));
                            // ~2 hours at the 5-minute refetch cadence.
                            while self.hodo_history.len() > 24 {
                                self.hodo_history.pop_front();
                            }
                        }
                        self.hodo_data = levels;
                        self.hodo_site = Some(site);
                    }
                }
                OverlayMsg::ArchiveWarnings(bucket, feats) => {
                    self.arch_warns.put(bucket, feats);
                    if self.arch_warn_inflight == Some(bucket) {
                        self.arch_warn_inflight = None;
                    }
                }
                OverlayMsg::Metar(obs, tafs) => {
                    self.metars = obs;
                    self.tafs = tafs;
                }
                OverlayMsg::Webcams(sites) => {
                    // Drop the cached stills with the list they belonged to. Windy's free-tier
                    // image URLs expire after ten minutes, and a kept texture would otherwise
                    // show the same frame until the layer was toggled off and on.
                    self.pf_icon_tex.retain(|k, _| !k.starts_with("cam:"));
                    self.webcams = sites;
                }
                OverlayMsg::Aqi(obs) => self.aqi = obs,
                OverlayMsg::Fires(perims, incidents) => {
                    self.fire_perims = perims;
                    self.fire_incidents = incidents;
                    // Perimeters ride the tessellated overlay layer, so the assembled feature
                    // set has to be rebuilt — bumping the generation alone re-tessellates the
                    // old list and the perimeters never appear.
                    self.rebuild_overlays();
                }
                OverlayMsg::Stations(obs) => self.stations.ingest(obs),
                OverlayMsg::Ppef(p) => self.stations.ppef = Some(p),
                OverlayMsg::DotCams(cams) => self.stations.cams = cams,
                OverlayMsg::Mill(kv) => self.stations.mill_kv_per_m = Some(kv),
                OverlayMsg::Dat(mut points, tracks) => {
                    // Weakest first, so the EF4/EF5 points end up painted on top of the EF0 and
                    // straight-line-wind ones that outnumber them ten to one.
                    points.sort_by_key(|p| wxdata::dat::ef_number(&p.efscale).unwrap_or(0));
                    self.dat_points = points;
                    self.dat_tracks = tracks;
                }
                OverlayMsg::Mosaic(field, sites, oldest) => {
                    self.mosaic_sites = sites;
                    self.mosaic_oldest = Some(oldest);
                    let layer = crate::render::FieldLayer::Mosaic;
                    let upload = self.field_upload(layer, &field);
                    if let Some(s) = self.fields.get_mut(&layer) {
                        s.pending = Some(upload);
                    }
                }
                OverlayMsg::Gauges(g) => self.gauges = g,
                OverlayMsg::Contours(kind, lines, valid) => {
                    // Keep only if the selection didn't change while the fetch was in flight.
                    if kind == self.contour_kind {
                        self.contours = lines;
                        self.contour_valid = Some(valid);
                    }
                }
                OverlayMsg::Tropical(data) => self.tropical = Some(data),
                OverlayMsg::Outages(f) => self.outage_features = f,
                OverlayMsg::Wind(w) => {
                    self.wind_inflight = None;
                    // Keep only if the selection didn't change while the fetch was in flight.
                    if self.wind_fetched == Some((w.level, w.fcst_hour)) {
                        self.wind = Some(*w);
                    }
                }
            }
            changed = true;
        }
        if changed {
            // One rebuild covers every message kind (ProbSevere/Tropical included).
            self.rebuild_overlays();
        }
    }

    /// The alert features to display right now: live alerts, or the archived set while the active
    /// pane is scrubbed off-live to a bucket we've fetched (feature W).
    fn active_alert_features(&self) -> &[GeoFeature] {
        if let Some(b) = self.arch_warn_shown {
            if let Some(f) = self.arch_warns.peek(&b) {
                return f;
            }
        }
        &self.alert_features
    }

    /// The 5-min UTC bucket (Unix secs / 300) of the active pane's displayed frame, or `None` when
    /// following live (archive warnings only apply to scrubbed archive views).
    fn archive_bucket(&self) -> Option<i64> {
        let v = &self.views[self.active];
        if v.timeline.following {
            return None;
        }
        Some(v.volume.as_ref()?.time.timestamp() / 300)
    }

    /// Drive the archived-warning overlay from the active pane's playhead: fetch the bucket the
    /// scrubbed frame falls in, and swap it in for the live alerts (or back to live at the head).
    fn sync_archive_warnings(&mut self, ctx: &egui::Context) {
        match self.archive_bucket() {
            None => {
                if self.arch_warn_shown.is_some() {
                    self.arch_warn_shown = None;
                    self.rebuild_overlays();
                }
            }
            Some(b) => {
                let cached = self.arch_warns.contains(&b);
                if !cached && self.arch_warn_inflight != Some(b) {
                    self.arch_warn_inflight = Some(b);
                    self.spawn_overlay(ctx, OverlaySource::ArchiveWarnings(b));
                }
                if cached && self.arch_warn_shown != Some(b) {
                    self.arch_warn_shown = Some(b);
                    self.rebuild_overlays();
                }
            }
        }
    }

    /// The storm reports to display right now: the live trailing window, or the archived set
    /// while the active pane is scrubbed off-live (feature CC).
    /// The storm cells to show, which is none of them once the playhead is in the archive.
    ///
    /// Warnings and LSRs have archived equivalents that get swapped in ([`Self::sync_archive_warnings`],
    /// [`Self::sync_archive_lsr`]); Level 3 SCIT does not — the products are only published for the
    /// last couple of days. Leaving the live set on screen drew this afternoon's cells, tracks and
    /// arrival cones over a storm from 2011.
    fn active_storm_cells(&self) -> &[Cell] {
        if self.archive_bucket().is_some() {
            return &[];
        }
        &self.storm_cells
    }

    fn active_storm_reports(&self) -> &[wxdata::spc::StormReport] {
        if let Some(b) = self.arch_lsr_shown {
            if let Some(r) = self.arch_lsr.peek(&b) {
                return r;
            }
        }
        &self.storm_reports
    }

    /// Drive the archived-LSR set from the active pane's playhead (mirrors
    /// [`Self::sync_archive_warnings`], on 30-min buckets).
    fn sync_archive_lsr(&mut self, ctx: &egui::Context) {
        if !self.show_storm_reports {
            return;
        }
        let bucket = (|| {
            let v = &self.views[self.active];
            if v.timeline.following {
                return None;
            }
            Some(v.volume.as_ref()?.time.timestamp() / 1800)
        })();
        match bucket {
            None => self.arch_lsr_shown = None,
            Some(b) => {
                let cached = self.arch_lsr.contains(&b);
                if !cached && self.arch_lsr_inflight != Some(b) {
                    self.arch_lsr_inflight = Some(b);
                    self.spawn_overlay(ctx, OverlaySource::StormReports(Some(b)));
                }
                if cached {
                    self.arch_lsr_shown = Some(b);
                }
            }
        }
    }

    /// Fetch the Area Forecast Discussion for the active site's WFO (feature DD).
    fn fetch_afd(&mut self) {
        let Some((lat, lon)) = self.views[self.active]
            .site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
            .map(|s| (s.latitude as f64, s.longitude as f64))
        else {
            self.afd_error = Some("no site selected".into());
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.afd_rx = Some(rx);
        self.afd_busy = true;
        self.afd_error = None;
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = wxdata::afd::fetch(&http, lat, lon)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Fetch one NHC text product for `storm_id`.
    ///
    /// The URL comes out of the storm feed rather than being built from the id: NHC's product
    /// filenames key off the basin bin (`EP4`), not the storm id, and the feed already carries
    /// the exact page for the current advisory number.
    fn fetch_tropical_text(&mut self, storm_id: &str, product: ui::tropical_window::Product) {
        let Some(storm) = self
            .tropical
            .as_ref()
            .and_then(|t| t.storms.iter().find(|s| s.id == storm_id))
            .cloned()
        else {
            self.tropical_window.error = Some("that storm is no longer being advised on".into());
            return;
        };
        let Some(url) = product.url(&storm).map(str::to_string) else {
            self.tropical_window.error = Some(format!(
                "no {} published for {}",
                product.label(),
                storm.name
            ));
            self.tropical_window.text = None;
            return;
        };
        let title = format!(
            "{} {} — {}",
            storm.classification,
            storm.name,
            product.label()
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.tropical_text_rx = Some(rx);
        self.tropical_window.busy = true;
        self.tropical_window.error = None;
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = wxdata::tropical::fetch_advisory(&http, &title, &url)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Drive the METAR station-plot fetch (feature U): only when enabled and zoomed in enough,
    /// refetching every 75 s or when the view center drifts out of the fetched bbox's middle half.
    fn sync_metar(&mut self, ctx: &egui::Context) {
        // Pilot reports share this function's view-bbox logic but not its zoom gate: they are
        // sparse enough to plot nationwide, and on their own two-minute clock.
        if self.show_pireps
            && self
                .pirep_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 120)
        {
            let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
            self.pirep_last_fetch = Some(Instant::now());
            self.spawn_overlay(
                ctx,
                OverlaySource::Pireps(min_lat, min_lon, max_lat, max_lon),
            );
        }
        if !self.show_metar {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        if (max_lon - min_lon) > 12.0 {
            return; // too zoomed out — a nationwide plot would be unreadable and huge
        }
        let (clon, clat) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        let stale = self
            .metar_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 75);
        // Refetch when the center leaves the middle half of the last fetched bbox.
        let drifted = self.metar_bounds.is_none_or(|(la0, lo0, la1, lo1)| {
            let (mlon, mlat) = ((lo0 + lo1) * 0.5, (la0 + la1) * 0.5);
            let (hw, hh) = ((lo1 - lo0) * 0.25, (la1 - la0) * 0.25);
            (clon - mlon).abs() > hw || (clat - mlat).abs() > hh
        });
        if stale || drifted {
            // Pad the fetch bbox 20% past the view, clamped to 15° per side.
            let pad_lon = ((max_lon - min_lon) * 0.2).min(15.0);
            let pad_lat = ((max_lat - min_lat) * 0.2).min(15.0);
            let (lat0, lon0) = (min_lat - pad_lat, min_lon - pad_lon);
            let (lat1, lon1) = (max_lat + pad_lat, max_lon + pad_lon);
            self.metar_last_fetch = Some(Instant::now());
            self.metar_bounds = Some((lat0, lon0, lat1, lon1));
            self.spawn_overlay(ctx, OverlaySource::Metar(lat0, lon0, lat1, lon1));
        }
    }

    /// Keep the FAA camera sites in view loaded. Same shape as [`Self::sync_metar`] but far
    /// lazier: the camera network doesn't move, so this only refetches when the view drifts out
    /// of the last box (or every 10 min, to pick up sites going in and out of maintenance).
    /// Keep the multi-radar composite current: refetch on the L3 cadence, and immediately when the
    /// camera has moved far enough that the sites in view changed.
    ///
    /// Live only. An archive scrub would need per-site historic N0B for the same minute, which is a
    /// different (and much slower) fetch; until that exists the toggle simply has nothing to show,
    /// so it reports that rather than lying with a live composite over a historic scene.
    /// One line describing the live composite for the drawer: which radars are in it and how stale
    /// its oldest scan is.
    fn mosaic_status(&self) -> String {
        if !self.views[self.active].timeline.following {
            return "Live only — the composite is hidden while you're scrubbing the archive."
                .to_string();
        }
        if self.mosaic_sites.is_empty() {
            return "building…".to_string();
        }
        let age = self
            .mosaic_oldest
            .map(|t| (chrono::Utc::now() - t).num_minutes().max(0))
            .unwrap_or(0);
        format!(
            "{} — oldest scan {age} min old",
            self.mosaic_sites.join(", ")
        )
    }

    fn sync_mosaic(&mut self, ctx: &egui::Context) {
        use crate::render::FieldLayer as FL;
        if !self.field_wanted(FL::Mosaic) {
            return;
        }
        if !self.views[self.active].timeline.following {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        let cap = if cfg!(target_os = "android") { 4 } else { 6 };
        // A metered link pays per site; halve the composite rather than drop it.
        let cap = if crate::platform::is_metered() {
            cap / 2
        } else {
            cap
        };
        let sites = wxdata::mosaic::sites_for_view(
            self.views[self.active].site.as_deref(),
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            cap,
        );
        if sites.is_empty() {
            return;
        }
        let stale = self.fields.get(&FL::Mosaic).is_some_and(|s| {
            s.last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(FL::Mosaic))
        });
        let moved = self.mosaic_bounds != Some((min_lon, min_lat, max_lon, max_lat))
            && sites != self.mosaic_sites;
        if stale || moved {
            if let Some(s) = self.fields.get_mut(&FL::Mosaic) {
                s.last_fetch = Some(Instant::now());
            }
            self.mosaic_bounds = Some((min_lon, min_lat, max_lon, max_lat));
            self.spawn_overlay(ctx, OverlaySource::Mosaic(sites));
        }
    }

    /// Pull AirNow AQI for the view. Monitors report hourly, so does this; without a key the
    /// layer stays empty rather than firing requests that can only 401.
    fn sync_aqi(&mut self, ctx: &egui::Context) {
        if !self.show_aqi || self.settings.airnow_key.is_empty() {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        let stale = self
            .aqi_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 900);
        let (clon, clat) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        let drifted = self.aqi_bounds.is_none_or(|(lo0, la0, lo1, la1)| {
            let (mlon, mlat) = ((lo0 + lo1) * 0.5, (la0 + la1) * 0.5);
            let (hw, hh) = ((lo1 - lo0) * 0.25, (la1 - la0) * 0.25);
            (clon - mlon).abs() > hw || (clat - mlat).abs() > hh
        });
        if stale || drifted {
            let b = (min_lon, min_lat, max_lon, max_lat);
            self.aqi_last_fetch = Some(Instant::now());
            self.aqi_bounds = Some(b);
            self.spawn_overlay(
                ctx,
                OverlaySource::Aqi(b.0, b.1, b.2, b.3, self.settings.airnow_key.clone()),
            );
        }
    }

    /// Pull wildfire perimeters/incidents for the view. Fires move on the scale of hours, so a
    /// 15-minute clock is plenty; the bbox check is what keeps panning cheap.
    fn sync_fires(&mut self, ctx: &egui::Context) {
        if !self.show_fires {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        let stale = self
            .fire_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 900);
        let (clon, clat) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        let drifted = self.fire_bounds.is_none_or(|(lo0, la0, lo1, la1)| {
            let (mlon, mlat) = ((lo0 + lo1) * 0.5, (la0 + la1) * 0.5);
            let (hw, hh) = ((lo1 - lo0) * 0.25, (la1 - la0) * 0.25);
            (clon - mlon).abs() > hw || (clat - mlat).abs() > hh
        });
        if stale || drifted {
            let b = (min_lon, min_lat, max_lon, max_lat);
            self.fire_last_fetch = Some(Instant::now());
            self.fire_bounds = Some(b);
            self.spawn_overlay(ctx, OverlaySource::Fires(b.0, b.1, b.2, b.3));
        }
    }

    fn sync_webcams(&mut self, ctx: &egui::Context) {
        if !self.show_webcams {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        if (max_lon - min_lon) > 20.0 {
            return; // zoomed out past the point where individual cameras mean anything
        }
        let (clon, clat) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        // Under ten minutes on purpose: Windy's free-tier image URLs carry a token that expires at
        // exactly ten, so a 600 s clock would race it and serve 401s.
        let stale = self
            .webcam_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 480);
        let drifted = self.webcam_bounds.is_none_or(|(lo0, la0, lo1, la1)| {
            let (mlon, mlat) = ((lo0 + lo1) * 0.5, (la0 + la1) * 0.5);
            let (hw, hh) = ((lo1 - lo0) * 0.25, (la1 - la0) * 0.25);
            (clon - mlon).abs() > hw || (clat - mlat).abs() > hh
        });
        if stale || drifted {
            let pad_lon = ((max_lon - min_lon) * 0.25).min(10.0);
            let pad_lat = ((max_lat - min_lat) * 0.25).min(10.0);
            let b = (
                min_lon - pad_lon,
                min_lat - pad_lat,
                max_lon + pad_lon,
                max_lat + pad_lat,
            );
            self.webcam_last_fetch = Some(Instant::now());
            self.webcam_bounds = Some(b);
            self.spawn_overlay(
                ctx,
                OverlaySource::Webcams(b.0, b.1, b.2, b.3, self.settings.windy_key.clone()),
            );
        }
    }

    /// Drive the live-station layer: poll the networks on a clock, refresh the electric field
    /// far more slowly (the model publishes every five minutes), and pull the camera catalog only
    /// when the view has moved somewhere new.
    ///
    /// The poll is what fills every open card's ring buffer, so it keeps running while any card is
    /// open even if the layer itself has been switched off.
    fn sync_stations(&mut self, ctx: &egui::Context) {
        if !self.show_stations && self.stations.cards.is_empty() {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        // A continental view would ask for thousands of stations to draw a dot each; the cards are
        // a close-in tool, so the layer waits until the view is regional.
        if (max_lon - min_lon) > 20.0 {
            return;
        }
        if self
            .station_last_poll
            .is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            self.station_last_poll = Some(Instant::now());
            // Still-only cameras (every camera, on a phone) get a fresh frame on the same clock.
            let (rt, http) = (self.spawner.clone(), self.http.clone());
            self.stations.refresh_stills(&rt, &http, ctx);
            self.spawn_overlay(
                ctx,
                OverlaySource::Stations {
                    bbox: (min_lat, min_lon, max_lat, max_lon),
                    center: ((min_lat + max_lat) * 0.5, (min_lon + max_lon) * 0.5),
                    tempest: self.settings.tempest_token.clone(),
                    wu: self.settings.wu_key.clone(),
                    synoptic: self.settings.synoptic_token.clone(),
                },
            );
            if !self.settings.field_mill_url.is_empty() {
                self.spawn_overlay(
                    ctx,
                    OverlaySource::Mill(self.settings.field_mill_url.clone()),
                );
            }
        }
        if self
            .ppef_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 300)
        {
            self.ppef_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Ppef);
        }
        // The camera catalog is megabytes of slow-changing agency data: fetch it per view box, not
        // per tick.
        let bbox = (
            (min_lon * 2.0).round() / 2.0,
            (min_lat * 2.0).round() / 2.0,
            (max_lon * 2.0).round() / 2.0,
            (max_lat * 2.0).round() / 2.0,
        );
        if self.dotcam_bounds != Some(bbox) {
            self.dotcam_bounds = Some(bbox);
            self.spawn_overlay(ctx, OverlaySource::DotCams(bbox.0, bbox.1, bbox.2, bbox.3));
        }
    }

    /// Drive the damage-survey overlay. Surveys are immutable history, so the fetch key is just the
    /// (rounded) view box and the day the active pane is looking at — scrub the timeline onto a
    /// storm date and the survey for that day appears over it.
    fn sync_dat(&mut self, ctx: &egui::Context) {
        if !self.show_dat {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        if (max_lon - min_lon) > 12.0 {
            return; // a continental query is tens of thousands of points
        }
        let day = self.views[self.active]
            .volume
            .as_ref()
            .map(|v| v.time.date_naive())
            .unwrap_or_else(|| chrono::Utc::now().date_naive());
        // Snap the box to a half-degree grid so panning around inside a county doesn't refetch.
        let snap = |v: f64, up: bool| {
            let g = v * 2.0;
            (if up { g.ceil() } else { g.floor() }) / 2.0
        };
        let bbox = (
            snap(min_lon, false),
            snap(min_lat, false),
            snap(max_lon, true),
            snap(max_lat, true),
        );
        if self.dat_key == Some((bbox, day)) {
            return;
        }
        self.dat_key = Some((bbox, day));
        self.spawn_overlay(ctx, OverlaySource::Dat(bbox, day));
    }

    /// Open a camera site's detail popup and start pulling its newest frame.
    ///
    /// The image rides the placefile-icon texture cache: same fetch, same decode, same upload, and
    /// the key is known before the fetch resolves, so the window can show a placeholder and swap
    /// the picture in when it lands.
    fn open_webcam(&mut self, site: &wxdata::webcams::CamSite, ctx: &egui::Context) {
        let mut body = String::new();
        if !site.icao.is_empty() {
            body.push_str(&format!("{} ({})\n", site.icao, site.ident));
        }
        for c in &site.cameras {
            let state = if c.out_of_order {
                "  (out of order)"
            } else {
                ""
            };
            body.push_str(&format!("{}  {}{}\n", c.name, c.direction, state));
        }
        // Which network this came from, and the credit each one is owed. Windy's terms require a
        // visible "Webcams provided by Windy.com" wherever their cameras appear.
        let from_windy = site.link.is_some();
        body.push_str(if from_windy {
            "\nWebcams provided by Windy.com"
        } else {
            "\nFAA WeatherCams"
        });
        // The first working camera is the one we show; the rest are listed above.
        let cam = site.cameras.iter().find(|c| !c.out_of_order);
        let key = cam.map(|c| format!("cam:{}", c.id));
        self.detail = Some(Detail {
            title: format!("{} webcam", site.name),
            body,
            color: [110, 180, 240, 255],
            image: key.clone(),
            // Not decoration: the link back to the camera's own page is a condition of using
            // Windy's images at all.
            link: site
                .link
                .clone()
                .map(|u| ("View on Windy.com".to_string(), u)),
        });
        let (Some(key), Some(cam)) = (key, cam) else {
            return;
        };
        if self.pf_icon_tex.contains_key(&key) {
            return; // already loaded, or a fetch is already in flight
        }
        self.pf_icon_tex.insert(key.clone(), None);
        let http = self.http.clone();
        let tx = self.pf_icon_tx.clone();
        let ctx2 = ctx.clone();
        // Windy hands back the still's URL inline, so only the FAA needs a second round trip.
        let inline = cam.image_url.clone();
        let cam_id = cam.id;
        self.spawner.spawn(async move {
            let url = match inline {
                Some(u) => Some(u),
                None => match wxdata::webcams::latest_image(&http, cam_id).await {
                    Ok(u) => u,
                    Err(e) => {
                        log::warn!("webcam {cam_id} lookup failed: {e}");
                        None
                    }
                },
            };
            let Some(url) = url else {
                log::info!("camera {cam_id} has no recent image");
                return;
            };
            match fetch_icon_sheet(&http, &url).await {
                Ok(image) => {
                    let _ = tx.send((key, image));
                    ctx2.request_repaint();
                }
                Err(e) => log::warn!("webcam image {url} failed: {e}"),
            }
        });
    }

    /// The weather-radio rows: pick a configured relay, play or stop it, and see honestly whether
    /// it is actually on the air. One stream at a time.
    #[cfg(target_arch = "wasm32")]
    fn nwr_rows(&mut self, ui: &mut egui::Ui) {
        // The player decodes MP3 into a native audio device; the web build says so rather than
        // showing a play button that can't do anything.
        ui.weak("Weather radio playback is desktop and Android only.");
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn nwr_rows(&mut self, ui: &mut egui::Ui) {
        if self.settings.nwr_streams.is_empty() {
            ui.weak("No relays yet — add one in Settings → Alerts.");
            ui.weak("NOAA broadcasts on VHF only; these are listener-run relays.");
            return;
        }
        let playing = self.nwr.as_ref().map(|p| p.name.clone());
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("nwr_pick")
                .selected_text(if self.nwr_pick.is_empty() {
                    "Pick a relay".to_string()
                } else {
                    self.nwr_pick.clone()
                })
                .show_ui(ui, |ui| {
                    for s in &self.settings.nwr_streams {
                        ui.selectable_value(&mut self.nwr_pick, s.name.clone(), &s.name);
                    }
                });
            match &playing {
                Some(_) => {
                    if ui.button("⏹ Stop").clicked() {
                        self.nwr = None; // Drop stops the thread.
                    }
                }
                None => {
                    let stream = self
                        .settings
                        .nwr_streams
                        .iter()
                        .find(|s| s.name == self.nwr_pick)
                        .cloned();
                    if ui
                        .add_enabled(stream.is_some(), egui::Button::new("▶ Play"))
                        .clicked()
                    {
                        if let Some(s) = stream {
                            self.nwr = Some(crate::nwr::Player::start(
                                s.name,
                                s.url,
                                self.settings.alert_volume,
                                self._rt.handle(),
                            ));
                        }
                    }
                }
            }
        });
        if let Some(p) = &self.nwr {
            match p.status() {
                crate::nwr::Status::Playing => ui.weak(format!("🔊 {}", p.name)),
                crate::nwr::Status::Connecting => ui.weak("connecting…"),
                crate::nwr::Status::Offline(why) => ui.colored_label(
                    egui::Color32::from_rgb(230, 150, 90),
                    format!("stream offline ({why}) — retrying"),
                ),
                crate::nwr::Status::Stopped => ui.weak("stopped"),
            };
        }
    }

    /// Open a damage-survey point's detail popup, pulling its survey photo when one was attached.
    /// Shares the placefile-icon texture cache with the webcam popup.
    fn open_damage_point(&mut self, p: &wxdata::dat::DamagePoint, ctx: &egui::Context) {
        let mut body = String::new();
        if !p.damage.is_empty() {
            body.push_str(&format!("{}\n", p.damage));
        }
        if !p.dod.is_empty() {
            body.push_str(&format!("{}\n", p.dod));
        }
        if let Some(w) = p.windspeed {
            body.push_str(&format!("Estimated wind: {w} mph\n"));
        }
        if p.deaths > 0 || p.injuries > 0 {
            body.push_str(&format!("Deaths {} · injuries {}\n", p.deaths, p.injuries));
        }
        if let Some(t) = p.storm {
            body.push_str(&format!("Storm: {}\n", t.format("%Y-%m-%d %H:%M UTC")));
        }
        if let Some(c) = &p.comments {
            body.push_str(&format!("\n{c}\n"));
        }
        body.push_str(&format!("\nNWS {} survey (DAT)", p.office));
        let key = p.image.as_ref().map(|url| format!("dat:{url}"));
        let color = ef_color(&p.efscale).to_array();
        self.detail = Some(Detail {
            title: format!(
                "{} damage",
                if p.efscale.is_empty() {
                    "Surveyed"
                } else {
                    &p.efscale
                }
            ),
            body,
            color,
            image: key.clone(),
            link: None,
        });
        let (Some(key), Some(url)) = (key, p.image.clone()) else {
            return;
        };
        if self.pf_icon_tex.contains_key(&key) {
            return; // loaded, or already in flight
        }
        self.pf_icon_tex.insert(key.clone(), None);
        let http = self.http.clone();
        let tx = self.pf_icon_tx.clone();
        let ctx2 = ctx.clone();
        self.spawner.spawn(async move {
            match fetch_icon_sheet(&http, &url).await {
                Ok(image) => {
                    let _ = tx.send((key, image));
                    ctx2.request_repaint();
                }
                Err(e) => log::warn!("survey photo {url} failed: {e}"),
            }
        });
    }

    /// Drive the river-gauge fetch (NWPS), mirroring [`Self::sync_metar`] but with a slower cadence
    /// (gauge stages update every ~15 min upstream, so 300 s is plenty).
    fn sync_gauges(&mut self, ctx: &egui::Context) {
        if !self.show_gauges {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        if (max_lon - min_lon) > 12.0 {
            return; // too zoomed out — too many gauges to be readable
        }
        let (clon, clat) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        let stale = self
            .gauge_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 300);
        let drifted = self.gauge_bounds.is_none_or(|(la0, lo0, la1, lo1)| {
            let (mlon, mlat) = ((lo0 + lo1) * 0.5, (la0 + la1) * 0.5);
            let (hw, hh) = ((lo1 - lo0) * 0.25, (la1 - la0) * 0.25);
            (clon - mlon).abs() > hw || (clat - mlat).abs() > hh
        });
        if stale || drifted {
            let pad_lon = ((max_lon - min_lon) * 0.2).min(15.0);
            let pad_lat = ((max_lat - min_lat) * 0.2).min(15.0);
            let (lat0, lon0) = (min_lat - pad_lat, min_lon - pad_lon);
            let (lat1, lon1) = (max_lat + pad_lat, max_lon + pad_lon);
            self.gauge_last_fetch = Some(Instant::now());
            self.gauge_bounds = Some((lat0, lon0, lat1, lon1));
            self.spawn_overlay(ctx, OverlaySource::Gauges(lat0, lon0, lat1, lon1));
        }
    }

    /// Drive the HRRR contour fetch: refetch on a kind change or every 15 min. The HRRR surface
    /// run updates hourly and contouring is cheap enough to redo on the model cadence.
    fn sync_contours(&mut self, ctx: &egui::Context) {
        if self.contour_kind == ContourKind::Off {
            if !self.contours.is_empty() {
                self.contours.clear();
                self.contour_valid = None;
            }
            self.contour_fetched_kind = None;
            return;
        }
        let changed = self.contour_fetched_kind != Some((self.contour_kind, self.env_model));
        let stale = self
            .contour_last_fetch
            .is_none_or(|t| t.elapsed().as_secs() >= 900);
        if changed {
            self.contours.clear();
            self.contour_valid = None;
        }
        if changed || stale {
            self.contour_last_fetch = Some(Instant::now());
            self.contour_fetched_kind = Some((self.contour_kind, self.env_model));
            self.spawn_overlay(
                ctx,
                OverlaySource::Contours(self.contour_kind, self.env_model),
            );
        }
    }

    /// Storm-follow camera: re-lock onto the tracked cell in the freshly-applied volume and recenter
    /// the active pane on it. Called from the `Cells` apply arm. Reacquires across SCIT renumbering
    /// by predicting the cell's position from its last motion and adopting the nearest new cell.
    fn update_follow(&mut self) {
        let Some((fsite, last, since)) = self.follow_cell.take() else {
            return;
        };
        // Active site changed out from under the follow (site switch) → stop silently.
        if self.cells_site.as_deref() != Some(fsite.as_str()) {
            return;
        }
        // Same SCIT id in the new volume → the easy case.
        if let Some(c) = self
            .storm_cells
            .iter()
            .find(|c| !c.id.is_empty() && c.id == last.id)
            .cloned()
        {
            self.recenter_follow(&c);
            self.follow_cell = Some((fsite, c, Instant::now()));
            return;
        }
        // Renumber/miss: predict where the cell drifted and adopt the nearest new cell within 15 km.
        let elapsed_h = since.elapsed().as_secs_f64() / 3600.0;
        let pred = match (last.mvt_deg, last.mvt_kt) {
            (Some(dir), Some(kt)) if kt > 0.0 => crate::geo::destination_point(
                [last.lon, last.lat],
                dir as f64,
                kt as f64 * 1.852 * elapsed_h,
            ),
            _ => [last.lon, last.lat],
        };
        if let Some(c) = nearest_cell(&self.storm_cells, pred[0], pred[1], 15.0).cloned() {
            self.recenter_follow(&c);
            self.follow_cell = Some((fsite, c, Instant::now()));
        } else {
            self.follow_notice = Some((format!("Lost {} — follow ended", last.id), Instant::now()));
            // follow_cell already taken → stays None.
        }
    }

    /// Snap the active pane's camera onto a followed cell, keeping the current zoom.
    fn recenter_follow(&mut self, c: &Cell) {
        self.views[self.active].camera.center =
            crate::render::mercator::lonlat_to_world(c.lon, c.lat);
    }

    /// Top-right badge for the storm-follow camera: a tap-to-stop pill while following, or a
    /// transient "follow ended" note for ~5 s after the tracked cell is lost. Same slot on
    /// desktop + Android (just below the top bar).
    fn follow_badge(&mut self, ctx: &egui::Context) {
        if self
            .follow_notice
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed().as_secs() >= 5)
        {
            self.follow_notice = None;
        }
        let following = self.follow_cell.is_some();
        let text = if let Some((_, c, _)) = &self.follow_cell {
            format!("⌖ Following {}  ✕", c.id)
        } else if let Some((msg, _)) = &self.follow_notice {
            msg.clone()
        } else {
            return;
        };
        // Desktop: just under the menu bar. Android: below the top glass bar + chrome-hide EYE
        // button (which sits at inset_top + 66; see app/mobile.rs), so nothing stacks.
        let y = if cfg!(target_os = "android") {
            let inset_top = (ctx.content_rect().top() - ctx.viewport_rect().top()).max(0.0);
            inset_top + 116.0
        } else {
            // Below the whole control column, in the badge lane.
            crate::ui::style::lane_right_badge_y(CONTROL_BUTTONS)
        };
        egui::Area::new("follow_badge".into())
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(crate::ui::style::LANE_RIGHT_BADGE_X, y),
            )
            // Only the following state carries a button; the notice is a read-only badge and must
            // not occlude the map's pinch test.
            .interactable(following)
            .show(ctx, |ui| {
                let fill = if following {
                    egui::Color32::from_rgba_unmultiplied(40, 90, 150, 220)
                } else {
                    egui::Color32::from_black_alpha(160)
                };
                egui::Frame::new()
                    .fill(fill)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        if following {
                            let btn = egui::Button::new(
                                egui::RichText::new(&text).color(egui::Color32::WHITE),
                            )
                            .frame(false);
                            if ui
                                .add(btn)
                                .on_hover_text("Stop following this storm")
                                .clicked()
                            {
                                self.follow_cell = None;
                            }
                        } else {
                            ui.colored_label(egui::Color32::from_white_alpha(210), &text);
                        }
                    });
            });
        // Keep the notice's expiry ticking without input.
        if self.follow_notice.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Reassemble the displayed overlay set from the fetched sources and current filters.
    fn rebuild_overlays(&mut self) {
        let mut v = Vec::new();
        if (1..=8).contains(&self.filters.outlook_day) {
            v.extend(
                self.outlook_features[(self.filters.outlook_day - 1) as usize]
                    .iter()
                    .cloned(),
            );
        }
        if self.filters.show_mds {
            v.extend(self.md_features.iter().cloned());
        }
        if self.filters.show_watches {
            v.extend(self.watch_features.iter().cloned());
        }
        if (1..=3).contains(&self.filters.wssi_day) {
            v.extend(self.wssi_features.iter().cloned());
        }
        if (1..=3).contains(&self.filters.ero_day) {
            v.extend(self.ero_features.iter().cloned());
        }
        if self.show_outages {
            v.extend(self.outage_features.iter().cloned());
        }
        if self.filters.show_alerts {
            for f in self.active_alert_features() {
                if self.filters.alert_cats[alerts::category(&f.title).index()] {
                    v.push(f.clone());
                }
            }
        }
        if self.show_probsevere {
            v.extend(self.probsevere.iter().cloned());
        }
        if self.show_tropical {
            if let Some(t) = &self.tropical {
                // Surge and wind field go under the cones: the cone is the headline, these are
                // the context it sits on.
                v.extend(t.surge.iter().cloned());
                v.extend(t.wind_radii.iter().cloned());
                v.extend(t.cones.iter().cloned());
            }
        }
        if self.show_aviation {
            v.extend(self.aviation_features.iter().cloned());
        }
        if self.show_tfr {
            // Shapes that failed to parse are kept as empty placeholders so they are not
            // refetched forever; they have nothing to draw.
            v.extend(
                self.tfr_features
                    .values()
                    .filter(|f| !f.rings.is_empty())
                    .cloned(),
            );
        }
        if self.show_fires {
            v.extend(self.fire_perims.iter().cloned());
        }
        self.overlays = v;
        self.overlay_gen = self.overlay_gen.wrapping_add(1);
    }

    /// Approximate map view range in nautical miles (viewport height), for placefile thresholds.
    /// `// ponytail: coarse mercator estimate; fine for zoom-gating, not for measuring.`
    fn view_range_nmi(&self) -> f32 {
        let cam = &self.views[self.active].camera;
        let world_h = self.last_viewport.1 as f64 * cam.world_per_pixel();
        let s = (cam.center.1 * 2.0 - 1.0) * std::f64::consts::PI;
        let coslat = (1.0 / s.cosh()).max(0.05); // cos(lat) = sech(mercator y)
        (world_h * 40075.017 * coslat / 1.852) as f32
    }

    /// Placefile items currently visible (enabled, zoom threshold met, within time range), as
    /// `(item, opacity, loaded-placefile index)`. Iterated in `settings.placefiles` order, which
    /// is the paint order the Layer Manager reorders.
    fn visible_placefile_items(&self) -> Vec<(&wxdata::placefile::PlaceItem, f32, usize)> {
        crate::prof_scope!("visible_placefile_items");
        self.visible_placefile_iter().collect()
    }

    /// The same set, lazily — so a caller that only asks whether there *are* any does not build
    /// the whole list to find out.
    fn visible_placefile_iter(
        &self,
    ) -> impl Iterator<Item = (&wxdata::placefile::PlaceItem, f32, usize)> {
        let range = self.view_range_nmi();
        let now = Utc::now();
        // Configured placefiles in Layer-Manager order, then plugin output on top of them: a
        // plugin is something the user wrote for this session, so it should not be buried.
        let sources = self
            .settings
            .placefiles
            .iter()
            .map(|c| (std::borrow::Cow::Borrowed(c.url.as_str()), c.opacity))
            .chain(self.settings.plugins.iter().map(|p| {
                (
                    std::borrow::Cow::Owned(format!("plugin:{}", p.name)),
                    1.0f32,
                )
            }));
        sources.flat_map(move |(url, opacity)| {
            self.placefiles
                .iter()
                .enumerate()
                .find(|(_, lp)| lp.url == url)
                .filter(|(_, lp)| lp.enabled)
                .into_iter()
                .flat_map(move |(li, lp)| {
                    lp.pf
                        .items
                        .iter()
                        .filter(move |it| !(it.threshold_nmi > 0.0 && range > it.threshold_nmi))
                        .filter(move |it| it.time.is_none_or(|(a, b)| now >= a && now <= b))
                        .map(move |it| (it, opacity, li))
                })
        })
    }

    /// [`Self::placefile_labels`] memoised for the frame's inputs — it deep-clones every item's
    /// strings, and ran once per frame over every enabled placefile.
    fn placefile_labels_cached(&mut self) -> std::sync::Arc<[PlaceLabel]> {
        // Time is in the inputs because items have on/off windows and thresholds; a minute's
        // granularity is finer than any placefile's own cadence.
        let fingerprint: usize = self
            .placefiles
            .iter()
            .map(|p| p.pf.items.len() + usize::from(p.enabled))
            .sum::<usize>()
            + self.pf_icon_tex.len();
        let key = (
            fingerprint,
            chrono::Utc::now().timestamp() / 60,
            self.view_range_nmi() as i32,
        );
        if self
            .placefile_label_cache
            .as_ref()
            .is_none_or(|(k, _)| *k != key)
        {
            let labels: std::sync::Arc<[PlaceLabel]> = self.placefile_labels().into();
            self.placefile_label_cache = Some((key, labels));
        }
        self.placefile_label_cache
            .as_ref()
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| Vec::new().into())
    }

    /// Owned labels/markers for the visible placefile items (drawn by the egui painter).
    /// Icons resolve their sheet cell here, so the painter just blits a quad.
    fn placefile_labels(&self) -> Vec<PlaceLabel> {
        crate::prof_scope!("placefile_labels");
        use wxdata::placefile::PlaceKind;
        self.visible_placefile_items()
            .iter()
            .filter_map(|(it, opacity, li)| {
                let fade = |c: [u8; 4]| rgba32(c).gamma_multiply(*opacity);
                Some(match &it.kind {
                    PlaceKind::Text {
                        color,
                        pos,
                        text,
                        hover,
                    } => PlaceLabel {
                        color: fade(*color),
                        pos: *pos,
                        anchor: it.anchor,
                        hover: hover.clone(),
                        kind: PlaceLabelKind::Text(text.clone()),
                    },
                    PlaceKind::Icon {
                        color,
                        pos,
                        angle,
                        sheet,
                        hover,
                    } => PlaceLabel {
                        color: fade(*color),
                        pos: *pos,
                        anchor: it.anchor,
                        hover: hover.clone(),
                        kind: self
                            .sprite_for(*li, *sheet, *angle)
                            .unwrap_or(PlaceLabelKind::Marker),
                    },
                    _ => return None,
                })
            })
            .collect()
    }

    /// Resolve an icon's `(file, index)` against the placefile's sheets and the loaded textures.
    /// `None` whenever anything is missing — the caller falls back to a plain marker.
    fn sprite_for(
        &self,
        li: usize,
        sheet: Option<(u32, u32)>,
        angle: f32,
    ) -> Option<PlaceLabelKind> {
        let (file, index) = sheet?;
        let sh = self.placefiles.get(li)?.pf.icon_files.get(&file)?;
        let tex = self.pf_icon_tex.get(&sh.url)?.as_ref()?;
        let [tw, th] = tex.size();
        let (cols, rows) = (
            (tw as u32 / sh.icon_w).max(1),
            (th as u32 / sh.icon_h).max(1),
        );
        // Icon numbering is 1-based, left to right then top to bottom.
        let i = index.saturating_sub(1);
        if i >= cols * rows {
            return None;
        }
        let (cx, cy) = (i % cols, i / cols);
        let (u0, v0) = (cx * sh.icon_w, cy * sh.icon_h);
        let uv = egui::Rect::from_min_max(
            egui::pos2(u0 as f32 / tw as f32, v0 as f32 / th as f32),
            egui::pos2(
                (u0 + sh.icon_w) as f32 / tw as f32,
                (v0 + sh.icon_h) as f32 / th as f32,
            ),
        );
        Some(PlaceLabelKind::Sprite {
            tex: tex.id(),
            uv,
            size: egui::vec2(sh.icon_w as f32, sh.icon_h as f32),
            hot: egui::vec2(sh.hot_x as f32, sh.hot_y as f32),
            angle,
        })
    }

    /// Fetch + decode any icon sheet a loaded placefile references but we don't have yet, and
    /// upload arrivals. `// ponytail: no disk cache — sheets are a few KB and refetch on launch.`
    fn sync_pf_icons(&mut self, ctx: &egui::Context) {
        while let Ok((url, image)) = self.pf_icon_rx.try_recv() {
            let tex =
                ctx.load_texture(format!("pficon:{url}"), image, egui::TextureOptions::LINEAR);
            self.pf_icon_tex.insert(url, Some(tex));
        }
        let wanted: Vec<String> = self
            .placefiles
            .iter()
            .filter(|lp| lp.enabled)
            .flat_map(|lp| lp.pf.icon_files.values().map(|s| s.url.clone()))
            .filter(|u| !self.pf_icon_tex.contains_key(u))
            .collect();
        for url in wanted {
            // Insert the negative entry first: it doubles as the in-flight guard.
            self.pf_icon_tex.insert(url.clone(), None);
            let http = self.http.clone();
            let tx = self.pf_icon_tx.clone();
            let ctx2 = ctx.clone();
            self.spawner.spawn(async move {
                match fetch_icon_sheet(&http, &url).await {
                    Ok(image) => {
                        let _ = tx.send((url, image));
                        ctx2.request_repaint();
                    }
                    Err(e) => note_feed_error("Placefile icons", format!("{url}: {e}")),
                }
            });
        }
    }

    /// Re-tessellate the overlay when its set or the zoom bucket changed.
    fn sync_overlay(&mut self) {
        crate::prof_scope!("sync_overlay");
        // Asked lazily: on a frame with nothing to rebuild — which is most of them — this is the
        // only question, and building the whole visible list a second time to answer it was the
        // second-largest per-frame allocation in the pane.
        if self.overlays.is_empty() && self.visible_placefile_iter().next().is_none() {
            self.overlay_ready = false;
            return;
        }
        let zoom = self.views[self.active].camera.zoom;
        let bucket = (zoom * 2.0).round() as i32;
        // A pinch crosses several half-zoom buckets, and each crossing used to re-run lyon over
        // every overlay ring on the UI thread, mid-gesture. Deferred while a finger is down: the
        // frame after the release rebuilds to whatever bucket the gesture landed on, so the
        // resting state is the same one and the intermediate tessellations were never seen for
        // more than a frame anyway.
        //
        // A geometry change (`overlay_gen`) is not deferred — that is new data arriving, not the
        // camera moving, and it should appear when it lands.
        let theme_changed = self.settings.theme != self.built_theme;
        if should_retess(
            self.gesture_live,
            self.overlay_gen != self.built_gen || theme_changed,
            bucket != self.built_zoom_bucket || theme_changed,
        ) {
            let mut geom =
                overlay_build::build_with_theme(&self.overlays, zoom, self.settings.theme);
            let pf: Vec<(&wxdata::placefile::PlaceItem, f32)> = self
                .visible_placefile_iter()
                .map(|(it, op, _)| (it, op))
                .collect();
            overlay_build::append_placefiles_with_theme(&mut geom, &pf, zoom, self.settings.theme);
            self.overlay_ready = !geom.indices.is_empty();
            self.pending_overlay = Some(OverlayUpload {
                vertices: geom.vertices,
                indices: geom.indices,
            });
            self.built_gen = self.overlay_gen;
            self.built_zoom_bucket = bucket;
            self.built_theme = self.settings.theme;
        }
    }

    /// Spawn a background fetch of the latest volume for `site`, routed back to `view_idx`.
    /// `current_name = None` forces a re-download even if the newest volume is unchanged.
    fn spawn_fetch(
        &self,
        view_idx: usize,
        site: String,
        current_name: Option<String>,
        ctx: egui::Context,
    ) {
        let tx = self.msg_tx.clone();
        let network = wxdata::sites::network(&site);
        if network != wxdata::sites::Network::Nexrad {
            // None of these has a Level 2 feed or an archive: one volume per poll, synthesized
            // from the newest Level 3 tilt products (TDWR) or assembled from the newest ODIM
            // files (DWD, OPERA).
            let http = self.http.clone();
            self.spawner.spawn(async move {
                use wxdata::sites::Network;
                // Each of these asks its feed what the newest volume is *before* downloading it,
                // and answers `None` when that is what we are already showing. The NEXRAD path
                // below has always worked this way — it lists, then compares, then downloads;
                // these three used to download the whole volume and compare afterwards, which on
                // DWD meant ~50 requests and ~2 MB discarded on nine polls out of ten.
                let cur = current_name.as_deref();
                let fetched = match network {
                    Network::Dwd => wxdata::dwd::fetch_volume(&http, &site, cur).await,
                    Network::Opera => wxdata::opera::fetch_volume(&http, &site, cur).await,
                    Network::Tdwr | Network::Nexrad => {
                        wxdata::tdwr::fetch_volume(&http, &site, cur).await
                    }
                };
                let msg = match fetched {
                    Ok(None) => DataMsg::UpToDate {
                        view: view_idx,
                        site,
                    },
                    // A feed that hands back the name we already hold anyway — a probe that could
                    // not decode, say — is still up to date.
                    Ok(Some((name, _, _))) if current_name.as_deref() == Some(name.as_str()) => {
                        DataMsg::UpToDate {
                            view: view_idx,
                            site,
                        }
                    }
                    Ok(Some((name, time, scan))) => DataMsg::Volume {
                        view: view_idx,
                        site,
                        name,
                        time,
                        scan,
                    },
                    Err(e) => DataMsg::Error {
                        view: view_idx,
                        site,
                        err: e.to_string(),
                    },
                };
                let _ = tx.send(msg);
                ctx.request_repaint();
            });
            return;
        }
        self.spawner.spawn(async move {
            // Ask for two: the newest volume is usually still uploading, and one caught before
            // its metadata record lands can't be decoded at all. Falling back one volume shows
            // ~5-minute-old data instead of nothing.
            let msg = match level2::latest_identifiers(&site, 2).await.map(|mut v| {
                let first = v.remove(0);
                (first, v.pop())
            }) {
                Ok((id, prev)) => {
                    let name = id.name().to_string();
                    if current_name.as_deref() == Some(name.as_str()) {
                        DataMsg::UpToDate {
                            view: view_idx,
                            site,
                        }
                    } else {
                        let time = id.date_time().unwrap_or_else(Utc::now);
                        // No cache at the live head: the newest object can still be uploading, and a
                        // half-written volume is not something to keep.
                        let fetched = match crate::volume::fetch(id, None).await {
                            Ok(scan) => Ok((name.clone(), time, scan)),
                            Err(e) => match prev {
                                // Only worth retrying when there IS an older volume and we're not
                                // already showing it.
                                Some(p) if current_name.as_deref() != Some(p.name()) => {
                                    let pname = p.name().to_string();
                                    let ptime = p.date_time().unwrap_or_else(Utc::now);
                                    log::debug!(
                                        "newest volume unusable ({e}); falling back to {pname}"
                                    );
                                    crate::volume::fetch(p, None)
                                        .await
                                        .map(|scan| (pname, ptime, scan))
                                        .map_err(|_| e)
                                }
                                _ => Err(e),
                            },
                        };
                        match fetched {
                            Ok((name, time, scan)) => DataMsg::Volume {
                                view: view_idx,
                                site,
                                name,
                                time,
                                scan,
                            },
                            Err(e) => DataMsg::Error {
                                view: view_idx,
                                site,
                                err: e.to_string(),
                            },
                        }
                    }
                }
                Err(e) => DataMsg::Error {
                    view: view_idx,
                    site,
                    err: e.to_string(),
                },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn poll_messages(&mut self) {
        self.drain_feed_errors();
        while let Ok(msg) = self.msg_rx.try_recv() {
            let idx = msg.view();
            // LiveEnded must be handled even after a site change (to drop the stream handle).
            if matches!(msg, DataMsg::LiveEnded { .. }) {
                if let DataMsg::LiveEnded { view, gen, .. } = msg {
                    if self
                        .live_stream
                        .as_ref()
                        .is_some_and(|(v, _, g)| *v == view && *g == gen)
                    {
                        self.live_stream = None; // interval polling resumes automatically
                    }
                }
                continue;
            }
            if idx >= self.views.len() || self.views[idx].site.as_deref() != Some(msg.site()) {
                continue; // view gone or its site changed since the fetch spawned
            }
            match msg {
                DataMsg::Volume {
                    view,
                    name,
                    time,
                    scan,
                    ..
                } => {
                    let scan = Arc::new(scan);
                    self.scan_cache.put(name.clone(), Arc::clone(&scan));
                    let v = &mut self.views[view];
                    let looping = v.timeline.live_looping();
                    // A newly-arrived live head (following): roll the day at UTC midnight, or grow
                    // the frame list so the loop window slides forward. A frame-fetch result for a
                    // scrubbed/loop-display frame is older than the head and isn't a new head.
                    let new_head = v.timeline.following
                        && v.site.as_deref().is_some_and(wxdata::sites::is_nexrad)
                        && {
                            let last_time = v.timeline.frames.last().and_then(|id| id.date_time());
                            if time.date_naive() != v.timeline.date {
                                v.timeline.date = time.date_naive(); // re-list fires via frames_key
                                true
                            } else if last_time.is_none_or(|t| time > t)
                                && v.timeline.frames.last().map(|id| id.name())
                                    != Some(name.as_str())
                            {
                                v.timeline.append_head(Identifier::new(name.clone()));
                                true
                            } else {
                                false
                            }
                        };
                    // While looping, the playhead frame owns the display; a genuinely new head is
                    // only appended, not shown. Every other case updates the displayed volume.
                    if !(looping && new_head) {
                        v.show_volume(scan, name, time);
                    }
                    v.loading = false;
                    v.error = None;
                    v.clamp_tilt();
                    v.clamp_moment();
                    self.pane_shown.remove(&view);
                    self.scan_chime(view, time);
                }
                DataMsg::Frames {
                    view,
                    site,
                    date,
                    frames,
                } => {
                    let v = &mut self.views[view];
                    v.timeline.listing = false;
                    if v.timeline.date == date && v.site.as_deref() == Some(site.as_str()) {
                        v.timeline.set_frames(frames, (site, date));
                        self.pane_shown.remove(&view);
                    }
                }
                DataMsg::Live {
                    view,
                    name,
                    time,
                    scan,
                    changed,
                    ..
                } => {
                    let v = &mut self.views[view];
                    if v.timeline.playing {
                        continue; // looping pane owns its displayed frame (cf. Volume above)
                    }
                    match &mut v.volume {
                        Some(vol) => vol.apply_live(scan, name, time, &changed),
                        None => v.volume = Some(Volume::new(scan, name, time)),
                    }
                    v.loading = false;
                    v.error = None;
                    v.clamp_tilt();
                    v.clamp_moment();
                    // A healthy stream pushes the poll deadline forward — this line IS the
                    // fallback: if the stream dies, interval polling resumes on schedule.
                    v.last_poll = Some(Instant::now());
                    self.pane_shown.remove(&view);
                    self.scan_chime(view, time);
                }
                DataMsg::UpToDate { view, .. } => self.views[view].loading = false,
                DataMsg::Prefetched { name, scan, .. } => {
                    self.scan_cache.put(name, Arc::new(scan));
                }
                DataMsg::Error { view, err, .. } => {
                    let v = &mut self.views[view];
                    v.loading = false;
                    // The newest archive volume is published while the radar is still writing it,
                    // so the head can briefly lack its VCP message. That is a "not finished yet",
                    // not a failure: the next poll gets a complete file a minute later. Showing a
                    // red chip for it left the map looking broken while nothing was wrong.
                    if err.contains("missing coverage pattern") {
                        log::debug!("head volume not complete yet: {err}");
                    } else {
                        v.error = Some(err);
                    }
                }
                DataMsg::LiveEnded { .. } => unreachable!("handled above"),
            }
        }
    }

    /// Start/stop the live chunk stream for the active view. One stream at a time (the active
    /// view); a healthy stream starves interval polling, a dead one lets polling take over.
    /// Runs on the web too: the chunk objects are on a bucket that allows cross-origin reads, and
    /// the streamer's waits and backfill were already written without tokio.
    fn manage_stream(&mut self, ctx: &egui::Context) {
        let idx = self.active;
        let (want, site, base) = {
            let v = &self.views[idx];
            // Stream only while pinned to the live head; scrubbing pauses it. A live loop is
            // suppressed too — the loop shows past frames, so interval polling (not the sweep
            // stream) carries new-volume arrival. ponytail: stream resumes on pause / go_head.
            // Only WSR-88Ds have a Level 2 chunk stream to merge — asking for one downloads a
            // WSR-88D-shaped file that isn't there and decodes garbage.
            let want = v.timeline.following
                && !v.timeline.playing
                && v.site.as_deref().is_some_and(wxdata::sites::is_nexrad)
                && v.volume.is_some();
            (
                want,
                v.site.clone(),
                // Shared with the pane: the streamer merges into a new Scan rather than mutating
                // this one, so it only needs a refcount, not the tens of MB a deep copy cost on
                // the UI thread at every stream start.
                v.volume.as_ref().map(|vol| Arc::clone(&vol.scan)),
            )
        };

        // Abort an existing stream if it no longer matches the active view/site or isn't wanted.
        if let Some((sv, ss, _)) = &self.live_stream {
            if !want || *sv != idx || Some(ss.as_str()) != site.as_deref() {
                // ponytail: the cancelled stream notices within a second (its wait is sliced),
                // so a fast site switch overlaps two streams for about that long and at most
                // one in-flight chunk fetch. An abort channel if even that shows up.
                self.live_gen
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.live_stream = None;
                // A new site shouldn't inherit the old one's 60 s retry gate.
                self.last_stream_attempt = None;
            }
        }

        if want && self.live_stream.is_none() {
            let due = self
                .last_stream_attempt
                .is_none_or(|t| t.elapsed().as_secs() >= 60);
            // `want` already implies both, but the two are computed a screen away from here.
            if let (true, Some(site), Some(base)) = (due, site, base) {
                self.last_stream_attempt = Some(Instant::now());
                let gen = self.live_gen.load(std::sync::atomic::Ordering::Relaxed);
                self.spawn_stream(idx, site.clone(), base, ctx.clone(), gen);
                self.live_stream = Some((idx, site, gen));
            }
        }
    }

    /// Spawn the live chunk streamer for `site`, routing merged volumes back to `view_idx`.
    ///
    /// `gen` is the generation this stream belongs to; it ends itself once the app has moved on.
    fn spawn_stream(
        &self,
        view_idx: usize,
        site: String,
        base: Arc<Scan>,
        ctx: egui::Context,
        gen: u64,
    ) {
        let tx = self.msg_tx.clone();
        let live_gen = Arc::clone(&self.live_gen);
        let active = move || {
            live_gen.load(std::sync::atomic::Ordering::Relaxed) == gen
                && crate::platform::activity::is_active()
        };
        self.spawner.spawn(async move {
            let end_site = site.clone();
            let cb_tx = tx.clone();
            let cb_ctx = ctx.clone();
            let cb_site = site.clone();
            log::info!("live stream started for {end_site}");
            let res = live::stream(site, base, active, move |u| {
                let _ = cb_tx.send(DataMsg::Live {
                    view: view_idx,
                    site: cb_site.clone(),
                    name: u.name,
                    time: u.time,
                    scan: u.scan,
                    changed: u.changed,
                });
                cb_ctx.request_repaint();
            })
            .await;
            if let Err(e) = &res {
                log::warn!("live stream for {end_site} ended: {e}");
            }
            let _ = tx.send(DataMsg::LiveEnded {
                view: view_idx,
                site: end_site,
                gen,
            });
            ctx.request_repaint();
        });
    }

    fn apply_action(&mut self, action: BindableAction, ctx: &egui::Context) {
        use BindableAction as A;
        match action {
            // Everything the registry already knows how to do runs through the one executor.
            A::Palette(p) => self.apply_palette(p, ctx),
            A::TiltUp => {
                let v = &mut self.views[self.active];
                if let Some(vol) = &v.volume {
                    if v.tilt + 1 < vol.elevations.len() {
                        v.tilt += 1;
                    }
                }
            }
            A::TiltDown => {
                let v = &mut self.views[self.active];
                v.tilt = v.tilt.saturating_sub(1);
            }
            A::OpenSiteDialog => {
                if self.site_dialog.is_none() {
                    self.site_dialog = Some(Default::default());
                }
            }
            A::ToggleAlertPanel => {
                // The bell tab and the panel are one surface now: the key opens the panel on
                // Alerts, and closes it if that's already what's showing.
                let showing = self.panel_open && self.show_alert_panel;
                self.panel_open = !showing;
                self.show_alert_panel = true;
            }
            A::ToggleObs => {
                self.obs_mode = !self.obs_mode;
                if !self.obs_mode {
                    self.obs_tour = false;
                }
            }
            A::ToggleObsTour => {
                self.obs_tour = !self.obs_tour;
                self.obs_tour_last = None; // step immediately on enable
                if self.obs_tour {
                    self.obs_mode = true;
                }
            }
            A::ToggleDrawer => {
                // Hidden: bring it back and land in the search box. Visible: focus the search,
                // which is what the key always did.
                self.panel_open = true;
                self.show_alert_panel = false;
                self.sidebar_focus_search = true;
            }
            A::StepBack => self.views[self.active].timeline.step(-1),
            A::StepForward => self.views[self.active].timeline.step(1),
            A::Fullscreen => {
                // Desktop only; mobile is already fullscreen.
                if !cfg!(target_os = "android") {
                    let cur = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!cur));
                }
            }
            A::CommandSearch => {
                self.layers_query.clear();
                self.panel_open = true;
                self.show_alert_panel = false;
                self.sidebar_focus_search = true;
            }
            A::CheatSheet => self.show_cheatsheet = !self.show_cheatsheet,
            A::ToggleMute => {
                self.settings.mute_alerts = !self.settings.mute_alerts;
                let msg = if self.settings.mute_alerts {
                    "Audio alerts muted"
                } else {
                    "Audio alerts unmuted"
                };
                self.toast(ToastKind::Info, msg);
            }
        }
    }

    /// Streamer/OBS auto-tour: every ~12 s, fly the active camera to the next active-warning
    /// centroid (highest-severity first), cycling. No-op with no warnings in the feed.
    fn drive_obs_tour(&mut self) {
        if !self.obs_tour {
            return;
        }
        if self
            .obs_tour_last
            .is_some_and(|t| t.elapsed().as_secs() < 12)
        {
            return;
        }
        // Centroids of active warning polygons, tornado/severe first.
        let mut targets: Vec<(u8, f64, f64)> = self
            .alert_features
            .iter()
            .filter(|f| f.kind == overlay::FeatureKind::Warning)
            .filter_map(|f| {
                let (w, s, e, n) = f.bbox()?;
                let sev = if f.title.to_lowercase().contains("tornado") {
                    0
                } else {
                    1
                };
                Some((sev, (w + e) / 2.0, (s + n) / 2.0))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        targets.sort_by_key(|t| t.0);
        self.obs_tour_last = Some(Instant::now());
        self.obs_tour_idx = (self.obs_tour_idx + 1) % targets.len();
        let (_, lon, lat) = targets[self.obs_tour_idx];
        let cam = &mut self.views[self.active].camera;
        cam.center = crate::render::mercator::lonlat_to_world(lon, lat);
        cam.zoom = cam.zoom.max(8.5);
    }

    /// Force-refresh the active view: re-list the day's volumes, and refetch the head volume
    /// when following live.
    fn trigger_reload(&mut self, ctx: &egui::Context) {
        let idx = self.active;
        self.views[idx].timeline.frames_key = None; // force a fresh listing
        let site = self.views[idx].site.clone();
        if self.views[idx].timeline.following {
            if let Some(s) = site {
                self.views[idx].loading = true;
                self.views[idx].last_poll = Some(Instant::now());
                self.spawn_fetch(idx, s, None, ctx.clone());
            }
        }
    }

    /// The MRMS product the national field loop fetches for `layer`, or `None` for a layer that
    /// has a fetch block of its own (HRRR forecast, the environment suite, the global models,
    /// per-site Level 3 grids).
    ///
    /// One exhaustive match rather than a skip list and a second match that had to agree with it:
    /// they drifted, the global model fields were missing from the skip list, and switching one on
    /// walked into an `unreachable!()` and took the app down. A layer added to `FieldLayer` now
    /// fails to compile here instead of panicking at runtime.
    fn mrms_product(&self, layer: crate::render::FieldLayer) -> Option<String> {
        use crate::render::FieldLayer as FL;
        Some(match layer {
            FL::Mrms => wxdata::mrms::REFLECTIVITY.to_string(),
            FL::Lightning => {
                wxdata::mrms::lightning_density(self.settings.lightning_minutes).to_string()
            }
            FL::Mesh => wxdata::mrms::MESH.to_string(),
            FL::AzShear => wxdata::mrms::AZSHEAR.to_string(),
            FL::Rotation => wxdata::mrms::rotation_track(self.rotation_minutes).to_string(),
            FL::PrecipRate => wxdata::mrms::PRECIP_RATE.to_string(),
            FL::Qpe1h => wxdata::mrms::QPE_01H.to_string(),
            FL::Qpe24h => wxdata::mrms::QPE_24H.to_string(),
            FL::PrecipType => wxdata::mrms::PRECIP_TYPE.to_string(),
            FL::FlashFlood => wxdata::mrms::FLASH_ARI30.to_string(),
            FL::HailSwath => wxdata::mrms::hail_swath(self.hail_minutes).to_string(),
            FL::Hrrr
            | FL::Cape
            | FL::Srh
            | FL::Vil
            | FL::EchoTops
            | FL::Hca
            | FL::UpdraftHelicity
            | FL::Smoke
            | FL::Mosaic
            | FL::CompositeLocal
            | FL::VilLocal
            | FL::VilDensity
            | FL::EtopLocal
            | FL::HailMehs
            | FL::HailPosh
            | FL::Snowfall
            | FL::SnowAnalysis
            | FL::GlobalMslp
            | FL::GlobalHeight500
            | FL::GlobalTemp2m
            | FL::GlobalDewpoint2m
            | FL::GlobalWind10m
            | FL::GlobalPrecip
            | FL::ModelDiff
            | FL::GlmFed
            // Built from two grids at once, so it has a fetch block of its own.
            | FL::SnowBands
            // Model layers, fetched on the forecast-hour scrub rather than a product path.
            | FL::ThunderProb => return None,
        })
    }

    /// Per-frame per-pane: react to site changes, keep the timeline current, and (for the active
    /// pane) manage the live stream. Each pane fetches its own volume via its view index.
    fn sync_pane(&mut self, idx: usize, ctx: &egui::Context) {
        // Site change: clear the old volume, recenter, and (if a real site) refetch.
        let site_changed = self.views[idx].site != self.views[idx].loaded_site;
        if site_changed {
            let v = &mut self.views[idx];
            v.loaded_site = v.site.clone();
            v.volume = None;
            v.forget_recent();
            v.moments_seen = [false; Moment::ALL.len()];
            v.error = None;
            // Clear a stuck in-flight flag: if the previous site's fetch is still running when the
            // site changes, its result is dropped on arrival (site mismatch) without clearing
            // `loading`, which would then block the new site's fetch forever ("no volume").
            v.loading = false;
            // The old site's frame list is not this site's. Left in place it is both what the
            // scrub path displays and what it downloads until the new listing lands — i.e. the
            // wrong radar's volumes at an index that means nothing here. A scrubbed pane keeps
            // the time it was looking at; a live one just goes back to the head.
            // ...unless a deep link, an event or a replay bundle already asked for an instant:
            // that target is the whole point of the jump, and overwriting it with the frame the
            // pane happened to be showing sent every Event Library entry to the wrong day.
            if !v.timeline.following && v.timeline.seek_target.is_none() {
                v.timeline.seek_target = v.timeline.current().and_then(|id| id.date_time());
            }
            v.timeline.frames.clear();
            v.timeline.playhead = 0;
            match &v.site {
                // ...unless a deep link already aimed the camera at something specific.
                Some(_) if std::mem::take(&mut v.camera_placed) => {}
                Some(s) => ui::site_dialog::center_on_site(&mut v.camera, s),
                None => {
                    self.pane_shown.remove(&idx);
                }
            }
            // Storm cells follow the active pane's site: drop the old ones (and any open old-site
            // storm popup / trend history — the ring-click path in try_pick_site does the same) and
            // refetch.
            if idx == self.active {
                self.storm_cells.clear();
                self.cells_site = None;
                self.cell_trends.clear();
                self.cell_popup = None;
                if let Some(site) = self.views[idx].site.clone() {
                    let http = self.http.clone();
                    let tx = self.overlay_tx.clone();
                    let ctx2 = ctx.clone();
                    self.spawner.spawn(async move {
                        let cells = level3::fetch_cells(&http, &site).await;
                        let _ = tx.send(OverlayMsg::Cells(site, cells));
                        ctx2.request_repaint();
                    });
                }
            }
        }

        // Advance playback (if playing) then reconcile the displayed volume with the timeline.
        self.views[idx].timeline.live_window = if cfg!(target_os = "android") {
            self.settings.live_loop_frames.clamp(1, ANDROID_LOOP_WINDOW)
        } else if cfg!(target_arch = "wasm32") {
            self.settings.live_loop_frames.clamp(1, WEB_LOOP_WINDOW)
        } else {
            self.settings.live_loop_frames.max(1)
        };

        // The browser demo opens playing. A single frozen frame is indistinguishable from a broken
        // map to someone who has never seen this app, and the last fifteen minutes is what makes
        // radar readable — you cannot tell which way a storm is moving from a still.
        //
        // Waits for two frames rather than starting on one, so the first thing the visitor sees
        // move is an actual loop and not a one-frame stutter. `backfill_loop_frames` is what
        // fetches the tail; once playing, ordinary prefetch takes over.
        if self.autoplay_pending {
            let tl = &self.views[idx].timeline;
            if tl.following && !tl.playing {
                let window = tl.live_window.max(1);
                let start = tl.frames.len().saturating_sub(window);
                let ready = tl.frames[start..]
                    .iter()
                    .filter(|id| self.scan_cache.contains(&id.name().to_string()))
                    .count();
                if ready >= 2 {
                    log::info!("loop: playing {ready}/{window} frames");
                    self.views[idx].timeline.toggle_play();
                    self.autoplay_pending = false;
                } else {
                    self.backfill_loop_frames(idx, ctx);
                }
            }
        }
        // Hold the playhead while the next frame is still downloading. Advancing on the wall clock
        // regardless meant playback skipped frames it hadn't got yet and the loop read as juddery;
        // waiting reads as buffering, which is what it is. Only while there's a fetch to wait for,
        // so a permanently-failed frame can't stall the loop.
        let next_pending = {
            let tl = &self.views[idx].timeline;
            tl.playing
                && tl
                    .frames
                    .get(tl.playhead + 1)
                    .map(|id| id.name().to_string())
                    .is_some_and(|n| {
                        !self.scan_cache.contains(&n)
                            && book(&self.prefetching).get(&n).is_some_and(|at| {
                                // Bounded: a fetch that never answers must not park the loop.
                                at.elapsed() < std::time::Duration::from_secs(8)
                            })
                    })
        };
        if !next_pending {
            self.views[idx].timeline.tick();
        }
        // Playback paces itself rather than riding whatever the idle heartbeat happens to give
        // it: ask for a repaint exactly when the next frame is due.
        if let Some(dt) = self.views[idx].timeline.time_to_next_frame() {
            ctx.request_repaint_after(dt);
        }
        self.sync_timeline(idx, ctx, site_changed);

        // Live streaming is limited to the active pane; others poll their head.
        if idx == self.active {
            self.manage_stream(ctx);
        }
    }

    /// Reconcile the frame listing and the displayed volume with the timeline: keep the
    /// listing current, poll the live head while following, or load the scrubbed frame.
    fn sync_timeline(&mut self, idx: usize, ctx: &egui::Context, site_changed: bool) {
        if !crate::platform::activity::is_active() {
            return; // backgrounded: no listings, no head polls, no downloads
        }
        // (Re)list volumes when the site or selected date changed.
        let (site, date, following, need_list, listing) = {
            let v = &self.views[idx];
            let key = v.site.clone().map(|s| (s, v.timeline.date));
            // TDWRs and DWD radars have no archive to list; their timeline stays empty and
            // always live.
            let need = v.site.as_deref().is_some_and(wxdata::sites::is_nexrad)
                && v.timeline.frames_key != key;
            (
                v.site.clone(),
                v.timeline.date,
                v.timeline.following,
                need,
                v.timeline.listing,
            )
        };
        if let Some(s) = &site {
            if need_list && !listing {
                self.views[idx].timeline.listing = true;
                self.spawn_list_frames(idx, s.clone(), date, ctx.clone());
            }
        }

        let looping = self.views[idx].timeline.live_looping();
        if following {
            // Live head: poll for the newest volume. While looping, the displayed volume is a
            // middle loop frame, so compare against the newest *frame* (not the shown volume) to
            // decide whether the head advanced — otherwise every poll re-downloads the head.
            let (site, current_name, due) = {
                let v = &self.views[idx];
                let due = v
                    .last_poll
                    .is_none_or(|t| t.elapsed().as_secs() >= self.poll_interval_secs());
                let current_name = if looping {
                    v.timeline.frames.last().map(|id| id.name().to_string())
                } else {
                    v.volume.as_ref().map(|vol| vol.name.clone())
                };
                (v.site.clone(), current_name, due)
            };
            if site.is_some() && !self.views[idx].loading && (site_changed || due) {
                if let Some(s) = site {
                    self.views[idx].loading = true;
                    self.views[idx].last_poll = Some(Instant::now());
                    self.spawn_fetch(idx, s, current_name, ctx.clone());
                }
            }
        }
        if !following || looping {
            // Archive / loop: display the volume at the playhead (cache hit is synchronous).
            let target = self.views[idx].timeline.current().map(|id| {
                (
                    id.name().to_string(),
                    id.date_time().unwrap_or_else(Utc::now),
                    id.clone(),
                )
            });
            if let Some((name, time, id)) = target {
                let shown = self.views[idx].volume.as_ref().map(|v| v.name.clone());
                if shown.as_deref() != Some(name.as_str()) {
                    if let Some(scan) = self.scan_cache.get(&name).map(Arc::clone) {
                        wxdata::stats::bump(wxdata::stats::Counter::ScanCacheHits);
                        let v = &mut self.views[idx];
                        v.show_volume(scan, name, time);
                        v.loading = false;
                        v.error = None;
                        v.clamp_tilt();
                        v.clamp_moment();
                        self.pane_shown.remove(&idx);
                    } else if !self.views[idx].loading {
                        wxdata::stats::bump(wxdata::stats::Counter::ScanCacheMisses);
                        let s = self.views[idx].site.clone().unwrap_or_default();
                        self.views[idx].loading = true;
                        self.spawn_frame_fetch(idx, s, id, ctx.clone());
                    }
                }
                // Pull neighbouring frames in behind the playhead, on their own in-flight book so
                // they never compete with the frame being shown or with the head poll. Without
                // this, playback is a serial download per frame with the loop stalled between —
                // and scrubbing, which runs this same path paused, was a cold download per step.
                self.prefetch_frames(idx, ctx);
            }
        }
    }

    /// Debug builds only: frame time and tile-queue depth in the top-left corner. `dumpsys
    /// gfxinfo` and logcat cover most of what this shows, but neither says which frames were slow
    /// while a gesture was in progress.
    ///
    /// ponytail: an unsmoothed millisecond readout, no history graph. Add one if a single number
    /// stops being enough to tell a stutter from a stall.
    #[cfg(debug_assertions)]
    fn frame_time_overlay(&mut self, ctx: &egui::Context) {
        let (dt, pinching) = ctx.input(|i| (i.unstable_dt * 1000.0, i.multi_touch().is_some()));
        let text = format!(
            "{dt:.1} ms ({:.0} fps)  z{:.2}{}",
            1000.0 / dt.max(0.001),
            self.views[self.active].camera.zoom,
            if pinching { "  pinch" } else { "" }
        );
        egui::Area::new(egui::Id::new("frame_time_overlay"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(6.0, 6.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(text)
                        .monospace()
                        .size(11.0)
                        .color(egui::Color32::from_rgb(120, 255, 120))
                        .background_color(egui::Color32::from_black_alpha(160)),
                );
            });
    }

    /// Fetch the two frames after the playhead into the scan cache, so playback isn't a serial
    /// download-per-frame. At most two are in flight; each task gives its slot back when it ends,
    /// and anything still booked well past the fetch deadline is aged out.
    fn prefetch_frames(&mut self, idx: usize, ctx: &egui::Context) {
        // Longer than a volume fetch is allowed to take, so this only ever reaps entries whose
        // task is genuinely gone. At 15 s it reaped entries whose download was still running,
        // and every tick after that started the same download again.
        book(&self.prefetching)
            .retain(|_, at| at.elapsed() < VOLUME_TIMEOUT + std::time::Duration::from_secs(15));
        if book(&self.prefetching).len() >= MAX_PREFETCH_INFLIGHT {
            return;
        }
        let Some(site) = self.views[idx].site.clone() else {
            return;
        };
        let tl = &self.views[idx].timeline;
        let wanted: Vec<Identifier> = prefetch_offsets(tl.playing)
            .iter()
            .filter_map(|d| tl.frames.get(tl.playhead.checked_add_signed(*d)?))
            .cloned()
            .collect();
        for id in wanted {
            if book(&self.prefetching).len() >= MAX_PREFETCH_INFLIGHT {
                break;
            }
            self.spawn_prefetch(idx, id, &site, ctx);
        }
    }

    /// Start one frame download into the scan cache, unless it is already cached or in flight.
    fn spawn_prefetch(&mut self, idx: usize, id: Identifier, site: &str, ctx: &egui::Context) {
        let name = id.name().to_string();
        if self.scan_cache.contains(&name) || book(&self.prefetching).contains_key(&name) {
            return;
        }
        book(&self.prefetching).insert(name, Instant::now());
        let tx = self.msg_tx.clone();
        let (site, ctx) = (site.to_string(), ctx.clone());
        let view = idx;
        let slot = self.prefetching.clone();
        self.spawner.spawn(async move {
            let name = id.name().to_string();
            let fetched = wxdata::task::timeout(
                VOLUME_TIMEOUT,
                crate::volume::fetch(id, crate::paths::cache_dir()),
            )
            .await
            .unwrap_or_else(Err);
            match fetched {
                Ok(scan) => {
                    let _ = tx.send(DataMsg::Prefetched {
                        view,
                        site,
                        name: name.clone(),
                        scan,
                    });
                }
                Err(e) => log::debug!("prefetch failed: {e}"),
            }
            // Whatever happened, this slot is free: the message above may be dropped as stale
            // (the site changed under it) and the book must not depend on it arriving.
            book(&slot).remove(&name);
            ctx.request_repaint();
        });
    }

    /// Fill the loop window *backwards* from the head, for the browser's opening auto-play.
    ///
    /// Ordinary prefetch only looks ahead of the playhead, which is the right thing once a loop is
    /// running and useless before one starts: at the live head there is nothing ahead. This walks
    /// back from the newest frame instead, under the same in-flight budget, so the visitor's first
    /// volume is the current one and the recent past fills in behind it.
    fn backfill_loop_frames(&mut self, idx: usize, ctx: &egui::Context) {
        book(&self.prefetching)
            .retain(|_, at| at.elapsed() < VOLUME_TIMEOUT + std::time::Duration::from_secs(15));
        if book(&self.prefetching).len() >= MAX_PREFETCH_INFLIGHT {
            return;
        }
        let Some(site) = self.views[idx].site.clone() else {
            return;
        };
        let tl = &self.views[idx].timeline;
        let window = tl.live_window.max(1);
        let start = tl.frames.len().saturating_sub(window);
        let tail: Vec<Identifier> = tl.frames[start..].iter().rev().cloned().collect();
        for id in tail {
            if book(&self.prefetching).len() >= MAX_PREFETCH_INFLIGHT {
                break;
            }
            self.spawn_prefetch(idx, id, &site, ctx);
        }
    }

    /// Download a specific archive volume (a scrubbed timeline frame), routed to `view_idx`.
    fn spawn_frame_fetch(&self, view_idx: usize, site: String, id: Identifier, ctx: egui::Context) {
        let tx = self.msg_tx.clone();
        self.spawner.spawn(async move {
            let name = id.name().to_string();
            let time = id.date_time().unwrap_or_else(Utc::now);
            // The `Err` arm is what matters here: it clears the pane's `loading` flag. Without a
            // deadline a swallowed request never took either arm, and a pane stuck on `loading`
            // stops polling for the rest of the session.
            let fetched = wxdata::task::timeout(
                VOLUME_TIMEOUT,
                crate::volume::fetch(id, crate::paths::cache_dir()),
            )
            .await
            .unwrap_or_else(Err);
            let msg = match fetched {
                Ok(scan) => DataMsg::Volume {
                    view: view_idx,
                    site,
                    name,
                    time,
                    scan,
                },
                Err(e) => DataMsg::Error {
                    view: view_idx,
                    site,
                    err: e.to_string(),
                },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    /// List the archive volumes for `site` on `date` (timeline frames).
    fn spawn_list_frames(
        &self,
        view_idx: usize,
        site: String,
        date: NaiveDate,
        ctx: egui::Context,
    ) {
        let tx = self.msg_tx.clone();
        self.spawner.spawn(async move {
            match level2::list_volumes(&site, date).await {
                Ok(frames) => {
                    let _ = tx.send(DataMsg::Frames {
                        view: view_idx,
                        site,
                        date,
                        frames,
                    });
                    ctx.request_repaint();
                }
                Err(e) => note_feed_error("Archive frame list", format!("{site} {date}: {e}")),
            }
        });
    }

    /// Radar upload for pane `idx`, binning the shared volume in `data` (usually the active pane)
    /// with pane `idx`'s product/tilt. Returns `(upload_when_changed, draw_radar)`; the pane's GPU
    /// buffer persists, so `None` means "reuse what's uploaded". Caches per pane via `pane_shown`.
    fn pane_radar(&mut self, idx: usize, data: usize) -> (Option<RadarUpload>, bool) {
        crate::prof_scope!("pane_radar");
        let has_volume = self.views[data].volume.is_some();
        if !self.views[idx].show_radar || !has_volume {
            self.pane_shown.remove(&idx);
            self.pane_lut.remove(&idx);
            return (None, false);
        }
        let count = self.views[data].elevation_count();
        self.views[idx].clamp_tilt_to(&count);
        let (moment, tilt, threshold, smooth, storm_uv) = {
            let v = &self.views[idx];
            (
                v.moment,
                v.tilt,
                v.active_threshold(),
                v.smooth,
                v.storm_motion_uv(),
            )
        };
        // The pane's product list is the union over every volume from this site, so a single frame
        // in a loop can lack the selected moment (a legacy volume, a split cut that hasn't arrived).
        // Draw what this volume does have rather than blanking the radar for that frame.
        // Caller resolved `data` to a view that has a volume; a "wait" is the honest answer if
        // that stops being true rather than taking the frame down.
        let Some(have) = self.views[data].volume.as_ref().map(|v| v.moments) else {
            return (None, true);
        };
        let moment = if have[moment.index()] {
            moment
        } else {
            match Moment::ALL.into_iter().find(|m| have[m.index()]) {
                Some(m) => m,
                None => return (None, true), // nothing decodable yet: keep the last image up
            }
        };
        let storm_uv = if moment == Moment::Velocity {
            storm_uv
        } else {
            None
        };
        let Some(name) = self.views[data].volume.as_ref().map(|v| v.name.clone()) else {
            return (None, true);
        };
        let uv_key = storm_uv.map(|(e, n)| (e.to_bits(), n.to_bits()));
        // Dealiasing only applies to Doppler velocity, and only where it is actually folded:
        // a TDWR's Level 3 velocity is already unfolded before it leaves the radar.
        let dealias = self.settings.dealias_velocity
            && moment == Moment::Velocity
            && !self.views[idx]
                .site
                .as_deref()
                .is_some_and(wxdata::tdwr::is_tdwr);
        let key: ShownKey = (
            name,
            moment,
            tilt,
            threshold,
            smooth,
            uv_key,
            dealias,
            self.settings.precip_tint.then_some(self.precip_flag_gen),
        );
        let lut_gen = self.palettes.gen.wrapping_add(
            if crate::theme::is_high_contrast(self.settings.theme) {
                0x9e37_79b9_7f4a_7c15
            } else {
                0
            },
        );
        // Same sweep already up: the only thing left that can differ is the color table, and
        // that is a 3 KB write into the texture already bound.
        let lut_only = self.pane_shown.get(&idx) == Some(&key);
        if lut_only && self.pane_lut.get(&idx) == Some(&lut_gen) {
            return (None, true);
        }
        let table_owned =
            crate::colormap::effective_table(&self.palettes, moment, self.settings.theme);
        let table = &table_owned;
        // Cheap handle taken before the volume is borrowed mutably below.
        let precip = self
            .settings
            .precip_tint
            .then(|| self.precip_flag_grid.clone())
            .flatten();
        let upload = {
            let Some(vol) = self.views[data].volume.as_mut() else {
                return (None, true);
            };
            // No tilts yet (a volume that has only just started arriving) is a "wait", not a
            // failure: erroring here put "tilt 0 out of range" on the map once per volume.
            if vol.elevations.is_empty() {
                return (None, true);
            }
            vol.binned(moment, tilt, dealias).map(|s| {
                to_upload(
                    s,
                    table,
                    threshold,
                    smooth,
                    storm_uv,
                    precip.as_deref(),
                    lut_only,
                )
            })
        };
        match upload {
            Ok(up) => {
                self.pane_shown.insert(idx, key);
                self.pane_lut.insert(idx, lut_gen);
                (Some(up), true)
            }
            Err(e) => {
                self.views[idx].error = Some(e.to_string());
                (None, false)
            }
        }
    }

    /// The always-on-top mini loop: a small undecorated window showing the active pane, so the
    /// radar stays visible over whatever else is on screen.
    ///
    /// An *immediate* viewport, not a deferred one: deferred viewports run their closure on the
    /// egui side and demand `'static + Send + Sync`, which `HookEchoApp` is not (wgpu handles,
    /// `Rc`s in the tile caches). Immediate renders inline on this thread, which is exactly what
    /// reusing [`Self::render_pane`] needs.
    /// The loop keeps its own camera, cloned from the active pane when the window opens and
    /// swapped in for the duration of the render — panning the little window is how you look
    /// somewhere else while the main map stays where it was.
    // ponytail: not persisted; it is a window, not a layer (see `OverlayToggle::session_only`).
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    fn mini_loop_viewport(&mut self, ctx: &egui::Context) {
        if !self.mini_loop {
            return;
        }
        let idx = self.active.min(self.views.len() - 1);
        let caption = {
            let v = &self.views[idx];
            let time = v.volume.as_ref().map_or_else(
                || "—".to_string(),
                |vol| vol.time.format("%H:%MZ").to_string(),
            );
            format!(
                "{} {} {time}",
                v.site.as_deref().unwrap_or("—"),
                v.moment.short_name()
            )
        };
        let builder = egui::ViewportBuilder::default()
            .with_title("HookEcho — mini loop")
            .with_inner_size([340.0, 260.0])
            .with_decorations(false)
            // Honoured on X11 only. The comment here used to blame GNOME's policy and say KDE
            // was fine — it is not a policy question: winit's Wayland backend implements
            // `set_window_level` as an empty function (winit 0.30, `platform_impl/linux/wayland/
            // window/mod.rs`), so no Wayland compositor is ever asked. Nothing to fix here
            // without going around winit to `xdg-foreign`/layer-shell, which is not worth it for
            // one optional window. The tool's own description says so under Wayland rather than
            // leaving the user to wonder why their window keeps disappearing behind the browser.
            .with_always_on_top();
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("mini-loop"),
            builder,
            |ctx, _class| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let full = ui.max_rect();
                        // Undecorated, so we draw our own caption strip: label, drag handle, close.
                        let (bar, prect) = (
                            egui::Rect::from_min_max(
                                full.min,
                                egui::pos2(full.max.x, full.min.y + 18.0),
                            ),
                            egui::Rect::from_min_max(
                                egui::pos2(full.min.x, full.min.y + 18.0),
                                full.max,
                            ),
                        );
                        ui.painter()
                            .rect_filled(bar, 0.0, egui::Color32::from_gray(24));
                        let handle = ui.interact(
                            bar,
                            egui::Id::new("mini-loop-bar"),
                            egui::Sense::click_and_drag(),
                        );
                        if handle.drag_started() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        ui.painter().text(
                            bar.left_center() + egui::vec2(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            &caption,
                            egui::FontId::proportional(11.0),
                            egui::Color32::from_gray(190),
                        );
                        let x = egui::Rect::from_center_size(
                            bar.right_center() - egui::vec2(10.0, 0.0),
                            egui::vec2(16.0, 16.0),
                        );
                        if ui
                            .interact(x, egui::Id::new("mini-loop-close"), egui::Sense::click())
                            .clicked()
                        {
                            close = true;
                        }
                        ui.painter().text(
                            x.center(),
                            egui::Align2::CENTER_CENTER,
                            "✕",
                            egui::FontId::proportional(11.0),
                            egui::Color32::from_gray(190),
                        );
                        let vctx = ui.ctx().clone();
                        // Swap this window's camera in around the render, and take back whatever
                        // the pointer did to it. Nothing returns early in between, so the pane
                        // always gets its own camera back.
                        let mine = self.mini_cam.take().unwrap_or(self.views[idx].camera);
                        let pane_cam = std::mem::replace(&mut self.views[idx].camera, mine);
                        self.render_pane(
                            // `first`/`last` false: the mini-loop viewport is a passenger — the
                            // main window's pane loop owns draining and evicting the tile caches.
                            ui,
                            &vctx,
                            idx,
                            prect,
                            false,
                            false,
                            false,
                            false,
                            &[],
                        );
                        self.mini_cam =
                            Some(std::mem::replace(&mut self.views[idx].camera, pane_cam));
                    });
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            },
        );
        if close {
            self.mini_loop = false;
            // Reopening should frame what the main map is looking at now, not where the loop was
            // pointed an hour ago.
            self.mini_cam = None;
        }
    }

    /// Render one pane into `prect`: input, tiles, radar, paint callback, and painter overlays.
    #[allow(clippy::too_many_arguments)]
    fn render_pane(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        idx: usize,
        prect: egui::Rect,
        clear_tiles: bool,
        clear_vector: bool,
        first: bool,
        last: bool,
        placefile_labels: &[PlaceLabel],
    ) {
        crate::prof_scope!("render_pane");
        use crate::tiles::BasemapStyle;
        // This pane's own basemap: panes are independent, the tile caches are keyed by style.
        // `Auto` resolves here rather than where it is stored, so the stored choice keeps
        // following the theme instead of being frozen the first time it is rendered.
        let pane_style = self.views[idx].basemap.resolve(ui.visuals().dark_mode);
        // Hybrid satellite is both: Esri imagery from the raster path, roads and boundaries from
        // the vector one drawn on top of it.
        let is_vector = pane_style.vector_palette().is_some();
        let is_raster = pane_style.is_raster();
        let vp = (prect.width(), prect.height());
        let response = ui.interact(
            prect,
            egui::Id::new(("pane", idx)),
            egui::Sense::click_and_drag(),
        );

        // --- Input (mutates this pane's camera / selects it active) ---
        // During a multi-touch gesture the first finger still drives the egui pointer, so a pinch
        // would ALSO register as a drag and fight the zoom — the gesture block below owns both
        // pan and zoom while two fingers are down.
        let gesture = ui.input(|i| i.multi_touch());
        if gesture.is_some() {
            self.last_gesture_end = Some(wxdata::clock::Instant::now());
        }
        // A finger still down after the other lifted is the tail of a pinch, not a new drag or a
        // tap on the map. 150 ms is long enough to cover a normal two-finger lift and short
        // enough to be invisible when you really did mean to tap.
        let gesture_tail = self
            .last_gesture_end
            .is_some_and(|t| t.elapsed().as_millis() < 150);
        let quiet = gesture.is_none() && !gesture_tail;
        // Watch-zone tool: a double-click (or Enter) closes the ring being clicked out.
        if self.tool == MapTool::AlertZone
            && self.zone_pts.len() >= 3
            && (response.double_clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            let mut ring = std::mem::take(&mut self.zone_pts);
            // The two clicks of the closing double-click each dropped a vertex on the same spot.
            ring.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
            let n = self.settings.alert_polygons.len() + 1;
            self.zone_naming = Some((ring, format!("Zone {n}")));
            self.tool = MapTool::Interrogate;
        }
        // The draw tool takes the drag away from the pan, the same deal the measure tool makes
        // with the click: while it's armed, a drag draws. Disarm it (Esc / another tool) to pan.
        if self.tool == MapTool::Draw && quiet {
            if response.dragged() {
                self.active = idx;
                if let Some(pos) = response.interact_pointer_pos() {
                    let cam = self.views[idx].camera;
                    let px = (pos.x - prect.left(), pos.y - prect.top());
                    let w = cam.screen_to_world(px, vp);
                    let ll = crate::render::mercator::world_to_lonlat(w.0, w.1);
                    draw_append(
                        &mut self.strokes,
                        [ll.0, ll.1],
                        self.draw_color,
                        response.drag_started(),
                    );
                }
            }
        } else if response.dragged() && quiet {
            self.active = idx;
            let d = response.drag_delta();
            match self.tap_zoom {
                // Double-tap-drag: the map zoom every phone map has, and the only one you can do
                // one-handed. Drag up to zoom in, anchored on the point that was tapped, so the
                // thing you double-tapped is the thing that stays put.
                Some(anchor) => {
                    let cursor = (anchor.x - prect.left(), anchor.y - prect.top());
                    self.views[idx]
                        .camera
                        .zoom_at(-d.y as f64 * 0.01, cursor, vp);
                }
                None => {
                    self.views[idx].camera.pan_pixels(d.x, d.y, vp);
                    self.follow_cell = None; // a manual pan takes over the camera
                }
            }
        }
        if cfg!(target_os = "android") {
            if response.drag_started() {
                // Within a third of a second of the last tap and within a thumb's width of it:
                // this is the second tap, still down. Anything else is an ordinary drag.
                self.tap_zoom = response.interact_pointer_pos().filter(|p| {
                    self.last_tap.is_some_and(|(t, at)| {
                        ui.input(|i| i.time) - t < 0.35 && at.distance(*p) < 44.0
                    })
                });
            }
            if response.drag_stopped() {
                self.tap_zoom = None;
                // One zoom per double tap: the next one has to be armed by a fresh tap.
                self.last_tap = None;
            }
        }
        // Wheel, trackpad, and pinch all land here. `zoom_delta` is a scale factor carrying the
        // macOS/precision-touchpad pinch gesture (and ctrl+wheel, which egui folds into the same
        // signal and subtracts from the scroll delta, so the two never double-apply). Horizontal
        // scroll pans: on a trackpad a two-finger swipe is the obvious way to move the map, and on
        // a mouse it is a tilt wheel nobody was using.
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if let Some(pos) = response.hover_pos().filter(|p| prect.contains(*p)) {
            let cursor = (pos.x - prect.left(), pos.y - prect.top());
            // `zoom_delta()` reports a live touchscreen pinch too, which the gesture block below
            // already owns (and it is the one that knows about the mobile chrome) — skip it here
            // or a phone pinch zooms twice.
            if gesture.is_none() && (zoom - 1.0).abs() > f32::EPSILON {
                self.active = idx;
                self.views[idx]
                    .camera
                    .zoom_at((zoom as f64).log2(), cursor, vp);
            }
            if scroll.y.abs() > 0.0 {
                self.active = idx;
                self.views[idx]
                    .camera
                    .zoom_at(scroll.y as f64 * 0.005, cursor, vp);
            }
            if scroll.x.abs() > 0.0 {
                self.active = idx;
                self.views[idx].camera.pan_pixels(scroll.x, 0.0, vp);
                self.follow_cell = None; // a manual pan takes over the camera
            }
        }
        // Two-finger gesture (touchscreens): pan by the gesture's translation, and zoom by the
        // pinch. `zoom_delta` is a scale factor, so its log2 is the change in the camera's log2
        // zoom level; anchor it at the gesture center so the pinched point stays put. Fires for
        // the pane the gesture centers over. No-op with no touch.
        if let Some(mt) = gesture {
            // The mobile chrome floats over the map, and `multi_touch()` is raw input with no
            // notion of which layer the fingers are on — so a pinch on the bottom sheet used to
            // zoom the map underneath it. The chrome publishes what it covers; skip those rects.
            // Explicit rects cover the always-on chrome; the layer test covers every window and
            // popup on top of it, which is what keeps a pinch on a Skew-T or a full-screen
            // settings surface from also zooming the map behind it.
            // `is_pointer_over_egui()` cannot be used here: it tests egui's single pointer
            // position, which during a two-finger gesture is one arbitrary finger, and it treats
            // the map's own background layer as "over egui" once the central panel has consumed
            // the root ui's available rect — so it answered true over bare map and killed every
            // pinch. Ask about the gesture's own center instead: any layer above the background
            // there is real chrome.
            let over_layer = ui
                .ctx()
                .layer_id_at(mt.center_pos)
                .is_some_and(|l| l.order != egui::Order::Background);
            let occluded = over_layer
                || self
                    .mobile_occlusion
                    .iter()
                    .any(|r| r.contains(mt.center_pos));
            if prect.contains(mt.center_pos) && !occluded {
                self.active = idx;
                // Zoom first, then pan: the translation is in screen pixels, and applying it at
                // the pre-zoom scale over-moves the map by the pinch's own scale factor — which is
                // what made the anchor trail the fingers.
                if (mt.zoom_delta - 1.0).abs() > f32::EPSILON {
                    let cursor = (
                        mt.center_pos.x - prect.left(),
                        mt.center_pos.y - prect.top(),
                    );
                    self.views[idx]
                        .camera
                        .zoom_at((mt.zoom_delta as f64).log2(), cursor, vp);
                }
                let t = mt.translation_delta;
                if t != egui::Vec2::ZERO {
                    self.views[idx].camera.pan_pixels(t.x, t.y, vp);
                    self.follow_cell = None; // a manual pan takes over the camera (pinch-zoom does not)
                }
            }
        }
        // Long-press inspects, whatever tool is armed. A phone has no right-click and no hover,
        // and arming Interrogate first to ask "what is that" is a step nobody discovers — so the
        // press that means "tell me about this" is the press people already try.
        let long_press = cfg!(target_os = "android") && response.long_touched();
        if long_press {
            // Before anything is drawn: the buzz is what says the press was heard.
            crate::platform::haptic(crate::platform::Haptic::Press);
        }
        if (response.clicked() || long_press) && quiet {
            self.active = idx;
            // What this press means. A long press is always an interrogation; anything else is
            // whatever the toolbar says.
            let tool = if long_press {
                MapTool::Interrogate
            } else {
                self.tool
            };
            if let Some(pos) = response.interact_pointer_pos() {
                // Remember the tap for the double-tap-drag zoom below.
                self.last_tap = Some((ui.input(|i| i.time), pos));
                let cam = self.views[idx].camera;
                let px = (pos.x - prect.left(), pos.y - prect.top());
                let w = cam.screen_to_world(px, vp);
                let (lon, lat) = crate::render::mercator::world_to_lonlat(w.0, w.1);
                // Your own markers win over everything else on the map: you put them at a place you
                // chose, so a tap there means that pin, not whatever the radar drew underneath.
                // Nearest wins, so clustered markers stay individually reachable.
                let marker_hit = matches!(tool, MapTool::Interrogate | MapTool::Marker)
                    .then(|| {
                        self.settings
                            .markers
                            .iter()
                            .enumerate()
                            .filter_map(|(i, m)| {
                                let w = crate::render::mercator::lonlat_to_world(m.lon, m.lat);
                                let (sx, sy) = cam.world_to_screen(w, vp);
                                let (dx, dy) =
                                    (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                let d2 = dx * dx + dy * dy;
                                (d2 <= tap_r2(14.0)).then_some((i, d2))
                            })
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(i, _)| i)
                    })
                    .flatten();
                // A chase partner's dot, if they published a stream with their position.
                let peer_hit = (marker_hit.is_none() && tool == MapTool::Interrogate)
                    .then(|| {
                        self.peers
                            .values()
                            .filter(|p| !p.video_url.trim().is_empty())
                            .find(|p| {
                                let w = crate::render::mercator::lonlat_to_world(p.lon, p.lat);
                                let (sx, sy) = cam.world_to_screen(w, vp);
                                let (dx, dy) =
                                    (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                dx * dx + dy * dy <= tap_r2(12.0)
                            })
                            .map(|p| (p.name.clone(), p.video_url.trim().to_string()))
                    })
                    .flatten();
                // A watch zone under the click, when nothing more specific is there.
                let zone_hit =
                    (marker_hit.is_none() && peer_hit.is_none() && tool == MapTool::Interrogate)
                        .then(|| {
                            self.settings
                                .alert_polygons
                                .iter()
                                .position(|z| point_in_ring_ll(&z.ring, lon, lat))
                        })
                        .flatten();
                // Interrogate + a click on a radar-site ring switches radars (storm features win,
                // handled inside try_pick_site). Consumes the click so no popup opens underneath.
                let picked_site = marker_hit.is_none()
                    && tool == MapTool::Interrogate
                    && self.show_radar_sites
                    && self.try_pick_site(idx, pos, cam, prect, vp);
                // A camera site under an interrogate click wins over everything below it: the
                // markers are sparse, so a tap on one is never ambiguous.
                let cam_site = (marker_hit.is_none()
                    && !picked_site
                    && tool == MapTool::Interrogate
                    && self.show_webcams)
                    .then(|| {
                        self.webcams
                            .iter()
                            .find(|s| {
                                let w = crate::render::mercator::lonlat_to_world(s.lon, s.lat);
                                let (sx, sy) = cam.world_to_screen(w, vp);
                                let (dx, dy) =
                                    (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                dx * dx + dy * dy <= tap_r2(12.0)
                            })
                            .cloned()
                    })
                    .flatten();
                // A live station under an interrogate click. Ranked above the FAA cameras: where
                // both sit on one airport, the card carries the camera and the telemetry.
                let station_hit = (marker_hit.is_none()
                    && !picked_site
                    && tool == MapTool::Interrogate
                    && self.show_stations)
                    .then(|| {
                        let to_screen = |lon: f64, lat: f64| {
                            let w = crate::render::mercator::lonlat_to_world(lon, lat);
                            let (sx, sy) = cam.world_to_screen(w, vp);
                            egui::pos2(prect.left() + sx, prect.top() + sy)
                        };
                        self.stations.hit(pos, tap_r2(12.0), to_screen)
                    })
                    .flatten();
                let cam_site = if station_hit.is_some() {
                    None
                } else {
                    cam_site
                };
                // A surveyed damage point under an interrogate click, same rule as the cameras.
                let dat_hit = (marker_hit.is_none()
                    && !picked_site
                    && cam_site.is_none()
                    && tool == MapTool::Interrogate
                    && self.show_dat)
                    .then(|| {
                        self.dat_points
                            .iter()
                            .find(|p| {
                                let w = crate::render::mercator::lonlat_to_world(p.lon, p.lat);
                                let (sx, sy) = cam.world_to_screen(w, vp);
                                let (dx, dy) =
                                    (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                dx * dx + dy * dy <= tap_r2(10.0)
                            })
                            .cloned()
                    })
                    .flatten();
                if let Some(ob) = station_hit {
                    self.cell_popup = None;
                    self.warning_popup = None;
                    let (rt, http) = (self.spawner.clone(), self.http.clone());
                    self.stations.open_card(ob, &rt, &http, ctx);
                    return;
                }
                match tool {
                    // Also catches the drop tool: a second marker within a finger's width of an
                    // existing one is never what someone meant, and this makes a stray drop undoable.
                    _ if marker_hit.is_some() => {
                        self.marker_popup = marker_hit;
                        self.cell_popup = None;
                        self.detail = None;
                    }
                    _ if peer_hit.is_some() => {
                        let (name, url) = peer_hit.expect("checked Some");
                        self.cell_popup = None;
                        self.watch_stream(name, url);
                    }
                    _ if zone_hit.is_some() => {
                        self.zone_popup = zone_hit;
                        self.cell_popup = None;
                    }
                    _ if picked_site => {}
                    _ if cam_site.is_some() => {
                        self.cell_popup = None;
                        self.warning_popup = None;
                        let site = cam_site.expect("checked Some");
                        self.open_webcam(&site, ctx);
                    }
                    _ if dat_hit.is_some() => {
                        self.cell_popup = None;
                        self.warning_popup = None;
                        let p = dat_hit.expect("checked Some");
                        self.open_damage_point(&p, ctx);
                    }
                    MapTool::Measure => {
                        if self.measure.len() >= 2 {
                            self.measure.clear();
                        }
                        self.measure.push([lon, lat]);
                    }
                    MapTool::Marker => {
                        let n = self.settings.markers.len() + 1;
                        self.settings.markers.push(crate::settings::Marker {
                            id: crate::settings::new_marker_id(),
                            name: format!("Marker {n}"),
                            lat,
                            lon,
                            icon: None,
                            alert_radius_mi: crate::settings::default_alert_radius_mi(),
                            video_url: String::new(),
                            home: false,
                        });
                    }
                    MapTool::CrossSection => {
                        if self.xsection_pts.len() >= 2 {
                            self.xsection_pts.clear();
                        }
                        self.xsection_pts.push([lon, lat]);
                        if self.xsection_pts.len() == 2 {
                            self.build_xsection(idx, ctx);
                        }
                    }
                    MapTool::Sounding => self.fetch_sounding(lon, lat),
                    MapTool::Forecast => self.fetch_point_forecast(lon, lat),
                    MapTool::Chase => {
                        self.chase_mode = true;
                        self.chase_pos = Some((lon, lat));
                    }
                    MapTool::Climatology => self.query_climatology(lon, lat),
                    // Drawing happens on drag, not on click; a bare click leaves no mark.
                    MapTool::Draw => {}
                    MapTool::AlertZone => self.zone_pts.push([lon, lat]),
                    // Labelled so the storm-marker shortcut below can bail out of the hit-test
                    // chain without returning from `render_pane` and costing this pane its
                    // tiles and radar for the frame.
                    MapTool::Interrogate => 'interrogate: {
                        // A tropical cyclone's own marker sits above everything else: it is the
                        // one feature whose words matter more than its geometry, and the cone
                        // polygon underneath would otherwise swallow the click.
                        let storm_hit = self
                            .show_tropical
                            .then(|| {
                                self.tropical.as_ref().and_then(|t| {
                                    t.storms
                                        .iter()
                                        .find(|s| {
                                            let w = crate::render::mercator::lonlat_to_world(
                                                s.lon, s.lat,
                                            );
                                            let (sx, sy) = cam.world_to_screen(w, vp);
                                            let (dx, dy) = (
                                                prect.left() + sx - pos.x,
                                                prect.top() + sy - pos.y,
                                            );
                                            dx * dx + dy * dy <= tap_r2(14.0)
                                        })
                                        .map(|s| s.id.clone())
                                })
                            })
                            .flatten();
                        if let Some(id) = storm_hit {
                            self.tropical_window.open = true;
                            self.tropical_window.storm_id = Some(id.clone());
                            let product = self.tropical_window.product;
                            self.fetch_tropical_text(&id, product);
                            break 'interrogate;
                        }
                        // Storm reports sit on top: a click near a report dot opens its detail.
                        let report = self
                            .show_storm_reports
                            .then(|| {
                                self.active_storm_reports().iter().find(|r| {
                                    let w = crate::render::mercator::lonlat_to_world(r.lon, r.lat);
                                    let (sx, sy) = cam.world_to_screen(w, vp);
                                    let (dx, dy) =
                                        (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                    dx * dx + dy * dy <= tap_r2(12.0)
                                })
                            })
                            .flatten()
                            .cloned();
                        // Fire incident points sit alongside the reports in the same hit test.
                        let fire = self
                            .show_fires
                            .then(|| {
                                self.fire_incidents.iter().find(|f| {
                                    let w = crate::render::mercator::lonlat_to_world(f.lon, f.lat);
                                    let (sx, sy) = cam.world_to_screen(w, vp);
                                    let (dx, dy) =
                                        (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                    dx * dx + dy * dy <= tap_r2(12.0)
                                })
                            })
                            .flatten()
                            .cloned();
                        let air = self
                            .show_aqi
                            .then(|| {
                                self.aqi.iter().find(|o| {
                                    let w = crate::render::mercator::lonlat_to_world(o.lon, o.lat);
                                    let (sx, sy) = cam.world_to_screen(w, vp);
                                    let (dx, dy) =
                                        (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                    dx * dx + dy * dy <= tap_r2(12.0)
                                })
                            })
                            .flatten()
                            .cloned();
                        if let Some(o) = air {
                            self.cell_popup = None;
                            self.warning_popup = None;
                            let c = o.color();
                            self.detail = Some(Detail {
                                title: format!("AQI {} — {}", o.aqi, o.category_name()),
                                body: format!("{}\n{}", o.site, o.param),
                                color: [c[0], c[1], c[2], 255],
                                image: None,
                                link: None,
                            });
                        } else if let Some(f) = fire {
                            self.cell_popup = None;
                            self.warning_popup = None;
                            self.detail = Some(Detail {
                                title: format!("{} Fire", f.name),
                                body: format!(
                                    "{}\n{}",
                                    f.acres
                                        .map(|a| format!("{a:.0} acres"))
                                        .unwrap_or_else(|| "size not reported".to_string()),
                                    f.containment
                                        .map(|c| format!("{c:.0}% contained"))
                                        .unwrap_or_else(|| "containment not reported".to_string()),
                                ),
                                color: [235, 110, 40, 255],
                                image: None,
                                link: None,
                            });
                        } else if let Some(r) = report {
                            self.cell_popup = None;
                            self.warning_popup = None;
                            self.detail = Some(Detail {
                                title: format!("{} Report — {}", r.kind.label(), r.magnitude),
                                body: format!(
                                    "{}, {}\nCounty: {}\nTime: {}Z\n\n{}",
                                    r.location, r.state, r.county, r.time, r.comments
                                ),
                                color: report_color(r.kind),
                                image: None,
                                link: None,
                            });
                        } else {
                            let cell_hit = self.filters.show_cells
                                && self.cells_site.as_deref() == self.views[idx].site.as_deref()
                                && !self.active_storm_cells().is_empty();
                            let picked = cell_hit
                                .then(|| {
                                    self.active_storm_cells().iter().find(|c| {
                                        let w =
                                            crate::render::mercator::lonlat_to_world(c.lon, c.lat);
                                        let (sx, sy) = cam.world_to_screen(w, vp);
                                        let (dx, dy) =
                                            (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                        dx * dx + dy * dy <= tap_r2(14.0)
                                    })
                                })
                                .flatten()
                                .cloned();
                            match picked {
                                // A storm cell with an id opens the attributes window; a standalone
                                // detection (empty id) falls back to a generic detail popup.
                                Some(c) if !c.id.is_empty() => {
                                    self.detail = None;
                                    self.cell_popup = Some(c);
                                }
                                Some(c) => {
                                    self.cell_popup = None;
                                    self.detail = Some(Detail {
                                        title: c.title.clone(),
                                        body: c.summary(),
                                        color: cell_color(c.kind),
                                        image: None,
                                        link: None,
                                    });
                                }
                                None => {
                                    // Warnings/watches open the warning window (deduped by alert id
                                    // across MultiPolygon parts); other features use the generic popup.
                                    let hits = overlay::hit_all(&self.overlays, lon, lat);
                                    let mut seen = std::collections::HashSet::new();
                                    let cards: Vec<ui::warning_window::WarnCard> = hits
                                        .iter()
                                        .filter_map(|f| f.alert.as_ref().map(|a| (a, f.stroke)))
                                        .filter(|(a, _)| seen.insert(a.id.clone()))
                                        .map(|(a, color)| ui::warning_window::WarnCard {
                                            info: a.clone(),
                                            color,
                                        })
                                        .collect();
                                    if !cards.is_empty() {
                                        self.detail = None;
                                        // Open straight to the full bulletin of the top alert; the
                                        // Back button reveals the stack when polygons overlap.
                                        self.warning_popup =
                                            Some(ui::warning_window::WarningPopup {
                                                cards,
                                                selected: Some(0),
                                            });
                                    } else {
                                        self.warning_popup = None;
                                        self.detail = hits.first().map(|f| Detail {
                                            title: f.title.clone(),
                                            body: f.detail.clone(),
                                            color: f.stroke,
                                            image: None,
                                            link: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Tiles (shared caches, per-pane visible list) ---
        let cam = self.views[idx].camera;
        // High-DPI screens render a 256-px raster tile across `ppp`× more physical pixels, which
        // looks blurry (bad on the S24's ~3.75× density). Fetch `round(log2(ppp))` levels deeper so
        // tiles land near 1:1. Desktop (ppp 1) → +0.
        //
        // Capped at +1, not +2: each level quadruples the tiles covering the same ground, so +2
        // asked a phone to fetch, decode, and hold 16× the imagery of a desktop for a screen that
        // is a few inches across. +1 is 4×, still sharper than the panel resolves.
        // A metered link drops to +0: the deeper level is a sharpness nicety, and it costs four
        // tile downloads for every one.
        let bias_cap = if crate::platform::is_metered() {
            0.0
        } else {
            1.0
        };
        let raster_bias = if pane_style.tiles_are_512() || self.tiles.is_retina(pane_style) {
            // 512-px and `@2x` providers already carry the extra detail in the tile itself;
            // biasing on top would fetch four of them per screen tile for nothing.
            0.0
        } else {
            ctx.pixels_per_point()
                .max(1.0)
                .log2()
                .round()
                .clamp(0.0, bias_cap) as f64
        };
        let visible = if is_raster {
            let vis = self.tiles.visible(pane_style, &cam, vp, raster_bias);
            self.tiles.request_missing(pane_style, &vis);
            self.tiles.promote_visible(pane_style, &vis);
            vis
        } else {
            Vec::new()
        };
        // Always run the vector-tile pipeline for its city/town labels — raster basemaps (satellite)
        // bake in faint labels that are hard to read, so we overlay crisp haloed ones. Only the
        // *geometry* is basemap-specific: vector basemaps draw it, raster keeps its own imagery.
        let (visible_vector, vlabels, visible_vector_tiles) = {
            let vis = self.vtiles.visible(&cam, vp);
            self.vtiles.request_missing(&vis);
            let ids: Vec<crate::render::TileId> = vis.iter().map(|v| v.id).collect();
            // Deep-copying every visible place name every frame (for every pane) showed up at the
            // 4-10 fps the phone runs at. The set only changes when the visible tiles do, or when
            // a tile's labels finish tessellating — both bump the key below.
            // Compared before it is built: the key holds a `Vec` of every visible tile id, and
            // cloning it to ask "did this change" was itself a per-pane, per-frame allocation.
            let gen = self.vtiles.label_generation();
            let stale = self
                .vlabel_cache
                .as_ref()
                .is_none_or(|((k_ids, k_gen), _)| *k_gen != gen || *k_ids != ids);
            if stale {
                let labels: std::sync::Arc<[crate::vector_tiles::PlaceLabel]> = self
                    .vtiles
                    .labels_for(ids.iter())
                    .into_iter()
                    .cloned()
                    .collect();
                self.vlabel_cache = Some(((ids.clone(), gen), labels));
            }
            // An `Arc` handle, not a deep copy of every visible place name — the cache existed to
            // stop the *lookup* running every frame, but the copy it returned survived it.
            let labels = self
                .vlabel_cache
                .as_ref()
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| Vec::new().into());
            (if is_vector { ids } else { Vec::new() }, labels, vis)
        };
        // Drain finished fetches once (on the first pane) — they upload into the shared cache.
        // Eviction lives with the tile manager (it also owns `requested`/`uploaded`), and only
        // the first pane runs it so a multi-pane frame doesn't evict what a later pane needs.
        // Eviction runs once per frame, on the last pane — after every pane has promoted its own
        // visible tiles, so a multi-pane frame can't evict what a pane it already drew still needs.
        let drop_tiles = if last {
            self.tiles.evict_excess()
        } else {
            Vec::new()
        };
        let drop_vector_tiles = if first {
            self.vtiles.touch_visible(&visible_vector_tiles);
            self.vtiles.take_evicted()
        } else {
            Vec::new()
        };
        let (new_tiles, new_vector_tiles) = if first {
            let nt = self.tiles.drain_ready();
            // Drain vtiles regardless of basemap (it populates the label cache); only upload the
            // geometry to the GPU when a vector basemap will actually draw it.
            let nv = self.vtiles.drain_ready();
            if !nt.is_empty() || !nv.is_empty() {
                ctx.request_repaint();
            }
            (nt, if is_vector { nv } else { Vec::new() })
        } else {
            (Vec::new(), Vec::new())
        };

        // --- Radar (this pane's product, its own volume) ---
        let (radar_upload, mut draw_radar) = self.pane_radar(idx, idx);
        // In the forecast-scrub tail there's no observed volume — show the HRRR field instead.
        if self.views[idx].timeline.forecast_hour().is_some() {
            draw_radar = false;
        }

        // Field layers: upload freshly-fetched grids on the first pane; every pane draws the
        // currently-enabled layers.
        // Field textures are evicted the way tiles are: decided here, where the state that knows
        // whether a re-upload will follow lives, and handed to the renderer to free.
        let drop_fields: Vec<crate::render::FieldLayer> = if first {
            let now = Instant::now();
            let on: std::collections::HashSet<crate::render::FieldLayer> = self
                .views
                .iter()
                .flat_map(|v| v.fields_on.iter().copied())
                .collect();
            let mut drop = Vec::new();
            for (layer, st) in self.fields.iter_mut() {
                if on.contains(layer) {
                    st.off_since = None;
                } else if let Some(since) = st.off_since {
                    if now.duration_since(since) >= FIELD_EVICT {
                        st.off_since = None;
                        // The grid is gone from the GPU, so the next enable must re-fetch it
                        // rather than trust its refresh cadence.
                        st.last_fetch = None;
                        drop.push(*layer);
                    }
                } else {
                    st.off_since = Some(now);
                }
            }
            drop
        } else {
            Vec::new()
        };
        let field_uploads: Vec<(crate::render::FieldLayer, crate::render::MrmsUpload)> = if first {
            self.fields
                .iter_mut()
                .filter_map(|(k, s)| s.pending.take().map(|u| (*k, u)))
                .collect()
        } else {
            Vec::new()
        };
        let field_draws: Vec<(crate::render::FieldLayer, f32)> = self.views[idx]
            .fields_on
            .iter()
            .map(|k| {
                (
                    *k,
                    self.settings.field_opacity.get(k).copied().unwrap_or(1.0),
                )
            })
            .collect();

        let cam = self.views[idx].camera;
        let (center, scale) = cam.world_to_clip_uniform(vp);
        let (wind_upload, wind) = self.wind_gpu_frame(idx, &cam, vp);
        let cb = MapCallback {
            pane: idx as u32,
            camera_center: center,
            camera_scale: scale,
            new_tiles,
            visible,
            basemap_key: pane_style.key(),
            vector_over_raster: pane_style == BasemapStyle::HybridSatellite,
            radar_upload,
            draw_radar,
            overlay_upload: if first {
                self.pending_overlay.take()
            } else {
                None
            },
            draw_overlay: self.overlay_ready,
            field_uploads,
            field_draws,
            drop_fields,
            clear_tiles,
            drop_tiles,
            new_vector_tiles,
            visible_vector,
            clear_vector,
            drop_vector_tiles,
            wind_upload,
            wind,
        };
        ui.painter()
            .add(egui_wgpu::Callback::new_paint_callback(prect, cb));

        // Per-pane product picker (multi-pane only): set THIS pane's moment directly, without
        // clicking to activate it first. Single-pane keeps using the product pill.
        if self.views.len() > 1 && !self.obs_mode {
            let cur = self.views[idx].moment;
            // Same union as the sidebar uses, so this picker doesn't blink either.
            let have = self.views[idx].moments();
            egui::Area::new(egui::Id::new(("pane_product", idx)))
                .order(egui::Order::Foreground)
                .fixed_pos(prect.left_top() + egui::vec2(6.0, 6.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::symmetric(4, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for m in Moment::ALL.into_iter().filter(|m| have[m.index()]) {
                                    if ui.selectable_label(m == cur, m.short_name()).clicked() {
                                        self.views[idx].moment = m;
                                        self.active = idx;
                                    }
                                }
                            });
                        });
                });
        }

        // Optical-flow nowcast points (needs &mut self to bin the sweep; done before the &view borrow).
        let nowcast_pts = if self.filters.show_nowcast && idx == self.active {
            self.compute_nowcast(idx)
        } else {
            Vec::new()
        };
        // A rule that watches a signature has to drive its detector even with the layer off —
        // otherwise arming "hail spike near home" and then hiding the layer silently disarms it.
        // The hits are computed here either way; only the drawing below checks the layer flag.
        let armed = |t: &crate::settings::RuleTrigger| {
            self.settings
                .alert_rules
                .iter()
                .any(|r| r.enabled && &r.trigger == t)
        };
        let want_tds = self.filters.show_tds || armed(&crate::settings::RuleTrigger::Tds);
        let want_tbss = self.filters.show_tbss || armed(&crate::settings::RuleTrigger::Tbss);
        let want_zdr =
            self.filters.show_zdr_columns || armed(&crate::settings::RuleTrigger::ZdrColumn);
        let want_couplets =
            self.filters.show_couplets || armed(&crate::settings::RuleTrigger::Rotation);
        let tds_hits = if want_tds && idx == self.active {
            self.compute_tds(idx)
        } else {
            Vec::new()
        };
        let tbss_hits = if want_tbss && idx == self.active {
            self.compute_tbss(idx)
        } else {
            Vec::new()
        };
        let zdr_hits = if want_zdr && idx == self.active {
            self.compute_zdr_columns(idx, ctx)
        } else {
            Vec::new()
        };
        let couplets = if want_couplets && idx == self.active {
            self.compute_couplets(idx)
        } else {
            Vec::new()
        };
        let local_tracks = if self.show_local_tracks && idx == self.active {
            self.compute_local_tracks()
        } else {
            Vec::new()
        };
        if idx == self.active {
            self.check_rain_arrival();
            self.evaluate_scan_rules(idx, &tds_hits, &tbss_hits, &zdr_hits, &couplets);
        }
        // Hidden layers computed only for a rule must not also be drawn.
        let tds_hits = if self.filters.show_tds {
            tds_hits
        } else {
            Vec::new()
        };
        let tbss_hits = if self.filters.show_tbss {
            tbss_hits
        } else {
            Vec::new()
        };
        let zdr_hits = if self.filters.show_zdr_columns {
            zdr_hits
        } else {
            Vec::new()
        };
        let couplets = if self.filters.show_couplets {
            couplets
        } else {
            Vec::new()
        };

        // Radar values under the cursor. Computed here, before the long immutable borrow of the
        // pane below, because sampling the volume needs it mutably — the binned sweeps are
        // cached on it as they are asked for.
        let gate_tooltip = (self.tool == MapTool::Interrogate && self.views[idx].show_radar)
            .then(|| {
                let hp = response.hover_pos().filter(|p| prect.contains(*p))?;
                let cam = self.views[idx].camera;
                let w = cam.screen_to_world((hp.x - prect.left(), hp.y - prect.top()), vp);
                let (lon, lat) = crate::render::mercator::world_to_lonlat(w.0, w.1);
                self.gate_readout(idx, lon, lat)
            })
            .flatten();

        // --- Painter overlays (clipped to this pane) ---
        let painter = ui.painter_at(prect);
        let view = &self.views[idx];
        let basemap = pane_style;

        // Storm-cell ids reserve their space first — they are the top tier, and the cell markers
        // themselves are drawn much further down with the rest of the cell layer. Reserving here
        // and drawing there is the whole reason the placer separates the two: a warning label
        // must not lose its slot to a town name that merely happened to be painted earlier.
        let cell_labels_shown: std::collections::HashSet<String> = if self.filters.show_cells
            && self.cells_site.as_deref() == view.site.as_deref()
        {
            let ids: Vec<(String, egui::Pos2)> = self
                .active_storm_cells()
                .iter()
                .filter(|c| c.kind == CellKind::Storm && !c.id.is_empty())
                .map(|c| {
                    let w = crate::render::mercator::lonlat_to_world(c.lon, c.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    (
                        c.id.clone(),
                        egui::pos2(prect.left() + sx, prect.top() + sy),
                    )
                })
                .collect();
            ids.into_iter()
                .filter(|(_, p)| prect.contains(*p))
                .filter(|(id, p)| {
                    // Matches the draw below: 11 pt text, left-bottom anchored, up and right
                    // of the marker.
                    let anchor = *p + egui::vec2(8.0, -8.0);
                    let size = egui::vec2(id.len() as f32 * 6.5, 13.0);
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(anchor.x, anchor.y - size.y), size)
                            .expand(2.0);
                    self.labels.place(
                        crate::labelplace::key(id),
                        rect,
                        crate::labelplace::Priority::Warning,
                    )
                })
                .map(|(id, _)| id)
                .collect()
        } else {
            std::collections::HashSet::new()
        };
        let view = &self.views[idx];

        // City/town labels, overlaid on every basemap. On raster (satellite) the baked-in labels
        // are faint over imagery + echoes, so we draw crisp white text with a solid black halo;
        // vector basemaps use their palette's label colors. Bigger fonts + an 8-way halo read well.
        if !vlabels.is_empty() {
            let (text_col, halo_col, big) = if is_vector {
                let st = crate::basemap_style::style(basemap.vector_palette().unwrap_or_default());
                (
                    egui::Color32::from_rgb(st.label[0], st.label[1], st.label[2]),
                    egui::Color32::from_rgb(st.label_halo[0], st.label_halo[1], st.label_halo[2]),
                    13.0,
                )
            } else {
                (
                    egui::Color32::WHITE,
                    egui::Color32::from_black_alpha(235),
                    14.5,
                )
            };
            let z = cam.zoom;
            let mut labels: Vec<&crate::vector_tiles::PlaceLabel> =
                vlabels.iter().filter(|l| z >= l.min_zoom as f64).collect();
            // Labels already on screen are offered their slot before newcomers of the same
            // importance; without that a name at the edge of a collision wins and loses on
            // alternate frames, which is exactly the flicker you see while panning.
            labels.sort_by_key(|l| {
                (
                    !self.labels.was_shown(crate::labelplace::key(&l.name)),
                    !l.city,
                    l.rank,
                )
            });
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            // 8-way halo (cardinals + diagonals) for a solid, readable outline.
            const HALO: [egui::Vec2; 8] = [
                egui::vec2(1.2, 0.0),
                egui::vec2(-1.2, 0.0),
                egui::vec2(0.0, 1.2),
                egui::vec2(0.0, -1.2),
                egui::vec2(1.0, 1.0),
                egui::vec2(1.0, -1.0),
                egui::vec2(-1.0, 1.0),
                egui::vec2(-1.0, -1.0),
            ];
            for l in labels {
                if !seen.insert(l.name.as_str()) {
                    continue;
                }
                let (sx, sy) = cam.world_to_screen((l.world[0] as f64, l.world[1] as f64), vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let font = egui::FontId::proportional(if l.city { big } else { big - 2.5 });
                let galley = painter.layout_no_wrap(l.name.clone(), font, text_col);
                let r = egui::Rect::from_min_size(p, galley.size()).expand(4.0);
                if !self.labels.place(
                    crate::labelplace::key(&l.name),
                    r,
                    crate::labelplace::Priority::Place,
                ) {
                    continue;
                }
                // One layout per label, reused for all nine draws. `painter.text` would lay the
                // string out again every time, which at eight halo offsets meant ten text
                // layouts per visible place name, every frame.
                for off in HALO {
                    painter.galley_with_override_text_color(p + off, galley.clone(), halo_col);
                }
                painter.galley_with_override_text_color(p, galley, text_col);
            }
            // OpenMapTiles/OpenStreetMap credit for the label data (raster imagery is credited below).
            painter.text(
                egui::pos2(prect.left() + 6.0, prect.bottom() - 18.0),
                egui::Align2::LEFT_BOTTOM,
                "© OpenMapTiles © OpenStreetMap",
                egui::FontId::proportional(10.0),
                egui::Color32::from_gray(200).gamma_multiply(0.55),
            );
        }

        // Raster basemap attribution (provider styles + USGS satellite).
        if pane_style.is_raster() {
            let col = egui::Color32::from_gray(200).gamma_multiply(0.6);
            painter.text(
                egui::pos2(prect.left() + 6.0, prect.bottom() - 4.0),
                egui::Align2::LEFT_BOTTOM,
                if pane_style == BasemapStyle::CustomXyz
                    && !self.settings.custom_tile_attribution.is_empty()
                {
                    self.settings.custom_tile_attribution.as_str()
                } else {
                    pane_style.attribution()
                },
                egui::FontId::proportional(10.0),
                col,
            );
        }

        // Radar-data attribution, opposite the basemap credit so neither has to know the other's
        // width. Only sources whose licence asks for a credit line get an arm.
        if let Some(credit) = view.site.as_deref().and_then(data_attribution) {
            painter.text(
                egui::pos2(prect.right() - 6.0, prect.bottom() - 4.0),
                egui::Align2::RIGHT_BOTTOM,
                credit,
                egui::FontId::proportional(10.0),
                egui::Color32::from_gray(200).gamma_multiply(0.6),
            );
        }

        // GOES lightning: one dot per flash, fading as it ages.
        if self.show_glm {
            if let Ok(feed) = self.glm.lock() {
                let now = chrono::Utc::now();
                // Reject off-screen flashes in lon/lat before projecting each one: a GLM feed can
                // carry tens of thousands of flashes while the pane shows a corner of one state.
                let corner = |px: (f32, f32)| {
                    let w = cam.screen_to_world(px, vp);
                    crate::render::mercator::world_to_lonlat(w.0, w.1)
                };
                let (c0, c1) = (corner((0.0, 0.0)), corner((vp.0, vp.1)));
                let (lon_lo, lon_hi) = (c0.0.min(c1.0), c0.0.max(c1.0));
                let (lat_lo, lat_hi) = (c0.1.min(c1.1), c0.1.max(c1.1));
                for f in feed.flashes() {
                    if f.lon < lon_lo || f.lon > lon_hi || f.lat < lat_lo || f.lat > lat_hi {
                        continue;
                    }
                    let w = crate::render::mercator::lonlat_to_world(f.lon, f.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                    if !prect.contains(p) {
                        continue;
                    }
                    let age = (now - f.time).num_seconds().max(0) as f32;
                    let (col, r) = glm_style(age);
                    painter.circle_filled(p, r, col);
                }
            }
        }

        // Ground strikes off the broker. Same shape as the GLM block above, including the
        // lon/lat prefilter, because the deque is just as long and the pane just as small.
        if self.show_strikes && !self.strikes.is_empty() {
            let now = chrono::Utc::now();
            let corner = |px: (f32, f32)| {
                let w = cam.screen_to_world(px, vp);
                crate::render::mercator::world_to_lonlat(w.0, w.1)
            };
            let (c0, c1) = (corner((0.0, 0.0)), corner((vp.0, vp.1)));
            let (lon_lo, lon_hi) = (c0.0.min(c1.0), c0.0.max(c1.0));
            let (lat_lo, lat_hi) = (c0.1.min(c1.1), c0.1.max(c1.1));
            for &(lon, lat, time) in &self.strikes {
                if lon < lon_lo || lon > lon_hi || lat < lat_lo || lat > lat_hi {
                    continue;
                }
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let age = (now - time).num_seconds().max(0) as f32;
                let (col, r) = strike_style(age);
                painter.circle_filled(p, r, col);
            }
        }

        // Animated wind particles, when they are being drawn on the CPU. The GPU path draws
        // inside the map callback instead (see `wind_gpu_frame`), which is also what puts it
        // under the warning polygons rather than over them.
        if self.show_wind && !self.wind_on_gpu {
            let alpha = self.wind_alpha(idx, cam.zoom);
            // Split borrow: the grids and the per-pane particle sets are disjoint fields, and the
            // advection needs both at once.
            let Self {
                wind,
                wind_particles,
                wind_dt,
                ..
            } = self;
            if let (Some(field), true) = (wind.as_ref(), alpha > 0.01) {
                let ps = wind_particles.entry(idx).or_default();
                ps.update(field, &cam, vp, *wind_dt);
                let mesh = ps.build_mesh(&cam, vp, prect.left_top(), alpha);
                if !mesh.is_empty() {
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        }

        // Surface analysis: fronts with their pips, plus H/L centers.
        if self.show_fronts {
            if let Some(a) = &self.fronts {
                let to_screen = |lon: f64, lat: f64| {
                    let w = crate::render::mercator::lonlat_to_world(lon, lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    egui::pos2(prect.left() + sx, prect.top() + sy)
                };
                crate::fronts_draw::draw(&painter, a, prect, to_screen);
            }
        }

        // HRRR model contours (MSLP / 2 m temp / dewpoint / CAPE / SRH): labeled isolines + banner.
        if self.contour_kind != ContourKind::Off && !self.contours.is_empty() {
            let to_screen = |lon: f64, lat: f64| {
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            let col = self.contour_kind.color();
            // This pane's lon/lat bounds, for culling lines by their precomputed bbox BEFORE
            // projecting any points — CAPE/SRH carry thousands of small rings.
            let (vmin_lon, vmin_lat, vmax_lon, vmax_lat) = {
                use crate::render::mercator::world_to_lonlat;
                let (wx0, wy0) = cam.screen_to_world((0.0, 0.0), vp);
                let (wx1, wy1) = cam.screen_to_world((vp.0, vp.1), vp);
                let (lon0, lat0) = world_to_lonlat(wx0, wy0);
                let (lon1, lat1) = world_to_lonlat(wx1, wy1);
                (
                    lon0.min(lon1),
                    lat0.min(lat1),
                    lon0.max(lon1),
                    lat0.max(lat1),
                )
            };
            for line in &self.contours {
                let (bx0, by0, bx1, by1) = line.bbox;
                if bx1 < vmin_lon || bx0 > vmax_lon || by1 < vmin_lat || by0 > vmax_lat {
                    continue; // fully off-view
                }
                let pts: Vec<egui::Pos2> = line
                    .pts
                    .iter()
                    .map(|&(lon, lat)| to_screen(lon, lat))
                    .collect();
                // Label the longest segment's midpoint when the line spans enough pixels.
                let seg = longest_segment(&pts);
                painter.add(egui::Shape::line(pts, egui::Stroke::new(1.2, col)));
                if let Some((a, b)) = seg {
                    if a.distance(b) > 60.0 {
                        let mid = a + (b - a) * 0.5;
                        let txt = format!("{:.0}", line.level);
                        let font = egui::FontId::proportional(11.0);
                        for dx in [-1.0, 1.0] {
                            for dy in [-1.0, 1.0] {
                                painter.text(
                                    mid + egui::vec2(dx, dy),
                                    egui::Align2::CENTER_CENTER,
                                    &txt,
                                    font.clone(),
                                    egui::Color32::from_black_alpha(200),
                                );
                            }
                        }
                        painter.text(mid, egui::Align2::CENTER_CENTER, &txt, font, col);
                    }
                }
            }
            if idx == self.active {
                let vt = self
                    .contour_valid
                    .map(|t| crate::timefmt::fmt_clock(t, self.active_tz(), false))
                    .unwrap_or_default();
                let text = format!("HRRR {} contours — valid {vt}", self.contour_kind.label());
                let font = egui::FontId::proportional(12.0);
                let anchor = egui::pos2(prect.left() + 8.0, prect.top() + 40.0);
                let galley =
                    painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
                let bg = egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                painter.rect_filled(
                    bg,
                    3.0,
                    egui::Color32::from_rgba_unmultiplied(60, 90, 60, 200),
                );
                painter.text(
                    anchor + egui::vec2(5.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    &text,
                    font,
                    egui::Color32::WHITE,
                );
            }
        }

        // Storm-cell dots + SCIT forecast tracks.
        if self.filters.show_cells && self.cells_site.as_deref() == view.site.as_deref() {
            let to_screen = |lon: f64, lat: f64| {
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            // Arrival-time cones: project each moving cell forward, shade the swept path, and
            // list ETAs to any watched marker the cone covers.
            if self.filters.show_arrival_cones {
                const LEAD_MIN: f64 = 60.0;
                const HALF_ANGLE: f64 = 18.0;
                // Indices, not strings: every marker inside every cone used to be formatted and
                // then thrown away by the `take(6)` below.
                let mut etas: Vec<(f64, usize, usize)> = Vec::new();
                let cells = self.active_storm_cells();
                for (ci, c) in cells.iter().enumerate() {
                    let (Some(dir), Some(kt)) = (c.mvt_deg, c.mvt_kt) else {
                        continue;
                    };
                    if kt <= 1.0 {
                        continue;
                    }
                    let lead_km = kt as f64 * 1.852 * (LEAD_MIN / 60.0);
                    let left = crate::geo::destination_point(
                        [c.lon, c.lat],
                        dir as f64 - HALF_ANGLE,
                        lead_km,
                    );
                    let right = crate::geo::destination_point(
                        [c.lon, c.lat],
                        dir as f64 + HALF_ANGLE,
                        lead_km,
                    );
                    let apex = to_screen(c.lon, c.lat);
                    let lp = to_screen(left[0], left[1]);
                    let rp = to_screen(right[0], right[1]);
                    let col = cell_color(c.kind);
                    let fill = egui::Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 40);
                    painter.add(egui::Shape::convex_polygon(
                        vec![apex, lp, rp],
                        fill,
                        egui::Stroke::NONE,
                    ));
                    // Center line toward the projected 60-min position.
                    let tip = crate::geo::destination_point([c.lon, c.lat], dir as f64, lead_km);
                    painter.line_segment(
                        [apex, to_screen(tip[0], tip[1])],
                        egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 160),
                        ),
                    );
                    // ETA to each watched marker inside this cone.
                    for (mi, m) in self.settings.markers.iter().enumerate() {
                        if let Some(min) = crate::geo::arrival_eta_min(
                            [c.lon, c.lat],
                            dir,
                            kt,
                            [m.lon, m.lat],
                            HALF_ANGLE,
                            LEAD_MIN,
                        ) {
                            etas.push((min, mi, ci));
                        }
                    }
                }
                if idx == self.active && !etas.is_empty() {
                    etas.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    let font = egui::FontId::proportional(12.0);
                    let mut y = prect.top() + 40.0;
                    for (min, mi, ci) in etas.iter().take(6) {
                        let (m, c) = (&self.settings.markers[*mi], &cells[*ci]);
                        let text = format!("⏱ {} — {} in {:.0} min", m.name, c.id, min);
                        let galley =
                            painter.layout_no_wrap(text, font.clone(), egui::Color32::WHITE);
                        let anchor = egui::pos2(prect.left() + 8.0, y);
                        let size = galley.size();
                        let bg = egui::Rect::from_min_size(anchor, size + egui::vec2(10.0, 4.0));
                        painter.rect_filled(
                            bg,
                            3.0,
                            egui::Color32::from_rgba_unmultiplied(150, 30, 30, 210),
                        );
                        // The galley just measured, drawn — one layout, not two.
                        painter.galley(anchor + egui::vec2(5.0, 2.0), galley, egui::Color32::WHITE);
                        y += size.y + 6.0;
                    }
                }
            }

            // Optical-flow nowcast: advected echo ghost + a lead-time banner.
            if !nowcast_pts.is_empty() {
                // The dots also grow with lead: a longer extrapolation is a blurrier claim
                // about where the echo will be, and a bigger, softer dot reads that way.
                let lead = self.filters.nowcast_lead_min;
                let radius = 2.5 + (1.0 - nowcast_confidence(lead)) * 3.0;
                for (lon, lat, col) in &nowcast_pts {
                    let p = to_screen(*lon, *lat);
                    if prect.contains(p) {
                        painter.circle_filled(p, radius, *col);
                    }
                }
                if idx == self.active {
                    let text = if lead > 45 {
                        format!(
                            "\u{25c8} NOWCAST +{lead} min \u{2014} extrapolation only; try HRRR \
                             future radar for an hour or more"
                        )
                    } else {
                        format!(
                            "\u{25c8} NOWCAST +{lead} min \u{2014} echo extrapolated from storm motion"
                        )
                    };
                    let font = egui::FontId::proportional(12.0);
                    let anchor = egui::pos2(prect.left() + 8.0, prect.top() + 20.0);
                    let galley =
                        painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
                    let bg =
                        egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                    painter.rect_filled(
                        bg,
                        3.0,
                        egui::Color32::from_rgba_unmultiplied(60, 60, 150, 200),
                    );
                    painter.text(
                        anchor + egui::vec2(5.0, 2.0),
                        egui::Align2::LEFT_TOP,
                        &text,
                        font,
                        egui::Color32::WHITE,
                    );
                }
            }

            // TDS markers: a magenta inverted triangle + label at each debris-signature cluster.
            for h in &tds_hits {
                let p = to_screen(h.lon, h.lat);
                if !prect.contains(p) {
                    continue;
                }
                let m = egui::Color32::from_rgb(240, 40, 210);
                let s = 8.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        p + egui::vec2(-s, -s),
                        p + egui::vec2(s, -s),
                        p + egui::vec2(0.0, s),
                    ],
                    egui::Color32::from_rgba_unmultiplied(240, 40, 210, 60),
                    egui::Stroke::new(2.0, m),
                ));
                painter.text(
                    p + egui::vec2(0.0, -s - 2.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("TDS ρ{:.2}", h.min_cc),
                    egui::FontId::proportional(11.0),
                    m,
                );
            }

            // Hail spikes: a hollow triangle at the core the spike points away from. Yellow, not
            // magenta — this is a hail flag, and nothing here should read like a debris signature.
            for h in &tbss_hits {
                let p = to_screen(h.lon, h.lat);
                if !prect.contains(p) {
                    continue;
                }
                let col = egui::Color32::from_rgb(250, 210, 60);
                let s = 7.0;
                painter.add(egui::Shape::closed_line(
                    vec![
                        p + egui::vec2(0.0, -s),
                        p + egui::vec2(s, s),
                        p + egui::vec2(-s, s),
                    ],
                    egui::Stroke::new(2.0, col),
                ));
                painter.text(
                    p + egui::vec2(0.0, -s - 2.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("TBSS {:.0} dBZ", h.core_dbz),
                    egui::FontId::proportional(11.0),
                    col,
                );
            }

            // ZDR columns: an upward arrow with the depth above the freezing level.
            for h in &zdr_hits {
                let p = to_screen(h.lon, h.lat);
                if !prect.contains(p) {
                    continue;
                }
                let col = egui::Color32::from_rgb(120, 230, 160);
                let s = 8.0;
                painter.line_segment(
                    [p + egui::vec2(0.0, s), p + egui::vec2(0.0, -s)],
                    egui::Stroke::new(2.0, col),
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        p + egui::vec2(0.0, -s - 4.0),
                        p + egui::vec2(-4.0, -s + 1.0),
                        p + egui::vec2(4.0, -s + 1.0),
                    ],
                    col,
                    egui::Stroke::NONE,
                ));
                painter.text(
                    p + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("ZDR +{:.1} km", h.depth_km),
                    egui::FontId::proportional(11.0),
                    col,
                );
            }

            // Where the melting layer is, read off the same volume's CC. One line, because the
            // number is the whole product: it tells you which "heavy rain" is a bright band.
            if self.filters.show_zdr_columns && idx == self.active {
                if let Some((_, _, Some(bb))) = &self.zdr_cache {
                    let text = format!("melting layer ~{:.1} km (bright band)", bb.height_km);
                    let font = egui::FontId::proportional(12.0);
                    painter.text(
                        egui::pos2(prect.left() + 8.0, prect.bottom() - 8.0),
                        egui::Align2::LEFT_BOTTOM,
                        text,
                        font,
                        egui::Color32::from_rgb(160, 200, 230),
                    );
                }
            }

            // Rotation couplets: a ring at each cluster — solid red at strong-TVS strength
            // (≥36 m/s rotational velocity), hollow orange below it.
            for h in &couplets {
                let p = to_screen(h.lon, h.lat);
                if !prect.contains(p) {
                    continue;
                }
                let strong = h.vrot_ms >= 36.0;
                let col = if strong {
                    egui::Color32::from_rgb(240, 60, 60)
                } else {
                    egui::Color32::from_rgb(245, 160, 50)
                };
                painter.circle_stroke(p, 11.0, egui::Stroke::new(2.0, col));
                if strong {
                    painter.circle_filled(p, 3.5, col);
                }
                painter.text(
                    p + egui::vec2(0.0, 13.0),
                    egui::Align2::CENTER_TOP,
                    format!("ROT {:.0} kt", h.vrot_ms * 1.943_844),
                    egui::FontId::proportional(11.0),
                    col,
                );
            }

            // Locally-computed cell tracks, in cyan so they never read as the Level 3 storm-cell
            // table drawn below in white and red. Same grammar: past polyline, dots, `T+NNm` labels.
            if !local_tracks.is_empty() {
                let cyan = egui::Color32::from_rgb(80, 220, 255);
                for tr in &local_tracks {
                    let pts: Vec<egui::Pos2> = tr
                        .points
                        .iter()
                        .map(|&(lon, lat, _)| to_screen(lon, lat))
                        .collect();
                    painter.add(egui::Shape::line(
                        pts.clone(),
                        egui::Stroke::new(1.5, cyan.gamma_multiply(0.6)),
                    ));
                    let Some(&head) = pts.last() else { continue };
                    if !prect.contains(head) {
                        continue;
                    }
                    painter.circle_stroke(head, 6.0, egui::Stroke::new(2.0, cyan));
                    for minutes in [15.0, 30.0] {
                        let Some((lon, lat)) = tr.extrapolate(minutes) else {
                            continue;
                        };
                        let p = to_screen(lon, lat);
                        painter.line_segment(
                            [head, p],
                            egui::Stroke::new(1.0, cyan.gamma_multiply(0.5)),
                        );
                        painter.circle_filled(p, 3.0, cyan);
                        if cam.zoom >= 7.0 {
                            painter.text(
                                p + egui::vec2(5.0, -2.0),
                                egui::Align2::LEFT_CENTER,
                                format!("T+{minutes:.0}m"),
                                egui::FontId::proportional(10.0),
                                cyan,
                            );
                        }
                    }
                    // With a GPS fix, the number a chaser actually wants: how close this cell comes
                    // to where they are standing, and in how many minutes.
                    if let Some((lon, lat)) = self.chase_pos {
                        let last = tr.points[tr.points.len() - 1];
                        let (km, min) = crate::geo::closest_approach(
                            [last.0, last.1],
                            tr.dir_deg,
                            tr.speed_kt,
                            [lon, lat],
                            60.0,
                        );
                        if min >= 1.0 {
                            painter.text(
                                head + egui::vec2(0.0, 9.0),
                                egui::Align2::CENTER_TOP,
                                format!("\u{2248}{min:.0} min / {km:.0} km"),
                                egui::FontId::proportional(10.0),
                                cyan,
                            );
                        }
                    }
                }
            }

            let label_tracks = self.filters.show_tracks && cam.zoom >= 7.0;
            for c in self.active_storm_cells() {
                let p = to_screen(c.lon, c.lat);
                // Past track (packet 23): faint gray polyline leading up to the current position.
                if self.filters.show_tracks && c.past_track.len() >= 2 {
                    let gray = egui::Color32::from_gray(150).gamma_multiply(0.7);
                    let pts: Vec<egui::Pos2> = c
                        .past_track
                        .iter()
                        .map(|&(lon, lat)| to_screen(lon, lat))
                        .collect();
                    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, gray)));
                }
                // Forecast track: cell -> future positions, ticks + T+NNm labels.
                if self.filters.show_tracks && !c.track.is_empty() {
                    let white = egui::Color32::from_rgb(235, 235, 235);
                    let mut prev = p;
                    for tp in &c.track {
                        let tpp = to_screen(tp.lon, tp.lat);
                        painter.line_segment([prev, tpp], egui::Stroke::new(1.5, white));
                        painter.circle_filled(tpp, 3.0, white);
                        if label_tracks {
                            let txt = format!("T+{}m", tp.minutes);
                            let lp = tpp + egui::vec2(5.0, -2.0);
                            for off in [egui::vec2(1.0, 1.0), egui::vec2(-1.0, -1.0)] {
                                painter.text(
                                    lp + off,
                                    egui::Align2::LEFT_CENTER,
                                    &txt,
                                    egui::FontId::proportional(10.0),
                                    egui::Color32::from_black_alpha(180),
                                );
                            }
                            painter.text(
                                lp,
                                egui::Align2::LEFT_CENTER,
                                &txt,
                                egui::FontId::proportional(10.0),
                                egui::Color32::from_rgb(255, 90, 90),
                            );
                        }
                        prev = tpp;
                    }
                }
                if !prect.contains(p) {
                    continue;
                }
                let col = cell_color(c.kind);
                let color = egui::Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 255);
                painter.circle_stroke(p, 6.0, egui::Stroke::new(2.0, color));
                painter.circle_filled(p, 2.0, color);
                if c.kind == CellKind::Storm && cell_labels_shown.contains(&c.id) {
                    painter.text(
                        p + egui::vec2(8.0, -8.0),
                        egui::Align2::LEFT_BOTTOM,
                        &c.id,
                        egui::FontId::proportional(11.0),
                        color,
                    );
                }
            }
        }

        // Storm-report dots (live LSRs, or the archived window while scrubbed).
        if self.show_storm_reports {
            for r in self.active_storm_reports() {
                let w = crate::render::mercator::lonlat_to_world(r.lon, r.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let col = report_color(r.kind);
                let color = egui::Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 255);
                // Small filled diamond so reports read distinctly from round storm-cell dots.
                let d = 4.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        p + egui::vec2(0.0, -d),
                        p + egui::vec2(d, 0.0),
                        p + egui::vec2(0.0, d),
                        p + egui::vec2(-d, 0.0),
                    ],
                    color,
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                ));
            }
        }

        // AirNow monitors: a dot in the EPA category color with the AQI beside it.
        if self.show_aqi {
            for o in &self.aqi {
                let w = crate::render::mercator::lonlat_to_world(o.lon, o.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let c = o.color();
                painter.circle(
                    p,
                    4.0,
                    egui::Color32::from_rgb(c[0], c[1], c[2]),
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                );
                painter.text(
                    p + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    o.aqi.to_string(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_rgb(c[0], c[1], c[2]),
                );
            }
        }

        // Wildfire incident points (the perimeters ride the tessellated overlay layer).
        if self.show_fires {
            for f in &self.fire_incidents {
                let w = crate::render::mercator::lonlat_to_world(f.lon, f.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                painter.circle(
                    p,
                    4.0,
                    egui::Color32::from_rgb(235, 110, 40),
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                );
            }
        }

        // Hurricane-hunter flight track: one dot per 30-second observation, colored by the
        // surface wind the SFMR measured (or flight-level wind when it reported nothing).
        if self.show_recon {
            for o in &self.recon {
                let w = crate::render::mercator::lonlat_to_world(o.lon, o.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let kt = o.sfmr_kt.or(o.wspd_kt).unwrap_or(0.0);
                let (_, c) = wxdata::tropical::saffir_simpson(kt);
                painter.circle_filled(p, 2.5, egui::Color32::from_rgb(c[0], c[1], c[2]));
                let hit = egui::Rect::from_center_size(p, egui::vec2(10.0, 10.0));
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                    let fl = o
                        .wspd_kt
                        .map_or_else(|| "\u{2014}".into(), |v| format!("{v:.0} kt"));
                    let sfc = o
                        .sfmr_kt
                        .map_or_else(|| "\u{2014}".into(), |v| format!("{v:.0} kt"));
                    let mb = o
                        .press_mb
                        .map_or_else(|| "\u{2014}".into(), |v| format!("{v:.1} mb"));
                    response.clone().show_tooltip_text(format!(
                        "{} \u{2014} flight level {fl}, surface {sfc}\n{mb}",
                        o.mission
                    ));
                }
            }
        }

        // Pilot reports: a small triangle per report, filled when it carries a hazard so a
        // turbulence report stands out from a routine sky observation.
        if self.show_pireps {
            for r in &self.pireps {
                let w = crate::render::mercator::lonlat_to_world(r.lon, r.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let col = if r.urgent {
                    egui::Color32::from_rgb(235, 70, 70)
                } else if r.hazard.is_empty() {
                    egui::Color32::from_rgb(150, 165, 185)
                } else {
                    egui::Color32::from_rgb(240, 190, 50)
                };
                let d = 5.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        p + egui::vec2(0.0, -d),
                        p + egui::vec2(d, d * 0.8),
                        p + egui::vec2(-d, d * 0.8),
                    ],
                    col,
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                ));
                // Hover → altitude, aircraft and the raw report, which is what pilots read.
                let hit = egui::Rect::from_center_size(p, egui::vec2(16.0, 16.0));
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                    let alt = r
                        .alt_ft
                        .map_or_else(|| "—".to_string(), |a| format!("{a} ft"));
                    response.clone().show_tooltip_text(format!(
                        "{alt}  {}\n{}\n{}",
                        r.ac_type, r.hazard, r.raw
                    ));
                }
            }
        }

        // Crowd precipitation-type reports: a lettered dot per report, so the rain/snow line
        // reads straight off the map.
        if self.show_mping {
            for r in &self.mping_reports {
                let w = crate::render::mercator::lonlat_to_world(r.lon, r.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let c = r.precip.color();
                let color = egui::Color32::from_rgb(c[0], c[1], c[2]);
                painter.circle_filled(p, 5.0, color);
                painter.circle_stroke(p, 5.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
                painter.text(
                    p,
                    egui::Align2::CENTER_CENTER,
                    r.precip.glyph(),
                    egui::FontId::proportional(8.0),
                    egui::Color32::BLACK,
                );
                let hit = egui::Rect::from_center_size(p, egui::vec2(14.0, 14.0));
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                    let tz = self.settings.tz_for(self.views[idx].site.as_deref());
                    response.clone().show_tooltip_text(format!(
                        "{}\n{}",
                        r.description,
                        crate::timefmt::fmt_clock(r.time, tz, false)
                    ));
                }
            }
        }

        // Spotter Network positions, filtered to within Level-II range of this pane's site.
        // FAA camera sites: a small camera-blue dot per airport, named once you're close enough
        // to tell them apart. Clicking one opens its newest frame (see `open_webcam`).
        if self.show_webcams {
            let show_labels = cam.zoom >= 8.0;
            let col = egui::Color32::from_rgb(110, 180, 240);
            // A camera under a tornado or severe-thunderstorm warning is the one worth opening,
            // and it looks exactly like the other forty until you click them all. Ring it.
            // Polygons the alert layer already holds; no extra fetch and no extra geometry.
            let threat: Vec<&GeoFeature> = self
                .alert_features
                .iter()
                .filter(|f| {
                    f.kind == overlay::FeatureKind::Warning
                        && f.alert.as_ref().is_some_and(|a| {
                            let e = a.event.to_ascii_lowercase();
                            e.contains("tornado") || e.contains("severe thunderstorm")
                        })
                })
                .collect();
            for site in &self.webcams {
                let w = crate::render::mercator::lonlat_to_world(site.lon, site.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                painter.circle_filled(p, 4.0, col);
                painter.circle_stroke(
                    p,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(170)),
                );
                // `distance_km` is 0 inside the polygon, which is the test we want here.
                if threat
                    .iter()
                    .any(|f| f.distance_km(site.lon, site.lat) == 0.0)
                {
                    painter.circle_stroke(
                        p,
                        7.0,
                        egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 120, 60)),
                    );
                }
                if show_labels {
                    painter.text(
                        p + egui::vec2(6.0, -5.0),
                        egui::Align2::LEFT_BOTTOM,
                        &site.name,
                        egui::FontId::proportional(10.0),
                        col,
                    );
                }
            }
        }

        // Live stations: a dot per station, warm where it is hot and cool where it is not, so a
        // boundary reads off the map before any card is open. Clicking one opens its card.
        if self.show_stations {
            let show_labels = cam.zoom >= 8.0;
            let temp_unit = self.settings.temp_unit;
            for ob in &self.stations.obs {
                let w = crate::render::mercator::lonlat_to_world(ob.lon, ob.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let col = match ob.temp_c {
                    // Blue at freezing through red at 38 C, the span US surface weather lives in.
                    Some(t) => {
                        let f = ((t / 38.0).clamp(0.0, 1.0) * 255.0) as u8;
                        egui::Color32::from_rgb(f, 90, 255 - f)
                    }
                    None => egui::Color32::from_gray(150),
                };
                // A personal station usually sits within a mile of the airport METAR that already
                // has a dot here, so the networks get different shapes and opposite label sides —
                // otherwise the PWS is drawn, invisible, underneath the METAR.
                let stroke = egui::Stroke::new(1.0, egui::Color32::from_black_alpha(180));
                let metar = ob.network == wxdata::stations::Network::Metar;
                if metar {
                    painter.circle_filled(p, 5.0, col);
                    painter.circle_stroke(p, 5.0, stroke);
                } else {
                    let r = egui::Rect::from_center_size(p, egui::vec2(9.0, 9.0));
                    painter.rect_filled(r, 1.0, col);
                    painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Middle);
                }
                if show_labels {
                    let label = match ob.temp_c {
                        Some(t) => format!("{:.0}{}", temp_unit.from_c(t), temp_unit.label()),
                        None => ob.id.clone(),
                    };
                    let (off, align) = if metar {
                        (7.0, egui::Align2::LEFT_CENTER)
                    } else {
                        (-7.0, egui::Align2::RIGHT_CENTER)
                    };
                    painter.text(
                        p + egui::vec2(off, 0.0),
                        align,
                        label,
                        egui::FontId::proportional(10.0),
                        egui::Color32::from_gray(230),
                    );
                }
            }
        }

        // Damage surveys: the fitted path first, then a dot per surveyed indicator coloured by its
        // EF rating, so the rating gradient along the track reads at a glance.
        if self.show_dat {
            let to_screen = |lon: f64, lat: f64| {
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            for t in &self.dat_tracks {
                let pts: Vec<egui::Pos2> = t.path.iter().map(|p| to_screen(p[0], p[1])).collect();
                if pts.len() >= 2 {
                    painter.add(egui::Shape::line(
                        pts,
                        egui::Stroke::new(2.5, ef_color(&t.efscale).gamma_multiply(0.9)),
                    ));
                }
            }
            for p in &self.dat_points {
                let s = to_screen(p.lon, p.lat);
                if !prect.contains(s) {
                    continue;
                }
                painter.circle_filled(s, 3.5, ef_color(&p.efscale));
                painter.circle_stroke(
                    s,
                    3.5,
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(180)),
                );
            }
        }

        // Verification reports, while the lab is open: green where a warning was already out,
        // red where nothing was. The red dots are the point of the whole feature.
        if self.verify_window.open {
            if let Some(v) = &self.verify_window.data {
                for r in &v.reports {
                    let w = crate::render::mercator::lonlat_to_world(r.lon, r.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                    if !prect.contains(p) {
                        continue;
                    }
                    let color = if r.warned {
                        egui::Color32::from_rgb(120, 220, 140)
                    } else {
                        egui::Color32::from_rgb(230, 70, 70)
                    };
                    painter.circle_filled(p, 4.0, color);
                    painter.circle_stroke(
                        p,
                        4.0,
                        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(180)),
                    );
                }
            }
        }

        // Radiosonde sites, only while the sounding tool is armed — a click anywhere takes the
        // nearest of these, so showing them is how you know what "nearest" will pick.
        if self.tool == MapTool::Sounding {
            for st in &wxdata::raob::STATIONS {
                let w = crate::render::mercator::lonlat_to_world(st.lon, st.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let color = egui::Color32::from_rgb(150, 200, 255);
                painter.circle_stroke(p, 4.0, egui::Stroke::new(1.5, color));
                if cam.zoom >= 6.0 {
                    painter.text(
                        p + egui::vec2(6.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        // The station's place name, not its WMO number: "72249" on a map tells
                        // nobody anything.
                        st.name.split(',').next().unwrap_or(st.name),
                        egui::FontId::proportional(10.0),
                        color,
                    );
                }
            }
        }

        if self.show_spotters {
            if let Some(site_pos) = self.views[idx]
                .site
                .as_deref()
                .and_then(wxdata::sites::site_by_id)
                .map(|s| [s.longitude as f64, s.latitude as f64])
            {
                let now = Utc::now();
                let show_labels = cam.zoom >= 9.0;
                // The range limit is at most `range/110` degrees of latitude and, at CONUS
                // latitudes, ~1.4x that in longitude — a cheap box rejects almost every spotter
                // before the haversine runs. 0 means the user asked for the whole feed.
                let range_km = self.settings.spotter_range_km.max(0.0);
                let (max_dlat, max_dlon) = if range_km <= 0.0 {
                    (f64::INFINITY, f64::INFINITY)
                } else {
                    let dlat = range_km / 110.0;
                    (dlat, dlat * 1.45)
                };
                let mut spotter_click: Option<wxdata::spotters::Spotter> = None;
                for sp in &self.spotters {
                    if (sp.lon - site_pos[0]).abs() > max_dlon
                        || (sp.lat - site_pos[1]).abs() > max_dlat
                    {
                        continue;
                    }
                    if range_km > 0.0
                        && crate::geo::great_circle(site_pos, [sp.lon, sp.lat]).0 > range_km
                    {
                        continue;
                    }
                    let w = crate::render::mercator::lonlat_to_world(sp.lon, sp.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                    if !prect.contains(p) {
                        continue;
                    }
                    // Spotter Network green; faded when the report is stale (>30 min old).
                    let stale = (now - sp.time).num_minutes() > 30;
                    let color = {
                        let g = egui::Color32::from_rgb(0, 200, 80);
                        if stale {
                            g.gamma_multiply(0.35)
                        } else {
                            g
                        }
                    };
                    painter.circle_filled(p, 3.0, color);
                    painter.circle_stroke(
                        p,
                        3.0,
                        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                    );
                    // Movement arrow tick, heading clockwise from north.
                    if let Some(h) = sp.heading {
                        let r = h.to_radians();
                        let dir = egui::vec2(r.sin(), -r.cos());
                        painter.line_segment([p, p + dir * 8.0], egui::Stroke::new(1.5, color));
                    }
                    if show_labels {
                        painter.text(
                            p + egui::vec2(5.0, -5.0),
                            egui::Align2::LEFT_BOTTOM,
                            &sp.name,
                            egui::FontId::proportional(10.0),
                            color,
                        );
                    }
                    let hit = egui::Rect::from_center_size(p, egui::vec2(14.0, 14.0));
                    if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                        let hover = format!(
                            "{}\n{}\n{}",
                            sp.name,
                            crate::timefmt::fmt_date_clock(sp.time, self.active_tz()),
                            sp.status
                        );
                        response.clone().show_tooltip_text(hover);
                    }
                    if response.clicked()
                        && response
                            .interact_pointer_pos()
                            .is_some_and(|hp| hit.contains(hp))
                    {
                        spotter_click = Some(sp.clone());
                    }
                }
                // Deferred: the surrounding pane borrow is still live here.
                self.pending_spotter = spotter_click;
            }
        }

        // ProbSevere per-storm probability badges (polygons draw via the overlay pipeline).
        if self.show_probsevere {
            for f in &self.probsevere {
                let Some(ring) = f.rings.first() else {
                    continue;
                };
                if ring.is_empty() {
                    continue;
                }
                let (mut clon, mut clat) = (0.0, 0.0);
                for p in ring {
                    clon += p[0];
                    clat += p[1];
                }
                let cw = crate::render::mercator::lonlat_to_world(
                    clon / ring.len() as f64,
                    clat / ring.len() as f64,
                );
                let (sx, sy) = cam.world_to_screen(cw, vp);
                let c = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(c) {
                    continue;
                }
                let color = egui::Color32::from_rgb(f.stroke[0], f.stroke[1], f.stroke[2]);
                let font = egui::FontId::proportional(11.0);
                let galley =
                    painter.layout_no_wrap(f.title.clone(), font.clone(), egui::Color32::BLACK);
                let rect = egui::Rect::from_center_size(c, galley.size() + egui::vec2(8.0, 4.0));
                painter.rect_filled(rect, 3.0, color);
                painter.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    &f.title,
                    font,
                    egui::Color32::BLACK,
                );
            }
        }

        // Warning intelligence: warned-storm motion vector + projected path + ETA to markers, and
        // a pulsing outline on escalated (Tornado Emergency / PDS / destructive) warnings.
        if self.filters.show_alerts {
            let to_screen = |lon: f64, lat: f64| {
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            let mut any_escalated = false;
            // Indices, not strings: the `take(6)` below drew six of them however many marker ×
            // warning pairs were formatted.
            let mut etas: Vec<(f64, usize, usize)> = Vec::new();
            let time = ctx.input(|i| i.time);
            // Viewport-center lon/lat: a polygon with every vertex off-screen can still fill the
            // whole pane (zoomed inside it) — the primary chase case for an escalated warning.
            let (center_lon, center_lat) = {
                let w = cam.screen_to_world((vp.0 * 0.5, vp.1 * 0.5), vp);
                crate::render::mercator::world_to_lonlat(w.0, w.1)
            };
            let features = self.active_alert_features();
            for (fi, f) in features.iter().enumerate() {
                let Some(a) = &f.alert else { continue };
                // Pulsing outline for escalated warnings only — watches can carry PDS wording,
                // but pulsing a state-sized watch polygon would drown the map (and `escalation`
                // uppercases the whole bulletin, too heavy to run for every alert every frame).
                if f.kind == overlay::FeatureKind::Warning && wxdata::alerts::escalation(a) >= 2 {
                    let visible =
                        f.rings.first().is_some_and(|r| {
                            r.iter().any(|p| prect.contains(to_screen(p[0], p[1])))
                        }) || f.contains(center_lon, center_lat);
                    if visible {
                        any_escalated = true;
                        let w = 2.0 + 2.0 * (time * 4.0).sin().abs() as f32;
                        let col = egui::Color32::from_rgb(255, 40, 40);
                        for ring in &f.rings {
                            let pts: Vec<egui::Pos2> =
                                ring.iter().map(|p| to_screen(p[0], p[1])).collect();
                            if pts.len() >= 2 {
                                painter.add(egui::Shape::line(pts, egui::Stroke::new(w, col)));
                            }
                        }
                    }
                }
                // Motion vector + projected path (heading = FROM + 180).
                let Some(m) = &a.motion else { continue };
                let Some(&origin) = m.points.first() else {
                    continue;
                };
                if m.kt < 1.0 {
                    continue;
                }
                let heading = ((m.deg + 180.0) % 360.0) as f64;
                let apex = to_screen(origin[0], origin[1]);
                let col = egui::Color32::from_rgb(255, 235, 90);
                painter.circle_filled(apex, 4.0, col);
                let mut prev = apex;
                for min in [15.0_f64, 30.0, 45.0, 60.0] {
                    let km = m.kt as f64 * 1.852 * (min / 60.0);
                    let tp = crate::geo::destination_point(origin, heading, km);
                    let p = to_screen(tp[0], tp[1]);
                    painter.line_segment([prev, p], egui::Stroke::new(1.5, col));
                    painter.circle_filled(p, 2.5, col);
                    if cam.zoom >= 7.0 {
                        painter.text(
                            p + egui::vec2(5.0, -2.0),
                            egui::Align2::LEFT_CENTER,
                            format!("+{min:.0}m"),
                            egui::FontId::proportional(10.0),
                            col,
                        );
                    }
                    prev = p;
                }
                // ETA to any watched marker along the storm's heading.
                for (mi, mk) in self.settings.markers.iter().enumerate() {
                    if let Some(t) = crate::geo::arrival_eta_min(
                        origin,
                        heading as f32,
                        m.kt,
                        [mk.lon, mk.lat],
                        22.5,
                        90.0,
                    ) {
                        etas.push((t, mi, fi));
                    }
                }
            }
            if idx == self.active && !etas.is_empty() {
                etas.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let font = egui::FontId::proportional(12.0);
                let mut y = prect.top() + 64.0;
                for (t, mi, fi) in etas.iter().take(6) {
                    let mk = &self.settings.markers[*mi];
                    let event = features[*fi]
                        .alert
                        .as_ref()
                        .map(|a| a.event.as_str())
                        .unwrap_or_default();
                    let line = format!("⚠ {} — {} in {:.0} min", mk.name, event, t);
                    let galley = painter.layout_no_wrap(line, font.clone(), egui::Color32::WHITE);
                    let anchor = egui::pos2(prect.left() + 8.0, y);
                    let bg =
                        egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                    painter.rect_filled(
                        bg,
                        3.0,
                        egui::Color32::from_rgba_unmultiplied(150, 30, 30, 210),
                    );
                    // The galley just measured, drawn — `painter.text` would lay the same string
                    // out a second time.
                    let size = galley.size();
                    painter.galley(anchor + egui::vec2(5.0, 2.0), galley, egui::Color32::WHITE);
                    y += size.y + 6.0;
                }
            }
            if any_escalated {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
        }

        // Surface obs (METAR station plots): fltCat-colored circle, wind barb, T/Td in °F.
        if self.show_metar && cam.zoom >= 6.0 {
            crate::prof_scope!("metar_plots");
            let show_labels = cam.zoom >= 7.0;
            let flt_color = |c: &str| match c {
                "VFR" => egui::Color32::from_rgb(60, 200, 90),
                "MVFR" => egui::Color32::from_rgb(80, 150, 240),
                "IFR" => egui::Color32::from_rgb(230, 60, 60),
                "LIFR" => egui::Color32::from_rgb(220, 60, 200),
                _ => egui::Color32::from_gray(180),
            };
            // Projected and clipped before either sort: everything off screen is skipped by the
            // loop below anyway, and the whole national set was being sorted twice to get there.
            let mut obs: Vec<(egui::Pos2, &wxdata::metar::SurfaceOb)> = self
                .metars
                .iter()
                .filter_map(|ob| {
                    let w = crate::render::mercator::lonlat_to_world(ob.lon, ob.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                    prect.contains(p).then_some((p, ob))
                })
                .collect();
            // Windiest-first so the strongest stations survive decluttering.
            obs.sort_by(|(_, a), (_, b)| {
                b.wspd_kt
                    .partial_cmp(&a.wspd_kt)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let temp_unit = self.settings.temp_unit;
            // Same stickiness rule as the place names: a station already plotted keeps its cell
            // ahead of a windier one that has only just come into view.
            obs.sort_by_key(|(_, ob)| !self.labels.was_shown(crate::labelplace::key(&ob.icao)));
            for (p, ob) in obs {
                // Shared declutter: this also sees the place names drawn above it.
                let cell = egui::Rect::from_center_size(p, egui::vec2(44.0, 34.0));
                if !self.labels.place(
                    crate::labelplace::key(&ob.icao),
                    cell,
                    crate::labelplace::Priority::Station,
                ) {
                    continue;
                }
                let col = flt_color(&ob.flt_cat);
                painter.circle_stroke(p, 3.0, egui::Stroke::new(1.5, col));
                // Wind barb, rotated so the shaft points toward the wind source (FROM bearing).
                if let Some(dir) = ob.wdir_deg {
                    let th = dir.to_radians();
                    let (up, right) = ([th.sin(), -th.cos()], [th.cos(), th.sin()]);
                    let map = |u: [f32; 2]| {
                        p + egui::vec2(
                            (u[0] * right[0] + u[1] * up[0]) * 22.0,
                            (u[0] * right[1] + u[1] * up[1]) * 22.0,
                        )
                    };
                    for (a, b) in wxdata::metar::barb_segments(ob.wspd_kt) {
                        painter.line_segment([map(a), map(b)], egui::Stroke::new(1.3, col));
                    }
                }
                // Temperature (red, upper-left) and dewpoint (green, lower-left) in °F.
                if show_labels {
                    let f = egui::FontId::proportional(11.0);
                    if let Some(t) = ob.temp_c {
                        painter.text(
                            p + egui::vec2(-6.0, -6.0),
                            egui::Align2::RIGHT_BOTTOM,
                            format!("{:.0}", temp_unit.from_c(t)),
                            f.clone(),
                            egui::Color32::from_rgb(240, 90, 90),
                        );
                    }
                    if let Some(d) = ob.dewp_c {
                        painter.text(
                            p + egui::vec2(-6.0, 6.0),
                            egui::Align2::RIGHT_TOP,
                            format!("{:.0}", temp_unit.from_c(d)),
                            f,
                            egui::Color32::from_rgb(90, 220, 120),
                        );
                    }
                    // Sea state, under the plot — buoys only, by construction.
                    if let Some(h) = ob.wvht_ft {
                        let period = ob
                            .dpd_s
                            .map(|s| format!(" {s:.0}s"))
                            .unwrap_or_else(String::new);
                        painter.text(
                            p + egui::vec2(0.0, 9.0),
                            egui::Align2::CENTER_TOP,
                            format!("{h:.1}ft{period}"),
                            egui::FontId::proportional(10.0),
                            egui::Color32::from_rgb(120, 200, 230),
                        );
                    }
                }
                // Hover → the raw METAR text.
                let hit = egui::Rect::from_center_size(p, egui::vec2(16.0, 16.0));
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) && !ob.raw.is_empty() {
                    // Observation first, then the terminal forecast where the station files one:
                    // what it is doing, then what it is expected to do.
                    let text = match self.tafs.get(&ob.icao) {
                        Some(taf) => format!("{}\n\n{taf}", ob.raw),
                        None => ob.raw.clone(),
                    };
                    response.clone().show_tooltip_text(text);
                }
            }
        }
        // ponytail: the station plots follow the Units setting; the gridded contour labels
        // (K → °F, `field_ramps`) still do not, and want the same treatment when asked.

        // River flood gauges (NWPS): category-colored inverted-triangle droplet + stage tooltip.
        // ponytail: hover tooltip carries name/stage/forecast; skipped a click→Detail popup — the
        // hover already answers "how high is this river", add the popup if users want to pin it.
        if self.show_gauges && cam.zoom >= 6.0 {
            use wxdata::river::FloodCat;
            let gcolor = |c: FloodCat| match c {
                FloodCat::Major => egui::Color32::from_rgb(170, 60, 220),
                FloodCat::Moderate => egui::Color32::from_rgb(230, 40, 40),
                FloodCat::Minor => egui::Color32::from_rgb(255, 140, 0),
                FloodCat::Action => egui::Color32::from_rgb(240, 200, 40),
                FloodCat::NoFlooding => egui::Color32::from_rgb(80, 200, 220),
                FloodCat::Unknown => egui::Color32::from_gray(150),
            };
            let glabel = |c: FloodCat| match c {
                FloodCat::Major => "major flooding",
                FloodCat::Moderate => "moderate flooding",
                FloodCat::Minor => "minor flooding",
                FloodCat::Action => "action stage",
                FloodCat::NoFlooding => "no flooding",
                FloodCat::Unknown => "no current reading",
            };
            // Already-drawn gauges get their slot back before a newcomer takes it, the same way
            // the METAR and place-name layers already do. Without it a gauge at the edge of a
            // collision wins and loses on alternate frames, which reads as flicker while panning.
            //
            // Two passes — returning labels, then the rest — rather than sorting into a vector
            // that would be allocated and thrown away on every frame.
            for returning in [true, false] {
                crate::prof_scope!("river_gauges");
                for g in &self.gauges {
                    if self.labels.was_shown(crate::labelplace::key(&g.lid)) != returning {
                        continue;
                    }
                    let w = crate::render::mercator::lonlat_to_world(g.lon, g.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                    if !prect.contains(p) {
                        continue;
                    }
                    // Shared declutter: gauges are the lowest tier, so they fill what is left.
                    let cell = egui::Rect::from_center_size(p, egui::vec2(15.0, 15.0));
                    if !self.labels.place(
                        crate::labelplace::key(&g.lid),
                        cell,
                        crate::labelplace::Priority::Minor,
                    ) {
                        continue;
                    }
                    let s = 6.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            p + egui::vec2(-s * 0.85, -s * 0.6),
                            p + egui::vec2(s * 0.85, -s * 0.6),
                            p + egui::vec2(0.0, s),
                        ],
                        gcolor(g.cat).gamma_multiply(0.85),
                        egui::Stroke::new(1.2, egui::Color32::from_gray(20)),
                    ));
                    let hit = egui::Rect::from_center_size(p, egui::vec2(16.0, 16.0));
                    if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                        let stage = g
                            .stage_ft
                            .map_or_else(|| "n/a".to_string(), |v| format!("{v:.1} ft"));
                        let mut tip =
                            format!("{} ({})\n{stage} — {}", g.name, g.lid, glabel(g.cat));
                        if let Some(f) = g.forecast_ft {
                            tip.push_str(&format!(
                                "\nFcst: {f:.1} ft ({})",
                                glabel(g.forecast_cat)
                            ));
                        }
                        response.clone().show_tooltip_text(tip);
                    }
                }
            }
        }

        // County power outages: hatching, not a fill — see `outage_draw`.
        if self.show_outages && !self.outage_features.is_empty() {
            crate::outage_draw::draw(&painter, &self.outage_features, prect, |lon, lat| {
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            });
        }
        // NHC tropical suite: dashed cone edge, forecast track, and per-point callouts.
        if self.show_tropical {
            if let Some(t) = &self.tropical {
                crate::tropical_draw::draw(&painter, t, prect, cam.zoom as f32, |lon, lat| {
                    let w = crate::render::mercator::lonlat_to_world(lon, lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    egui::pos2(prect.left() + sx, prect.top() + sy)
                });
            }
        }

        // HRRR "future radar" banner — unmistakable that this is model forecast, not observation.
        if idx == self.active && view.fields_on.contains(&crate::render::FieldLayer::Hrrr) {
            let valid = self
                .hrrr_valid
                .map(|v| crate::timefmt::fmt_date_clock(v, self.active_tz()))
                .unwrap_or_else(|| "loading…".to_string());
            let text = format!(
                "⚠ FORECAST +{}h — HRRR MODEL, NOT OBSERVED — valid {}",
                self.hrrr_fcst_hour, valid
            );
            let font = egui::FontId::proportional(13.0);
            let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::BLACK);
            let pad = egui::vec2(10.0, 4.0);
            let center = egui::pos2(prect.center().x, prect.top() + 16.0);
            let rect = egui::Rect::from_center_size(center, galley.size() + pad * 2.0);
            painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(255, 170, 60));
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                &text,
                font,
                egui::Color32::BLACK,
            );
        }

        // Placefile labels/icons.
        for label in placefile_labels {
            // An anchored label is placed by projecting its object's anchor and then stepping the
            // stated pixels from it (y up), so it holds its offset as the map zooms.
            let (base, off) = match label.anchor {
                Some(a) => (a, egui::vec2(label.pos[0] as f32, -label.pos[1] as f32)),
                None => (label.pos, egui::Vec2::ZERO),
            };
            let w = crate::render::mercator::lonlat_to_world(base[0], base[1]);
            let (sx, sy) = cam.world_to_screen(w, vp);
            let p = egui::pos2(prect.left() + sx, prect.top() + sy) + off;
            if !prect.contains(p) {
                continue;
            }
            let mut hit_size = egui::vec2(16.0, 16.0);
            match &label.kind {
                PlaceLabelKind::Text(text) => {
                    painter.text(
                        p,
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(12.0),
                        label.color,
                    );
                }
                PlaceLabelKind::Marker => {
                    painter.circle_stroke(p, 5.0, egui::Stroke::new(1.5, label.color));
                    painter.circle_filled(p, 1.5, label.color);
                }
                PlaceLabelKind::Sprite {
                    tex,
                    uv,
                    size,
                    hot,
                    angle,
                } => {
                    draw_sprite(&painter, *tex, *uv, p, *size, *hot, *angle, label.color);
                    hit_size = *size;
                }
            }
            if !label.hover.is_empty() {
                let hit = egui::Rect::from_center_size(p, hit_size);
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                    response.clone().show_tooltip_text(&label.hover);
                }
            }
        }

        // The inspector readout, sampled above. Every competitor has one and this had none: the
        // app could draw a 68 dBZ core and a velocity couplet and never tell you a single number
        // behind either.
        if let Some(text) = &gate_tooltip {
            response.clone().show_tooltip_text(text);
        }

        // The difference layer reads as "they disagree here" and nothing more without a number,
        // so the cursor samples the grid it was drawn from.
        if view
            .fields_on
            .contains(&crate::render::FieldLayer::ModelDiff)
        {
            if let (Some(grid), Some(hp)) = (self.diff_grid.as_ref(), response.hover_pos()) {
                let w = cam.screen_to_world((hp.x - prect.left(), hp.y - prect.top()), vp);
                let (lon, lat) = crate::render::mercator::world_to_lonlat(w.0, w.1);
                if let Some(v) = grid.sample_bilinear(lon, lat) {
                    let f = self.diff_field;
                    let v = v * f.input_scale();
                    let (a, b) = f.pair();
                    response.clone().show_tooltip_text(format!(
                        "{}: {v:+.1} {} ({a} \u{2212} {b})",
                        f.label(),
                        f.units()
                    ));
                }
            }
        }

        // Beam-vs-terrain blockage shading, under the reference annotations. The raster covers a
        // world-space rect, which maps linearly to screen, so it is one stretched image — and while
        // a rebuild is in flight the previous rect keeps it registered to the ground.
        if self.show_blockage {
            if let Some((_, tex, world)) = &self.blockage_tex {
                let a = cam.world_to_screen((world[0], world[1]), vp);
                let b = cam.world_to_screen((world[2], world[3]), vp);
                let rect = egui::Rect::from_two_pos(
                    egui::pos2(prect.left() + a.0, prect.top() + a.1),
                    egui::pos2(prect.left() + b.0, prect.top() + b.1),
                );
                painter.image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }

        // Range rings + azimuth spokes around this pane's site (feature HH).
        if self.show_range_rings {
            if let Some(site) = view.site.as_deref().and_then(wxdata::sites::site_by_id) {
                let origin = [site.longitude as f64, site.latitude as f64];
                let col = egui::Color32::from_gray(150).gamma_multiply(0.55);
                let to_screen = |lon: f64, lat: f64| {
                    let w = crate::render::mercator::lonlat_to_world(lon, lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    egui::pos2(prect.left() + sx, prect.top() + sy)
                };
                for km in [50.0, 100.0, 150.0, 200.0] {
                    let pts: Vec<egui::Pos2> = (0..=72)
                        .map(|i| {
                            let p = crate::geo::destination_point(origin, i as f64 * 5.0, km);
                            to_screen(p[0], p[1])
                        })
                        .collect();
                    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.0, col)));
                    if cam.zoom >= 6.0 {
                        let top = crate::geo::destination_point(origin, 0.0, km);
                        painter.text(
                            to_screen(top[0], top[1]),
                            egui::Align2::CENTER_BOTTOM,
                            format!("{km:.0} km"),
                            egui::FontId::proportional(10.0),
                            col,
                        );
                    }
                }
                for az in (0..360).step_by(45) {
                    let far = crate::geo::destination_point(origin, az as f64, 200.0);
                    painter.line_segment(
                        [to_screen(origin[0], origin[1]), to_screen(far[0], far[1])],
                        egui::Stroke::new(0.6, col.gamma_multiply(0.7)),
                    );
                }
            }
        }

        // Radar sites: a ring per site — both networks, so a TDWR you can select is a TDWR you can
        // see. The active site in accent, others muted. IDs only when zoomed in so the CONUS view
        // isn't cluttered. Click handled in the Interrogate tool.
        if self.show_radar_sites {
            let accent = crate::theme::accent(self.settings.theme);
            let current = self.views[idx].site.as_deref();
            // Sticky, for the same reason as the gauges above: a site id that wins and loses the
            // same collision on alternate frames is the flicker, not the collision.
            for returning in [true, false] {
                let show_labels = cam.zoom >= 5.0;
                for (s, w) in sites_in_world() {
                    if self.labels.was_shown(crate::labelplace::key(s.id)) != returning {
                        continue;
                    }
                    let (sx, sy) = cam.world_to_screen(*w, vp);
                    let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                    if !prect.contains(p) {
                        continue;
                    }
                    let is_current = current == Some(s.id);
                    let col = if is_current {
                        accent
                    } else {
                        egui::Color32::from_rgb(120, 190, 255)
                    };
                    let r = if is_current { 5.0 } else { 3.5 };
                    painter.circle_stroke(p, r, egui::Stroke::new(1.5, col));
                    painter.circle_filled(p, 1.5, col);
                    // The dot always draws — it is the click target, and it is small enough not to
                    // matter. Only the four-letter id competes for space, and it loses to city names:
                    // "TDAL" sitting across "Grapevine" is the exact overlap this pass exists for.
                    let id_rect = egui::Rect::from_min_size(
                        p + egui::vec2(6.0, -6.0),
                        egui::vec2(s.id.len() as f32 * 6.5, 12.0),
                    )
                    .expand(1.0);
                    if show_labels
                        && self.labels.place(
                            crate::labelplace::key(s.id),
                            id_rect,
                            crate::labelplace::Priority::Minor,
                        )
                    {
                        painter.text(
                            p + egui::vec2(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            s.id,
                            egui::FontId::monospace(10.0),
                            col,
                        );
                    }
                }
            }
        }

        // Location markers.
        for m in &self.settings.markers {
            let w = crate::render::mercator::lonlat_to_world(m.lon, m.lat);
            let (sx, sy) = cam.world_to_screen(w, vp);
            let p = egui::pos2(prect.left() + sx, prect.top() + sy);
            if !prect.contains(p) {
                continue;
            }
            let col = crate::theme::accent(self.settings.theme);
            // Home wears its watch radius: the ring is the ground truth for "within 20 miles",
            // and a circle you can see beats a number you have to trust.
            if m.home && m.alert_radius_mi > 0.0 {
                let km = m.alert_radius_mi * crate::geo::KM_PER_MILE;
                let edge = crate::geo::destination_point([m.lon, m.lat], 90.0, km);
                let ew = crate::render::mercator::lonlat_to_world(edge[0], edge[1]);
                let (ex, _) = cam.world_to_screen(ew, vp);
                let r = (prect.left() + ex - p.x).abs();
                if r > 4.0 && r < 4000.0 {
                    painter.circle_stroke(
                        p,
                        r,
                        egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 70),
                        ),
                    );
                }
            }
            // Uploaded icon if one is loaded; otherwise the default accent dot.
            let tex = m
                .icon
                .as_ref()
                .and_then(|n| self.marker_icon_tex.get(n))
                .and_then(|t| t.as_ref());
            let label_dx = if let Some(tex) = tex {
                // Round the icon into a disc with a white ring, so a marker reads as a map pin
                // rather than a photo pasted on the map. A corner radius of half the size is a
                // circle; the ring also separates a dark photo from a dark basemap.
                let d = crate::ui::marker_window::ICON_D;
                let r = egui::Rect::from_center_size(p, egui::vec2(d, d));
                painter.add(
                    egui::epaint::RectShape::filled(
                        r,
                        egui::CornerRadius::same((d / 2.0) as u8),
                        egui::Color32::WHITE,
                    )
                    .with_texture(
                        tex.id(),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    ),
                );
                painter.circle_stroke(
                    p,
                    d / 2.0,
                    egui::Stroke::new(1.5, egui::Color32::from_white_alpha(230)),
                );
                d / 2.0 + 2.0
            } else {
                painter.circle_filled(p, 4.0, col);
                painter.circle_stroke(p, 4.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                7.0
            };
            painter.text(
                p + egui::vec2(label_dx, 0.0),
                egui::Align2::LEFT_CENTER,
                &m.name,
                egui::FontId::proportional(12.0),
                col,
            );
        }

        // You, and anyone sharing their position with you. Drawn after the saved markers so a
        // moving dot is never hidden under a static one.
        let me = self.chase_pos.map(|(lon, lat)| crate::share::Peer {
            id: String::new(),
            name: "You".to_string(),
            lon,
            lat,
            ts: crate::share::now(),
            video_url: self.settings.share_video_url.clone(),
        });
        for (p, is_me) in me
            .iter()
            .map(|p| (p, true))
            .chain(self.peers.values().map(|p| (p, false)))
        {
            let w = crate::render::mercator::lonlat_to_world(p.lon, p.lat);
            let (sx, sy) = cam.world_to_screen(w, vp);
            let pt = egui::pos2(prect.left() + sx, prect.top() + sy);
            if !prect.contains(pt) {
                continue;
            }
            // You are blue (the convention every map app trained people on); peers are amber, and
            // fade as their fix ages so a frozen dot looks frozen.
            let col = if is_me {
                egui::Color32::from_rgb(60, 140, 255)
            } else {
                let age = (crate::share::now() - p.ts).clamp(0, crate::share::STALE_SECS) as f32;
                let a = 255.0 - 155.0 * (age / crate::share::STALE_SECS as f32);
                egui::Color32::from_rgba_unmultiplied(255, 180, 60, a as u8)
            };
            painter.circle_filled(
                pt,
                9.0,
                egui::Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 45),
            );
            painter.circle_filled(pt, 5.0, col);
            painter.circle_stroke(pt, 5.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
            let label = if is_me {
                p.name.clone()
            } else {
                let mins = (crate::share::now() - p.ts) / 60;
                if mins > 0 {
                    format!("{} ({mins}m)", p.name)
                } else {
                    p.name.clone()
                }
            };
            painter.text(
                pt + egui::vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.0),
                col,
            );
        }

        // Freehand annotation strokes. Painted with the rest of the tool graphics so they sit
        // above every overlay, and drawn in OBS mode too — circling a storm on a stream is the
        // whole point of the tool.
        for st in &self.strokes {
            if st.points.len() < 2 {
                continue;
            }
            let pts: Vec<egui::Pos2> = st
                .points
                .iter()
                .map(|ll| {
                    let w = crate::render::mercator::lonlat_to_world(ll[0], ll[1]);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    egui::pos2(prect.left() + sx, prect.top() + sy)
                })
                .collect();
            painter.add(egui::Shape::line(pts, egui::Stroke::new(2.5, st.color)));
        }

        // Saved watch zones, plus the one being clicked out right now.
        {
            let screen = |ll: [f64; 2]| {
                let w = crate::render::mercator::lonlat_to_world(ll[0], ll[1]);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            let zone_col = egui::Color32::from_rgb(120, 200, 255);
            for z in &self.settings.alert_polygons {
                if z.ring.len() < 3 {
                    continue;
                }
                let pts: Vec<egui::Pos2> = z.ring.iter().map(|&p| screen(p)).collect();
                painter.add(egui::Shape::convex_polygon(
                    pts.clone(),
                    egui::Color32::from_rgba_unmultiplied(120, 200, 255, 26),
                    egui::Stroke::new(1.5, zone_col),
                ));
                if let Some(first) = pts.first() {
                    painter.text(
                        *first + egui::vec2(4.0, -4.0),
                        egui::Align2::LEFT_BOTTOM,
                        &z.name,
                        egui::FontId::proportional(11.0),
                        zone_col,
                    );
                }
            }
            if !self.zone_pts.is_empty() {
                let pts: Vec<egui::Pos2> = self.zone_pts.iter().map(|&p| screen(p)).collect();
                for p in &pts {
                    painter.circle_filled(*p, 3.0, zone_col);
                }
                if pts.len() >= 2 {
                    // Closing edge dashed in, so the shape being made is obvious before it is.
                    let mut loop_pts = pts.clone();
                    loop_pts.push(pts[0]);
                    painter.add(egui::Shape::line(
                        loop_pts,
                        egui::Stroke::new(1.5, zone_col),
                    ));
                }
            }
        }

        // Chase breadcrumbs: where this session has been, under everything else on the map.
        if self.settings.chase_log && self.chase_track.points.len() > 1 {
            let col = egui::Color32::from_rgb(255, 170, 60);
            let pts: Vec<egui::Pos2> = self
                .chase_track
                .points
                .iter()
                .map(|f| {
                    let w = crate::render::mercator::lonlat_to_world(f.lon, f.lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    egui::pos2(prect.left() + sx, prect.top() + sy)
                })
                .collect();
            painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(2.0, col)));
            for (idx, label) in &self.chase_track.waypoints {
                if let Some(p) = pts.get(*idx) {
                    painter.circle_filled(*p, 4.0, col);
                    painter.text(
                        *p + egui::vec2(6.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(11.0),
                        col,
                    );
                }
            }
        }

        // Measure tool.
        if !self.measure.is_empty() {
            let col = egui::Color32::from_rgb(255, 210, 80);
            let screen = |ll: [f64; 2]| {
                let w = crate::render::mercator::lonlat_to_world(ll[0], ll[1]);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            for &pt in &self.measure {
                painter.circle_filled(screen(pt), 3.5, col);
            }
            if self.measure.len() == 2 {
                let (a, b) = (screen(self.measure[0]), screen(self.measure[1]));
                painter.line_segment([a, b], egui::Stroke::new(2.0, col));
                let (km, brg) = crate::geo::great_circle(self.measure[0], self.measure[1]);
                let mut txt = format!(
                    "{}  @ {brg:.0}°",
                    crate::geo::fmt_distance(km, self.metric_in(idx), 1)
                );
                // How high the beam is over the far end of the line. The number that decides
                // whether "there's nothing on radar there" means the storm is weak or means the
                // scan is looking over its head, and until now it lived only in the cross-section.
                if let Some(h) = self.beam_height_ft(idx, self.measure[1]) {
                    txt.push_str(&format!("  ·  beam {h:.0} ft"));
                }
                let mid = a + (b - a) * 0.5;
                painter.text(
                    mid + egui::vec2(0.0, -10.0),
                    egui::Align2::CENTER_BOTTOM,
                    txt,
                    egui::FontId::proportional(12.0),
                    col,
                );
            }
        }

        // Cross-section endpoints + line (cyan, distinct from the yellow measure tool).
        if !self.xsection_pts.is_empty() {
            let col = egui::Color32::from_rgb(90, 220, 255);
            let screen = |ll: [f64; 2]| {
                let w = crate::render::mercator::lonlat_to_world(ll[0], ll[1]);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            for &pt in &self.xsection_pts {
                painter.circle_filled(screen(pt), 3.5, col);
            }
            if self.xsection_pts.len() == 2 {
                let (a, b) = (screen(self.xsection_pts[0]), screen(self.xsection_pts[1]));
                painter.line_segment([a, b], egui::Stroke::new(2.0, col));
                painter.text(
                    a,
                    egui::Align2::RIGHT_BOTTOM,
                    "A",
                    egui::FontId::proportional(12.0),
                    col,
                );
                painter.text(
                    b,
                    egui::Align2::LEFT_BOTTOM,
                    "B",
                    egui::FontId::proportional(12.0),
                    col,
                );
            }
        }

        // Historical tornado tracks from the last climatology query (magnitude-colored segments).
        if self.climo_open && !self.climo_hits.is_empty() {
            let screen = |lon: f64, lat: f64| {
                let w = crate::render::mercator::lonlat_to_world(lon, lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                egui::pos2(prect.left() + sx, prect.top() + sy)
            };
            for t in &self.climo_hits {
                let col = tornado_mag_color(t.mag);
                let a = screen(t.slon, t.slat);
                let b = screen(t.elon, t.elat);
                if !prect.contains(a) && !prect.contains(b) {
                    continue;
                }
                painter.line_segment([a, b], egui::Stroke::new(2.0, col));
                painter.circle_filled(a, 2.5, col);
            }
            if let Some((lon, lat)) = self.climo_center {
                let c = screen(lon, lat);
                painter.circle_stroke(c, 5.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
            }
        }

        // The boxed legend is desktop-only; Android draws a full-width color scale in the mobile
        // chrome (see `app::mobile`), so drawing both would be redundant.
        if view.show_legend && !cfg!(target_os = "android") {
            // The moment's scale floats over this pane's right edge (no panel, no card) so the map
            // keeps the pixels; the field/wind ramps still need their cards.
            if view.volume.is_some() {
                let (df, dl) = display_units(view.moment, &self.settings);
                ui::legend::draw_vertical(
                    &painter,
                    prect,
                    view.moment,
                    self.palettes.table(view.moment),
                    view.active_threshold(),
                    df,
                    dl,
                );
            }
            // The field cards stack down the pane's top-left corner, which in the full-overlay
            // chrome is where the search pill floats — the first card was drawn half under it.
            // Every pane ducks by the same amount rather than only the top row: in a 2x2 grid the
            // lower cards then sit a little further from their pane's edge, which nobody notices,
            // and the alternative is a rect comparison that has to know about window insets.
            let mut y = 48.0;
            // Whichever gridded layer the user actually sees on top — the last enabled one in
            // paint order — gets its scale keyed underneath. Without this, MESH/QPE/VIL and the
            // categorical classifications were unlabeled color.
            if let Some(top) = crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .rev()
                .find(|l| view.fields_on.contains(l))
            {
                y += if *top == crate::render::FieldLayer::ModelDiff {
                    ui::legend::draw_diff(&painter, prect, self.diff_field, y)
                } else {
                    ui::legend::draw_field(&painter, prect, *top, y)
                };
            }
            // Wind particles carry their own scale — it isn't a FieldLayer, so it needs its own
            // call rather than a slot in DRAW_ORDER.
            if self.show_wind && self.wind.is_some() {
                ui::legend::draw_ramp(&painter, prect, &crate::render::field_ramps::WIND, y);
            }
        }
    }

    /// Resize the pane grid to `n` (1/2/4). New panes copy the active pane's site/camera but
    /// default to a distinct product, so a 4-panel shows REF/VEL/ZDR/RHO out of the box.
    /// Four panes of the SAME product at four different tilts — the layout you build by hand
    /// every time you want to see how a couplet leans with height.
    ///
    /// SAILS/MRLE re-scan the lowest cut mid-volume, so the elevation list repeats angles; taking
    /// four *distinct* ones is what makes the quad show four heights instead of three plus a
    /// duplicate.
    fn apply_all_tilts(&mut self) {
        let src = &self.views[self.active];
        let moment = src.moment;
        let srv = src.srv;
        let elevations = src
            .volume
            .as_ref()
            .map(|v| v.elevations.clone())
            .unwrap_or_default();
        let picks = distinct_tilts(&elevations, 4);
        self.set_pane_count(4);
        for (i, v) in self.views.iter_mut().enumerate() {
            v.moment = moment;
            v.srv = srv;
            if let Some(&t) = picks.get(i) {
                v.tilt = t;
            }
        }
        // Four heights of one storm only reads if all four look at the same place.
        self.link_cameras = true;
        self.pane_shown.clear();
    }

    /// How visible the wind layer should be in this pane: faded out past the zoom where the
    /// 0.04-degree regrid goes visibly piecewise-linear, dimmed over reflectivity so the radar
    /// stays readable, and dimmed again when the pane is scrubbed to a time these grids are not
    /// valid for.
    fn wind_alpha(&self, idx: usize, zoom: f64) -> f32 {
        let zoom_fade = (13.0 - zoom).clamp(0.0, 1.0) as f32;
        let v = &self.views[idx];
        let over_radar = v.show_radar
            && matches!(v.moment, wxdata::level2::Moment::Reflectivity)
            && v.volume.is_some();
        let off_live = !v.timeline.following;
        zoom_fade * if over_radar { 0.7 } else { 1.0 } * if off_live { 0.4 } else { 1.0 }
    }

    /// What the GPU wind layer needs this frame: the field to upload if it changed, and the
    /// per-frame camera and timing. `None` when the layer is off, faded out, or on the CPU.
    fn wind_gpu_frame(
        &mut self,
        idx: usize,
        cam: &Camera,
        vp: (f32, f32),
    ) -> (
        Option<Box<crate::wind_gpu::WindGrid>>,
        Option<crate::wind_gpu::Frame>,
    ) {
        let alpha = self.wind_alpha(idx, cam.zoom);
        if !self.show_wind || !self.wind_on_gpu || alpha <= 0.01 {
            return (None, None);
        }
        let Some(field) = self.wind.as_ref() else {
            return (None, None);
        };
        // The grid's own bbox in mercator world units — the particles live in it, so it is also
        // the space their positions are stored in.
        let (west, north) =
            crate::render::mercator::lonlat_to_world(field.u.lon_west, field.u.lat_north);
        let (east, south) =
            crate::render::mercator::lonlat_to_world(field.u.lon_east, field.u.lat_south);
        let bbox_min = [west as f32, north as f32];
        let bbox_max = [east as f32, south as f32];

        // Warp the lon/lat grid onto the mercator-uniform texture the shader samples. Once per
        // new field, on the frame it arrives — not per frame, and not on the GPU, where it would
        // cost a dependent texture fetch in the hot path.
        let key = (field.run, field.fcst_hour, field.level);
        let upload = (self.wind_uploaded != Some(key)).then(|| {
            self.wind_uploaded = Some(key);
            Box::new(crate::wind_gpu::WindGrid {
                rgba: crate::wind_gpu::warp_field(field, bbox_min, bbox_max),
                bbox_min,
                bbox_max,
            })
        });

        let (center, scale) = cam.world_to_clip_uniform(vp);
        (
            upload,
            Some(crate::wind_gpu::Frame {
                bbox_min,
                bbox_max,
                center,
                scale,
                dt: self.wind_dt,
                // Pixels per world unit is the camera's own scale factor; the particle step is
                // calibrated in pixels, so the shader needs its inverse.
                world_per_px: (1.0 / (256.0 * 2f64.powf(cam.zoom))) as f32,
                opacity: alpha,
                viewport: (vp.0 as u32, vp.1 as u32),
            }),
        )
    }

    /// Snapshot the current arrangement. Auto-named: naming things is a peacetime activity.
    fn capture_workspace(&mut self) -> crate::workspace::Workspace {
        let overlays_on: Vec<String> = OverlayToggle::ALL
            .into_iter()
            .filter(|t| !t.session_only() && *self.overlay_flag(*t))
            .map(|t| t.slug())
            .collect();
        crate::workspace::Workspace {
            name: format!("Workspace {}", self.settings.workspaces.len() + 1),
            panes: self
                .views
                .iter()
                .map(crate::workspace::PaneSnap::capture)
                .collect(),
            active: self.active,
            link_cameras: self.link_cameras,
            overlays_on,
            // A workspace you saved records the sites you had open; only the shipped starters
            // adopt whatever is on screen.
            adopt_site: false,
            // The workspace-wide list stays as the union across panes: it is what an older build
            // reads, and what a pane snapshot written before per-pane layers falls back to.
            fields_on: crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .filter(|l| self.field_wanted(**l))
                .map(|l| l.slug().to_string())
                .collect(),
            chrome: Some(self.capture_chrome()),
        }
    }

    /// Restore a saved arrangement. Panes come back empty of data and fill through the normal
    /// poll, exactly as a freshly split pane does.
    fn apply_workspace(&mut self, ws: &crate::workspace::Workspace, ctx: &egui::Context) {
        if ws.panes.is_empty() {
            return;
        }
        let adopted = ws
            .adopt_site
            .then(|| self.views[self.active].site.clone())
            .flatten();
        self.set_pane_count(ws.panes.len());
        for (v, snap) in self.views.iter_mut().zip(&ws.panes) {
            snap.apply(v);
            if v.site.is_none() {
                if let Some(site) = &adopted {
                    v.site = Some(site.clone());
                    // The starter's camera is a placeholder over the plains; let the site
                    // recenter this pane the way a fresh one does.
                    v.camera_placed = false;
                }
            }
        }
        self.active = ws.active.min(self.views.len() - 1);
        self.link_cameras = ws.link_cameras;
        // Overlay names this build doesn't know are skipped, same as the settings restore.
        for t in OverlayToggle::ALL {
            if t.session_only() {
                continue;
            }
            *self.overlay_flag(t) = ws.overlays_on.iter().any(|s| *s == t.slug());
        }
        // Same rule for the national field layers: an unknown slug is a layer this build
        // doesn't have, which is a thing to skip rather than an error.
        for (v, snap) in self.views.iter_mut().zip(&ws.panes) {
            // A pane snapshot written before per-pane layers existed carries `None`, and falls
            // back to the workspace-wide list so an old file still restores what it meant. An
            // empty list is a pane that had its layers off, which is a decision, not a gap.
            let list = snap.fields_on.as_ref().unwrap_or(&ws.fields_on);
            v.fields_on = crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .copied()
                .filter(|l| list.iter().any(|s| s == l.slug()))
                .collect();
        }
        if let Some(c) = &ws.chrome {
            self.apply_chrome(c, ctx);
        }
        self.rebuild_overlays();
        self.pane_shown.clear();
    }

    fn set_pane_count(&mut self, n: usize) {
        let n = n.clamp(1, 4);
        while self.views.len() < n {
            let src = &self.views[self.active];
            let (site, camera, basemap, tilt, date) = (
                src.site.clone(),
                src.camera,
                src.basemap,
                src.tilt,
                src.timeline.date,
            );
            // Split a scrubbed view and the new panes have to land on the same instant, not on
            // live. Without this, splitting an archive view gave one populated pane and three
            // empty ones, each quietly polling today's head for a site that has no storm on it.
            let seek = (!src.timeline.following)
                .then(|| src.timeline.current().and_then(|id| id.date_time()))
                .flatten();
            let mut v = MapView::new(site, camera);
            v.smooth = self.settings.smooth_radar;
            v.basemap = basemap;
            v.tilt = tilt;
            v.timeline.date = date;
            v.timeline.following = seek.is_none();
            v.timeline.seek_target = seek;
            v.moment = Moment::ALL[self.views.len() % Moment::ALL.len()];
            self.views.push(v);
        }
        self.views.truncate(n);
        if self.active >= n {
            self.active = n - 1;
        }
        self.pane_shown.clear();
    }

    /// The app's own commands — the ones that aren't a layer, product, tool or window, and so
    /// have no place in the action registry: view toggles, chase, capture, settings bundles.
    /// Rendered inside the drawer, and (for one more commit) inside the old More popup.
    fn app_rows(&mut self, ui: &mut egui::Ui) {
        let metric = self.metric();
        {
            ui.label(egui::RichText::new("View").strong());
            {
                let v = &mut self.views[self.active];
                let mut on = v.basemap != crate::tiles::BasemapStyle::None;
                if ui.checkbox(&mut on, "Basemap").changed() {
                    v.basemap = if on {
                        crate::tiles::BasemapStyle::default()
                    } else {
                        crate::tiles::BasemapStyle::None
                    };
                }
                ui.checkbox(&mut v.show_radar, "Radar");
                ui.checkbox(&mut v.show_legend, "Legend");
            }
            if ui
                .checkbox(&mut self.obs_mode, "Streamer / OBS mode (F8)")
                .on_hover_text(
                    "Hide all panels, leaving only the map — clean capture for streaming",
                )
                .changed()
                && !self.obs_mode
            {
                self.obs_tour = false;
            }
            if ui
                .checkbox(&mut self.obs_tour, "Auto-tour active warnings (F9)")
                .on_hover_text("Cycle the camera through active warning polygons every ~12 s")
                .changed()
            {
                self.obs_tour_last = None;
                if self.obs_tour {
                    self.obs_mode = true;
                }
            }

            ui.separator();
            ui.label(egui::RichText::new("Chase").strong());
            if ui
                .checkbox(&mut self.chase_mode, "Chase mode (follow me)")
                .changed()
                && !self.chase_mode
            {
                self.chase_applied = None;
            }
            if self.chase_mode {
                match self
                    .chase_pos
                    .and_then(|(lon, lat)| crate::geo::nearest_site_id(lon, lat))
                {
                    Some(s) => ui.weak(format!("nearest radar: {s}")),
                    None => ui.weak("pick a location with Tool: Set chase location"),
                };
            }
            ui.checkbox(&mut self.settings.chase_log, "Log the chase")
                .on_hover_text(
                    "Record a breadcrumb track of your GPS fixes, to draw on the map and save \
                     as GPX. In memory until you save it; nothing is uploaded.",
                );
            if self.settings.chase_log && !self.chase_track.points.is_empty() {
                ui.weak(format!(
                    "{} points · {}",
                    self.chase_track.points.len(),
                    crate::geo::fmt_distance(self.chase_track.km(), metric, 0)
                ));
                ui.horizontal(|ui| {
                    if ui
                        .button("\u{1f4cd} Mark")
                        .on_hover_text("Name this spot in the track (saved into the GPX)")
                        .clicked()
                    {
                        let n = self.chase_track.waypoints.len() + 1;
                        self.chase_track.mark(format!("Mark {n}"));
                    }
                    if ui.button("Save GPX…").clicked() {
                        let gpx = self.chase_track.to_gpx();
                        match crate::dialog::save_bytes("chase.gpx", "gpx", gpx.as_bytes()) {
                            crate::dialog::Saved::Where(w) => {
                                self.toast(ToastKind::Success, format!("Saved to {w}"))
                            }
                            crate::dialog::Saved::Failed(e) => {
                                self.toast(ToastKind::Error, format!("GPX save failed: {e}"))
                            }
                            crate::dialog::Saved::Cancelled => {}
                        }
                    }
                    if ui
                        .button("Clear")
                        .on_hover_text("Forget the track so far")
                        .clicked()
                    {
                        self.chase_track.clear();
                    }
                });
            }
            // Desktop streams from a local gpsd; Android polls the system LocationManager over
            // JNI (see platform.rs); the web watches the browser's own Geolocation. All three
            // feed the same `gps_rx` channel.
            if self.gps_rx.is_none() {
                let (label, tip) = if cfg!(target_os = "android") {
                    (
                        "Enable GPS (chase)",
                        "Follow your device's position (asks for the location permission)",
                    )
                } else if cfg!(target_arch = "wasm32") {
                    (
                        "Enable GPS (chase)",
                        "Follow your position (asks the browser for the location permission)",
                    )
                } else {
                    (
                        "Connect GPS (gpsd)",
                        "Stream your live position from a local gpsd on :2947",
                    )
                };
                if ui.button(label).on_hover_text(tip).clicked() {
                    let rx = if cfg!(target_os = "android") {
                        crate::platform::start_location()
                    } else {
                        crate::gps::spawn()
                    };
                    match rx {
                        Some(rx) => {
                            self.gps_rx = Some(rx);
                            self.chase_mode = true;
                        }
                        None => log::warn!("no position source available"),
                    }
                }
            } else {
                // getLastKnownLocation is null until the first fix lands (cold start,
                // indoors, or permission still pending) — say so rather than look dead.
                if self.chase_pos.is_some() {
                    ui.weak("📡 GPS connected");
                } else {
                    ui.weak("📡 waiting for GPS fix…");
                }
                if ui.button("Disconnect GPS").clicked() {
                    self.gps_rx = None;
                }
            }
            // Position sharing: the phone in the field and the desktop at home showing each other
            // as dots on the same radar. LAN needs no setup; the relay covers cellular.
            ui.checkbox(&mut self.settings.share_position, "Share my position")
                .on_hover_text(
                    "Broadcast your GPS fix to other HookEcho instances, and show theirs",
                );
            if self.settings.share_position {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.share_name)
                            .hint_text("me")
                            .desired_width(120.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Relay");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.share_relay)
                            .hint_text("https://… (optional)")
                            .desired_width(180.0),
                    )
                    .on_hover_text(
                        "HTTP endpoint you host: POST a position, GET the list. Leave empty for \
                         same-network sharing only. The endpoint sees your live position.",
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Stream");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.share_video_url)
                            .hint_text("https://… (optional)")
                            .desired_width(180.0),
                    )
                    .on_hover_text(
                        "A live-video URL published with your dot, so partners can click it and \
                         watch. Direct HLS/MJPEG plays in-app; YouTube and Twitch open a browser.",
                    );
                });
                match self.peers.len() {
                    0 => ui.weak("no one else sharing yet"),
                    n => ui.weak(format!("👥 {n} sharing")),
                };
            }

            ui.separator();
            ui.label(egui::RichText::new("Weather radio").strong());
            self.nwr_rows(ui);

            ui.separator();
            ui.label(egui::RichText::new("Capture").strong());
            if ui.button("Save screenshot…").clicked() {
                if let Some(path) = crate::dialog::save_path("hookecho.png", "png") {
                    self.request_capture(ui.ctx(), ShotDest::File(path));
                }
            }
            if ui.button("Copy view to clipboard").clicked() {
                self.request_capture(ui.ctx(), ShotDest::Clipboard);
            }
            ui.checkbox(&mut self.settings.share_card, "Caption shared images")
                .on_hover_text(
                    "Stamp the site, product, valid time and source onto saved and copied \
                     images, so a screenshot still says what it is once it leaves here",
                );
            if ui
                .add_enabled(
                    self.loop_export.is_none(),
                    egui::Button::new("Export loop (GIF)…"),
                )
                .on_hover_text("Capture the archive timeline as a looping animation")
                .clicked()
            {
                self.start_loop_export(crate::loopexport::LoopFormat::Gif);
            }
            // MP4 export shells out to the `ffmpeg` CLI, which isn't present on Android; GIF
            // export (pure Rust) stays. Hide the MP4 item there rather than fail on click.
            if !cfg!(target_os = "android")
                && ui
                    .add_enabled(
                        self.loop_export.is_none(),
                        egui::Button::new("Export loop (MP4)…"),
                    )
                    .on_hover_text("Capture the archive timeline as an MP4 (requires ffmpeg)")
                    .clicked()
            {
                self.start_loop_export(crate::loopexport::LoopFormat::Mp4);
            }
            if ui.button("Clear measurement").clicked() {
                self.measure.clear();
            }

            ui.separator();
            ui.label(egui::RichText::new("Settings").strong());
            if ui
                .button("Export settings…")
                .on_hover_text("Save settings + color tables to a portable bundle")
                .clicked()
            {
                self.export_settings_bundle();
            }
            if ui
                .button("Import settings…")
                .on_hover_text("Load a settings bundle from another machine")
                .clicked()
            {
                self.import_settings_bundle();
            }

            ui.separator();
            ui.weak("HookEcho — NEXRAD radar viewer");
            ui.weak("github.com/d4vid87/hookecho");
            if ui.button("Set up again…").clicked() {
                self.firstrun.start();
            }
            if ui.button("Take the tour…").clicked() {
                self.tour.start();
            }
            if ui.button("Exit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Put `style` under the active pane and remember it.
    ///
    /// Remembering is the point: the basemap used to live only on the view, so a style picked
    /// during a chase was gone at the next launch, which read `settings.basemap` and found
    /// whatever the first-run card wrote months earlier.
    pub(crate) fn set_basemap(&mut self, style: crate::tiles::BasemapStyle) {
        self.views[self.active].basemap = style;
        if self.settings.basemap != style.slug() {
            self.settings.basemap = style.slug().to_string();
            self.settings.save();
        }
    }

    /// Deep-link the active pane to a site + camera, and (for archive) seek the timeline to
    /// `time`. Passing `time = None` leaves the pane live at the head.
    pub(crate) fn goto_view(
        &mut self,
        site: &str,
        lon: f64,
        lat: f64,
        zoom: f64,
        time: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        use crate::render::mercator::lonlat_to_world;
        let v = &mut self.views[self.active];
        // An empty site means "fly there, keep the radar" — the Android alert notification uses it,
        // since the service knows a latitude and longitude but nothing about radar coverage.
        if !site.is_empty() {
            v.site = Some(site.to_ascii_uppercase());
        }
        v.camera = crate::render::mercator::Camera {
            center: lonlat_to_world(lon, lat),
            zoom,
        };
        v.camera_placed = true;
        if let Some(t) = time {
            v.timeline.date = t.date_naive();
            v.timeline.following = false;
            v.timeline.playing = false;
            v.timeline.seek_target = Some(t);
        } else {
            v.timeline.go_head();
        }
    }

    /// Save the active pane's current view as a named bookmark (archive time captured if scrubbed).
    pub(crate) fn add_bookmark(&mut self, name: String, span_min: u16) {
        let v = &self.views[self.active];
        let Some(site) = v.site.clone() else { return };
        let time_secs = v
            .timeline
            .current()
            .and_then(|id| id.date_time())
            .map(|t| t.timestamp());
        self.settings.bookmarks.push(crate::settings::Bookmark {
            name,
            site,
            x: v.camera.center.0,
            y: v.camera.center.1,
            zoom: v.camera.zoom,
            time_secs,
            span_min,
        });
    }

    /// Export settings + referenced color tables to a portable JSON bundle (a save dialog on
    /// desktop, a download in a browser).
    fn export_settings_bundle(&mut self) {
        let json = match self.settings.export_bundle() {
            Ok(json) => json,
            Err(e) => {
                log::warn!("settings export failed: {e}");
                self.toast(ToastKind::Error, format!("Settings export failed: {e}"));
                return;
            }
        };
        match crate::dialog::save_bytes("hookecho-settings.json", "json", json.as_bytes()) {
            crate::dialog::Saved::Where(w) => {
                self.toast(ToastKind::Success, format!("Settings saved to {w}"))
            }
            crate::dialog::Saved::Failed(e) => {
                log::warn!("settings export failed: {e}");
                self.toast(ToastKind::Error, format!("Settings export failed: {e}"));
            }
            crate::dialog::Saved::Cancelled => {}
        }
    }

    /// Import a settings bundle (rfd open dialog). The next-frame dirty-diff reloads palettes
    /// and persists, and the UI (theme, layers, markers…) updates live from the new settings.
    fn import_settings_bundle(&mut self) {
        crate::dialog::request_open(crate::dialog::ImportKind::SettingsBundle, "");
    }

    /// Route a picked file to whatever asked for it.
    fn apply_import(&mut self, import: crate::dialog::Import) {
        use crate::dialog::ImportKind as K;
        match import.kind {
            K::SettingsBundle => self.apply_settings_bundle(&import),
            K::Palette if import.tag == crate::ui::palette_editor::EDITOR_TAG => {
                match import.text() {
                    Ok(text) => self.palette_editor.pending_import = Some(text),
                    Err(e) => self.toast(ToastKind::Error, format!("Palette import failed: {e}")),
                }
            }
            K::Palette => {
                // A platform that handed over content rather than a path (the browser) has no
                // path worth storing — the content goes into the settings and the override names
                // it, which is what `palette_paths` resolves back to a table.
                let value = match &import.bytes {
                    None => import.path.to_string_lossy().into_owned(),
                    Some(_) => match import.text() {
                        Ok(text) => {
                            let name = import.name();
                            self.settings.web_files.insert(name.clone(), text);
                            name
                        }
                        Err(e) => {
                            self.toast(ToastKind::Error, format!("Palette import failed: {e}"));
                            return;
                        }
                    },
                };
                // Setting the override triggers the next-frame dirty-diff palette reload.
                self.settings.palettes.insert(import.tag, value);
            }
            K::ChaseGpx => match import.text() {
                Ok(xml) => {
                    let track = crate::chaselog::from_gpx(&xml);
                    if track.points.len() < 2 {
                        self.toast(ToastKind::Error, "No track points in that file".to_string());
                    } else {
                        let n = track.points.len();
                        self.chase_replay.load(track, import.name());
                        self.toast(ToastKind::Info, format!("Loaded {n} track points"));
                    }
                }
                Err(e) => self.toast(ToastKind::Error, format!("GPX import failed: {e}")),
            },
            K::MarkerIcon => {
                let idx = import.tag.parse::<usize>().ok();
                match (
                    idx.and_then(|i| self.settings.markers.get_mut(i)),
                    crate::ui::marker_window::store_icon(&import.path),
                ) {
                    (Some(m), Some(name)) => m.icon = Some(name),
                    _ => log::warn!("marker icon import went nowhere (marker {})", import.tag),
                }
            }
            K::AlertSound => {
                let file = import.path.to_string_lossy().into_owned();
                let sound = crate::settings::AlertSound::Custom(file);
                match import.tag.as_str() {
                    "New scan" => self.settings.scan_sound = sound,
                    "Warning" => self.settings.warn_sound = sound,
                    "Emergency" => self.settings.emergency_sound = sound,
                    "TDS" => self.settings.tds_sound = sound,
                    "Rotation" => self.settings.rotation_sound = sound,
                    "Lightning" => self.settings.lightning_sound = sound,
                    other => log::warn!("no alert sound row named '{other}'"),
                }
            }
        }
    }

    /// Apply a settings bundle the user picked.
    fn apply_settings_bundle(&mut self, import: &crate::dialog::Import) {
        match import
            .text()
            .and_then(|s| crate::settings::Settings::import_bundle(&s))
        {
            Ok(settings) => {
                self.settings = settings;
                self.toast(ToastKind::Success, "Settings imported");
            }
            Err(e) => {
                log::warn!("settings import failed: {e}");
                self.toast(ToastKind::Error, format!("Settings import failed: {e}"));
            }
        }
    }

    /// Start a loop export (GIF or MP4): rewind the active timeline and capture every frame.
    fn start_loop_export(&mut self, format: crate::loopexport::LoopFormat) {
        use crate::loopexport::LoopFormat;
        let (name, ext) = match format {
            LoopFormat::Gif => ("hookecho-loop.gif", "gif"),
            LoopFormat::Mp4 => ("hookecho-loop.mp4", "mp4"),
        };
        let Some(path) = crate::dialog::save_path(name, ext) else {
            return;
        };
        let v = &mut self.views[self.active];
        let slots = v.timeline.frames.len(); // observed frames only (skip forecast tail)
        if slots == 0 {
            log::warn!("loop export: no timeline frames");
            self.toast(
                ToastKind::Info,
                "Nothing to export — no frames in the timeline yet",
            );
            return;
        }
        let speed = v.timeline.speed;
        v.timeline.go_begin();
        self.loop_export = Some(LoopExport {
            dest: path,
            format,
            frames: Vec::with_capacity(slots),
            remaining: slots,
            settle: LOOP_SETTLE_FRAMES,
            capturing: false,
            fps: speed,
        });
    }

    /// Advance the loop export: wait for the stepped radar to settle, then request a screenshot.
    fn drive_loop_export(&mut self, ctx: &egui::Context) {
        let Some(le) = &mut self.loop_export else {
            return;
        };
        if le.capturing {
            return; // waiting for the screenshot event
        }
        if le.settle > 0 {
            le.settle -= 1;
            ctx.request_repaint();
            return;
        }
        le.capturing = true;
        self.screenshot_pending = Some(ShotDest::Loop);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    /// Record one captured loop frame; step to the next, or finish + encode the GIF.
    fn record_loop_frame(&mut self, image: &egui::ColorImage) {
        let Some(le) = &mut self.loop_export else {
            return;
        };
        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for px in &image.pixels {
            buf.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
        }
        if let Some(img) = image::RgbaImage::from_raw(w, h, buf) {
            le.frames.push(img);
        }
        le.capturing = false;
        le.remaining -= 1;
        if le.remaining > 0 {
            self.views[self.active].timeline.step(1);
            if let Some(le) = &mut self.loop_export {
                le.settle = LOOP_SETTLE_FRAMES;
            }
        } else {
            let le = self.loop_export.take().unwrap();
            use crate::loopexport::LoopFormat;
            let res = match le.format {
                #[cfg(not(target_arch = "wasm32"))]
                LoopFormat::Gif => crate::loopexport::encode_gif(
                    &le.frames,
                    (1000.0 / le.fps.max(0.1)) as u16,
                    &le.dest,
                ),
                // Unreachable on the web: an export needs a destination path and there is none in
                // a browser, so `start_loop_export` returns before a capture ever begins.
                #[cfg(target_arch = "wasm32")]
                LoopFormat::Gif => Err(anyhow::anyhow!("GIF export needs a filesystem")),
                // The scrubber's slider is 1..=15 fps, which is also what the encoder accepts.
                LoopFormat::Mp4 => crate::loopexport::encode_mp4(
                    &le.frames,
                    le.fps.round().clamp(1.0, 15.0) as u32,
                    &le.dest,
                ),
            };
            match res {
                Ok(()) => {
                    log::info!(
                        "loop saved: {} ({} frames)",
                        le.dest.display(),
                        le.frames.len()
                    );
                    let msg = format!("Loop saved ({} frames)", le.frames.len());
                    self.toast(ToastKind::Success, msg);
                }
                Err(e) => {
                    log::warn!("loop encode failed: {e}");
                    self.toast(ToastKind::Error, format!("Loop export failed: {e}"));
                }
            }
        }
    }

    /// Write the widget's picture, downscaled, and tell the widget about it.
    ///
    /// Downscaled because `RemoteViews.setImageViewBitmap` sends the bitmap over a Binder
    /// transaction with a ~1 MB ceiling, and the decoded bitmap is 4 bytes a pixel — so the budget
    /// is in *pixels*, not width. 150k of them is ~600 KB decoded, and more than a home-screen
    /// tile can show anyway. (A full-screen 1440x3120 grab is 18 MB decoded: the widget would
    /// throw rather than draw.)
    fn save_widget_snapshot(&self, path: &std::path::Path, image: &egui::ColorImage) {
        const MAX_PIXELS: f32 = 150_000.0;
        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
        if w == 0 || h == 0 {
            return;
        }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for px in &image.pixels {
            rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
        }
        let Some(buf) = image::RgbaImage::from_raw(w, h, rgba) else {
            return;
        };
        let scale = (MAX_PIXELS / (w as f32 * h as f32)).sqrt();
        let small = if scale < 1.0 {
            let (nw, nh) = (
                ((w as f32 * scale).round() as u32).max(1),
                ((h as f32 * scale).round() as u32).max(1),
            );
            image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle)
        } else {
            buf
        };
        // Silent: nobody asked for this one, so a toast would be the app talking to itself. A
        // failed write costs the widget one stale caption.
        match small.save(path) {
            Ok(()) => {
                self.save_widget_caption();
                crate::platform::refresh_radar_widget();
            }
            Err(e) => log::warn!("widget snapshot failed: {e}"),
        }
    }

    /// Write the widget's storm line beside the picture, measured from the place the user cares
    /// about: where they are while chasing, else their home marker. Neither one means no line —
    /// a distance from an arbitrary map centre is a number that misleads.
    ///
    /// Deleted rather than left stale when there is nothing to say: the file is read on its own
    /// clock by a widget that has no idea how old it is.
    fn save_widget_caption(&self) {
        let Some(path) = crate::paths::widget_caption() else {
            return;
        };
        let here = self.chase_pos.or_else(|| {
            self.settings
                .markers
                .iter()
                .find(|m| m.home)
                .map(|m| (m.lon, m.lat))
        });
        let line = here.and_then(|(lon, lat)| {
            widget_storm_line(self.active_storm_cells(), lon, lat, self.metric())
        });
        match line {
            Some(line) => {
                if let Err(e) = std::fs::write(&path, line) {
                    log::warn!("widget caption failed: {e}");
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// Keep the Android home-screen widget's picture fresh while the app is on screen.
    ///
    /// Every few minutes, not every scan: the widget's own clock cannot beat 30 minutes anyway,
    /// and a full-resolution PNG per volume is a write nobody asked for. Nothing runs off Android.
    fn drive_widget_snapshot(&mut self, ctx: &egui::Context) {
        let every = std::time::Duration::from_secs(if self.settings.battery_saver {
            900
        } else {
            300
        });
        if !cfg!(target_os = "android")
            || self.screenshot_pending.is_some()
            || self.share_card.is_some()
            || self.views[self.active].volume.is_none()
        {
            return;
        }
        if self.widget_shot_at.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        let Some(path) = crate::paths::widget_snapshot() else {
            return;
        };
        self.widget_shot_at = Some(Instant::now());
        self.screenshot_pending = Some(ShotDest::Widget(path));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    /// A warning asked for a radar picture: take one, as long as nothing else is mid-capture.
    ///
    /// Deliberately the live view rather than a headless render — it is the picture the user is
    /// looking at, product, overlays, zoom and all, and it costs one frame instead of a second
    /// render path. Skipped on Android, where the alert path runs in a service with no surface.
    fn drive_snapshot_push(&mut self, ctx: &egui::Context) {
        if self.snapshot_push.is_none()
            || self.screenshot_pending.is_some()
            || self.share_card.is_some()
            || cfg!(target_os = "android")
        {
            return;
        }
        let title = self.snapshot_push.take().expect("checked above");
        self.request_capture(ctx, ShotDest::Push(title));
    }

    /// PUT a captured frame to the user's ntfy topic as an attachment.
    fn push_snapshot(&self, title: String, image: &egui::ColorImage) {
        let topic = self.settings.ntfy_topic.trim().to_string();
        if topic.is_empty() {
            return;
        }
        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for px in &image.pixels {
            rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
        }
        let mut png = std::io::Cursor::new(Vec::new());
        if let Err(e) = image::write_buffer_with_format(
            &mut png,
            &rgba,
            w,
            h,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        ) {
            log::warn!("alert snapshot encode failed: {e}");
            return;
        }
        let body = png.into_inner();
        // ntfy caps attachments (a few MB on the public server); a screen-sized PNG is well under,
        // but say so in the log rather than wondering why nothing arrived.
        log::debug!("pushing alert snapshot: {} KiB", body.len() / 1024);
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = http
                .put(format!("https://ntfy.sh/{topic}"))
                .header("Title", title)
                .header("Filename", "radar.png")
                .body(body)
                .send()
                .await;
            if let Err(e) = res {
                log::warn!("ntfy snapshot push failed: {e}");
            }
        });
    }

    /// Ask the viewport for an image, `dest` decides where it lands.
    ///
    /// With the share card on, the request waits two frames while [`share_card_footer`] draws the
    /// caption band, so the capture contains it.
    ///
    /// ponytail: the caption is drawn by egui into the frame rather than composited into the
    /// pixels afterwards — text layout, fonts and the theme are already solved here, and
    /// compositing them onto a raw RGBA buffer is a rasterizer we would have to grow.
    fn request_capture(&mut self, ctx: &egui::Context, dest: ShotDest) {
        if self.settings.share_card {
            // Two frames: one to lay the band out, one to be sure it is on screen when the
            // viewport grabs the image.
            self.share_card = Some((dest, 2));
            ctx.request_repaint();
        } else {
            self.screenshot_pending = Some(dest);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// The share card: a caption band across the bottom of the map naming what the picture shows,
    /// drawn only on the frames a capture is waiting for. A radar screenshot with no site, product
    /// or time on it is a pretty picture; this is the difference between that and a report.
    fn share_card_footer(&mut self, ctx: &egui::Context) {
        let Some((_, frames)) = &mut self.share_card else {
            return;
        };
        *frames -= 1;
        let fire = *frames == 0;

        let v = &self.views[self.active];
        let site = v.site.clone().unwrap_or_else(|| "no site".to_string());
        let product = crate::products::info(v.moment).name.to_string();
        let time = v
            .volume
            .as_ref()
            .map(|vol| crate::timefmt::fmt_date_clock(vol.time, self.active_tz()))
            .unwrap_or_default();
        let accent = crate::theme::accent(self.settings.theme);
        let logo = crate::icon::texture(ctx, 64);
        egui::Area::new(egui::Id::new("share_card"))
            .order(egui::Order::Foreground)
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -10.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 245).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&site)
                                .size(crate::ui::style::FONT_BASE)
                                .strong()
                                .color(accent),
                        );
                        let mut line = product;
                        if !time.is_empty() {
                            line.push_str(" · ");
                            line.push_str(&time);
                        }
                        ui.label(
                            egui::RichText::new(line)
                                .size(crate::ui::style::FONT_BASE)
                                .color(egui::Color32::from_gray(238)),
                        );
                        // The mark rides with the caption: a shared loop travels without the app
                        // around it, and the wordmark alone is not what anyone recognises.
                        ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(16.0, 16.0)));
                        ui.label(
                            egui::RichText::new("HookEcho · data: NOAA/NWS")
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(160)),
                        );
                    });
                });
            });

        if fire {
            // The band is on screen: take the picture, and keep the caption for this one frame.
            let (dest, _) = self.share_card.take().expect("checked above");
            self.screenshot_pending = Some(dest);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }

    /// If a screenshot was requested, save the delivered image event to the pending path.
    fn save_pending_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot_pending.is_none() {
            return;
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let dest = self.screenshot_pending.take().unwrap();
            match dest {
                ShotDest::File(path) => {
                    let (w, h) = (image.size[0] as u32, image.size[1] as u32);
                    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                    for px in &image.pixels {
                        rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
                    }
                    match image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8) {
                        Ok(()) => {
                            log::info!("screenshot saved: {}", path.display());
                            let msg = format!("Saved {}", path.display());
                            self.toast(ToastKind::Success, msg);
                        }
                        Err(e) => {
                            log::warn!("screenshot save failed: {e}");
                            self.toast(ToastKind::Error, format!("Screenshot failed: {e}"));
                        }
                    }
                }
                ShotDest::Clipboard => {
                    // `image` here is already an egui ColorImage; hand it straight to the clipboard.
                    ctx.copy_image((*image).clone());
                    log::info!("view copied to clipboard");
                    self.toast(ToastKind::Success, "View copied to clipboard");
                }
                ShotDest::Loop => self.record_loop_frame(&image),
                ShotDest::Push(title) => self.push_snapshot(title, &image),
                ShotDest::Widget(path) => self.save_widget_snapshot(&path, &image),
            }
        }
    }
}

/// Convert a binned sweep into a GPU upload with its world-space bounding box.
///
/// `threshold` (physical units) is baked into the color LUT; `None` shows all values.
/// `smooth` enables bilinear sampling in the shader. `table` selects the colormap.
/// `storm_uv` is the storm motion (east, north) in m/s for storm-relative velocity, or
/// `None` for ground-relative. `precip` is the MRMS surface precipitation-type field, when the
/// user has asked for reflectivity to be tinted by it.
pub(crate) fn to_upload(
    s: &BinnedSweep,
    table: &ColorTable,
    threshold: Option<f32>,
    smooth: bool,
    storm_uv: Option<(f32, f32)>,
    precip: Option<&PrecipGrid>,
    // Only the color table changed, so the sweep and precipitation-flag bytes the GPU already
    // holds are still correct and are not copied.
    lut_only: bool,
) -> RadarUpload {
    use crate::render::mercator::lonlat_to_world;
    let max_range_km = s.first_gate_km + s.gate_count as f32 * s.gate_interval_km;
    let dlat = (max_range_km / 111.32) as f64;
    let coslat = (s.radar_lat as f64 * std::f64::consts::PI / 180.0)
        .cos()
        .max(0.01);
    let dlon = (max_range_km as f64 / 111.32) / coslat;
    let (lat, lon) = (s.radar_lat as f64, s.radar_lon as f64);
    let (wx0, wy0) = lonlat_to_world(lon - dlon, lat + dlat);
    let (wx1, wy1) = lonlat_to_world(lon + dlon, lat - dlat);
    // Premultiply storm motion into raw-index units (raw = 2 + t*253, t over value_span).
    let per_ms = 253.0 / (s.value_max - s.value_min).max(f32::EPSILON);
    let (srv, me, mn) = match storm_uv {
        Some((e, n)) => (1.0, e * per_ms, n * per_ms),
        None => (0.0, 0.0, 0.0),
    };
    // Only reflectivity is tinted: the tint says what kind of precipitation an echo is, and
    // that is not a statement about a velocity or a correlation coefficient.
    let tint = precip.filter(|_| s.moment == Moment::Reflectivity);
    let base = crate::colormap::bake_lut(table, (s.value_min, s.value_max), threshold);
    // Three rows, always. When the tint is off they are identical, which costs 2 KB and keeps
    // the shader and the bind group the same shape in both cases.
    let mut lut = Vec::with_capacity(1024 * 3);
    lut.extend_from_slice(&base);
    for kind in [
        crate::colormap::PrecipTint::Snow,
        crate::colormap::PrecipTint::Mix,
    ] {
        match tint {
            Some(_) => lut.extend_from_slice(&crate::colormap::tint_lut(&base, kind)),
            None => lut.extend_from_slice(&base),
        }
    }
    let (precip_flag, flag_nx, flag_ny, flag_w, flag_n, flag_e, flag_s) = match tint {
        Some(g) => (
            if lut_only {
                Vec::new()
            } else {
                g.classes.clone()
            },
            g.nx,
            g.ny,
            g.west,
            g.north,
            g.east,
            g.south,
        ),
        None => (Vec::new(), 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
    };

    RadarUpload {
        az_bins: s.az_bins as u32,
        gate_count: s.gate_count as u32,
        data: if lut_only { Vec::new() } else { s.data.clone() },
        uniform: [
            s.radar_lat,
            s.radar_lon,
            s.first_gate_km,
            s.gate_interval_km,
            s.az_bins as f32,
            s.gate_count as f32,
            if smooth { 1.0 } else { 0.0 },
            srv,
            me,
            mn,
            if tint.is_some() { 1.0 } else { 0.0 },
            flag_nx,
            flag_ny,
            flag_w,
            flag_n,
            flag_e,
            flag_s,
            0.0,
            0.0,
            0.0,
        ],
        lut,
        precip_flag,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        lut_only,
    }
}

/// Whether the overlay geometry should be re-tessellated this frame.
///
/// A zoom-bucket crossing waits for the gesture to end; new geometry does not.
fn should_retess(gesture_live: bool, geometry_changed: bool, bucket_changed: bool) -> bool {
    geometry_changed || (bucket_changed && !gesture_live)
}

/// Every radar site with its world-space position, projected once.
///
/// The table is static and the projection is a `ln(tan(...))` per site; ~350 of them ran every
/// frame, per pane, for a set of points that cannot move.
fn sites_in_world() -> &'static [(&'static wxdata::sites::SiteEntry, (f64, f64))] {
    static SITES: std::sync::OnceLock<Vec<(&'static wxdata::sites::SiteEntry, (f64, f64))>> =
        std::sync::OnceLock::new();
    SITES.get_or_init(|| {
        wxdata::sites::all()
            .map(|s| {
                (
                    s,
                    crate::render::mercator::lonlat_to_world(s.longitude as f64, s.latitude as f64),
                )
            })
            .collect()
    })
}

/// MRMS surface precipitation classes on their own lat/lon grid, ready for the GPU.
pub(crate) struct PrecipGrid {
    /// One byte per cell: 0 rain, 1 snow, 2 mix.
    pub classes: Vec<u8>,
    pub nx: f32,
    pub ny: f32,
    pub west: f32,
    pub north: f32,
    pub east: f32,
    pub south: f32,
}

impl PrecipGrid {
    fn new(f: &wxdata::mrms::MrmsField) -> Self {
        Self {
            classes: f.values.iter().map(|v| precip_class(*v)).collect(),
            nx: f.nx as f32,
            ny: f.ny as f32,
            west: f.lon_west as f32,
            north: f.lat_north as f32,
            east: f.lon_east as f32,
            south: f.lat_south as f32,
        }
    }
}

/// MRMS `PrecipFlag` categories collapsed to the three the tint distinguishes.
///
/// The product carries more classes than that — several flavours of rain, hail, tropical rain —
/// but colouring reflectivity is a coarse statement and only three destinations exist. Hail and
/// convective rain stay on the rain ramp deliberately: they are the cases where the existing
/// reflectivity colours already carry the meaning.
fn precip_class(flag: f32) -> u8 {
    match flag as i32 {
        // 3 snow, 4 wet snow.
        3 | 4 => 1,
        // 6 freezing rain, 7 ice pellets/sleet.
        6 | 7 => 2,
        _ => 0,
    }
}

/// Reflectivity at a lon/lat off a binned sweep, by inverting the polar bin geometry. `None`
/// outside the sweep's gates; below-threshold bins read as -inf (in coverage, but no echo).
fn refl_sampler(sweep: &wxdata::level2::BinnedSweep) -> impl Fn(f64, f64) -> Option<f32> + '_ {
    let span = (sweep.value_max - sweep.value_min).max(1e-3);
    let radar = [sweep.radar_lon as f64, sweep.radar_lat as f64];
    move |lon: f64, lat: f64| {
        let (range_km, bearing) = crate::geo::great_circle(radar, [lon, lat]);
        let gate = ((range_km as f32 - sweep.first_gate_km) / sweep.gate_interval_km).round();
        if gate < 0.0 || gate as usize >= sweep.gate_count {
            return None;
        }
        let az = (bearing / 360.0 * sweep.az_bins as f64).round() as usize % sweep.az_bins;
        let v = sweep.data[az * sweep.gate_count + gate as usize];
        if v < 2 {
            return Some(f32::NEG_INFINITY);
        }
        Some(sweep.value_min + (v as f32 - 2.0) / 253.0 * span)
    }
}

/// Convert an MRMS reflectivity field into a GPU upload: dBZ → 2..=255 index band
/// (no-data/NaN → 0 = transparent), the reflectivity color LUT, and the grid's
/// mercator world-space quad (plate-carrée corners projected).
fn mrms_upload(f: &wxdata::mrms::MrmsField, table: &ColorTable) -> crate::render::MrmsUpload {
    use crate::render::mercator::lonlat_to_world;
    let (vmin, vmax) = Moment::Reflectivity.value_range();
    let span = (vmax - vmin).max(f32::EPSILON);
    let data: Vec<u8> = f
        .values
        .iter()
        .map(|&v| {
            if v.is_nan() {
                0
            } else {
                let t = ((v - vmin) / span).clamp(0.0, 1.0);
                (2.0 + t * 253.0) as u8
            }
        })
        .collect();
    let (wx0, wy0) = lonlat_to_world(f.lon_west, f.lat_north);
    let (wx1, wy1) = lonlat_to_world(f.lon_east, f.lat_south);
    crate::render::MrmsUpload {
        data,
        nx: f.nx as u32,
        ny: f.ny as u32,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        uniform: [
            f.lon_west as f32,
            f.lat_north as f32,
            f.lon_east as f32,
            f.lat_south as f32,
            f.nx as f32,
            f.ny as f32,
            1.0, // opacity; rewritten per frame from settings.field_opacity
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        lut: crate::colormap::bake_lut(table, (vmin, vmax), None).to_vec(),
    }
}

/// Build a field-layer GPU upload from a grid: `map` turns each cell value into a LUT index
/// (0 = transparent, 2..=255 = data), `lut` is the 256-entry RGBA color table.
pub(crate) fn field_index_upload(
    f: &wxdata::mrms::MrmsField,
    map: impl Fn(f32) -> u8,
    lut: Vec<u8>,
) -> crate::render::MrmsUpload {
    use crate::render::mercator::lonlat_to_world;
    let data: Vec<u8> = f
        .values
        .iter()
        .map(|&v| if v.is_nan() { 0 } else { map(v) })
        .collect();
    let (wx0, wy0) = lonlat_to_world(f.lon_west, f.lat_north);
    let (wx1, wy1) = lonlat_to_world(f.lon_east, f.lat_south);
    crate::render::MrmsUpload {
        data,
        nx: f.nx as u32,
        ny: f.ny as u32,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        uniform: [
            f.lon_west as f32,
            f.lat_north as f32,
            f.lon_east as f32,
            f.lat_south as f32,
            f.nx as f32,
            f.ny as f32,
            1.0, // opacity; rewritten per frame from settings.field_opacity
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        lut,
    }
}

/// Interpolate a 256-entry RGBA LUT from `(t, [r,g,b])` stops; index 0 is always transparent.
fn ramp_lut(stops: &[(f32, [u8; 3])]) -> Vec<u8> {
    ramp_lut_a(stops, 255)
}

/// Like [`ramp_lut`] but with a caller-chosen opacity for non-zero indices (index 0 stays clear).
/// Environment overlays (CAPE/SRH) use a translucent alpha so the basemap reads through.
fn ramp_lut_a(stops: &[(f32, [u8; 3])], alpha: u8) -> Vec<u8> {
    let mut lut = vec![0u8; 256 * 4];
    for i in 0..256 {
        let t = i as f32 / 255.0;
        let mut rgb = stops[0].1;
        for w in stops.windows(2) {
            let (t0, c0) = w[0];
            let (t1, c1) = w[1];
            if t >= t0 && t <= t1 {
                let k = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
                rgb = [
                    (c0[0] as f32 + (c1[0] as f32 - c0[0] as f32) * k) as u8,
                    (c0[1] as f32 + (c1[1] as f32 - c0[1] as f32) * k) as u8,
                    (c0[2] as f32 + (c1[2] as f32 - c0[2] as f32) * k) as u8,
                ];
                break;
            }
        }
        let a = if i == 0 { 0 } else { alpha };
        lut[i * 4..i * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
    }
    lut
}

/// Build a 256-entry categorical LUT: every listed `(index, rgb)` gets `alpha`, all others clear.
/// Used for the MRMS precipitation-type flag (discrete categories, not a continuous ramp).
fn categorical_lut(slots: &[(u8, [u8; 3])], alpha: u8) -> Vec<u8> {
    let mut lut = vec![0u8; 256 * 4];
    for &(i, rgb) in slots {
        let o = i as usize * 4;
        lut[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
    }
    lut
}

/// Map color for a tornado's F/EF magnitude (green→yellow→orange→red→violet; gray = unknown).
fn tornado_mag_color(mag: i8) -> egui::Color32 {
    match mag {
        0 => egui::Color32::from_rgb(120, 200, 120),
        1 => egui::Color32::from_rgb(230, 220, 80),
        2 => egui::Color32::from_rgb(240, 170, 50),
        3 => egui::Color32::from_rgb(235, 90, 60),
        4 => egui::Color32::from_rgb(220, 50, 90),
        5 => egui::Color32::from_rgb(200, 60, 220),
        _ => egui::Color32::from_rgb(150, 150, 160),
    }
}

/// Load the SPC tornado-track database, preferring an on-disk cache; on a cache miss, download the
/// CSV and write it for next time. Parsing is the same either way.
async fn load_or_fetch_climo(
    http: &reqwest::Client,
    cache: Option<std::path::PathBuf>,
) -> anyhow::Result<Vec<wxdata::torclimo::TornadoTrack>> {
    if let Some(path) = &cache {
        if let Ok(csv) = std::fs::read_to_string(path) {
            return Ok(wxdata::torclimo::parse_tracks(&csv));
        }
    }
    // Cache miss: download once, parse, and persist the raw CSV.
    let csv = http
        .get("https://www.spc.noaa.gov/wcm/data/1950-2022_actual_tornadoes.csv")
        .header("User-Agent", wxdata::alerts::USER_AGENT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    if let Some(path) = &cache {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, &csv);
    }
    Ok(wxdata::torclimo::parse_tracks(&csv))
}

/// Lightning-density upload (strikes/km²/min → log index), kept public for the headless harness.
pub(crate) fn lightning_upload(f: &wxdata::mrms::MrmsField) -> crate::render::MrmsUpload {
    let map = |v: f32| {
        if v <= 0.0 {
            0
        } else {
            (2.0 + ((v.log10() + 1.7) / 2.0).clamp(0.0, 1.0) * 253.0) as u8
        }
    };
    field_index_upload(
        f,
        map,
        ramp_lut(&[
            (0.0, [255, 255, 255]),
            (0.35, [255, 240, 120]),
            (0.65, [255, 160, 40]),
            (1.0, [230, 60, 200]),
        ]),
    )
}

/// Build the GPU upload for an index-mapped field layer (everything except the reflectivity
/// mosaic, which needs the app's color table). Kept public for the headless harness.
pub(crate) fn field_upload_indexed(
    layer: crate::render::FieldLayer,
    f: &wxdata::mrms::MrmsField,
) -> crate::render::MrmsUpload {
    use crate::render::field_ramps::{ramp_for, FieldScale};
    use crate::render::FieldLayer as FL;
    // Lightning keeps its own mapping (density counts, not a physical scale).
    if layer == FL::Lightning {
        return lightning_upload(f);
    }
    let Some(r) = ramp_for(layer) else {
        // The reflectivity-palette layers (mosaic, HRRR) route through the app method instead.
        return field_index_upload(f, |_| 0, vec![0u8; 256 * 4]);
    };
    let lut = match r.scale {
        FieldScale::Ramp { stops, .. } => crate::render::field_ramps::bake_ramp_lut(stops, r.alpha),
        FieldScale::Categorical(cats) => {
            let slots: Vec<(u8, [u8; 3])> = cats.iter().map(|&(i, rgb, _)| (i, rgb)).collect();
            categorical_lut(&slots, r.alpha)
        }
    };
    field_index_upload(f, |v| r.index(v), lut)
}

impl HookEchoApp {
    /// Build the GPU upload for `layer` from its freshly-fetched grid, picking the value→index
    /// mapping and color LUT that suit the product's units.
    fn field_upload(
        &self,
        layer: crate::render::FieldLayer,
        f: &wxdata::mrms::MrmsField,
    ) -> crate::render::MrmsUpload {
        use crate::render::FieldLayer as FL;
        match layer {
            // Mosaic + HRRR forecast are both dBZ → the reflectivity palette.
            FL::Mrms | FL::Mosaic | FL::Hrrr => {
                mrms_upload(f, self.palettes.table(Moment::Reflectivity))
            }
            other => field_upload_indexed(other, f),
        }
    }
}

/// Marker color for a storm-cell kind (sRGB).
/// `[r,g,b,a]` -> egui `Color32` (unmultiplied).
/// Blit one icon-sheet cell centered on its hot spot, rotated `angle_deg` clockwise.
/// A raw 4-vertex mesh because `Painter::image` can't rotate.
#[allow(clippy::too_many_arguments)] // a params struct for one call site buys nothing
fn draw_sprite(
    painter: &egui::Painter,
    tex: egui::TextureId,
    uv: egui::Rect,
    at: egui::Pos2,
    size: egui::Vec2,
    hot: egui::Vec2,
    angle_deg: f32,
    tint: egui::Color32,
) {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    // Corner offsets relative to the hot spot, then rotated about it.
    let corner = |dx: f32, dy: f32| {
        let (x, y) = (dx - hot.x, dy - hot.y);
        at + egui::vec2(x * cos - y * sin, x * sin + y * cos)
    };
    let mut mesh = egui::Mesh::with_texture(tex);
    for (dx, dy, u, v) in [
        (0.0, 0.0, uv.left(), uv.top()),
        (size.x, 0.0, uv.right(), uv.top()),
        (size.x, size.y, uv.right(), uv.bottom()),
        (0.0, size.y, uv.left(), uv.bottom()),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: corner(dx, dy),
            uv: egui::pos2(u, v),
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Where a placefile's icon sheet is kept between runs, if this platform has a disk.
///
/// Named by a hash of the URL: sheets come from arbitrary hosts and their paths are not safe to
/// use as filenames.
fn icon_sheet_cache_path(url: &str) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    Some(
        crate::paths::cache_dir()?
            .join("pficons")
            .join(format!("{:016x}", h.finish())),
    )
}

/// Download and decode a placefile icon sheet (PNG/GIF) into an egui image.
async fn fetch_icon_sheet(http: &reqwest::Client, url: &str) -> anyhow::Result<egui::ColorImage> {
    // Read-through disk cache, same shape as the basemap tiles: an icon sheet is a few KB of PNG
    // that never changes, and re-fetching every placefile's sheet at every startup is the kind of
    // traffic somebody else's web host notices. A corrupt file simply fails to decode and is
    // refetched. Nothing on the web, where `cache_dir()` is None.
    let cached = icon_sheet_cache_path(url);
    let hit = cached
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| image::load_from_memory(&b).ok());
    let img = match hit {
        Some(img) => img.to_rgba8(),
        None => {
            // Generic web hosts, so use the browser-ish UA the tile fetches already send.
            let bytes = http
                .get(url)
                .header("User-Agent", crate::tiles::USER_AGENT)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            let img = image::load_from_memory(&bytes)?.to_rgba8();
            if let Some(path) = &cached {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(path, &bytes);
            }
            img
        }
    };
    let (w, h) = img.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        img.as_raw(),
    ))
}

fn rgba32(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

fn cell_color(kind: CellKind) -> [u8; 4] {
    match kind {
        CellKind::Storm => [255, 235, 60, 255], // yellow
        CellKind::Hail => [80, 220, 120, 255],  // green
        CellKind::Meso => [255, 70, 70, 255],   // red
    }
}

/// The storm cell (with a non-empty SCIT id) nearest to `(lon, lat)` within `max_km`, if any.
/// Used by the storm-follow camera to reacquire a tracked cell after SCIT renumbers it.
fn nearest_cell(cells: &[Cell], lon: f64, lat: f64, max_km: f64) -> Option<&Cell> {
    cells
        .iter()
        .filter(|c| !c.id.is_empty())
        .map(|c| (c, crate::geo::great_circle([lon, lat], [c.lon, c.lat]).0))
        .filter(|(_, km)| *km <= max_km)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(c, _)| c)
}

/// The widget's storm line: the nearest tracked cell to `(lon, lat)`, in the words a glance can
/// read — "R3 12 mi SSW, moving NE 35 mph". `None` when nothing is being tracked nearby, which
/// the widget renders as no line at all rather than as "no storms" (the app may simply have been
/// looking at another part of the country).
fn widget_storm_line(cells: &[Cell], lon: f64, lat: f64, metric: bool) -> Option<String> {
    let c = nearest_cell(cells, lon, lat, 300.0)?;
    let (km, bearing) = crate::geo::great_circle([c.lon, c.lat], [lon, lat]);
    // Bearing is measured from the cell to you, so the compass point is the side of you the storm
    // sits on once it is flipped back.
    let from = crate::geo::compass(((bearing + 180.0) % 360.0) as f32);
    let mut line = format!(
        "{} {} {from}",
        if c.id.is_empty() { &c.title } else { &c.id },
        crate::geo::fmt_distance(km, metric, 0)
    );
    if let (Some(dir), Some(kt)) = (c.mvt_deg, c.mvt_kt) {
        if kt >= 1.0 {
            line.push_str(&format!(
                ", moving {} {:.0} mph",
                crate::geo::compass(dir),
                kt * 1.150_779
            ));
        }
    }
    Some(line)
}

/// The longest consecutive segment of a screen-space polyline (for placing a contour label).
fn longest_segment(pts: &[egui::Pos2]) -> Option<(egui::Pos2, egui::Pos2)> {
    pts.windows(2).map(|w| (w[0], w[1])).max_by(|a, b| {
        a.0.distance(a.1)
            .partial_cmp(&b.0.distance(b.1))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Marker color for an SPC storm-report kind.
fn report_color(kind: wxdata::spc::ReportKind) -> [u8; 4] {
    use wxdata::spc::ReportKind as R;
    match kind {
        R::Tornado => [230, 40, 40, 255], // red
        R::Wind => [70, 130, 240, 255],   // blue
        R::Hail => [70, 210, 110, 255],   // green
        R::Flood => [0, 150, 90, 255],    // dark green
        R::Other => [180, 180, 180, 255], // gray
    }
}

/// Display-unit factor and label for a moment: velocity/spectrum-width honor the Units
/// setting (internal data stays m/s), everything else uses its native unit.
pub(crate) fn display_units(moment: Moment, settings: &Settings) -> (f32, &'static str) {
    match moment {
        Moment::Velocity | Moment::SpectrumWidth => (
            settings.velocity_unit.factor_from_ms(),
            settings.velocity_unit.label(),
        ),
        _ => (1.0, moment.units()),
    }
}

/// A coarse "N ago" string for volume age.
/// The GOES frame closest to `t`, or `None` when the nearest one is too far off to be the same
/// weather (or there are no frames yet).
///
/// GIBS geostationary layers are 10-minute imagery; half an hour of tolerance covers a gap in the
/// index or a radar volume from a slow VCP without ever pairing a scan with imagery from another
/// part of the day. `None` means "leave it on the latest", which is the pre-existing behaviour.
fn nearest_goes(
    times: &[chrono::DateTime<chrono::Utc>],
    t: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    const TOLERANCE_MIN: i64 = 30;
    times
        .iter()
        .copied()
        .min_by_key(|x| (*x - t).num_seconds().abs())
        .filter(|x| (*x - t).num_minutes().abs() <= TOLERANCE_MIN)
}

/// Compass point for a bearing in degrees from north (used in alarm text).
fn cardinal(bearing_deg: f64) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    POINTS[(((bearing_deg % 360.0 + 360.0) % 360.0) / 45.0).round() as usize % 8]
}

fn humanize(secs: i64) -> String {
    const DAY: i64 = 86_400;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 2 * DAY {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs < 365 * DAY {
        // Scrub back to a historic event and the hours keep counting: a 2011 storm read
        // "133672h40m ago", which is technically true and completely useless.
        format!("{}d", secs / DAY)
    } else {
        format!("{:.1}y", secs as f64 / (365.25 * DAY as f64))
    }
}

/// True if the feature's bounding box overlaps `box = (min_lon, min_lat, max_lon, max_lat)`.
/// Features with no geometry (no bbox) are treated as not overlapping.
fn feature_in_box(f: &GeoFeature, bx: (f64, f64, f64, f64)) -> bool {
    let Some((x0, y0, x1, y1)) = f.bbox() else {
        return false;
    };
    let (bx0, by0, bx1, by1) = bx;
    x1 >= bx0 && x0 <= bx1 && y1 >= by0 && y0 <= by1
}

impl eframe::App for HookEchoApp {
    /// Flush any settings change the one-second dirty-diff throttle hasn't picked up yet.
    fn on_exit(&mut self) {
        // Remember where we were looking, so a relaunch picks up the map where it was left. Only
        // while no explicit startup view is saved — that one is the user's choice, not ours.
        // Written here rather than per-frame: Android's alert service reads this file concurrently.
        #[cfg(not(target_os = "android"))]
        if self.settings.start_view.is_none() {
            log::debug!("remembering the last view for next launch");
            let view = &self.views[self.active];
            if let Some(site) = &view.site {
                self.settings.last_view = Some(crate::settings::StartView {
                    site: site.clone(),
                    x: view.camera.center.0,
                    y: view.camera.center.1,
                    zoom: view.camera.zoom,
                });
            }
        }
        // Anything quiet hours is still holding: it is owed to the user when the window ends, and
        // that can be after a restart.
        // A poisoned lock still holds the data; dropping it here would silently lose
        // notifications the user is owed.
        self.settings.quiet_pending = match self.quiet_queue.lock() {
            Ok(q) => q.clone(),
            Err(p) => p.into_inner().clone(),
        };
        if self.settings != self.saved {
            self.settings.save();
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Android: feed the status-bar / gesture-bar insets so no UI draws under system chrome.
        crate::platform::apply_safe_area(ctx, raw_input);
        // Android: GameActivity's IME reports edits as editor state, not keystrokes; turn that
        // state into the text/backspace events egui's focused field expects.
        crate::platform::pump_ime(raw_input);
        // Android: clipboard text fetched by the paste bar lands as a real egui Paste event, so
        // the focused text field inserts it exactly like Ctrl+V would.
        if let Some(text) = self.pending_paste.take() {
            raw_input.events.push(egui::Event::Paste(text));
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::profiling::new_frame();
        crate::prof_scope!("ui");
        let ctx = root.ctx().clone();
        let ctx = &ctx;

        // Stamp this frame for the background workers' foreground gate (see
        // `platform::activity`). A frame that follows a gap means the app just came back, so
        // force one refresh rather than making the user wait out the poll interval.
        self.frame_nr = self.frame_nr.wrapping_add(1);
        wxdata::stats::bump(wxdata::stats::Counter::FramesDrawn);
        self.gesture_live = ctx.input(|i| i.pointer.any_down() || i.any_touches());
        #[cfg(not(target_arch = "wasm32"))]
        self.perf.tick(ctx);
        #[cfg(debug_assertions)]
        self.frame_time_overlay(ctx);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if crate::platform::activity::mark_frame(focused) {
            self.overlay_last_fetch = None;
            for v in &mut self.views {
                v.last_poll = None;
            }
            // A resume is also how a notification tap arrives: the activity wrote the target
            // before handing us back the surface.
            self.drain_goto_file();
        }
        // A deep link that arrives while the app is already on screen never produces a resume, so
        // the drain above would never see it — on Android the activity is reused
        // (`launchMode="singleTask"`) and only `onNewIntent` fires; on desktop the second process
        // hands its link over and exits. ponytail: a one-second stat of a path that usually does
        // not exist, rather than a callback into the event loop.
        if self
            .goto_poll
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1))
        {
            self.goto_poll = Some(Instant::now());
            self.drain_goto_file();
        }
        // The settings window has no HTTP client or runtime, so the voice-download button raises
        // a flag and the work happens here, on the same spawner everything else fetches on.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(id) = crate::speech::take_voice_request() {
            self.download_voice(id);
        }
        // A file the user picked, from any of the import buttons. Routed here rather than at the
        // button, because on Android the picker is an activity result that lands long after the
        // click — through the same file handover a notification tap uses.
        if let Some(import) = crate::dialog::take_result() {
            self.apply_import(import);
        }

        // Android paste: re-focus the text field that lost focus to the Paste-button tap, before
        // any window draws, so the queued Paste event (see `raw_input_hook`) lands in it.
        if let Some(id) = self.paste_target.take() {
            ctx.memory_mut(|m| m.request_focus(id));
        }

        // Tray menu commands (Linux StatusNotifier): restore the window or quit for real.
        // Keep the tray menu telling the truth: alert count, mute state, starred sites. Sent only
        // when it changes — every send is a D-Bus round trip on the tray thread.
        {
            let want = crate::tray::TrayState {
                alerts: self.active_alert_features().len(),
                muted: self.settings.mute_alerts,
                starred: self.settings.presets.clone(),
            };
            if self.tray_state != want {
                self.tray_state = want.clone();
                crate::tray::set_state(want);
            }
        }
        {
            // Drained first: the handlers below need `&mut self`, and the receiver lives in it.
            let cmds: Vec<crate::tray::TrayCmd> = self.tray_rx.try_iter().collect();
            for cmd in cmds {
                match cmd {
                    crate::tray::TrayCmd::Show => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    crate::tray::TrayCmd::Quit => {
                        self.really_quit = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    crate::tray::TrayCmd::Mute => {
                        self.apply_action(BindableAction::ToggleMute, ctx);
                    }
                    crate::tray::TrayCmd::Site(id) => {
                        // Same path the site dialog uses: `sync_pane` does the rest next frame.
                        self.views[self.active].site = Some(id);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
            }
        }

        // Strikes age out on their own clock: the deque only shrinks here, so a quiet topic
        // still empties the map rather than freezing the last minute of a storm on it.
        if !self.strikes.is_empty() {
            let cutoff = chrono::Utc::now() - chrono::Duration::seconds(STRIKE_WINDOW_SECS);
            while self.strikes.front().is_some_and(|&(_, _, t)| t < cutoff) {
                self.strikes.pop_front();
            }
        }

        // Commands off the broker, drained the same way and applied through the same paths the
        // tray uses. They land on the next repaint rather than instantly, which for "point at the
        // storm" is close enough and keeps every state change on one thread.
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        for cmd in crate::mqtt::drain() {
            match cmd {
                crate::mqtt::Cmd::Mute(want) => {
                    if self.settings.mute_alerts != want {
                        self.apply_action(BindableAction::ToggleMute, ctx);
                    }
                }
                crate::mqtt::Cmd::Site(id) => {
                    if wxdata::sites::site_by_id(&id).is_some() {
                        self.views[self.active].site = Some(id);
                    } else {
                        log::warn!("mqtt: no such site {id}");
                    }
                }
                crate::mqtt::Cmd::Product(code) => {
                    if let Some(m) = Moment::from_code(&code) {
                        let srv = self.views[self.active].srv;
                        self.apply_palette(PaletteAction::SetMoment(m, srv), ctx);
                    }
                }
                crate::mqtt::Cmd::Strike { lon, lat, time } => {
                    self.strikes.push_back((lon, lat, time));
                    // A busy night over a whole continent is a lot of strikes, and the painter
                    // walks the whole deque. Cap it and let the oldest fall off early.
                    while self.strikes.len() > STRIKE_CAP {
                        self.strikes.pop_front();
                    }
                }
            }
        }

        // Run-in-background: when the user closes the window and close-to-tray is on (and it wasn't
        // a tray "Quit"), cancel the quit and hide instead — the app keeps polling alerts and
        // pushing ntfy. Restore via the tray icon (or the taskbar when no tray host is present).
        if self.settings.close_to_tray
            && !self.really_quit
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            let cmd = if self.tray_present.load(std::sync::atomic::Ordering::Relaxed) {
                egui::ViewportCommand::Visible(false) // hide fully; the tray restores it
            } else {
                egui::ViewportCommand::Minimized(true) // no tray → keep a taskbar entry
            };
            ctx.send_viewport_cmd(cmd);
        }

        // "Modern dark pro" styling (palette/spacing/rounding/accent). Re-applied when the theme
        // or the system light/dark preference changes — it rebuilds and installs a whole
        // `egui::Style`, which is wasted work on every other frame.
        let system_dark = ctx.input(|i| i.raw.system_theme) != Some(egui::Theme::Light);
        let theme_key = (
            self.settings.theme,
            system_dark,
            self.settings.density,
            self.settings.accent,
        );
        if self.theme_applied != Some(theme_key) {
            crate::theme::apply(
                ctx,
                self.settings.theme,
                system_dark,
                self.settings.density,
                self.settings.accent,
            );
            self.theme_applied = Some(theme_key);
        }

        // UI scale: apply the setting when the slider moved, else absorb built-in keyboard zoom
        // (Ctrl+= / Ctrl+- / Ctrl+0) back into the setting so it persists.
        if (self.settings.ui_scale - self.ui_scale_applied).abs() > 1e-3 {
            ctx.set_zoom_factor(self.settings.ui_scale);
            self.ui_scale_applied = self.settings.ui_scale;
        } else {
            let z = ctx.zoom_factor();
            self.settings.ui_scale = z;
            self.ui_scale_applied = z;
        }

        self.drive_snapshot_push(ctx);
        self.drive_widget_snapshot(ctx);
        self.save_pending_screenshot(ctx);
        self.load_marker_icons(ctx);
        self.drive_loop_export(ctx);
        self.apply_chase();
        self.sync_share(ctx);
        self.poll_sync();
        self.sync_forecast_scrub();
        self.poll_messages();
        self.poll_overlays();
        // Time-machine warnings + storm reports: swap in archived sets while scrubbed.
        self.sync_archive_warnings(ctx);
        self.sync_archive_lsr(ctx);
        // Surface obs (METAR station plots).
        self.sync_metar(ctx);
        self.sync_webcams(ctx);
        self.sync_fires(ctx);
        self.sync_aqi(ctx);
        self.sync_stations(ctx);
        self.sync_dat(ctx);
        self.sync_mosaic(ctx);
        // River flood gauges (NWPS).
        self.sync_gauges(ctx);
        // HRRR model contours.
        self.sync_contours(ctx);
        // Hurricane-hunter observations: a mission transmits every 30 s, so 10 min is plenty.
        if self.show_recon
            && self
                .recon_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 600)
        {
            self.recon_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Recon);
        }
        // County outages: ODIN's upstream updates about every 15 min; 5 min keeps a fast-moving
        // event current without asking much of a free, keyless API.
        if self.show_outages
            && self
                .outages_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 300)
        {
            self.outages_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Outages);
        }
        // NHC tropical suite: refresh every 15 min while enabled.
        if self.show_tropical
            && self
                .tropical_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 900)
        {
            self.tropical_last_fetch = Some(Instant::now());
            self.spawn_overlay(
                ctx,
                OverlaySource::Tropical(self.tropical_wind_kt, self.tropical_surge),
            );
        }
        // Periodic overlay refresh (~2 min), honoring live weather cadence. Skipped entirely
        // while backgrounded — see `platform::activity`.
        let overlay_secs = if crate::platform::is_metered() {
            240
        } else {
            120
        };
        // On the web the first pass through here *is* the boot fetch (App::new skips it), so hold
        // it until there is radar on screen — or five seconds have gone by and the radar is
        // evidently not coming, in which case the overlays are the only thing left to draw.
        let overlays_may_start = !cfg!(target_arch = "wasm32")
            || self.views[self.active].volume.is_some()
            || self.boot_at.elapsed().as_secs() >= 5;
        if overlays_may_start
            && crate::platform::activity::is_active()
            && self
                .overlay_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= overlay_secs)
        {
            self.fetch_overlays(ctx);
        }
        // MRMS national mosaic: fetch when enabled, refresh at the ~2-min product cadence.
        // National field layers: fetch each enabled layer at its product cadence.
        use crate::render::FieldLayer as FL;
        for layer in FL::DRAW_ORDER {
            // Layers with a fetch block of their own answer `None` and are skipped here.
            let Some(product) = self.mrms_product(layer) else {
                continue;
            };
            // The reflectivity tint reads the precipitation-type grid whether or not that
            // layer is being drawn, so wanting the tint counts as wanting the layer's data.
            let wanted =
                self.field_wanted(layer) || (layer == FL::PrecipType && self.settings.precip_tint);
            let stale = wanted
                && self.fields.get(&layer).is_none_or(|s| {
                    s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
                });
            if stale {
                self.fields.entry(layer).or_default().last_fetch = Some(Instant::now());
                self.spawn_overlay(ctx, OverlaySource::Field(layer, product));
            }
        }
        // Snow bands: the mosaic and the precipitation-type grid, cut to the banded snow.
        {
            let layer = FL::SnowBands;
            let stale = self.field_wanted(layer)
                && self.fields.get(&layer).is_none_or(|s| {
                    s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
                });
            if stale {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.spawn_overlay(ctx, OverlaySource::SnowBands);
            }
        }
        // Environment suite (HRRR CAPE/SRH): fetch each enabled layer at f00, refresh ~15 min.
        for layer in [FL::Cape, FL::Srh] {
            let stale = self.field_wanted(layer)
                && self.fields.get(&layer).is_none_or(|s| {
                    s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
                });
            if stale {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.spawn_overlay(
                    ctx,
                    OverlaySource::Env(layer, self.env_model, self.env_cape_ml, self.env_srh_km),
                );
            }
        }
        // Global models: whichever source and forecast hour the user picked.
        for (layer, gfield) in [
            (FL::GlobalMslp, wxdata::global::GlobalField::Mslp),
            (FL::GlobalHeight500, wxdata::global::GlobalField::Height500),
            (FL::GlobalTemp2m, wxdata::global::GlobalField::Temp2m),
            (
                FL::GlobalDewpoint2m,
                wxdata::global::GlobalField::Dewpoint2m,
            ),
            (FL::GlobalWind10m, wxdata::global::GlobalField::Wind10m),
            (FL::GlobalPrecip, wxdata::global::GlobalField::Precip),
        ] {
            let fh = self.global_fcst_hour;
            let model = self.global_model;
            let on = self.field_wanted(layer);
            let stale = on
                && self.fields.get(&layer).is_some_and(|s| {
                    s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
                });
            // Changing the source or the hour has to refetch now, not on the next slow cadence.
            let changed = on && self.global_layer_key.get(&layer) != Some(&(model, fh));
            if stale || changed {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.global_layer_key.insert(layer, (model, fh));
                self.spawn_overlay(ctx, OverlaySource::Global(layer, model, gfield, fh));
            }
        }
        // Model difference: same cadence as a global layer, and the same refetch-on-change rule.
        {
            let layer = FL::ModelDiff;
            let fh = self.global_fcst_hour;
            let on = self.field_wanted(layer);
            let stale = on
                && self.fields.get(&layer).is_some_and(|s| {
                    s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
                });
            let changed = on && self.diff_key != Some((self.diff_field, fh));
            if stale || changed {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.diff_key = Some((self.diff_field, fh));
                self.spawn_overlay(ctx, OverlaySource::ModelDiff(self.diff_field, fh));
            }
        }
        // HRRR rotation tracks + smoke: same forecast-hour scrub as future radar, own cadences.
        for layer in [
            FL::UpdraftHelicity,
            FL::Smoke,
            FL::Snowfall,
            FL::ThunderProb,
        ] {
            let fh = self.hrrr_fcst_hour;
            let stale = self.field_wanted(layer)
                && self.fields.get(&layer).is_none_or(|s| {
                    s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
                });
            // Scrubbing the forecast tail must refetch immediately, not wait out the cadence.
            let hour_changed =
                self.field_wanted(layer) && self.hrrr_layer_hour.get(&layer) != Some(&fh);
            if stale || hour_changed {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.hrrr_layer_hour.insert(layer, fh);
                self.spawn_overlay(ctx, OverlaySource::HrrrLayer(layer, fh));
            }
        }
        // Quiet hours just ended: replay what it held back as one push, so waking up to a silent
        // night does not mean waking up to no idea what happened during it.
        let now_quiet = self.in_quiet_hours();
        if self.was_quiet && !now_quiet {
            let held = match self.quiet_queue.lock() {
                Ok(mut q) => std::mem::take(&mut *q),
                Err(_) => Vec::new(),
            };
            if !held.is_empty() {
                let (title, body) = quiet_summary(&held);
                self.notify_alert(&title, &body, false);
            }
        }
        self.was_quiet = now_quiet;
        // Hold the queue on disk as it changes, not only on a clean exit. A crash or a kill
        // during quiet hours used to lose the whole night's held alerts; the queue is a handful
        // of short strings, so comparing it every tick and writing only on a real change costs
        // nothing worth measuring.
        if let Ok(q) = self.quiet_queue.lock() {
            if *q != self.settings.quiet_pending {
                self.settings.quiet_pending = q.clone();
                drop(q);
                self.settings.save();
            }
        }

        // GOES lightning: granules land every 20 s, so poll about that often. One in flight at a
        // time — a slow fetch must not queue up behind itself.
        let glm_fed_on = self.field_wanted(FL::GlmFed);
        // A lightning-density rule needs the flashes polled and the grid built even with the
        // layer off, exactly like the scan signatures.
        let glm_rule_armed = self.settings.alert_rules.iter().any(|r| {
            r.enabled
                && matches!(
                    r.trigger,
                    crate::settings::RuleTrigger::GlmFed | crate::settings::RuleTrigger::GlmJump
                )
        });
        if (self.show_glm || glm_fed_on || glm_rule_armed)
            && self
                .glm_last_poll
                .is_none_or(|t| t.elapsed().as_secs() >= 20)
            && !self.glm_polling.load(std::sync::atomic::Ordering::Relaxed)
        {
            self.glm_last_poll = Some(Instant::now());
            self.glm_polling
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let feed = self.glm.clone();
            let west = self.settings.glm_goes_west;
            let busy = self.glm_polling.clone();
            let http = self.http.clone();
            let ctx2 = ctx.clone();
            self.spawner.spawn(async move {
                // Decode outside the lock: holding it across an await would stall the painter.
                let mut local = wxdata::glm::GlmFeed::new(15);
                local.set_west(west);
                if let Ok(f) = feed.lock() {
                    local.set_last_keys(f.last_keys().clone());
                }
                let added = local.poll(&http).await.unwrap_or(0);
                if let Ok(mut f) = feed.lock() {
                    f.absorb(local);
                }
                if added > 0 {
                    ctx2.request_repaint();
                }
                busy.store(false, std::sync::atomic::Ordering::Relaxed);
            });
        }

        // GLM flash-extent density: the same flashes the dots come from, gridded. Cheap enough
        // (one pass over a few thousand points) to do inline on the field-layer cadence rather
        // than spawning for it.
        if (glm_fed_on || glm_rule_armed)
            && self
                .glm_fed_last
                .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(FL::GlmFed))
        {
            self.glm_fed_last = Some(Instant::now());
            if let Some(s) = self.fields.get_mut(&FL::GlmFed) {
                s.last_fetch = Some(Instant::now());
            }
            let field = self.glm.lock().ok().and_then(|f| {
                wxdata::glm::flash_density(
                    f.flashes(),
                    self.settings.detectors.glm_fed_cell_deg,
                    chrono::Duration::minutes(self.settings.detectors.glm_fed_window_min),
                    Utc::now(),
                )
            });
            if let Some(field) = &field {
                self.evaluate_grid_rules(crate::settings::RuleTrigger::GlmFed, field);
                // The jump is the difference between this grid and the one before it, so it can
                // only be asked for once there is a previous one — the first grid after launch
                // has no rate.
                if let Some(prev) = &self.glm_fed_prev {
                    if let Some(jump) = wxdata::glm::flash_jump(prev, field) {
                        self.evaluate_grid_rules(crate::settings::RuleTrigger::GlmJump, &jump);
                    }
                }
                self.glm_fed_prev = Some(field.clone());
            }
            if let (Some(field), true) = (field, glm_fed_on) {
                let cap = self.field_texture_cap();
                let _ = self
                    .overlay_tx
                    .send(OverlayMsg::Field(FL::GlmFed, field.decimated(cap)));
            }
        }

        // Wind particles. HRRR posts hourly, so this gets its own 15-minute clock rather than
        // riding the 120 s overlay block — that would re-download 4.5 MB about thirty times per
        // useful update. Doubled on a metered connection.
        if self.show_wind {
            // Advection timestep, shared by every pane so they stay in step. Clamped because a
            // stalled frame or a resume from background would otherwise teleport the whole field.
            let now = Instant::now();
            self.wind_dt = self
                .wind_last_frame
                .map_or(0.0, |t| now.duration_since(t).as_secs_f32())
                // Headroom above the 100 ms Android cadence, so a normal phone frame is never
                // itself treated as a hitch and quietly slowed down.
                .clamp(0.0, 0.15);
            self.wind_last_frame = Some(now);
            // The app is otherwise idle-driven; this is its first always-on animation, and the
            // cost is not the particle mesh — it is re-rendering the whole map (radar warp,
            // vector basemap) every frame instead of sitting idle. Measured on an S24 Ultra:
            // 7% CPU idle, 78% animating at 20 fps, so the cadence is the battery knob. 10 fps on
            // a phone reads fine because the trail is itself the motion blur.
            // An unfocused window animating particles nobody is looking at is the whole cost
            // for none of the value; the idle heartbeat carries it until focus returns, and the
            // `wind_dt` clamp above absorbs the jump.
            if crate::platform::activity::is_active() && ctx.input(|i| i.focused) {
                let ms = if cfg!(target_os = "android") { 100 } else { 33 };
                ctx.request_repaint_after(std::time::Duration::from_millis(ms));
            }

            // Panes come and go with the layout; their particle sets should not outlive them.
            self.wind_particles.retain(|k, _| *k < self.views.len());

            let want = (self.wind_level, self.hrrr_fcst_hour);
            let interval = if crate::platform::is_metered() {
                1800
            } else {
                900
            };
            let stale = self
                .wind_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= interval);
            // A level or forecast-hour change refetches at once — but the ~200 ms floor keeps a
            // fast drag across the forecast tail from firing a request per frame.
            let changed = self.wind_fetched != Some(want)
                && self
                    .wind_last_fetch
                    .is_none_or(|t| t.elapsed().as_millis() >= 200);
            // A dropped fetch (spawn_overlay only logs errors) expires rather than wedging.
            let free = self
                .wind_inflight
                .is_none_or(|t| t.elapsed().as_secs() >= 60);
            if (stale || changed) && free {
                self.wind_last_fetch = Some(Instant::now());
                self.wind_inflight = Some(Instant::now());
                self.wind_fetched = Some(want);
                self.spawn_overlay(ctx, OverlaySource::Wind(want.0, want.1));
            }
        }

        // Observed snowfall analysis: its own block because the accumulation window is a knob,
        // and changing it must refetch at once rather than wait out the cadence.
        {
            let on = self.field_wanted(FL::SnowAnalysis);
            let stale = self.fields.get(&FL::SnowAnalysis).is_some_and(|s| {
                s.last_fetch
                    .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(FL::SnowAnalysis))
            });
            let window_changed = self.snow_fetched != Some(self.snow_hours);
            if on && (stale || window_changed) {
                if let Some(s) = self.fields.get_mut(&FL::SnowAnalysis) {
                    s.last_fetch = Some(Instant::now());
                }
                self.snow_fetched = Some(self.snow_hours);
                self.spawn_overlay(ctx, OverlaySource::Snow(self.snow_hours));
            }
        }
        // Crowd precip-type reports. Skipped entirely without a key — the layer is opt-in twice
        // over: you turn it on, and you supply your own mPING key.
        if self.show_mping
            && !self.settings.mping_key.trim().is_empty()
            && self
                .mping_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 300)
        {
            self.mping_last_fetch = Some(Instant::now());
            let key = self.settings.mping_key.trim().to_string();
            self.spawn_overlay(ctx, OverlaySource::Mping(key));
        }
        // Surface analysis: WPC reissues it a few times an hour.
        if self.show_fronts
            && self
                .fronts_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 1800)
        {
            self.fronts_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Fronts);
        }
        // Locally derived products: no fetch, just a recompute when the volume or threshold moves.
        self.recompute_derived(ctx);
        // Beam-blockage raster: rebuilt when the camera, site, or tilt moves (DEM tiles are cached).
        self.update_blockage(ctx);
        // Gridded L3 products (DVL/EET): per-site, refetch on the L3 cadence or a site change.
        let l3_site = self.views[self.active].site.clone();
        let site_changed = self.l3grid_site != l3_site;
        for layer in [FL::Vil, FL::EchoTops, FL::Hca] {
            let on = self.field_wanted(layer);
            if !on {
                continue;
            }
            let stale = self.fields.get(&layer).is_some_and(|s| {
                s.last_fetch
                    .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
            });
            if let Some(site) = &l3_site {
                if stale || site_changed {
                    if let Some(s) = self.fields.get_mut(&layer) {
                        s.last_fetch = Some(Instant::now());
                    }
                    self.spawn_overlay(ctx, OverlaySource::L3Grid(layer, site.clone()));
                }
            }
        }
        if site_changed
            && [FL::Vil, FL::EchoTops, FL::Hca]
                .iter()
                .any(|l| self.field_wanted(*l))
        {
            self.l3grid_site = l3_site;
        }
        // HRRR future radar: fetch when enabled and the forecast hour changed or the run refreshed
        // (~10-min throttle; a new run posts hourly).
        let hrrr_on = self.field_wanted(FL::Hrrr);
        if hrrr_on {
            let hour_changed = self.hrrr_fetched_hour != Some(self.hrrr_fcst_hour);
            let stale = self
                .hrrr_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 600);
            if hour_changed || stale {
                self.hrrr_fetched_hour = Some(self.hrrr_fcst_hour);
                self.hrrr_last_fetch = Some(Instant::now());
                self.spawn_overlay(ctx, OverlaySource::Hrrr(self.hrrr_fcst_hour));
            }
        }
        // Live LSR refresh (~2-min cadence; the IEM feed is minutes-fresh).
        if self.show_storm_reports
            && self
                .reports_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 120)
        {
            self.reports_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::StormReports(None));
        }
        // Aviation SIGMET/AIRMET refresh (10-min cadence).
        if self.show_aviation
            && self
                .aviation_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 600)
        {
            self.aviation_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Aviation);
        }
        // TFR refresh. Slow, because a restriction's shape never changes once issued — the
        // cadence is really about noticing new ones. While the first load is still filling in,
        // the next batch is asked for promptly instead.
        if self.show_tfr {
            let due = if self.tfr_pending > 0 { 5 } else { 900 };
            if self
                .tfr_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= due)
            {
                self.tfr_last_fetch = Some(Instant::now());
                let have: Vec<String> = self.tfr_features.keys().cloned().collect();
                self.spawn_overlay(ctx, OverlaySource::Tfr(have));
            }
        }
        // Spotter Network refresh (feed's own 1-min cadence).
        if self.show_spotters
            && self
                .spotters_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            self.spotters_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Spotters);
        }
        // ProbSevere refresh (~2-min product cadence).
        if self.show_probsevere
            && self
                .probsevere_last_fetch
                .is_none_or(|t| t.elapsed().as_secs() >= 120)
        {
            self.probsevere_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::ProbSevere);
        }
        // Sensors: fetch when the window is open and the site changed or the 10-min clock elapsed.
        if self.show_sensors {
            if let Some(site) = self.views[self.active].site.clone() {
                let stale = self
                    .sensor_last_fetch
                    .is_none_or(|t| t.elapsed().as_secs() >= 600);
                let site_changed = self.sensor_site.as_deref() != Some(site.as_str());
                if stale || site_changed {
                    if let Some(s) = wxdata::sites::site_by_id(&site) {
                        if site_changed {
                            self.sensor_data = None; // show "loading" until the new site returns
                        }
                        self.sensor_last_fetch = Some(Instant::now());
                        self.spawn_overlay(
                            ctx,
                            OverlaySource::Obs {
                                site: site.clone(),
                                lat: s.latitude as f64,
                                lon: s.longitude as f64,
                            },
                        );
                    }
                }
            }
        }
        // VAD hodograph: fetch when open and the site changed or the 5-min clock elapsed.
        if self.show_hodo {
            if let Some(site) = self.views[self.active].site.clone() {
                let stale = self
                    .hodo_last_fetch
                    .is_none_or(|t| t.elapsed().as_secs() >= 300);
                let site_changed = self.hodo_site.as_deref() != Some(site.as_str());
                if stale || site_changed {
                    if site_changed {
                        self.hodo_data.clear();
                    }
                    self.hodo_last_fetch = Some(Instant::now());
                    self.spawn_overlay(ctx, OverlaySource::Vwp(site));
                }
            }
        }
        self.sync_placefiles(ctx);
        self.sync_pf_icons(ctx);
        // Bindings are polled once, globally: a hotkey works the same in OBS mode, on mobile, and
        // with the drawer open. `capture_key` suppresses the table while the Hotkeys tab is
        // listening for the next keypress.
        if !self.capture_key {
            let bindings = hotkeys::active(&self.settings).into_owned();
            for action in hotkeys::poll(ctx, &bindings) {
                self.apply_action(action, ctx);
            }
        }

        self.drive_obs_tour();

        // eframe's root Ui spans the full viewport (deliberately edge-to-edge), so panels ignore
        // egui's safe area; reserve the system-bar strips ourselves. Floating windows/areas
        // constrain to content_rect natively. Zero-size off-Android (insets only fed there).
        let vr = ctx.viewport_rect();
        let cr = ctx.content_rect();
        if cr.top() > vr.top() {
            egui::Panel::top("safe_top")
                .exact_size(cr.top() - vr.top())
                .frame(egui::Frame::NONE)
                .show(root, |_| {});
        }
        if vr.bottom() > cr.bottom() {
            egui::Panel::bottom("safe_bottom")
                .exact_size(vr.bottom() - cr.bottom())
                .frame(egui::Frame::NONE)
                .show(root, |_| {});
        }
        if cr.left() > vr.left() {
            egui::Panel::left("safe_left")
                .exact_size(cr.left() - vr.left())
                .frame(egui::Frame::NONE)
                .show(root, |_| {});
        }
        if vr.right() > cr.right() {
            egui::Panel::right("safe_right")
                .exact_size(vr.right() - cr.right())
                .frame(egui::Frame::NONE)
                .show(root, |_| {});
        }

        // The tour highlights real chrome, so the rects have to come from this frame's draw.
        self.tour_anchors = Default::default();

        // Docked desktop chrome. Declared before every floating Area so those constrain to what's
        // left of the viewport (`self.chrome_rect`) instead of covering the bars.
        // Embedded panes keep their chrome: without it there is no play button, no site picker and
        // no product menu, and an iframe is exactly where a user can't reach those any other way.
        // `embed` still buys the idle heartbeat and the state postMessage.
        let bare = self.obs_mode;

        self.chrome_rect = root.available_rect_before_wrap();
        // Before any chrome: everything below asks `motion::reduced()`, and the answer has to be
        // the same for every surface in a frame.
        ui::motion::frame(ctx, self.settings.reduce_motion);

        // Chrome: touch-first on Android (top chips + bottom sheet + docked toolbar), desktop
        // otherwise (the floating map-first chrome below). Both funnel into the same `UiActions`
        // handling. The occlusion rects are rebuilt from scratch every frame; a stale rect would
        // keep swallowing gestures over a sheet that closed.
        self.mobile_occlusion.clear();
        if !bare {
            // The phone draws its own top strips and back wiring first, and can ask for the rest
            // to be skipped entirely (the hide-all-chrome eye). Desktop draws the window frame
            // first instead: its drag strip covers the top edge, and everything after it takes
            // back the clicks that land on an actual control.
            let chrome = if cfg!(target_os = "android") {
                self.mobile_chrome(ctx)
            } else {
                self.window_frame(ctx);
                true
            };
            if chrome {
                self.sync_permalink();
                self.search_pill(ctx);
                self.control_column(ctx);
                self.scrubber(ctx);
                self.pane_strip(ctx);
                self.panel(ctx);
                self.basemap_panel(ctx);
                self.info_chip(ctx);
                self.error_chip(ctx);
                self.update_chip(ctx);
            }
        }

        // The one decoration with no job. Costs nothing after the first second and a half, and
        // never runs at all under OBS — a capture is not a place for a flourish.
        if !bare {
            ui::motion::intro(
                ctx,
                self.chrome_rect,
                crate::theme::accent(self.settings.theme),
            );
        }

        // Over everything, and only while a capture is pending.
        self.share_card_footer(ctx);

        // The spotlight tour, over the chrome it points at. Paused under the first-run card and
        // the cheat sheet — all three draw on `Order::Foreground` and would fight for it.
        if self.tour.open && !self.firstrun.open && !self.show_cheatsheet {
            let v = &self.views[self.active];
            let sig = ui::tour::Signals {
                moment: v.moment,
                srv: v.srv,
                playhead: v.timeline.playhead,
                following: v.timeline.following,
            };
            let accent = crate::theme::accent(self.settings.theme);
            self.tour.advance_if_done(sig);
            let anchors = self.tour_anchors;
            self.tour.show(ctx, &anchors, sig, accent);
        }

        // One quiet check per session, once the app has settled — so a stale build tells you so
        // without anyone opening About.
        if ctx.input(|i| i.time) > 30.0 {
            self.check_for_update(ctx);
        }
        while let Ok(state) = self.update_rx.try_recv() {
            self.update_state = state;
        }
        if self.about_open {
            let accent = crate::theme::accent(self.settings.theme);
            let mut open = self.about_open;
            ui::about_window::show(ctx, &mut open, &self.update_state, accent, &mut self.drawer);
            self.about_open = open;
        }

        // The `?` cheat sheet floats over everything, including the first-run card.
        if self.show_cheatsheet {
            let entries = self.palette_entries();
            let bindings = hotkeys::active(&self.settings).into_owned();
            let accent = crate::theme::accent(self.settings.theme);
            self.show_cheatsheet = ui::cheatsheet::show(ctx, &bindings, &entries, accent);
        }

        // First run: pick a radar (or let the location do it) and get out of the way.
        if let Some(fin) = ui::firstrun::show(ctx, &mut self.firstrun, &mut self.settings) {
            self.settings.setup_done = true;
            self.settings.save();
            let v = &mut self.views[self.active];
            v.site = Some(fin.site.clone());
            ui::site_dialog::center_on_site(&mut v.camera, &fin.site);
            if fin.located {
                // Nobody chose this site, so say which one it is and that it can be changed.
                self.toast(
                    ToastKind::Info,
                    format!(
                        "Nearest radar: {} — change it any time in the panel",
                        fin.site
                    ),
                );
            }
            if fin.take_tour {
                self.tour.start();
            }
        }
        if !self.firstrun.open && !self.settings.setup_done {
            // Dismissed without finishing. Setup is optional and re-runnable from three places,
            // so take the ✕ at its word rather than reopening this on every launch.
            self.settings.setup_done = true;
            self.settings.save();
        }

        // Floating windows.
        if let Some(dialog) = &mut self.site_dialog {
            let keep = ui::site_dialog::show(
                ctx,
                dialog,
                &mut self.views[self.active],
                &mut self.settings,
                &mut self.drawer,
            );
            if !keep {
                self.site_dialog = None;
            }
        }
        // Only the open settings window reads these; building the registry for a closed window
        // was a few hundred String allocations every frame.
        let entries = if self.settings_window.open {
            self.palette_entries()
        } else {
            std::sync::Arc::from(Vec::new())
        };
        let sync_view = ui::settings_window::SyncView {
            signed_in: self.sync_tokens.is_some(),
            status: &self.sync_status,
            login_url: self.sync_login.as_ref().map(|p| p.url.as_str()),
            last_sync: self.sync_state.last_sync,
        };
        let sync_action = self.settings_window.show(
            ctx,
            &mut self.settings,
            &self.palettes,
            sync_view,
            &entries,
            &mut self.drawer,
        );
        self.capture_key = self.settings_window.capturing;
        if std::mem::take(&mut self.settings_window.run_setup) {
            self.firstrun.start();
        }
        if std::mem::take(&mut self.settings_window.run_tour) {
            self.tour.start();
        }
        match sync_action {
            Some(ui::settings_window::SyncAction::SignIn) => self.sync_sign_in(),
            Some(ui::settings_window::SyncAction::SignOut) => self.sync_sign_out(),
            Some(ui::settings_window::SyncAction::SyncNow) => self.sync_now(),
            None => {}
        }
        let pf_status: Vec<ui::placefile_window::PlacefileStatus> = self
            .placefiles
            .iter()
            .map(|lp| ui::placefile_window::PlacefileStatus {
                url: lp.url.clone(),
                loaded: lp.loaded,
                items: lp.pf.items.len(),
                title: lp.pf.title.clone(),
                error: lp.error.clone(),
            })
            .collect();
        self.placefile_window
            .show(ctx, &mut self.settings, &pf_status, &mut self.drawer);
        // Names come from the action registry, so a layer reads the same here as in the layers
        // panel — the enum's Debug spelling ("Mrms") is not a label.
        let names: std::collections::HashMap<crate::render::FieldLayer, String> =
            if self.layer_window_open {
                self.palette_entries()
                    .iter()
                    .filter_map(|e| match e.action {
                        PaletteAction::ToggleField(l) => Some((l, e.label.clone())),
                        _ => None,
                    })
                    .collect()
            } else {
                Default::default()
            };
        let active_fields: Vec<(crate::render::FieldLayer, String)> =
            crate::render::FieldLayer::DRAW_ORDER
                .into_iter()
                .filter(|l| self.field_wanted(*l))
                .map(|l| {
                    let name = names.get(&l).cloned().unwrap_or_else(|| format!("{l:?}"));
                    (l, name)
                })
                .collect();
        if ui::layer_window::show(
            ctx,
            &mut self.layer_window_open,
            &mut self.settings,
            &active_fields,
            &mut self.drawer,
        ) {
            self.overlay_gen += 1; // paint order / opacity changed — re-tessellate
        }
        // The edge's geo-IP fix, if it beat the user to it: move the default view to the radar
        // that covers them. Skipped once they have panned or picked a site themselves.
        while let Ok((lon, lat)) = self.ipgeo_rx.try_recv() {
            if self.geocode_nav || self.views.len() > 1 {
                continue;
            }
            if let Some(site) = crate::geo::nearest_site_id(lon, lat) {
                self.goto_view(&site, lon, lat, 8.0, None);
            }
        }
        // Drain geocode results: the search pill navigates, the marker window adds a marker.
        while let Ok(res) = self.geocode_rx.try_recv() {
            if std::mem::take(&mut self.geocode_nav) {
                match res {
                    Ok((name, lat, lon)) => {
                        let cam = &mut self.views[self.active].camera;
                        cam.center = crate::render::mercator::lonlat_to_world(lon, lat);
                        cam.zoom = cam.zoom.max(9.0);
                        self.save_offer = Some((
                            short_place_name(&name).to_string(),
                            lat,
                            lon,
                            Instant::now(),
                        ));
                        self.place_status = Some((name, Instant::now()));
                        self.place_query.clear();
                    }
                    Err(e) => self.place_status = Some((e, Instant::now())),
                }
                continue;
            }
            self.marker_window.searching = false;
            match res {
                Ok((name, lat, lon)) => {
                    self.settings.markers.push(crate::settings::Marker {
                        id: crate::settings::new_marker_id(),
                        name: name.clone(),
                        lat,
                        lon,
                        icon: None,
                        alert_radius_mi: crate::settings::default_alert_radius_mi(),
                        video_url: String::new(),
                        home: false,
                    });
                    self.settings.save();
                    // Fly the active pane to the new marker (same idiom as the alert panel).
                    let cam = &mut self.views[self.active].camera;
                    cam.center = crate::render::mercator::lonlat_to_world(lon, lat);
                    cam.zoom = cam.zoom.max(9.0);
                    self.marker_window.status = Some(format!("Added \"{name}\""));
                    self.marker_window.query.clear();
                }
                Err(e) => self.marker_window.status = Some(e),
            }
        }
        let metric = self.metric();
        let query = self.marker_window.show(
            ctx,
            &mut self.settings,
            &self.marker_icon_tex,
            &mut self.drawer,
            metric,
        );
        // The map popup indexes into the same list: a delete above it leaves it describing the
        // wrong marker, which is the one way this UI can lie about which place you are editing.
        if let (Some(gone), Some(open)) = (self.marker_window.removed, self.marker_popup) {
            self.marker_popup = match gone.cmp(&open) {
                std::cmp::Ordering::Less => Some(open - 1),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(open),
            };
        }
        if let Some(query) = query {
            self.marker_window.searching = true;
            self.marker_window.status = Some("Searching…".into());
            let http = self.http.clone();
            let tx = self.geocode_tx.clone();
            let ctx2 = ctx.clone();
            self.spawner.spawn(async move {
                let _ = tx.send(wxdata::geocode::search(&http, &query).await);
                ctx2.request_repaint();
            });
        }
        // Drain chase-pack worker outcomes; drop the download state once every tile is accounted for.
        if let Some(pack) = &mut self.chasepack {
            while let Ok((ok, n)) = pack.rx.try_recv() {
                pack.done += 1;
                pack.bytes += n;
                if !ok {
                    pack.errors += 1;
                }
            }
            if pack.done >= pack.total {
                self.chasepack = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        self.palette_editor
            .show(ctx, &mut self.settings, &self.palettes, &mut self.drawer);
        // Storm digest: poll a pending Claude result, then render + handle Generate.
        if let Some(rx) = &self.digest_rx {
            if let Ok(res) = rx.try_recv() {
                self.digest_window.busy = false;
                self.digest_rx = None;
                match res {
                    Ok(text) => {
                        self.digest_window.text = text;
                        self.digest_window.enhanced = true;
                    }
                    Err(e) => log::warn!("digest enhancement failed: {e}"),
                }
            }
        }
        if let Some(ui::digest_window::DigestAction::Generate) =
            self.digest_window.show(ctx, &mut self.drawer)
        {
            self.generate_digest();
        }
        // Live station cards. Video keeps arriving between input events, so a playing card asks
        // for the next frame itself rather than waiting for the idle heartbeat.
        {
            // A card opened before the camera catalog landed gets its camera as soon as it does.
            let (rt, http) = (self.spawner.clone(), self.http.clone());
            self.stations.pair_cameras(&rt, &http, ctx);
            let tz = self.active_tz();
            if self.stations.show_cards(ctx, tz) {
                ctx.request_repaint_after(std::time::Duration::from_millis(33));
            }
        }
        // Area Forecast Discussion: poll the async fetch, then render the text window.
        if let Some(rx) = &self.afd_rx {
            if let Ok(res) = rx.try_recv() {
                self.afd_busy = false;
                self.afd_rx = None;
                match res {
                    Ok(afd) => self.afd = Some(afd),
                    Err(e) => self.afd_error = Some(e),
                }
            }
        }
        if self.afd_open {
            let refresh = ui::afd_window::show(
                ctx,
                &mut self.afd_open,
                self.afd.as_ref(),
                self.afd_busy,
                self.afd_error.as_deref(),
                &mut self.drawer,
            );
            if refresh {
                self.fetch_afd();
            }
        }
        // NHC text products: poll the fetch, then draw the reader.
        if let Some(rx) = &self.tropical_text_rx {
            if let Ok(res) = rx.try_recv() {
                self.tropical_window.busy = false;
                self.tropical_text_rx = None;
                match res {
                    Ok(a) => {
                        self.tropical_window.text = Some(a);
                        self.tropical_window.error = None;
                    }
                    Err(e) => {
                        self.tropical_window.text = None;
                        self.tropical_window.error = Some(e);
                    }
                }
            }
        }
        if self.tropical_window.open {
            let storms = self
                .tropical
                .as_ref()
                .map(|t| t.storms.clone())
                .unwrap_or_default();
            if let Some((id, product)) =
                ui::tropical_window::show(&mut self.tropical_window, ctx, &storms, &mut self.drawer)
            {
                self.fetch_tropical_text(&id, product);
            }
        }
        // Point sounding: poll the async fetch, then render the Skew-T / hodograph.
        if let Some(rx) = &self.sounding_rx {
            if let Ok(res) = rx.try_recv() {
                self.sounding_window.busy = false;
                self.sounding_rx = None;
                match res {
                    Ok(s) => self.sounding_window.sounding = Some(s),
                    Err(e) => {
                        self.sounding_window.error = Some(e);
                    }
                }
            }
        }
        if let Some(rx) = &self.raob_rx {
            if let Ok(res) = rx.try_recv() {
                self.raob_rx = None;
                match res {
                    Ok(s) => self.sounding_window.observed = Some(s),
                    Err(e) => self.sounding_window.observed_error = Some(e),
                }
            }
        }
        let tz = self.active_tz();
        self.sounding_window.show(ctx, tz, &mut self.drawer);
        if std::mem::take(&mut self.sounding_window.refetch) {
            self.refetch_sounding();
        }
        // Warning verification lab: drain the query, then draw and act on its clicks.
        if let Some(rx) = &self.verify_rx {
            if let Ok(res) = rx.try_recv() {
                self.verify_window.busy = false;
                self.verify_rx = None;
                match res {
                    Ok(v) => self.verify_window.data = Some(v),
                    Err(e) => {
                        self.verify_window.error = Some(format!("verification unavailable: {e}"));
                    }
                }
            }
        }
        let tz = self.active_tz();
        let vact = self.verify_window.show(ctx, tz, &mut self.drawer);
        if vact.refresh {
            self.verify_window.data = None;
            self.fetch_verify();
        }
        if let Some((lon, lat, time)) = vact.goto {
            let site = self.views[self.active].site.clone().unwrap_or_default();
            self.goto_view(&site, lon, lat, 8.5, time);
        }
        // Point forecast: drain the fetch, cache the win, then draw.
        if let Some((key, rx)) = &self.forecast_rx {
            if let Ok(res) = rx.try_recv() {
                let key = *key;
                self.forecast_rx = None;
                self.forecast_state = match res {
                    Ok(f) => {
                        self.forecast_cache.insert(key, (Instant::now(), f.clone()));
                        ui::forecast_window::State::Ready(Box::new(f))
                    }
                    Err(e) => ui::forecast_window::State::Failed(e),
                };
            }
        }
        if let Some((key, rx)) = &self.forecast_obs_rx {
            if let Ok((station, ob)) = rx.try_recv() {
                let key = *key;
                self.forecast_obs_rx = None;
                self.forecast_obs_cache
                    .insert(key, (Instant::now(), station, ob));
            }
        }
        if self.forecast_open {
            let at = self.forecast_at.unwrap_or((0.0, 0.0));
            let tz = self.active_tz();
            let minute = self.minute_profile(at).map(|m| m.to_vec());
            let key = ((at.1 * 20.0).round() as i32, (at.0 * 20.0).round() as i32);
            let now = self
                .forecast_obs_cache
                .get(&key)
                .map(|(_, station, ob)| (station.as_str(), ob));
            if !ui::forecast_window::show(
                ctx,
                &self.forecast_state,
                at,
                tz,
                minute.as_deref(),
                now,
                &mut self.popovers,
            ) {
                self.forecast_open = false;
            }
        }
        // Tornado climatology: receive the loaded database, then run any queued query.
        if let Some(rx) = &self.climo_rx {
            if let Ok(res) = rx.try_recv() {
                self.climo_loading = false;
                self.climo_rx = None;
                match res {
                    Ok(tracks) => {
                        let tracks = std::sync::Arc::new(tracks);
                        self.climo_tracks = Some(tracks.clone());
                        if let Some((lon, lat)) = self.climo_pending_query.take() {
                            self.climo_hits = wxdata::torclimo::near(&tracks, lon, lat, 40.0);
                            self.climo_center = Some((lon, lat));
                        }
                    }
                    Err(e) => self.climo_error = Some(e),
                }
            }
        }
        // Warning history for the same point (independent request; a failure just leaves the
        // section blank rather than sinking the whole card).
        if let Some(rx) = &self.climo_warn_rx {
            if let Ok(res) = rx.try_recv() {
                self.climo_warn_rx = None;
                match res {
                    Ok(s) => self.climo_warn = Some(s),
                    Err(e) => log::warn!("warning history: {e}"),
                }
            }
        }
        self.show_climatology_window(ctx);
        // GOES frame times arrived → keep the scrub at latest until the user moves it.
        if let Some(rx) = &self.goes_times_rx {
            if let Ok(times) = rx.try_recv() {
                self.goes_times = times;
                self.goes_times_rx = None;
            }
        }
        self.goes_time_bar(ctx);
        if let Some(act) = self
            .event_window
            .show(ctx, &mut self.settings, &mut self.drawer)
        {
            use ui::event_window::EventAction;
            match act {
                EventAction::Goto {
                    site,
                    lon,
                    lat,
                    zoom,
                    time,
                    span_min,
                } => {
                    self.goto_view(&site, lon, lat, zoom, time);
                    if span_min > 0 && time.is_some() {
                        self.views[self.active].timeline.replay_span_min = span_min;
                        // A replay without its warnings and its damage reports is just a loop;
                        // both already follow the playhead once they are on.
                        self.filters.show_alerts = true;
                        self.show_storm_reports = true;
                        self.rebuild_overlays();
                    }
                }
                EventAction::AddBookmark(span_min) => {
                    let n = self.settings.bookmarks.len() + 1;
                    self.add_bookmark(format!("Bookmark {n}"), span_min);
                }
            }
        }

        let metric = self.metric();
        if let Some(act) = self
            .chase_replay
            .show(ctx, &self.chase_track, &mut self.drawer, metric)
        {
            use ui::chase_replay::ReplayAction;
            match act {
                ReplayAction::Seek { lon, lat, time } => {
                    // Empty site: the replay flies the camera and moves the clock, and leaves the
                    // radar choice alone — the same handoff rule the deep links use.
                    let zoom = self.views[self.active].camera.zoom.max(8.0);
                    self.goto_view("", lon, lat, zoom, Some(time));
                }
                ReplayAction::OpenFile => {
                    crate::dialog::request_open(crate::dialog::ImportKind::ChaseGpx, "");
                }
            }
        }

        if let Some((i, day)) = self.rules_window.backtest_request.take() {
            self.start_backtest(i, day);
        }
        if let Some(detail) = &self.detail {
            let tex = detail
                .image
                .as_ref()
                .and_then(|k| self.pf_icon_tex.get(k))
                .and_then(|t| t.as_ref());
            if !ui::detail_window::show(ctx, detail, tex, &mut self.popovers) {
                self.detail = None;
            }
        }
        // Storm attributes table: clicking a row flies there and opens that cell's popup, the
        // same destination as clicking the dot on the map.
        let entries = self.palette_entries();
        let bindings = crate::hotkeys::active(&self.settings).into_owned();
        if self
            .help_hub
            .show(ctx, &mut self.drawer, &bindings, &entries)
        {
            self.tour.start();
        }
        let cells: &[Cell] = if self.archive_bucket().is_some() {
            &[]
        } else {
            &self.storm_cells
        };
        // A cell within 15 km of a ZDR column owns it — the column marks the updraft, and the
        // updraft belongs to the storm the table is already listing.
        let zdr_cells: std::collections::HashSet<String> = match &self.zdr_cache {
            Some((_, hits, _)) if self.filters.show_zdr_columns => cells
                .iter()
                .filter(|c| {
                    hits.iter().any(|h| {
                        // Small distances: a flat-earth step is plenty and needs no helper.
                        let dy = (h.lat - c.lat) * 111.0;
                        let dx = (h.lon - c.lon) * 111.0 * c.lat.to_radians().cos();
                        (dx * dx + dy * dy).sqrt() <= 15.0
                    })
                })
                .map(|c| c.id.clone())
                .collect(),
            _ => std::collections::HashSet::new(),
        };
        let metric = self.metric();
        if ui::rules_window::show(
            &mut self.rules_window,
            ctx,
            &mut self.settings,
            &mut self.drawer,
            metric,
        ) {
            self.settings.save();
        }
        // One score per cell so the table can rank them; the join lives in wxdata.
        let couplets: &[wxdata::rotation::CoupletHit] = match &self.couplet_cache {
            Some((_, hits)) => hits,
            None => &[],
        };
        let cell_scores = wxdata::cellscore::score_all(cells, &self.probsevere, couplets);
        if let Some(id) = ui::cells_window::show(
            &mut self.cells_window,
            ctx,
            cells,
            &cell_scores,
            &zdr_cells,
            &self.cell_trends,
            crate::theme::accent(self.settings.theme),
            &mut self.drawer,
        ) {
            if let Some(c) = self
                .active_storm_cells()
                .iter()
                .find(|c| c.id == id)
                .cloned()
            {
                let cam = &mut self.views[self.active].camera;
                cam.center = crate::render::mercator::lonlat_to_world(c.lon, c.lat);
                cam.zoom = cam.zoom.max(8.0);
                self.cell_popup = Some(c);
            }
        }
        let mut open_3d: Option<[f32; 6]> = None;
        if let Some(cell) = &self.cell_popup {
            let trend = self
                .cell_trends
                .get(&cell.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let following = self
                .follow_cell
                .as_ref()
                .is_some_and(|(_, c, _)| c.id == cell.id);
            let (open, toggled, to_3d) =
                ui::cell_window::show(ctx, cell, trend, following, &mut self.popovers);
            // Crop the volume to this storm before opening it: the full 300 km box is a wall of
            // echo you would then have to hunt through by hand. The clip is computed here, where
            // the cell is still borrowed, and applied below.
            if to_3d {
                open_3d = Some(
                    self.views[self.active]
                        .site
                        .as_deref()
                        .and_then(wxdata::sites::site_by_id)
                        .map(|site| {
                            let (slat, slon) = (site.latitude as f64, site.longitude as f64);
                            let dy = ((cell.lat - slat) * 111.0) as f32;
                            let dx = ((cell.lon - slon) * 111.0 * slat.to_radians().cos()) as f32;
                            wxdata::volume3d::clip_around(150.0, dx, dy, 30.0)
                        })
                        .unwrap_or([0.0, 1.0, 0.0, 1.0, 0.0, 1.0]),
                );
            }
            if toggled {
                if following {
                    self.follow_cell = None;
                } else if let Some(site) = self.cells_site.clone() {
                    self.follow_cell = Some((site, cell.clone(), Instant::now()));
                    self.follow_notice = None;
                }
            }
            if !open {
                self.cell_popup = None;
            }
        }
        if let Some(clip) = open_3d {
            self.vol3d.clip = clip;
            self.build_volume3d();
        }
        if let Some(i) = self.marker_popup {
            match self.settings.markers.get_mut(i) {
                // The list shrank under us (the manager window deleted a row this frame).
                None => self.marker_popup = None,
                Some(m) => {
                    let r = ui::marker_popup::show(ctx, m, &mut self.popovers);
                    let watch = r
                        .watch
                        .then(|| (m.name.clone(), m.video_url.trim().to_string()));
                    if r.manage {
                        self.marker_window.open = true;
                    }
                    if let Some((name, url)) = watch {
                        self.watch_stream(name, url);
                    }
                    if r.remove {
                        self.settings.markers.remove(i);
                        self.marker_popup = None;
                    } else if !r.open {
                        self.marker_popup = None;
                    }
                }
            }
        }
        self.zone_popup_card(ctx);
        self.zone_naming_dialog(ctx);
        if let Some(sp) = self.pending_spotter.take() {
            self.open_spotter(&sp);
        }
        if let Some(p) = &mut self.video_player {
            if !p.show(ctx, &mut self.drawer) {
                self.video_player = None; // dropping the player stops the download
            }
        }
        self.follow_badge(ctx);
        if !self.obs_mode {
            self.chase_hud(ctx);
        }
        if let Some(popup) = &mut self.warning_popup {
            if !ui::warning_window::show(ctx, popup, &mut self.popovers) {
                self.warning_popup = None;
            }
        }
        let tz = self.active_tz();
        if self.show_sensors
            && !ui::sensor_window::show(ctx, self.sensor_data.as_ref(), tz, &mut self.drawer)
        {
            self.show_sensors = false;
        }
        if self.show_hodo
            && !ui::hodograph_window::show(
                ctx,
                self.hodo_site.as_deref(),
                &self.hodo_data,
                self.hodo_history.make_contiguous(),
                &mut self.hodo_tab,
                self.settings.tz_for(self.hodo_site.as_deref()),
                &mut self.drawer,
            )
        {
            self.show_hodo = false;
        }
        if let (Some(xs), Some(tex)) = (&self.xsection, &self.xsection_tex) {
            let mut moment = self.xsection_moment;
            let open = ui::xsection_window::show(ctx, xs, tex, &mut moment, &mut self.drawer);
            if !open {
                self.xsection = None;
                self.xsection_tex = None;
                self.xsection_pts.clear();
            } else if moment != self.xsection_moment {
                self.xsection_moment = moment;
                let idx = self.active;
                self.build_xsection(idx, ctx);
            }
        }
        if self.show_3d {
            self.drain_volume3d(ctx);
            let mut open = true;
            ui::volume3d_window::show(
                ctx,
                &mut open,
                &mut self.vol3d,
                &mut self.vol3d_pending,
                VOL3D_N as u32,
                VOL3D_NZ as u32,
                self.vol3d_range,
                &mut self.drawer,
            );
            self.show_3d = open;
        }
        if self.show_cappi {
            self.update_cappi(ctx);
            let open = match self.cappi_tex.clone() {
                Some(tex) => ui::cappi_window::show(
                    ctx,
                    &tex,
                    &mut self.cappi_alt_km,
                    300.0,
                    &mut self.drawer,
                ),
                None => ui::cappi_window::show_empty(ctx, &mut self.drawer),
            };
            self.show_cappi = open;
        }
        self.show_warning_banners(ctx);
        self.show_toasts(ctx);

        // Turn this frame's UI mutations into uploads/fetches before painting the map.
        for idx in 0..self.views.len() {
            self.sync_pane(idx, ctx);
        }
        self.sync_overlay();

        // OBS-mode hint so the chrome-free view is still escapable.
        if self.obs_mode {
            egui::Area::new("obs_hint".into())
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
                .interactable(false)
                .show(root, |ui| {
                    let txt = if self.obs_tour {
                        "OBS · tour (F8 exit · F9 stop tour)"
                    } else {
                        "OBS mode (F8 exit · F9 tour)"
                    };
                    egui::Frame::new()
                        .fill(egui::Color32::from_black_alpha(150))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.colored_label(egui::Color32::from_white_alpha(200), txt)
                        });
                });
        }

        let placefile_labels = self.placefile_labels_cached();
        // One occupancy set for the whole frame, across every pane and every label layer. Panes
        // occupy disjoint screen rects, so sharing it between them costs nothing and saves
        // resetting it per pane.
        self.labels.begin();
        egui::CentralPanel::default().show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            let n = self.views.len();
            // A phone shows one pane at a time. Two 400x400 pt panes stacked is two views of
            // nothing; the pane strip above the scrubber is how you get to the others.
            // Only where one pane is all that fits: a tablet shows the split.
            let solo = cfg!(target_os = "android") && n > 1 && chrome::compact(ctx);
            let rects = if solo {
                vec![full; n]
            } else {
                pane_rects(full, n)
            };

            // If cameras are linked, mirror the active pane's camera to the others.
            if self.link_cameras {
                let cam = self.views[self.active.min(n - 1)].camera;
                for v in &mut self.views {
                    v.camera = cam;
                }
            }

            // Each pane fetches and draws its own `View::basemap` (see `render_pane`). Two things
            // stay global for now: the GOES frame cursor and the vector palette, both driven by
            // the active pane.
            // ponytail: one GOES cursor + one vector palette for all panes; split them when
            // someone actually wants two satellite times or two vector palettes side by side.
            use crate::tiles::BasemapStyle;
            let style = self.views[self.active.min(n - 1)]
                .basemap
                .resolve(ctx.theme() == egui::Theme::Dark);
            let is_vector = style.vector_palette().is_some();
            let raster_style = if style.is_raster() {
                style
            } else {
                BasemapStyle::None
            };
            self.tiles
                .set_keys(&self.settings.mapbox_key, &self.settings.maptiler_key);
            self.tiles
                .set_custom_template(&self.settings.custom_tile_url);
            self.tiles.set_custom_max_z(self.settings.custom_tile_max_z);
            // Ask for `@2x` tiles where the provider serves them: same tile count, twice the
            // pixels, labels drawn for the density instead of magnified. Off on a metered link —
            // a double-resolution tile is roughly double the bytes.
            let mut clear_tiles = self
                .tiles
                .set_retina(ctx.pixels_per_point() > 1.0 && !crate::platform::is_metered());
            // GOES sub-hourly scrub: fetch the available frame times when a GOES style becomes
            // active, and apply the selected frame (None = latest).
            if raster_style.timed() {
                // Which hour of imagery to ask for: the pane's own clock when it is replaying an
                // archive, otherwise the live window ending now. GIBS keeps GeoColor about two
                // weeks and Band 13 several months, so a replayed event usually has satellite.
                let radar_time = self.views[self.active.min(n - 1)]
                    .volume
                    .as_ref()
                    .map(|v| v.time);
                let (hour, from, to) = crate::tiles::goes_window(chrono::Utc::now(), radar_time);
                if self.goes_times_style != Some(raster_style) || self.goes_hour != hour {
                    self.goes_times_style = Some(raster_style);
                    self.goes_hour = hour;
                    self.goes_times.clear();
                    self.goes_time_idx = None;
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.goes_times_rx = Some(rx);
                    let http = self.http.clone();
                    self.spawner.spawn(async move {
                        let times =
                            crate::tiles::fetch_frame_times(&http, raster_style, from, to, 48)
                                .await;
                        let _ = tx.send(times);
                    });
                }
                // Following the radar is the default: scrubbing back through an event should
                // take the satellite back with it, which is the whole reason to look at both.
                // Stepping the GOES arrows by hand drops out of it.
                let selected = if self.goes_follow_radar {
                    self.views[self.active.min(n - 1)]
                        .volume
                        .as_ref()
                        .map(|v| v.time)
                        .and_then(|t| nearest_goes(&self.goes_times, t))
                } else {
                    self.goes_time_idx
                        .and_then(|i| self.goes_times.get(i).copied())
                };
                // `None` means GIBS's own default, which is the newest imagery there is — right
                // for a live pane, three days wrong for one replaying an archive. In an archive
                // window, fall back to the newest frame *in that window* instead.
                let selected = match (selected, self.goes_hour) {
                    (None, Some(_)) => self.goes_times.last().copied(),
                    (s, _) => s,
                };
                clear_tiles |= self.tiles.set_goes_time(selected);
            } else if self.goes_times_style.is_some() {
                self.goes_times_style = None;
                self.goes_hour = None;
                self.goes_times.clear();
                clear_tiles |= self.tiles.set_goes_time(None);
            }
            let mut clear_vector = false;
            if is_vector {
                clear_vector |= self
                    .vtiles
                    .set_style(style.vector_palette().unwrap_or_default());
                clear_vector |= self.vtiles.set_theme(self.settings.theme);
                clear_vector |= self
                    .vtiles
                    .note_zoom(self.views[self.active.min(n - 1)].camera.zoom);
            }
            self.last_viewport = rects
                .get(self.active)
                .map_or((full.width(), full.height()), |r| (r.width(), r.height()));

            // Which pane carries the once-per-frame work (tile-cache clears, the shared label
            // pass): the first one actually drawn, which under `solo` is the active one.
            let head = if solo { self.active.min(n - 1) } else { 0 };
            for (i, prect) in rects.iter().enumerate() {
                if solo && i != head {
                    continue;
                }
                let first = i == head;
                self.render_pane(
                    ui,
                    ctx,
                    i,
                    *prect,
                    clear_tiles && first,
                    clear_vector && first,
                    first,
                    solo || i + 1 == n,
                    &placefile_labels,
                );
            }

            // Pane borders; the active pane gets an accent outline. Nothing to outline under
            // `solo` — there is one pane on screen and the strip says which.
            if n > 1 && !solo {
                for (i, prect) in rects.iter().enumerate() {
                    let (w, col) = if i == self.active {
                        (2.0, crate::theme::accent(self.settings.theme))
                    } else {
                        (1.0, egui::Color32::from_gray(60))
                    };
                    ui.painter().rect_stroke(
                        *prect,
                        0.0,
                        egui::Stroke::new(w, col),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        });

        // Dirty-diff persistence: one write per actual change, from any mutation site. The
        // comparison walks the whole settings tree (palettes, placefiles, markers), so it runs at
        // most once a second rather than every frame; a change waits under a second to reach disk.
        let due = self
            .settings_checked
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
        if due {
            self.settings_checked = Some(Instant::now());
            // Fold the live overlay toggles into the settings so the diff below persists them
            // like any other change — no separate save path, no per-frame churn.
            let on: Vec<String> = OverlayToggle::ALL
                .into_iter()
                // Camera linking is a session decision about the panes on screen, not a layer.
                .filter(|t| !t.session_only() && *self.overlay_flag(*t))
                .map(|t| t.slug())
                .collect();
            self.settings.overlays_on = Some(on);
            // Same trick for the window: fold the live size in, and the ordinary diff-and-save
            // below persists it.
            //
            // Measured from the root `Ui`, not `ViewportInfo::inner_rect`: on Wayland the
            // compositor never tells a client where its window is, so `inner_rect` is `None`
            // there and this silently saved nothing at all. The root ui covers the whole
            // viewport, in egui points — logical points divided by the ui-scale zoom — so
            // multiplying the zoom back gives the units `with_inner_size` wants.
            //
            // While maximized the size on screen is the screen's, not the one to restore to, so
            // the previous size is kept and only the flag moves.
            #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
            {
                let (maximized, minimized) = root.ctx().input(|i| {
                    (
                        i.viewport().maximized.unwrap_or(false),
                        i.viewport().minimized.unwrap_or(false),
                    )
                });
                let size = root.max_rect().size() * root.ctx().zoom_factor();
                if !minimized && size.x > 1.0 && size.y > 1.0 {
                    let (width, height) = match self.settings.window {
                        Some(w) if maximized => (w.width, w.height),
                        _ => (size.x, size.y),
                    };
                    self.settings.window = Some(crate::settings::WindowGeom {
                        width,
                        height,
                        maximized,
                    });
                }
            }
        }
        if due && self.settings != self.saved {
            // A palette-map change reloads the color tables (bumps gen -> LUT re-bake).
            if self.settings.palettes != self.saved.palettes {
                self.palettes.reload(&self.settings.palette_paths());
            }
            self.settings.save();
            self.saved = self.settings.clone();
        }

        // Embedded in another page: hand the active pane's state to the parent frame so it can
        // persist it. Our own localStorage is partitioned (or wiped) inside a third-party iframe,
        // so the host is the only place this survives a reload. Same once-a-second tick as above,
        // and only when something actually moved.
        #[cfg(target_arch = "wasm32")]
        if due && self.embed {
            self.post_state_to_parent();
        }

        // Android text input: summon/dismiss the soft keyboard as egui focus moves in/out of
        // text fields, and float a Paste button (the system clipboard is unreachable from the
        // soft keyboard otherwise — egui gets the text as a Paste event next frame).
        if cfg!(target_os = "android") {
            let wants = ctx.egui_wants_keyboard_input();
            if wants != self.ime_shown {
                crate::platform::show_soft_input(wants);
                self.ime_shown = wants;
            }
            if wants {
                egui::Area::new(egui::Id::new("android_paste_bar"))
                    .anchor(egui::Align2::RIGHT_TOP, [-8.0, 64.0])
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            if ui.button("Paste").clicked() {
                                self.pending_paste = crate::platform::clipboard_text();
                                // Remember the field losing focus to this tap, to restore it next
                                // frame so the Paste event has somewhere to land.
                                self.paste_target = ui.ctx().memory(|m| m.focused());
                            }
                        });
                    });
            }
        }

        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        self.mini_loop_viewport(ctx);

        self.crash_report_window(ctx);

        // Idle heartbeat so clocks (volume age, countdowns) tick without input. Data arrivals and
        // animations (pulse, banners) request faster repaints on their own. Slower on Android to
        // spare the battery — nothing on screen changes faster than this between frames.
        // An untouched embed is a still picture on someone else's dashboard: one frame a minute
        // keeps the clocks honest without spending the host's CPU. The first interaction wakes it
        // for good; data arrivals still request their own repaints either way.
        let busy = ctx.input(|i| !i.events.is_empty() || i.pointer.any_down() || i.any_touches());
        if busy {
            self.last_input = Instant::now();
        }
        if self.embed && !self.embed_live && ctx.input(|i| i.pointer.any_down() || i.any_touches())
        {
            self.embed_live = true;
        }
        let idle = if self.embed && !self.embed_live {
            60_000
        } else if !crate::platform::activity::is_active() {
            2_000 // backgrounded: just enough to notice coming back
        } else if self.settings.battery_saver {
            // Four frames a second is still a live clock; it is not a live animation. Anything
            // that actually moves (a banner, a play head, an arriving volume) asks for its own
            // repaint and is unaffected.
            1_000
        } else if self.last_input.elapsed() > QUIET_AFTER {
            // Nobody has touched it for a while. This is a floor, not a schedule: egui takes the
            // *minimum* of every repaint request in a frame, so playback, the warning pulse, the
            // wind field and every arriving volume all still outbid it and animate at their own
            // rate. What it changes is the cost of a window nothing is happening in — ten wasted
            // full passes a second, each re-walking thirty poll clocks and re-projecting every
            // label, becomes two. The first event of any kind snaps it back the same frame.
            //
            // The visible price is that a clock can read up to 0.4 s stale in a still window.
            IDLE_QUIET_MS
        } else if cfg!(target_os = "android") {
            250
        } else {
            100
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.perf.idle_ms = idle;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(idle));
    }
}

/// How many held-back pushes the quiet-hours queue keeps. A long quiet window over a big outbreak
/// can queue hundreds; the summary only names a handful anyway.
const QUIET_QUEUE_MAX: usize = 50;

/// Fold the pushes quiet hours held back into one catch-up notification: `(title, body)`.
///
/// Named headlines are capped so the body stays inside what a phone push will show; the rest are
/// counted.
fn quiet_summary(held: &[(String, String)]) -> (String, String) {
    const NAMED: usize = 4;
    let title = if held.len() == 1 {
        "1 alert while you were away".to_string()
    } else {
        format!("{} alerts while you were away", held.len())
    };
    let mut body: String = held
        .iter()
        .take(NAMED)
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if held.len() > NAMED {
        body.push_str(&format!("\n(+{} more)", held.len() - NAMED));
    }
    (title, body)
}

#[cfg(test)]
mod quiet_summary_tests {
    use super::quiet_summary;

    fn held(n: usize) -> Vec<(String, String)> {
        (0..n)
            .map(|i| (format!("alert {i}"), "body".to_string()))
            .collect()
    }

    #[test]
    fn one_alert_reads_singular() {
        let (title, body) = quiet_summary(&held(1));
        assert_eq!(title, "1 alert while you were away");
        assert_eq!(body, "alert 0");
    }

    #[test]
    fn many_alerts_name_a_few_and_count_the_rest() {
        let (title, body) = quiet_summary(&held(9));
        assert_eq!(title, "9 alerts while you were away");
        assert!(body.starts_with("alert 0\nalert 1\nalert 2\nalert 3\n"));
        assert!(body.ends_with("(+5 more)"));
    }

    #[test]
    fn exactly_the_named_count_has_no_tail() {
        let (_, body) = quiet_summary(&held(4));
        assert!(!body.contains("more"));
    }
}

#[cfg(test)]
mod humanize_tests {
    use super::humanize;

    #[test]
    fn ages_stay_readable_all_the_way_back_to_the_archive() {
        assert_eq!(humanize(45), "45s");
        assert_eq!(humanize(600), "10m");
        assert_eq!(humanize(3 * 3600 + 20 * 60), "3h20m");
        // Past a couple of days, hours stop meaning anything to a reader.
        assert_eq!(humanize(5 * 86_400), "5d");
        // Moore 2013, seen from 2026 — used to render as "113000h40m ago".
        assert_eq!(humanize(13 * 365 * 86_400), "13.0y");
    }
}

#[cfg(test)]
mod place_name_tests {
    use super::short_place_name;

    #[test]
    fn keeps_the_place_and_drops_the_administrative_tail() {
        // What Nominatim actually returns for a town query.
        assert_eq!(
            short_place_name("Norman, Cleveland County, Oklahoma, United States"),
            "Norman"
        );
        // Already short, or oddly shaped: pass it through rather than blanking the name.
        assert_eq!(short_place_name("Dallas"), "Dallas");
        assert_eq!(short_place_name(""), "");
        assert_eq!(short_place_name("  Tulsa , Oklahoma"), "Tulsa");
    }
}

#[cfg(test)]
mod follow_tests {
    use super::{nearest_cell, widget_storm_line};
    use wxdata::level3::Cell;

    fn cell(id: &str, lon: f64, lat: f64) -> Cell {
        Cell {
            id: id.into(),
            lon,
            lat,
            ..Default::default()
        }
    }

    #[test]
    fn the_widget_line_reads_from_where_you_are() {
        // A cell due west of the reader, moving northeast at 30 kt.
        let mut c = cell("R3", -97.72, 35.30);
        c.mvt_deg = Some(45.0);
        c.mvt_kt = Some(30.0);
        let line = widget_storm_line(&[c.clone()], -97.5, 35.3, false).unwrap();
        // West of you: the compass point names the side the storm is on, not the bearing to it.
        assert!(line.starts_with("R3 12 mi W"), "{line}");
        assert!(line.ends_with("moving NE 35 mph"), "{line}");
        // Same cell on a German pane: the distance turns over, the speed does not.
        let km_line = widget_storm_line(&[c], -97.5, 35.3, true).unwrap();
        assert!(km_line.starts_with("R3 20 km W"), "{km_line}");
        // Nothing within 300 km is no line at all.
        assert!(widget_storm_line(&[cell("Z1", -80.0, 35.3)], -97.5, 35.3, false).is_none());
        assert!(widget_storm_line(&[], -97.5, 35.3, false).is_none());
    }

    #[test]
    fn nearest_cell_within_radius_and_none_outside() {
        // Three cells around a predicted point near (−97.5, 35.3).
        let cells = vec![
            cell("A7", -97.60, 35.30), // ~9 km west
            cell("B3", -97.51, 35.31), // ~1.5 km — the nearest
            cell("", -97.505, 35.305), // closest of all but no SCIT id → ineligible
        ];
        let got = nearest_cell(&cells, -97.5, 35.3, 15.0).unwrap();
        assert_eq!(got.id, "B3");
        // A prediction far from every cell (radius exceeded) → nothing to adopt.
        assert!(nearest_cell(&cells, -90.0, 30.0, 15.0).is_none());
    }
}

#[cfg(test)]
mod warning_scope_tests {
    use super::{feature_in_box, GeoFeature};
    use wxdata::overlay::FeatureKind;

    fn poly(x0: f64, y0: f64, x1: f64, y1: f64) -> GeoFeature {
        GeoFeature {
            rings: vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]],
            fill: [0; 4],
            stroke: [0; 4],
            kind: FeatureKind::Warning,
            title: String::new(),
            detail: String::new(),
            alert: None,
        }
    }

    #[test]
    fn feature_in_box_overlap() {
        // Box roughly around KFWS (Dallas): lon -97.3, lat 32.6, ±2.25°.
        let bx = (-99.55, 30.35, -95.05, 34.85);
        // A warning polygon overlapping the box.
        assert!(feature_in_box(&poly(-98.0, 32.0, -97.0, 33.0), bx));
        // A warning far away (Mississippi) — no overlap.
        assert!(!feature_in_box(&poly(-90.0, 32.0, -89.0, 33.0), bx));
        // Touching the edge counts as overlap.
        assert!(feature_in_box(&poly(-95.05, 32.0, -94.0, 33.0), bx));
        // Empty geometry never overlaps.
        let mut empty = poly(0.0, 0.0, 0.0, 0.0);
        empty.rings.clear();
        assert!(!feature_in_box(&empty, bx));
    }
}

#[cfg(test)]
mod field_lut_tests {
    use super::{categorical_lut, distinct_tilts, glm_style, ramp_lut, ramp_lut_a, windy_url};

    #[test]
    fn distinct_tilts_skips_sails_repeats() {
        // A VCP 212 style list: 0.5 appears three times (SAILS), 0.9 twice (MRLE).
        let els = [0.5, 0.5, 0.9, 0.5, 0.9, 1.3, 1.8, 2.4];
        let picks = distinct_tilts(&els, 4);
        assert_eq!(
            picks,
            vec![0, 2, 5, 6],
            "one index per distinct angle, lowest first"
        );
        let angles: Vec<f32> = picks.iter().map(|&i| els[i]).collect();
        assert_eq!(angles, vec![0.5, 0.9, 1.3, 1.8]);
    }

    #[test]
    fn distinct_tilts_clamps_to_what_exists() {
        assert_eq!(distinct_tilts(&[0.5, 0.5], 4), vec![0]);
        assert!(distinct_tilts(&[], 4).is_empty());
    }

    #[test]
    fn windy_url_puts_latitude_first() {
        // KTLX, zoom 7.4. Windy wants lat,lon — this codebase says (lon, lat) everywhere else,
        // so a swap here would silently send people to the Indian Ocean.
        let u = windy_url("radar", -97.3, 35.4, 7.4);
        assert_eq!(u, "https://www.windy.com/?radar,35.400,-97.300,7");
        // Decimals are mandatory: Windy ignores a whole-number coordinate.
        assert!(windy_url("wind", -97.0, 35.0, 5.0).contains("35.000,-97.000"));
        // Zoom is clamped into Windy's own range rather than passed through.
        assert!(windy_url("wind", 0.0, 0.0, 2.0).ends_with(",3"));
        assert!(windy_url("wind", 0.0, 0.0, 18.9).ends_with(",18"));
    }

    #[test]
    fn glm_flashes_fade_from_white_hot_to_ember() {
        let (fresh, r_fresh) = glm_style(0.0);
        let (old, r_old) = glm_style(900.0);
        assert_eq!(fresh.a(), 255, "a brand-new flash is fully opaque");
        assert!(old.a() < 100, "a 15-minute-old flash is nearly gone");
        assert!(r_fresh > r_old, "newest flashes draw largest");
        // Past the window the style clamps rather than inverting.
        assert_eq!(glm_style(5000.0), glm_style(900.0));
        // And it warms as it ages: green drops faster than red.
        assert!(old.g() < fresh.g() && old.r() <= fresh.r());
    }

    #[test]
    fn categorical_lut_sets_only_listed_slots() {
        let lut = categorical_lut(&[(1, [10, 20, 30]), (7, [200, 40, 40])], 200);
        assert_eq!(lut.len(), 256 * 4);
        // Index 0 clear.
        assert_eq!(&lut[0..4], &[0, 0, 0, 0]);
        // Index 1 set with alpha 200.
        assert_eq!(&lut[4..8], &[10, 20, 30, 200]);
        // Index 7 set.
        assert_eq!(&lut[28..32], &[200, 40, 40, 200]);
        // An unlisted index stays clear.
        assert_eq!(&lut[8..12], &[0, 0, 0, 0]);
    }

    #[test]
    fn ramp_lut_alpha_variants() {
        let opaque = ramp_lut(&[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])]);
        assert_eq!(opaque[255 * 4 + 3], 255, "top index opaque");
        assert_eq!(opaque[3], 0, "index 0 clear");
        let translucent = ramp_lut_a(&[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])], 150);
        assert_eq!(translucent[255 * 4 + 3], 150, "top index uses given alpha");
    }
}

#[cfg(test)]
mod tests {

    /// New geometry appears when it lands; a zoom-bucket crossing waits for the finger to lift,
    /// so a pinch across three buckets tessellates once instead of three times.
    #[test]
    fn a_pinch_defers_the_retessellation_but_new_geometry_never_waits() {
        use super::should_retess;
        assert!(
            should_retess(false, false, true),
            "quiet: rebuild on a bucket change"
        );
        assert!(!should_retess(true, false, true), "mid-gesture: wait");
        assert!(
            should_retess(true, true, true),
            "new geometry mid-gesture is data arriving, not the camera moving"
        );
        assert!(!should_retess(true, false, false));
        assert!(
            !should_retess(false, false, false),
            "nothing changed, nothing to do"
        );
    }

    /// A palette change must not re-send the sweep. Everything the GPU keeps (the gate bytes,
    /// the precipitation-flag grid) is left out of a LUT-only upload; the color table and the
    /// uniform, which are what a re-bake changes, are still there in full.
    #[test]
    fn a_palette_change_uploads_the_table_and_nothing_else() {
        let sweep = wxdata::level2::BinnedSweep {
            moment: wxdata::level2::Moment::Reflectivity,
            az_bins: 360,
            gate_count: 200,
            data: vec![7u8; 360 * 200],
            first_gate_km: 2.125,
            gate_interval_km: 0.25,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: 0.5,
            value_min: -32.0,
            value_max: 95.0,
        };
        let table = crate::colormap::default_table(wxdata::level2::Moment::Reflectivity);
        let full = super::to_upload(&sweep, table, None, false, None, None, false);
        let lut = super::to_upload(&sweep, table, None, false, None, None, true);
        assert_eq!(full.data.len(), 360 * 200);
        assert!(lut.data.is_empty(), "the gate texture is already uploaded");
        assert_eq!(lut.lut, full.lut, "the color table is what changed");
        assert_eq!(lut.uniform, full.uniform);
        assert_eq!((lut.az_bins, lut.gate_count), (360, 200));
        assert!(lut.lut_only);
    }

    /// The DWD arm exists because DL-DE/BY-2.0 requires the credit; the NOAA sites must stay
    /// uncredited so the corner is empty on the maps most users open.
    #[test]
    fn only_licences_that_ask_for_a_credit_get_one() {
        assert_eq!(
            super::data_attribution("DEBO"),
            Some("Radar data © Deutscher Wetterdienst (DL-DE/BY-2.0)")
        );
        assert_eq!(super::data_attribution("KTLX"), None);
    }

    /// Scrubbing reverses constantly, so the paused set has to reach backwards as well; playback
    /// only ever goes forward. Nearest frames come first either way, because the in-flight budget
    /// usually runs out before the list does.
    #[test]
    fn prefetch_reaches_backwards_only_when_paused() {
        let playing = super::prefetch_offsets(true);
        assert!(playing.iter().all(|d| *d > 0), "playback never looks back");
        let paused = super::prefetch_offsets(false);
        assert!(paused.contains(&-1), "scrubbing back a frame must be warm");
        assert_eq!(paused[0], 1, "nearest frame first");
        for set in [playing, paused] {
            let mut sorted = set.to_vec();
            sorted.sort_by_key(|d| d.abs());
            assert_eq!(set, sorted.as_slice(), "nearest-first order");
        }
    }

    #[test]
    fn goto_extras_carry_basemap_and_srv() {
        // New extras, in either order, alongside the old ones.
        let g = super::parse_goto("KTLX,-97.3,35.3,6.5,bm:dark,srv,VEL,2").unwrap();
        assert_eq!(g.site, "KTLX");
        assert_eq!(g.basemap.as_deref(), Some("dark"));
        assert!(g.srv);
        assert_eq!(g.tilt, Some(2));
        // Old links are unchanged: no basemap, not storm-relative.
        let g = super::parse_goto(",-97.3,35.3,6.5").unwrap();
        assert_eq!(g.site, "");
        assert_eq!(g.basemap, None);
        assert!(!g.srv);
        assert_eq!(g.zoom, 6.5);
    }

    #[test]
    fn goes_frame_follows_the_radar_clock() {
        use chrono::{TimeZone, Utc};
        let t = |m: u32| Utc.with_ymd_and_hms(2026, 5, 1, 20, m, 0).unwrap();
        let times = [t(0), t(10), t(20), t(30)];
        // Nearest wins, ties included.
        assert_eq!(super::nearest_goes(&times, t(21)), Some(t(20)));
        assert_eq!(super::nearest_goes(&times, t(26)), Some(t(30)));
        // Past the tolerance, stay on the latest instead of showing the wrong hour.
        let far = Utc.with_ymd_and_hms(2026, 5, 1, 22, 0, 0).unwrap();
        assert_eq!(super::nearest_goes(&times, far), None);
        assert_eq!(super::nearest_goes(&[], t(0)), None);
    }

    #[test]
    fn icon_sheet_cache_paths_are_per_url_and_filename_safe() {
        let Some(a) = super::icon_sheet_cache_path("https://example.com/a/icons.png") else {
            return; // no disk on this platform: nothing to name
        };
        let b = super::icon_sheet_cache_path("https://example.com/b/icons.png").unwrap();
        assert_ne!(a, b, "two sheets must not share a file");
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(
            name.chars().all(|c| c.is_ascii_hexdigit()),
            "a URL is not a filename: got {name}"
        );
    }
    use super::*;

    #[test]
    fn every_overlay_toggle_survives_a_slug_round_trip() {
        for t in OverlayToggle::ALL {
            assert_eq!(OverlayToggle::from_slug(&t.slug()), Some(t), "{t:?}");
        }
        // ALL has to actually be all of them — a variant left out would silently stop persisting.
        let mut slugs: Vec<String> = OverlayToggle::ALL.iter().map(|t| t.slug()).collect();
        slugs.sort();
        slugs.dedup();
        assert_eq!(slugs.len(), OverlayToggle::ALL.len());
        assert_eq!(OverlayToggle::from_slug("Teleportation"), None);
    }

    #[test]
    fn goto_parses_every_form_it_arrives_in() {
        let g = parse_goto("KTLX,-97.3,35.3,9").unwrap();
        assert_eq!(
            (g.site.as_str(), g.lon, g.lat, g.zoom),
            ("KTLX", -97.3, 35.3, 9.0)
        );
        assert!(g.time.is_none() && g.moment.is_none() && g.tilt.is_none());
        // The URL form is the same string behind a scheme.
        assert_eq!(
            parse_goto("hookecho://goto/KTLX,-97.3,35.3,9")
                .unwrap()
                .site,
            "KTLX"
        );
        // AlertService writes a site-less notification link; that must keep working.
        assert_eq!(parse_goto(",-97.3,35.3,9").unwrap().site, "");
        // Archive links carry a time.
        let g = parse_goto("KTLX,-97.3,35.3,9,2013-05-20T20:00:00Z").unwrap();
        assert_eq!(g.time.unwrap().to_rfc3339(), "2013-05-20T20:00:00+00:00");
        // A bare site resolves from the registry.
        let g = parse_goto("ktlx").unwrap();
        assert_eq!(g.site, "KTLX");
        assert!(g.lon < -97.0 && g.lat > 35.0 && g.zoom == 8.0);
        // Product and tilt are sniffed by shape, in either order.
        for v in ["KTLX,-97.3,35.3,9,VEL,2", "KTLX,-97.3,35.3,9,2,VEL"] {
            let g = parse_goto(v).unwrap();
            assert_eq!((g.moment, g.tilt), (Some(Moment::Velocity), Some(2)), "{v}");
        }
        assert!(parse_goto("").is_none());
        assert!(parse_goto("garbage").is_none());
    }

    #[test]
    fn goto_survives_a_percent_encoded_round_trip() {
        // What a chat client hands back after eating the link.
        let g = parse_goto("KTLX%2C-97.3%2C35.3%2C9%2C2013-05-20T20%3A00%3A00Z").unwrap();
        assert_eq!(g.site, "KTLX");
        assert_eq!(g.zoom, 9.0);
        assert_eq!(g.time.unwrap().to_rfc3339(), "2013-05-20T20:00:00+00:00");
        // Encoded spaces around the fields, and a stray `%` that is not an escape at all.
        assert_eq!(parse_goto("%20ktlx%20").unwrap().site, "KTLX");
        assert!(parse_goto("%GG").is_none());
    }

    #[test]
    fn goto_link_round_trips() {
        let base = |moment, tilt, threshold| Goto {
            site: "KFWS".to_string(),
            lon: -97.3031,
            lat: 32.5731,
            zoom: 8.5,
            time: None,
            moment: Some(moment),
            tilt: Some(tilt),
            basemap: None,
            threshold,
            srv: false,
        };
        let link = goto_link(&base(Moment::Reflectivity, 0, None));
        assert!(link.starts_with("hookecho://goto/KFWS,"), "{link}");
        let g = parse_goto(&link).unwrap();
        assert_eq!(g.site, "KFWS");
        assert!((g.lon - -97.3031).abs() < 1e-4 && (g.lat - 32.5731).abs() < 1e-4);
        assert_eq!(g.zoom, 8.5);
        // Reflectivity at the base tilt is the default, so it stays out of the link.
        assert!(!link.contains("dBZ"), "{link}");

        let link = goto_link(&base(Moment::Velocity, 3, None));
        let g = parse_goto(&link).unwrap();
        assert_eq!((g.moment, g.tilt), (Some(Moment::Velocity), Some(3)));
        // No threshold set means the link says nothing, leaving the recipient's own alone.
        assert!(!link.contains("thr:"), "{link}");

        // Issue #71: an embedded dashboard needs to deep-link a threshold, and the link the Copy
        // button produces has to come back as the threshold it was copied from.
        let link = goto_link(&base(Moment::Reflectivity, 0, Some(Some(25.0))));
        assert!(link.contains(",thr:25"), "{link}");
        assert_eq!(parse_goto(&link).unwrap().threshold, Some(Some(25.0)));
        // A view with the threshold switched off shares as "nothing to say", not as `thr:off` —
        // overriding the recipient's own setting with a default nobody chose.
        assert!(!goto_link(&base(Moment::Reflectivity, 0, Some(None))).contains("thr:"));
    }

    #[test]
    fn goto_parses_a_threshold_by_shape() {
        // Order does not matter, and the field is sniffed by shape like every other extra.
        assert_eq!(
            parse_goto("KTLX,-97.3,35.3,8,thr:25,VEL")
                .unwrap()
                .threshold,
            Some(Some(25.0))
        );
        // Off is a deliberate instruction, distinct from saying nothing at all.
        assert_eq!(
            parse_goto("KTLX,-97.3,35.3,8,thr:off").unwrap().threshold,
            Some(None)
        );
        assert_eq!(parse_goto("KTLX,-97.3,35.3,8").unwrap().threshold, None);
        // Garbage is ignored, not applied as zero.
        assert_eq!(
            parse_goto("KTLX,-97.3,35.3,8,thr:loud").unwrap().threshold,
            None
        );
    }

    /// The draw tool must append into the stroke in flight and start a new one per drag, and Undo
    /// must drop exactly one stroke — the whole contract of a scribble layer.
    #[test]
    fn draw_strokes_append_and_undo() {
        let red = DRAW_COLORS[0];
        let cyan = DRAW_COLORS[2];
        let mut strokes = Vec::new();
        draw_append(&mut strokes, [-97.0, 35.0], red, true);
        draw_append(&mut strokes, [-97.1, 35.1], red, false);
        // A repeated point (a still finger during a drag) adds nothing.
        draw_append(&mut strokes, [-97.1, 35.1], red, false);
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].points.len(), 2);

        draw_append(&mut strokes, [-98.0, 36.0], cyan, true);
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[1].color, cyan);

        strokes.pop(); // Undo
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].color, red);
    }
}

/// The archive-day control inside the LIVE/ARCHIVE badge menu. `Some` on the frame the user
/// picks a new day.
///
/// Native gets a calendar. The web build does not: the picker widget and the `jiff` date type
/// it takes cost about 120 KB gzipped, which is a real share of the wasm budget to spend on a
/// convenience, and a typed date reaches 1991 just as directly. Everything else about the menu
/// — the carets, the UTC-day caveat — is the same on both.
#[cfg(not(target_arch = "wasm32"))]
fn archive_day_input(ui: &mut egui::Ui, date: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
    let mut picked = to_jiff(date);
    let changed = ui
        .add(
            egui_extras::DatePickerButton::new(&mut picked)
                .id_salt("archive-day")
                .format("%Y-%m-%d")
                .highlight_weekends(false),
        )
        .on_hover_text("Archive days are UTC days — the S3 buckets are bucketed that way")
        .changed();
    changed.then(|| from_jiff(picked)).flatten()
}

/// Web: type the day instead. The buffer lives in egui's own memory rather than app state,
/// because a half-typed date is not something the app has any use for.
#[cfg(target_arch = "wasm32")]
fn archive_day_input(ui: &mut egui::Ui, date: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
    let id = egui::Id::new("archive-day-text");
    let shown = date.format("%Y-%m-%d").to_string();
    let mut buf: String = ui
        .data_mut(|d| d.get_temp(id))
        .unwrap_or_else(|| shown.clone());
    // Someone else moved the day (a caret, a deep link): follow it rather than argue.
    if !buf.starts_with(&shown[..4]) && chrono::NaiveDate::parse_from_str(&buf, "%Y-%m-%d").is_ok()
    {
        buf = shown.clone();
    }
    let resp = ui
        .add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(84.0)
                .font(egui::TextStyle::Monospace),
        )
        .on_hover_text(
            "Archive days are UTC days — the S3 buckets are bucketed that way. \
             Type YYYY-MM-DD; the archive starts 1991-06-05.",
        );
    let out = chrono::NaiveDate::parse_from_str(&buf, "%Y-%m-%d")
        .ok()
        .filter(|d| *d != date);
    ui.data_mut(|d| d.insert_temp(id, buf));
    // Only commit on a complete, parseable date — otherwise every keystroke mid-typing would
    // send the timeline somewhere.
    resp.changed().then_some(out).flatten()
}

/// A chrono date as a jiff one, for `egui_extras`'s date picker.
///
/// ponytail: the two crates model a civil date identically, so this is a field copy. It exists
/// because the picker is the only jiff-speaking thing in the app and converting one widget's
/// argument is cheaper than migrating every date in the codebase. Out-of-range dates cannot
/// happen — chrono's year range is a subset of jiff's — so the fallback is the epoch.
#[cfg(not(target_arch = "wasm32"))]
fn to_jiff(d: chrono::NaiveDate) -> jiff::civil::Date {
    use chrono::Datelike;
    jiff::civil::Date::new(d.year() as i16, d.month() as i8, d.day() as i8)
        .unwrap_or(jiff::civil::Date::ZERO)
}

/// The inverse of [`to_jiff`]; `None` for a date chrono cannot represent.
#[cfg(not(target_arch = "wasm32"))]
fn from_jiff(d: jiff::civil::Date) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::from_ymd_opt(d.year() as i32, d.month() as u32, d.day() as u32)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod date_picker_tests {
    /// The picker's date must survive the round trip, or picking a day would move it.
    #[test]
    fn dates_round_trip_through_jiff() {
        for (y, m, d) in [(1991, 6, 5), (2026, 8, 24), (2000, 2, 29), (2011, 12, 31)] {
            let orig = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
            assert_eq!(super::from_jiff(super::to_jiff(orig)), Some(orig));
        }
    }
}

/// How much to trust a pure advection at `lead_min` minutes, 1.0 down to ~0.35.
///
/// The nowcast moves the echo that exists along the mean storm motion. It cannot grow a cell,
/// collapse one, or turn it, and every minute of lead is another minute for those to happen. Up
/// to 45 minutes — the range this shipped with — it is taken at face value; past that it fades,
/// which is the only honest way to keep offering it.
fn nowcast_confidence(lead_min: u8) -> f32 {
    const FULL: f32 = 45.0;
    const FLOOR: f32 = 0.35;
    if lead_min as f32 <= FULL {
        return 1.0;
    }
    let over = (lead_min as f32 - FULL) / (120.0 - FULL);
    (1.0 - over.clamp(0.0, 1.0) * (1.0 - FLOOR)).clamp(FLOOR, 1.0)
}

#[cfg(test)]
mod nowcast_tests {
    use super::nowcast_confidence;

    #[test]
    fn confidence_is_full_inside_the_old_range_and_fades_past_it() {
        for lead in [15u8, 30, 45] {
            assert_eq!(nowcast_confidence(lead), 1.0, "{lead} min");
        }
        assert!(nowcast_confidence(60) < 1.0);
        assert!(nowcast_confidence(90) < nowcast_confidence(60));
        assert!((nowcast_confidence(120) - 0.35).abs() < 1e-5);
    }

    /// It must never fade to invisible, or the layer would silently stop existing.
    #[test]
    fn confidence_never_reaches_zero() {
        for lead in 0u8..=255 {
            assert!(nowcast_confidence(lead) >= 0.35, "{lead} min");
        }
    }
}
