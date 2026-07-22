//! Hook Echo-WX application shell: menu bar, radar toolbox, map view, and async data flow.
//!
//! UI code only mutates the active [`MapView`]; a single per-frame sync step turns those
//! mutations into GPU uploads and background fetches, so buttons and hotkeys share one path.

/// Touch-first Android chrome (top bar, bottom dock, slide-up sheets), replacing the desktop
/// menu bar / left toolbox / status bar / right alert dock. Only the chrome differs; the map,
/// windows, and every data path are shared.
mod mobile;

use crate::colormap::{ColorTable, Palettes};
use crate::hotkeys::{self, Action};
use crate::overlay_build;
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
use std::time::Instant;
use tokio::runtime::Runtime;
use wxdata::alerts::{self};
use wxdata::level2::{self, BinnedSweep, Identifier, Moment, Scan};
use wxdata::level3::{self, Cell, CellKind};
use wxdata::live;
use wxdata::overlay::{self, GeoFeature};

/// Frames to let a stepped archive volume load before grabbing it for the loop GIF.
const LOOP_SETTLE_FRAMES: u8 = 12;

/// Squared screen-space hit radius (px²) for a tap/click target of nominal `px` radius. Android
/// finger taps need a fatter target than a mouse cursor, so targets grow ~1.8× there; desktop is
/// unchanged.
fn tap_r2(px: f32) -> f32 {
    let r = if cfg!(target_os = "android") { px * 1.8 } else { px };
    r * r
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
}

impl Default for OverlayFilters {
    fn default() -> Self {
        Self {
            show_alerts: true,
            alert_cats: [true; 6],
            outlook_day: 0, // SPC outlook off by default; user opts in via the toolbox
            outlook_kind: wxdata::spc::OutlookKind::Categorical,

            show_mds: true,
            show_cells: true,
            show_tracks: true,
            show_arrival_cones: false,
            show_nowcast: false,
            nowcast_lead_min: 15,
            show_tds: false,
        }
    }
}

/// Background overlay fetch results.
enum OverlayMsg {
    Alerts(Vec<GeoFeature>),
    Outlook(u8, Vec<GeoFeature>),
    Mds(Vec<GeoFeature>),
    /// Storm cells for a specific site (dropped if the active site changed meanwhile).
    Cells(String, Vec<Cell>),
    /// A fetched placefile keyed by its URL.
    Placefile(String, wxdata::placefile::Placefile),
    /// The latest grid for a national field layer (mosaic, rotation, MESH, AzShear, lightning).
    Field(crate::render::FieldLayer, wxdata::mrms::MrmsField),
    /// Local storm reports: live trailing window (`None`) or an archive bucket (feature CC).
    StormReports(Option<i64>, Vec<wxdata::spc::StormReport>),
    /// Live Spotter Network positions (CONUS-wide; filtered to the active site at draw time).
    Spotters(Vec<wxdata::spotters::Spotter>),
    /// ProbSevere per-storm probability polygons.
    ProbSevere(Vec<GeoFeature>),
    /// An HRRR composite-reflectivity forecast (regridded + run/valid metadata).
    Hrrr(wxdata::hrrr::HrrrForecast),
    /// Nearest-station observations for a site (or an error string).
    Obs(String, Result<wxdata::obs::StationObs, String>),
    /// VAD wind profile for a site.
    Vwp(String, Vec<wxdata::level3::VwpLevel>),
    /// Archived storm-based warnings for a 5-min UTC bucket (feature W).
    ArchiveWarnings(i64, Vec<GeoFeature>),
    /// Surface observations (METAR station plots) for the requested bbox (feature U).
    Metar(Vec<wxdata::metar::SurfaceOb>),
    /// NHC tropical cyclones: cones + per-storm tracks (feature V).
    Tropical(wxdata::tropical::TropicalData),
    /// Aviation SIGMET/AIRMET hazard polygons (feature GG).
    Aviation(Vec<GeoFeature>),
}

/// One overlay data source to fetch.
#[derive(Clone)]
enum OverlaySource {
    Alerts,
    Mds,
    Outlook(u8, wxdata::spc::OutlookKind),
    Cells(String),
    Placefile(String),
    /// A national field layer plus the MRMS S3 product path to fetch it from.
    Field(crate::render::FieldLayer, String),
    /// Local storm reports: live (`None`) or a 30-min archive bucket (Unix secs / 1800).
    StormReports(Option<i64>),
    Spotters,
    ProbSevere,
    /// HRRR composite-reflectivity forecast for a forecast hour (0..=18).
    Hrrr(u8),
    /// HRRR environment field (CAPE/SRH) at f00; `ml` = mixed-layer CAPE, `srh_km` = SRH depth.
    Env(crate::render::FieldLayer, bool, u8),
    /// Gridded L3 product (DVL/EET) for a site, projected to a lat/lon field (feature X).
    L3Grid(crate::render::FieldLayer, String),
    /// Nearest-station observations for `site` at `(lat, lon)`.
    Obs { site: String, lat: f64, lon: f64 },
    /// VAD wind profile for `site`.
    Vwp(String),
    /// Archived storm-based warnings valid at a 5-min UTC bucket (Unix seconds, feature W).
    ArchiveWarnings(i64),
    /// Aviation SIGMET/AIRMET polygons (feature GG).
    Aviation,
    /// Surface observations within a lat/lon bbox `(lat0, lon0, lat1, lon1)` (feature U).
    Metar(f64, f64, f64, f64),
    /// NHC tropical cyclones (feature V).
    Tropical,
}

impl OverlaySource {
    async fn fetch(self, http: &reqwest::Client) -> anyhow::Result<OverlayMsg> {
        Ok(match self {
            OverlaySource::Alerts => OverlayMsg::Alerts(alerts::fetch_active(http).await?),
            OverlaySource::Mds => OverlayMsg::Mds(wxdata::spc::fetch_mesoscale_discussions(http).await?),
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
            OverlaySource::Field(layer, product) => {
                OverlayMsg::Field(layer, wxdata::mrms::fetch_latest(http, &product).await?)
            }
            OverlaySource::StormReports(bucket) => {
                // Archive bucket: the 6 h of reports ending at the bucket's close; live: last 6 h.
                let reports = match bucket {
                    Some(b) => {
                        let end = chrono::DateTime::from_timestamp((b + 1) * 1800, 0).unwrap_or_default();
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
            OverlaySource::Spotters => {
                OverlayMsg::Spotters(wxdata::spotters::fetch_spotters(http).await?)
            }
            OverlaySource::ProbSevere => {
                OverlayMsg::ProbSevere(wxdata::probsevere::fetch_probsevere(http).await?)
            }
            OverlaySource::Hrrr(fh) => OverlayMsg::Hrrr(wxdata::hrrr::fetch_forecast(http, fh).await?),
            OverlaySource::Env(layer, ml, srh_km) => {
                use crate::render::FieldLayer as FL;
                let (var, level, min_valid) = match layer {
                    FL::Cape if ml => ("CAPE", "90-0 mb above ground".to_string(), 0.0),
                    FL::Cape => ("CAPE", "surface".to_string(), 0.0),
                    FL::Srh => ("HLCY", format!("{}000-0 m above ground", srh_km), f64::NEG_INFINITY),
                    _ => ("REFC", "entire atmosphere".to_string(), -30.0),
                };
                let fc = wxdata::hrrr::fetch_field(http, var, &level, 0, min_valid).await?;
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
            OverlaySource::Obs { site, lat, lon } => {
                let r = wxdata::obs::fetch_nearest(http, lat, lon).await.map_err(|e| e.to_string());
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
                OverlayMsg::Metar(wxdata::metar::fetch_bbox(http, lat0, lon0, lat1, lon1).await?)
            }
            OverlaySource::Tropical => OverlayMsg::Tropical(wxdata::tropical::fetch_active(http).await?),
            OverlaySource::Aviation => {
                OverlayMsg::Aviation(wxdata::aviation::fetch_airsigmet(http).await?)
            }
        })
    }
}

/// What a left-click on the map does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum MapTool {
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
    /// Click a point to list historical tornado tracks near it (SPC climatology).
    Climatology,
}

/// Refresh cadence (seconds) for a national field layer's product.
fn field_refresh_secs(layer: crate::render::FieldLayer) -> u64 {
    use crate::render::FieldLayer as FL;
    match layer {
        FL::Lightning | FL::AzShear => 60,
        FL::Mrms | FL::Mesh | FL::Rotation | FL::Hrrr => 120,
        // QPE accumulations update on a ~2-minute MRMS cadence.
        FL::Qpe1h | FL::Qpe24h => 120,
        // MRMS precip type / flash-flood ARI on the ~2-min cadence; L3 grids on the 120 s L3 cadence.
        FL::PrecipType | FL::FlashFlood | FL::Vil | FL::EchoTops | FL::Hca => 120,
        // The 24-h hail-swath accumulation moves slowly.
        FL::HailSwath => 300,
        // Environment (HRRR CAPE/SRH) refreshes slowly — 15 min.
        FL::Cape | FL::Srh => 900,
    }
}

/// Per-field-layer UI + fetch state (toggle, pending upload, refresh clock).
#[derive(Default)]
pub(crate) struct FieldState {
    pub show: bool,
    pub pending: Option<crate::render::MrmsUpload>,
    pub last_fetch: Option<Instant>,
}

/// Where a pending screenshot is delivered: saved to a file, or copied to the clipboard, or
/// captured as one frame of a loop-GIF export.
enum ShotDest {
    File(std::path::PathBuf),
    Clipboard,
    Loop,
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
}

/// A placefile the app has fetched and is tracking (mirrors a `PlacefileConfig` by URL).
struct LoadedPlacefile {
    url: String,
    enabled: bool,
    pf: wxdata::placefile::Placefile,
    last_fetch: Option<Instant>,
    loaded: bool,
}

/// A background fetch result routed back to a specific view.
enum DataMsg {
    Volume { view: usize, site: String, name: String, time: DateTime<Utc>, scan: Scan },
    /// A live sweep-boundary update (merged full volume) from the chunk streamer.
    Live { view: usize, site: String, name: String, time: DateTime<Utc>, scan: Scan, changed: Vec<f32> },
    /// The live stream for `view` ended (error or clean exit); polling resumes.
    LiveEnded { view: usize, site: String },
    /// The archive volume listing for a site+date (timeline frames).
    Frames { view: usize, site: String, date: NaiveDate, frames: Vec<Identifier> },
    UpToDate { view: usize, site: String },
    Error { view: usize, site: String, err: String },
}

impl DataMsg {
    fn view(&self) -> usize {
        match self {
            DataMsg::Volume { view, .. }
            | DataMsg::Live { view, .. }
            | DataMsg::LiveEnded { view, .. }
            | DataMsg::Frames { view, .. }
            | DataMsg::UpToDate { view, .. }
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
            | DataMsg::Error { site, .. } => site,
        }
    }
}

/// What is currently uploaded to the GPU, so we only re-bin/re-upload on a real change.
/// The `u64` is the palette generation (a color-table reload forces a re-bake); the trailing
/// option is the storm-motion (east, north) m/s for storm-relative velocity.
type ShownKey = (String, Moment, usize, Option<f32>, bool, u64, Option<(u32, u32)>, bool);

pub struct HookEchoApp {
    _rt: Runtime,
    tiles: TileManager,
    vtiles: crate::vector_tiles::VectorTileManager,
    settings: Settings,
    saved: Settings,
    views: Vec<MapView>,
    active: usize,
    msg_rx: Receiver<DataMsg>,
    msg_tx: Sender<DataMsg>,
    /// Per-pane "what's uploaded" key, so each pane re-bins/re-uploads only on a real change.
    pane_shown: std::collections::HashMap<usize, ShownKey>,
    site_dialog: Option<ui::site_dialog::SiteDialog>,
    wizard: ui::wizard::Wizard,
    settings_window: ui::settings_window::SettingsWindow,
    cursor_ll: Option<(f64, f64)>,
    /// Active color tables (one per moment); reloaded when the palette settings change.
    palettes: Palettes,
    /// Live chunk stream for the active view: (view index, site, task handle).
    live_stream: Option<(usize, String, tokio::task::JoinHandle<()>)>,
    last_stream_attempt: Option<Instant>,
    /// Decoded-volume LRU keyed by AWS object name, so scrubbing back and forth on the
    /// timeline doesn't re-download. ~10 volumes; each ~a few MB.
    scan_cache: LruCache<String, Scan>,
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
    outlook_features: [Vec<GeoFeature>; 3],
    md_features: Vec<GeoFeature>,
    /// ProbSevere storm-probability polygons + badges (toggle + refresh clock).
    show_probsevere: bool,
    probsevere: Vec<GeoFeature>,
    probsevere_last_fetch: Option<Instant>,
    /// The currently-displayed, filtered feature set (hit-tested + tessellated).
    overlays: Vec<GeoFeature>,
    overlay_gen: u64,
    built_gen: u64,
    built_zoom_bucket: i32,
    pending_overlay: Option<OverlayUpload>,
    overlay_ready: bool,
    overlay_last_fetch: Option<Instant>,
    detail: Option<Detail>,
    /// Open "Storm {id} Attributes" window (a clicked storm cell).
    cell_popup: Option<Cell>,
    /// Open "Active Warnings" window (clicked warning/watch polygons).
    warning_popup: Option<ui::warning_window::WarningPopup>,
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
    placefile_window: ui::placefile_window::PlacefileWindow,
    /// Last map viewport size (px), used to estimate the view range for placefile thresholds.
    last_viewport: (f32, f32),
    /// Active left-click map tool.
    tool: MapTool,
    /// Measure-tool clicked endpoints in `[lon, lat]` (max 2).
    measure: Vec<[f64; 2]>,
    marker_window: ui::marker_window::MarkerWindow,
    event_window: ui::event_window::EventWindow,
    palette_editor: ui::palette_editor::PaletteEditor,
    digest_window: ui::digest_window::DigestWindow,
    digest_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    sounding_window: ui::sounding_window::SoundingWindow,
    sounding_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::sounding::Sounding, String>>>,
    /// Chase mode: follow a position, auto-switching the active pane to the nearest radar.
    chase_mode: bool,
    chase_pos: Option<(f64, f64)>,
    /// Tornado climatology: the loaded SPC track database (lazy), a pending async load, the last
    /// query result + its center, a window-open flag, and a query queued while the CSV loads.
    climo_tracks: Option<std::sync::Arc<Vec<wxdata::torclimo::TornadoTrack>>>,
    climo_rx: Option<std::sync::mpsc::Receiver<Result<Vec<wxdata::torclimo::TornadoTrack>, String>>>,
    climo_hits: Vec<wxdata::torclimo::TornadoTrack>,
    climo_center: Option<(f64, f64)>,
    climo_open: bool,
    climo_loading: bool,
    climo_error: Option<String>,
    climo_pending_query: Option<(f64, f64)>,
    chase_applied: Option<(f64, f64)>,
    /// Live position stream from gpsd, when the user has connected it.
    gps_rx: Option<std::sync::mpsc::Receiver<(f64, f64)>>,
    /// GOES satellite frame times (for the sub-hourly scrub), the style they were fetched for,
    /// and the selected index (`None` = latest).
    goes_times: Vec<chrono::DateTime<chrono::Utc>>,
    goes_times_style: Option<crate::tiles::BasemapStyle>,
    goes_time_idx: Option<usize>,
    goes_times_rx: Option<std::sync::mpsc::Receiver<Vec<chrono::DateTime<chrono::Utc>>>>,
    /// Where a requested screenshot should go once the image event arrives.
    screenshot_pending: Option<ShotDest>,
    loop_export: Option<LoopExport>,
    /// When true, all panes share the active pane's camera.
    link_cameras: bool,
    /// National gridded field layers (MRMS mosaic, rotation, MESH, AzShear, lightning), each with
    /// its own toggle + pending GPU upload + refresh throttle. Keyed by [`crate::render::FieldLayer`].
    fields: std::collections::HashMap<crate::render::FieldLayer, FieldState>,
    /// Selected rotation-track accumulation window (minutes): 30, 60, or 120.
    rotation_minutes: u16,
    /// Environment suite (HRRR CAPE/SRH): CAPE uses the mixed-layer (90-0 mb) parcel when true,
    /// else surface-based; SRH depth in km (1 = 0-1 km, 3 = 0-3 km). Changing either clears the
    /// layer's last_fetch so the next frame refetches.
    env_cape_ml: bool,
    env_srh_km: u8,
    /// The site the L3 gridded products (DVL/EET) were last fetched for (feature X); refetch on
    /// site change.
    l3grid_site: Option<String>,
    /// Surface obs (METAR station plots, feature U): toggle, current obs, fetch clock + bbox.
    show_metar: bool,
    metars: Vec<wxdata::metar::SurfaceOb>,
    metar_last_fetch: Option<Instant>,
    /// The `(lat0, lon0, lat1, lon1)` bbox the current `metars` were fetched for.
    metar_bounds: Option<(f64, f64, f64, f64)>,
    /// NHC tropical suite (feature V): toggle, fetched data, refresh clock.
    show_tropical: bool,
    tropical: Option<wxdata::tropical::TropicalData>,
    tropical_last_fetch: Option<Instant>,
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
    tray_rx: Option<std::sync::mpsc::Receiver<crate::tray::TrayCmd>>,
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
    /// Area Forecast Discussion window (feature DD): open flag, fetched text, in-flight receiver.
    afd_open: bool,
    afd: Option<wxdata::afd::Afd>,
    afd_error: Option<String>,
    afd_busy: bool,
    afd_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::afd::Afd, String>>>,
    /// Range rings + azimuth spokes around the active site (feature HH).
    show_range_rings: bool,
    /// Draw all NEXRAD radar sites on the map; clicking one switches the pane to that radar.
    show_radar_sites: bool,
    /// Show the left toolbox panel. Collapse it (View ▸ Toolbox / F9) for a full-width radar.
    show_toolbox: bool,
    /// Android only: which slide-up sheet the mobile chrome is showing (see `app::mobile`).
    mobile_sheet: mobile::MobileSheet,
    /// Spotter Network positions + toggle + refresh clock (filtered to active site at draw).
    show_spotters: bool,
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
    hodo_site: Option<String>,
    hodo_last_fetch: Option<Instant>,
    /// Streamer/OBS mode: hide all chrome (menu/toolbox/status/docks), leaving only the map.
    obs_mode: bool,
    /// Auto-tour: cycle the camera through active-warning centroids while in OBS mode.
    obs_tour: bool,
    obs_tour_last: Option<Instant>,
    obs_tour_idx: usize,
    /// Warning ids already seen, so a new warning is detected on arrival (not re-alerted).
    known_warning_ids: std::collections::HashSet<String>,
    /// False until the first alert fetch seeds `known_warning_ids` (avoids alerting on startup).
    warnings_seeded: bool,
    /// Per-location cooldown clock for the lightning-proximity alarm (re-alert after it goes quiet).
    lightning_alerted: std::collections::HashMap<String, Instant>,
    /// True while a TDS is currently detected, so the alert fires on the rising edge only.
    tds_active: bool,
    /// Active new-warning banners (event, area, first-seen time); expire after a while.
    warning_banners: Vec<(String, String, Instant)>,
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
    vol3d_az: f32,
    vol3d_el: f32,
    vol3d_dist: f32,
    vol3d_pending: Option<crate::render3d::Volume3dUpload>,
    /// GPU 2D texture-size cap (device limit), used to clamp field-grid decimation on mobile GPUs.
    max_texture_dim: u32,
}

/// Split `r` into `n` pane rects: 1 full, 2 side-by-side, 3–4 in a 2×2 grid.
fn pane_rects(r: egui::Rect, n: usize) -> Vec<egui::Rect> {
    let gap = 2.0;
    match n {
        0 | 1 => vec![r],
        2 => {
            let w = (r.width() - gap) / 2.0;
            vec![
                egui::Rect::from_min_size(r.min, egui::vec2(w, r.height())),
                egui::Rect::from_min_size(egui::pos2(r.min.x + w + gap, r.min.y), egui::vec2(w, r.height())),
            ]
        }
        _ => {
            let w = (r.width() - gap) / 2.0;
            let h = (r.height() - gap) / 2.0;
            let mut v = Vec::new();
            for row in 0..2 {
                for col in 0..2 {
                    let x = r.min.x + (w + gap) * col as f32;
                    let y = r.min.y + (h + gap) * row as f32;
                    v.push(egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)));
                }
            }
            v.truncate(n.clamp(1, 4));
            v
        }
    }
}

impl HookEchoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let render_state = cc.wgpu_render_state.as_ref().expect("wgpu backend");
        // The GPU's 2D texture-size cap: desktop/Adreno do 16384, but many mobile GPUs cap at
        // 4096. Field grids (MRMS rotation/AzShear reach 14000 px) are decimated to fit this.
        let max_texture_dim = render_state.device.limits().max_texture_dimension_2d;
        {
            let mut w = render_state.renderer.write();
            w.callback_resources
                .insert(RenderResources::new(&render_state.device, render_state.target_format));
            w.callback_resources.insert(crate::render3d::Volume3dResources::new(
                &render_state.device,
                render_state.target_format,
            ));
        }

        let settings = Settings::load();
        let tiles = TileManager::new(rt.handle().clone());
        let vtiles = crate::vector_tiles::VectorTileManager::new(rt.handle().clone());
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        let (overlay_tx, overlay_rx) = std::sync::mpsc::channel();
        let http = reqwest::Client::new();

        // Open on the saved startup view if set (and its site still resolves), else the default site.
        let (start, camera) = match &settings.start_view {
            Some(sv) if wxdata::sites::site_by_id(&sv.site).is_some() => (
                sv.site.clone(),
                Camera { center: (sv.x, sv.y), zoom: sv.zoom },
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
        // Restore the persisted basemap (empty slug = keep the default; from_slug("") = None).
        if !settings.basemap.is_empty() {
            view.basemap = crate::tiles::BasemapStyle::from_slug(&settings.basemap);
        }
        let settings_setup_done = settings.setup_done;

        let mut app = Self {
            vtiles,
            _rt: rt,
            tiles,
            saved: settings.clone(),
            settings,
            views: vec![view],
            active: 0,
            msg_rx,
            msg_tx,
            pane_shown: std::collections::HashMap::new(),
            site_dialog: None,
            wizard: {
                let mut w = ui::wizard::Wizard::default();
                if !settings_setup_done {
                    w.start();
                }
                w
            },
            settings_window: Default::default(),
            cursor_ll: None,
            palettes: Palettes::default(),
            live_stream: None,
            last_stream_attempt: None,
            // DVR: retain a deep buffer of decoded volumes so instant replay serves recent frames
            // from RAM without re-downloading (~30 volumes ≈ 2.5 h at a 5-min cadence).
            scan_cache: LruCache::new(NonZeroUsize::new(30).unwrap()),
            http,
            overlay_rx,
            overlay_tx,
            filters: OverlayFilters::default(),
            alert_features: Vec::new(),
            arch_warns: LruCache::new(NonZeroUsize::new(50).unwrap()),
            arch_warn_inflight: None,
            arch_warn_shown: None,
            arch_lsr: LruCache::new(NonZeroUsize::new(50).unwrap()),
            arch_lsr_inflight: None,
            arch_lsr_shown: None,
            outlook_features: [Vec::new(), Vec::new(), Vec::new()],
            md_features: Vec::new(),
            show_probsevere: false,
            probsevere: Vec::new(),
            probsevere_last_fetch: None,
            overlays: Vec::new(),
            overlay_gen: 0,
            built_gen: u64::MAX,
            built_zoom_bucket: i32::MIN,
            pending_overlay: None,
            overlay_ready: false,
            overlay_last_fetch: None,
            detail: None,
            cell_popup: None,
            warning_popup: None,
            storm_cells: Vec::new(),
            ui_scale_applied: -1.0,
ime_shown: false,
pending_paste: None,
paste_target: None,
            placefiles: Vec::new(),
            placefile_window: Default::default(),
            last_viewport: (1000.0, 800.0),
            tool: MapTool::default(),
            measure: Vec::new(),
            marker_window: Default::default(),
            event_window: Default::default(),
            palette_editor: Default::default(),
            digest_window: Default::default(),
            digest_rx: None,
            sounding_window: Default::default(),
            sounding_rx: None,
            chase_mode: false,
            chase_pos: None,
            climo_tracks: None,
            climo_rx: None,
            climo_hits: Vec::new(),
            climo_center: None,
            climo_open: false,
            climo_loading: false,
            climo_error: None,
            climo_pending_query: None,
            chase_applied: None,
            gps_rx: None,
            goes_times: Vec::new(),
            goes_times_style: None,
            goes_time_idx: None,
            goes_times_rx: None,
            screenshot_pending: None,
            loop_export: None,
            link_cameras: false,
            cells_site: None,
            cell_trends: std::collections::HashMap::new(),
            fields: crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .map(|&l| (l, FieldState::default()))
                .collect(),
            rotation_minutes: 30,
            env_cape_ml: false,
            env_srh_km: 3,
            l3grid_site: None,
            show_metar: false,
            metars: Vec::new(),
            metar_last_fetch: None,
            metar_bounds: None,
            show_tropical: false,
            tropical: None,
            tropical_last_fetch: None,
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
            tray_rx: crate::tray::spawn(),
            really_quit: false,
            show_storm_reports: false,
            storm_reports: Vec::new(),
            reports_last_fetch: None,
            show_aviation: false,
            aviation_features: Vec::new(),
            aviation_last_fetch: None,
            afd_open: false,
            afd: None,
            afd_error: None,
            afd_busy: false,
            afd_rx: None,
            show_range_rings: false,
            show_radar_sites: true,
            // Phone screens are ~360 pt wide — the toolbox starts as a hidden drawer there (☰ toggles).
show_toolbox: !cfg!(target_os = "android"),
mobile_sheet: mobile::MobileSheet::None,
            show_spotters: false,
            spotters: Vec::new(),
            spotters_last_fetch: None,
            show_sensors: false,
            sensor_data: None,
            sensor_site: None,
            sensor_last_fetch: None,
            show_hodo: false,
            hodo_data: Vec::new(),
            hodo_site: None,
            hodo_last_fetch: None,
            obs_mode: false,
            obs_tour: false,
            obs_tour_last: None,
            obs_tour_idx: 0,
            known_warning_ids: std::collections::HashSet::new(),
            warnings_seeded: false,
            lightning_alerted: std::collections::HashMap::new(),
            tds_active: false,
            warning_banners: Vec::new(),
            show_alert_panel: false,
            xsection_pts: Vec::new(),
            xsection: None,
            xsection_tex: None,
            marker_icon_tex: Default::default(),
            show_3d: false,
            vol3d_az: 30.0,
            vol3d_el: 25.0,
            vol3d_dist: 3.0,
            vol3d_pending: None,
            max_texture_dim,
        };
        app.palettes.reload(&app.settings.palette_paths());
        app.fetch_overlays(&cc.egui_ctx.clone());
        app
    }

    /// Spawn background fetches for all overlay sources (alerts, SPC outlooks, MDs).
    /// Spawn a background overlay fetch, routing the result to `overlay_rx`.
    fn spawn_overlay(&self, ctx: &egui::Context, source: OverlaySource) {
        let http = self.http.clone();
        let tx = self.overlay_tx.clone();
        let ctx = ctx.clone();
        self._rt.spawn(async move {
            match source.fetch(&http).await {
                Ok(msg) => {
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                }
                Err(e) => log::warn!("overlay fetch failed: {e}"),
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
        self.spawn_overlay(ctx, OverlaySource::Alerts);
        self.spawn_overlay(ctx, OverlaySource::Mds);
        // Only fetch the SPC outlook the user has selected (off = day 0 fetches nothing).
        if (1..=3).contains(&self.filters.outlook_day) {
            self.spawn_overlay(ctx, OverlaySource::Outlook(self.filters.outlook_day, self.outlook_kind_for_day()));
        }
        // Storm cells for the active view's site (Level 3 products are per-site).
        if let Some(site) = self.views[self.active].site.clone() {
            self.spawn_overlay(ctx, OverlaySource::Cells(site));
        }
    }

    /// Reconcile loaded placefiles with `settings.placefiles`: fetch new/enabled URLs, drop
    /// removed ones, mirror the enabled flag, and refetch on each file's `RefreshSeconds`.
    fn sync_placefiles(&mut self, ctx: &egui::Context) {
        // Drop entries no longer configured.
        let before = self.placefiles.len();
        self.placefiles
            .retain(|lp| self.settings.placefiles.iter().any(|c| c.url == lp.url));
        let mut changed = self.placefiles.len() != before;
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
            let stale = match lp.last_fetch {
                None => true,
                Some(t) => {
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
            self.spawn_overlay(ctx, OverlaySource::Placefile(url));
        }
        if changed {
            self.overlay_gen = self.overlay_gen.wrapping_add(1);
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

    /// Draw new-warning banners at top-center (auto-expire ~45s; click to dismiss all).
    fn show_warning_banners(&mut self, ctx: &egui::Context) {
        self.warning_banners.retain(|(_, _, at)| at.elapsed().as_secs() < 45);
        if self.warning_banners.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("warning_banners"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
            .show(ctx, |ui| {
                let mut dismiss = false;
                for (event, area, _) in &self.warning_banners {
                    let resp = egui::Frame::new()
                        .fill(egui::Color32::from_rgb(150, 20, 20))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 120, 120)))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("⚠").size(16.0).color(egui::Color32::WHITE));
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(format!("New {event}")).strong().color(egui::Color32::WHITE));
                                    if !area.is_empty() {
                                        ui.label(egui::RichText::new(area).small().color(egui::Color32::from_gray(230)));
                                    }
                                });
                            });
                        })
                        .response;
                    if resp.interact(egui::Sense::click()).clicked() {
                        dismiss = true;
                    }
                    ui.add_space(4.0);
                }
                if dismiss {
                    self.warning_banners.clear();
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    /// Detect warning-tier alerts whose id we haven't seen, raising a banner + audible cue for
    /// each new one. The first fetch only seeds the known set (no alert on already-active warnings).
    fn detect_new_warnings(&mut self, feats: &[GeoFeature]) {
        let mut alerted = false;
        let mut max_esc = 0u8; // highest escalation among newly-seen warnings this pass
        // Only banner warnings within the selected radar's coverage — a warning covering a saved
        // location still banners + pushes regardless (that's a watched place, not the viewed site).
        let site_box = self.active_site_bounds(250.0);
        for f in feats {
            if f.kind != overlay::FeatureKind::Warning {
                continue;
            }
            let Some(a) = &f.alert else { continue };
            // Mark every warning seen so it can't re-banner later, but only alert on genuinely new
            // ones after the first (seeding) pass.
            if self.known_warning_ids.insert(a.id.clone()) && self.warnings_seeded {
                let esc = wxdata::alerts::escalation(a);
                let urgent = esc >= 2;
                // A watched location (saved marker) inside the polygon always alerts + pushes.
                let hit = self
                    .settings
                    .markers
                    .iter()
                    .find(|m| f.contains(m.lon, m.lat));
                let (label, area) = match hit {
                    Some(m) => {
                        // Watched location covered → push to the phone (opt-in ntfy topic).
                        self.push_ntfy(
                            &format!("⚠ {} — {}", a.event, m.name),
                            if a.headline.is_empty() { &a.area } else { &a.headline },
                            urgent,
                        );
                        (format!("⚠ {}", a.event), format!("covers {}", m.name))
                    }
                    None => {
                        // No watched location: banner only if it's near the selected radar.
                        if site_box.is_none_or(|bx| !feature_in_box(f, bx)) {
                            continue;
                        }
                        (a.event.clone(), a.area.clone())
                    }
                };
                max_esc = max_esc.max(esc);
                self.warning_banners.push((label, area, Instant::now()));
                alerted = true;
            }
        }
        self.warnings_seeded = true;
        if alerted {
            print!("\x07"); // free terminal bell alongside the chime
            use std::io::Write;
            let _ = std::io::stdout().flush();
            if self.settings.alert_sound {
                // Escalated (Tornado Emergency / PDS / destructive) warnings use the emergency sound.
                let sound = if max_esc >= 2 {
                    &self.settings.emergency_sound
                } else {
                    &self.settings.warn_sound
                };
                crate::audio::play(sound, self.settings.alert_volume);
            }
        }
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
        for m in &self.settings.markers {
            if field.max_within_km(m.lon, m.lat, RADIUS_KM) < DENSITY_MIN {
                continue;
            }
            let recent = self
                .lightning_alerted
                .get(&m.name)
                .is_some_and(|t| t.elapsed() < COOLDOWN);
            if recent {
                continue;
            }
            self.lightning_alerted.insert(m.name.clone(), Instant::now());
            self.push_ntfy(
                &format!("⚡ Lightning near {}", m.name),
                &format!("Cloud-to-ground strikes within {RADIUS_KM:.0} km of {}", m.name),
                false,
            );
            self.warning_banners.push((
                format!("⚡ Lightning near {}", m.name),
                format!("within {RADIUS_KM:.0} km"),
                Instant::now(),
            ));
            fired = true;
        }
        if fired && self.settings.alert_sound {
            crate::audio::play(&self.settings.lightning_sound, self.settings.alert_volume);
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
                if let Some(s) = self.fields.get_mut(&FL::Hrrr) {
                    s.show = true;
                }
                self.hrrr_by_timeline = true;
            }
            None => {
                if self.hrrr_by_timeline {
                    if let Some(s) = self.fields.get_mut(&FL::Hrrr) {
                        s.show = false;
                    }
                    self.hrrr_by_timeline = false;
                }
            }
        }
    }

    /// Build the 3D reflectivity volume from the active pane and open the raymarch window.
    fn build_volume3d(&mut self) {
        const N: usize = 192;
        const NZ: usize = 48;
        let Some(vol) = self.views[self.active].volume.as_mut() else { return };
        let sweeps = vol.reflectivity_tilts();
        if sweeps.is_empty() {
            return;
        }
        let Some(v3) = wxdata::volume3d::build(&sweeps, N, NZ, 150.0, 18.0) else { return };
        let lut = crate::colormap::bake_lut(
            self.palettes.table(Moment::Reflectivity),
            (v3.value_min, v3.value_max),
            None,
        )
        .to_vec();
        self.vol3d_pending = Some(crate::render3d::Volume3dUpload {
            data: v3.data,
            n: v3.n as u32,
            nz: v3.nz as u32,
            lut,
        });
        self.show_3d = true;
    }

    /// Re-slice the active pane's cached volume into a CAPPI at `cappi_alt_km` when the key
    /// (volume name + altitude) changed, and refresh the window texture (feature AA).
    fn update_cappi(&mut self, ctx: &egui::Context) {
        const HALF_KM: f32 = 150.0;
        const N: usize = 256;
        let Some(name) = self.views[self.active].volume.as_ref().map(|v| v.name.clone()) else {
            self.cappi_tex = None;
            self.cappi_key = None;
            return;
        };
        let key = (name, self.cappi_alt_km.to_bits());
        if self.cappi_key.as_ref() == Some(&key) {
            return;
        }
        let Some(vol) = self.views[self.active].volume.as_mut() else { return };
        let sweeps = vol.reflectivity_tilts();
        if sweeps.is_empty() {
            return;
        }
        let Some(c) = wxdata::volume3d::cappi(&sweeps, self.cappi_alt_km, N, HALF_KM) else { return };
        let img = ui::cappi_window::to_image(&c, self.palettes.table(Moment::Reflectivity));
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
            && self.active_storm_reports().iter().any(|r| to_screen_hit(r.lon, r.lat) <= tap_r2(12.0)))
            || (self.cells_site.as_deref() == self.views[idx].site.as_deref()
                && self.storm_cells.iter().any(|c| to_screen_hit(c.lon, c.lat) <= tap_r2(14.0)));
        if near_storm {
            return false;
        }
        let hit = wxdata::sites::sites()
            .iter()
            .filter(|s| to_screen_hit(s.longitude as f64, s.latitude as f64) <= tap_r2(12.0))
            .min_by(|a, b| {
                to_screen_hit(a.longitude as f64, a.latitude as f64)
                    .partial_cmp(&to_screen_hit(b.longitude as f64, b.latitude as f64))
                    .unwrap()
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
        let Some(dir) = crate::settings::Settings::marker_icons_dir() else { return };
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
        let Some(vol) = self.views[idx].volume.as_mut() else { return };
        let sweeps = vol.reflectivity_tilts(); // owned → the &mut vol borrow ends here
        if sweeps.is_empty() {
            return;
        }
        let Some(xs) = wxdata::xsection::build(&sweeps, (a[0], a[1]), (b[0], b[1]), 300, 120, 18.0) else {
            return;
        };
        let img = ui::xsection_window::to_image(&xs, self.palettes.table(Moment::Reflectivity));
        self.xsection_tex = Some(ctx.load_texture("xsection", img, egui::TextureOptions::LINEAR));
        self.xsection = Some(xs);
    }

    /// POST a high-priority push notification to the user's ntfy.sh topic (no-op if unset).
    /// Best-effort on the shared tokio runtime; failures are logged, never fatal.
    fn push_ntfy(&self, title: &str, body: &str, urgent: bool) {
        let topic = self.settings.ntfy_topic.trim().to_string();
        if topic.is_empty() {
            return;
        }
        let http = self.http.clone();
        let (title, body) = (title.to_string(), body.to_string());
        let priority = if urgent { "urgent" } else { "high" };
        self._rt.spawn(async move {
            let res = http
                .post(format!("https://ntfy.sh/{topic}"))
                .header("Title", title)
                .header("Priority", priority)
                .header("Tags", "warning,cloud_with_lightning")
                .body(body)
                .send()
                .await;
            if let Err(e) = res {
                log::warn!("ntfy push failed: {e}");
            }
        });
    }

    /// Sub-hourly GOES scrub bar: shown when the active basemap is a GOES layer and its frame
    /// times are loaded. Steps through recent 10-min frames; "Latest" pins to the newest.
    fn goes_time_bar(&mut self, ctx: &egui::Context) {
        let active_is_goes = self.views[self.active].basemap.goes_layer().is_some();
        if !active_is_goes || self.goes_times.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("goes_time_bar"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -34.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let n = self.goes_times.len();
                        // Effective index (None = latest = n-1).
                        let cur = self.goes_time_idx.unwrap_or(n - 1);
                        ui.label("🛰 GOES:");
                        if ui.add_enabled(cur > 0, egui::Button::new("◀")).clicked() {
                            self.goes_time_idx = Some(cur.saturating_sub(1));
                        }
                        let label = self.goes_times[cur].format("%H:%MZ").to_string();
                        ui.monospace(label);
                        if ui.add_enabled(cur + 1 < n, egui::Button::new("▶")).clicked() {
                            let ni = cur + 1;
                            self.goes_time_idx = if ni >= n - 1 { None } else { Some(ni) };
                        }
                        if ui.add_enabled(self.goes_time_idx.is_some(), egui::Button::new("Latest")).clicked() {
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
            if let Some(pos) = latest {
                self.chase_pos = Some(pos);
            }
        }
        if !self.chase_mode {
            self.chase_applied = None;
            return;
        }
        let Some((lon, lat)) = self.chase_pos else { return };
        if self.chase_applied == Some((lon, lat)) {
            return;
        }
        if let Some(site) = crate::geo::nearest_site_id(lon, lat) {
            let zoom = self.views[self.active].camera.zoom.max(8.0);
            self.goto_view(&site, lon, lat, zoom, None);
        }
        self.chase_applied = Some((lon, lat));
    }

    /// Pull an HRRR point sounding at `(lon, lat)`, shown in the Skew-T window when it arrives.
    fn fetch_sounding(&mut self, lon: f64, lat: f64) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sounding_rx = Some(rx);
        self.sounding_window.open = true;
        self.sounding_window.busy = true;
        self.sounding_window.sounding = None;
        let http = self.http.clone();
        self._rt.spawn(async move {
            let res = wxdata::sounding::fetch(&http, lon, lat).await.map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Optical-flow nowcast: advect every strong reflectivity gate of the active pane forward by
    /// the mean SCIT storm motion to the configured lead time. Returns advected `(lon, lat, color)`
    /// points for the painter. Coarse (subsampled gates) — a first-order extrapolation, not a model.
    fn compute_nowcast(&mut self, idx: usize) -> Vec<(f64, f64, egui::Color32)> {
        let Some((dir, kt)) = self.scit_mean_motion() else { return Vec::new() };
        if kt <= 1.0 {
            return Vec::new();
        }
        let lead_km = kt as f64 * 1.852 * (self.filters.nowcast_lead_min as f64 / 60.0);
        let tilt = self.views[idx].tilt;
        let sweep = match self.views[idx].volume.as_mut().and_then(|v| v.binned(Moment::Reflectivity, tilt, false).ok()) {
            Some(s) => s.clone(),
            None => return Vec::new(),
        };
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
                out.push((adv[0], adv[1], egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 150)));
            }
        }
        out
    }

    /// Auto TDS detection for the active pane's lowest tilt: bin reflectivity + CC and flag debris
    /// signatures (low CC in high Z). Fires a chime + banner on the rising edge of a new detection.
    fn compute_tds(&mut self, idx: usize) -> Vec<wxdata::tds::TdsHit> {
        // Lowest tilt carries the near-ground debris; dual-pol CC must be present.
        let z = match self.views[idx].volume.as_mut().and_then(|v| v.binned(Moment::Reflectivity, 0, false).ok()) {
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
            self.warning_banners.push((
                "⚠ TDS detected".to_string(),
                format!("{} debris signature(s) — possible tornado", hits.len()),
                Instant::now(),
            ));
            self.push_ntfy("⚠ Tornado Debris Signature", "Low CC + high reflectivity detected on radar", true);
            if self.settings.alert_sound {
                crate::audio::play(&self.settings.tds_sound, self.settings.alert_volume);
            }
        }
        self.tds_active = now_active;
        hits
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

    /// The tornado-climatology results window: a magnitude histogram + strongest-first list of
    /// historical tornadoes near the clicked point.
    fn show_climatology_window(&mut self, ctx: &egui::Context) {
        if !self.climo_open {
            return;
        }
        let mut open = self.climo_open;
        crate::ui::fit_phone(ctx, egui::Window::new("Tornado climatology"))
            .open(&mut open)
            .default_width(360.0)
            .show(ctx, |ui| {
                if let Some((lon, lat)) = self.climo_center {
                    ui.label(format!("Within 25 mi of {lat:.3}, {lon:.3}"));
                }
                if self.climo_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading SPC tornado database (1950–2022)…");
                    });
                    return;
                }
                if let Some(e) = &self.climo_error {
                    ui.colored_label(egui::Color32::from_rgb(230, 90, 90), format!("Load failed: {e}"));
                    return;
                }
                ui.strong(format!("{} tornadoes on record", self.climo_hits.len()));
                let hist = wxdata::torclimo::mag_histogram(&self.climo_hits);
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in ["EF0", "EF1", "EF2", "EF3", "EF4", "EF5", "Unk"].iter().enumerate() {
                        crate::theme::stat_card(ui, label, &hist[i].to_string());
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    for t in self.climo_hits.iter().take(50) {
                        let mag = if t.mag < 0 { "EF?".to_string() } else { format!("EF{}", t.mag) };
                        ui.label(format!("{}  {}  start {:.2},{:.2}", t.year, mag, t.slat, t.slon));
                    }
                    if self.climo_hits.len() > 50 {
                        ui.weak(format!("… and {} more", self.climo_hits.len() - 50));
                    }
                });
            });
        self.climo_open = open;
    }

    /// Tornado-climatology query at `(lon, lat)`: if the SPC track database is loaded, list nearby
    /// historical tornadoes; otherwise start the (cached) async load and queue the query.
    fn query_climatology(&mut self, lon: f64, lat: f64) {
        const RADIUS_KM: f64 = 40.0; // ~25 mi
        self.climo_open = true;
        self.climo_error = None;
        if let Some(tracks) = self.climo_tracks.clone() {
            self.climo_hits = wxdata::torclimo::near(&tracks, lon, lat, RADIUS_KM);
            self.climo_center = Some((lon, lat));
            return;
        }
        self.climo_center = Some((lon, lat));
        self.climo_pending_query = Some((lon, lat));
        self.load_climatology();
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
        self._rt.spawn(async move {
            let res = load_or_fetch_climo(&http, cache).await.map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Build a plain-language briefing of the in-view weather. The templated summary shows
    /// instantly; if an Anthropic key is set, Claude rewrites it in the background.
    fn generate_digest(&mut self) {
        let bounds = self.view_bounds();
        let overlaps = |f: &GeoFeature| {
            let Some((w, s, e, n)) = f.bbox() else { return false };
            !(e < bounds.0 || w > bounds.2 || n < bounds.1 || s > bounds.3)
        };
        let alerts: Vec<crate::digest::AlertLine> = self
            .alert_features
            .iter()
            .filter(|f| overlaps(f))
            .filter_map(|f| f.alert.as_ref())
            .map(|a| crate::digest::AlertLine { event: a.event.clone(), area: a.area.clone() })
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
        self._rt.spawn(async move {
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
        (lon0.min(lon1), lat0.min(lat1), lon0.max(lon1), lat0.max(lat1))
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
    fn open_alert_popup(&mut self, id: &str) {
        let mut seen = std::collections::HashSet::new();
        let cards: Vec<ui::warning_window::WarnCard> = self
            .active_alert_features()
            .iter()
            .filter_map(|f| f.alert.as_ref().map(|a| (a, f.stroke)))
            .filter(|(a, _)| a.id == id && seen.insert(a.id.clone()))
            .map(|(a, color)| ui::warning_window::WarnCard { info: a.clone(), color })
            .collect();
        if !cards.is_empty() {
            self.detail = None;
            self.warning_popup = Some(ui::warning_window::WarningPopup { cards, selected: Some(0) });
        }
    }

    fn poll_overlays(&mut self) {
        let mut changed = false;
        while let Ok(msg) = self.overlay_rx.try_recv() {
            match msg {
                OverlayMsg::Alerts(f) => {
                    self.detect_new_warnings(&f);
                    self.alert_features = f;
                }
                OverlayMsg::Mds(f) => self.md_features = f,
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
                            let sample = ui::cell_window::CellSample { vil: c.vil, top: c.top_kft, dbz: c.max_dbz };
                            // Skip a duplicate of the last sample (same volume re-fetched).
                            if hist.last().is_none_or(|s| (s.vil, s.top, s.dbz) != (sample.vil, sample.top, sample.dbz)) {
                                hist.push(sample);
                                if hist.len() > 40 {
                                    hist.remove(0);
                                }
                            }
                        }
                        self.storm_cells = cells;
                        self.cells_site = Some(site);
                    }
                }
                OverlayMsg::Placefile(url, pf) => {
                    if let Some(lp) = self.placefiles.iter_mut().find(|lp| lp.url == url) {
                        lp.pf = pf;
                        lp.loaded = true;
                        lp.last_fetch = Some(Instant::now());
                    }
                }
                OverlayMsg::Field(layer, field) => {
                    if layer == crate::render::FieldLayer::Lightning {
                        self.check_lightning_proximity(&field);
                    }
                    // Some MRMS products (rotation/AzShear) are 14000×7000 — over the GPU texture
                    // cap; max-pool to the smaller of an 8192 ceiling and the device's real limit
                    // (mobile GPUs can be as low as 4096).
                    let cap = (self.max_texture_dim as usize).min(8192);
                    let field = field.decimated(cap);
                    let upload = self.field_upload(layer, &field);
                    if let Some(s) = self.fields.get_mut(&layer) {
                        s.pending = Some(upload);
                    }
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
                OverlayMsg::Spotters(spotters) => self.spotters = spotters,
                OverlayMsg::ProbSevere(f) => self.probsevere = f,
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
                OverlayMsg::Metar(obs) => self.metars = obs,
                OverlayMsg::Tropical(data) => self.tropical = Some(data),
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
        self._rt.spawn(async move {
            let res = wxdata::afd::fetch(&http, lat, lon).await.map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    /// Drive the METAR station-plot fetch (feature U): only when enabled and zoomed in enough,
    /// refetching every 75 s or when the view center drifts out of the fetched bbox's middle half.
    fn sync_metar(&mut self, ctx: &egui::Context) {
        if !self.show_metar {
            return;
        }
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        if (max_lon - min_lon) > 12.0 {
            return; // too zoomed out — a nationwide plot would be unreadable and huge
        }
        let (clon, clat) = ((min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5);
        let stale = self.metar_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 75);
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

    /// Reassemble the displayed overlay set from the fetched sources and current filters.
    fn rebuild_overlays(&mut self) {
        let mut v = Vec::new();
        if (1..=3).contains(&self.filters.outlook_day) {
            v.extend(self.outlook_features[(self.filters.outlook_day - 1) as usize].iter().cloned());
        }
        if self.filters.show_mds {
            v.extend(self.md_features.iter().cloned());
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
                v.extend(t.cones.iter().cloned());
            }
        }
        if self.show_aviation {
            v.extend(self.aviation_features.iter().cloned());
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

    /// Placefile items currently visible (enabled, zoom threshold met, within time range).
    fn visible_placefile_items(&self) -> Vec<&wxdata::placefile::PlaceItem> {
        let range = self.view_range_nmi();
        let now = Utc::now();
        let mut out = Vec::new();
        for lp in &self.placefiles {
            if !lp.enabled {
                continue;
            }
            for it in &lp.pf.items {
                if it.threshold_nmi > 0.0 && range > it.threshold_nmi {
                    continue;
                }
                if let Some((a, b)) = it.time {
                    if now < a || now > b {
                        continue;
                    }
                }
                out.push(it);
            }
        }
        out
    }

    /// Owned text/icon labels for the visible placefile items (drawn by the egui painter).
    /// `(color, [lon,lat], text, hover, is_icon)`.
    fn placefile_labels(&self) -> Vec<(egui::Color32, [f64; 2], String, String, bool)> {
        use wxdata::placefile::PlaceKind;
        self.visible_placefile_items()
            .iter()
            .filter_map(|it| match &it.kind {
                PlaceKind::Text { color, pos, text, hover } => {
                    Some((rgba32(*color), *pos, text.clone(), hover.clone(), false))
                }
                PlaceKind::Icon { color, pos, hover } => {
                    Some((rgba32(*color), *pos, String::new(), hover.clone(), true))
                }
                _ => None,
            })
            .collect()
    }

    /// Re-tessellate the overlay when its set or the zoom bucket changed.
    fn sync_overlay(&mut self) {
        let items = self.visible_placefile_items();
        if self.overlays.is_empty() && items.is_empty() {
            self.overlay_ready = false;
            return;
        }
        let zoom = self.views[self.active].camera.zoom;
        let bucket = (zoom * 2.0).round() as i32;
        if self.overlay_gen != self.built_gen || bucket != self.built_zoom_bucket {
            let mut geom = overlay_build::build(&self.overlays, zoom);
            overlay_build::append_placefiles(&mut geom, &items, zoom);
            self.overlay_ready = !geom.indices.is_empty();
            self.pending_overlay = Some(OverlayUpload { vertices: geom.vertices, indices: geom.indices });
            self.built_gen = self.overlay_gen;
            self.built_zoom_bucket = bucket;
        }
    }

    /// Spawn a background fetch of the latest volume for `site`, routed back to `view_idx`.
    /// `current_name = None` forces a re-download even if the newest volume is unchanged.
    fn spawn_fetch(&self, view_idx: usize, site: String, current_name: Option<String>, ctx: egui::Context) {
        let tx = self.msg_tx.clone();
        self._rt.spawn(async move {
            let msg = match level2::latest_identifier(&site).await {
                Ok(id) => {
                    let name = id.name().to_string();
                    if current_name.as_deref() == Some(name.as_str()) {
                        DataMsg::UpToDate { view: view_idx, site }
                    } else {
                        let time = id.date_time().unwrap_or_else(Utc::now);
                        match level2::download_scan(id).await {
                            Ok(scan) => DataMsg::Volume { view: view_idx, site, name, time, scan },
                            Err(e) => DataMsg::Error { view: view_idx, site, err: e.to_string() },
                        }
                    }
                }
                Err(e) => DataMsg::Error { view: view_idx, site, err: e.to_string() },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn poll_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            let idx = msg.view();
            // LiveEnded must be handled even after a site change (to drop the stream handle).
            if matches!(msg, DataMsg::LiveEnded { .. }) {
                if let DataMsg::LiveEnded { view, .. } = msg {
                    if self.live_stream.as_ref().is_some_and(|(v, _, _)| *v == view) {
                        self.live_stream = None; // interval polling resumes automatically
                    }
                }
                continue;
            }
            if idx >= self.views.len() || self.views[idx].site.as_deref() != Some(msg.site()) {
                continue; // view gone or its site changed since the fetch spawned
            }
            match msg {
                DataMsg::Volume { view, name, time, scan, .. } => {
                    self.scan_cache.put(name.clone(), scan.clone());
                    let v = &mut self.views[view];
                    let looping = v.timeline.live_looping();
                    // A newly-arrived live head (following): roll the day at UTC midnight, or grow
                    // the frame list so the loop window slides forward. A frame-fetch result for a
                    // scrubbed/loop-display frame is older than the head and isn't a new head.
                    let new_head = v.timeline.following && {
                        let last_time = v.timeline.frames.last().and_then(|id| id.date_time());
                        if time.date_naive() != v.timeline.date {
                            v.timeline.date = time.date_naive(); // re-list fires via frames_key
                            true
                        } else if last_time.is_none_or(|t| time > t)
                            && v.timeline.frames.last().map(|id| id.name()) != Some(name.as_str())
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
                        v.volume = Some(Volume::new(scan, name, time));
                    }
                    v.loading = false;
                    v.error = None;
                    v.clamp_tilt();
                    self.pane_shown.remove(&view);
                }
                DataMsg::Frames { view, site, date, frames } => {
                    let v = &mut self.views[view];
                    v.timeline.listing = false;
                    if v.timeline.date == date && v.site.as_deref() == Some(site.as_str()) {
                        v.timeline.set_frames(frames, (site, date));
                        self.pane_shown.remove(&view);
                    }
                }
                DataMsg::Live { view, name, time, scan, changed, .. } => {
                    let v = &mut self.views[view];
                    match &mut v.volume {
                        Some(vol) => vol.apply_live(scan, name, time, &changed),
                        None => v.volume = Some(Volume::new(scan, name, time)),
                    }
                    v.loading = false;
                    v.error = None;
                    v.clamp_tilt();
                    // A healthy stream pushes the poll deadline forward — this line IS the
                    // fallback: if the stream dies, interval polling resumes on schedule.
                    v.last_poll = Some(Instant::now());
                    self.pane_shown.remove(&view);
                }
                DataMsg::UpToDate { view, .. } => self.views[view].loading = false,
                DataMsg::Error { view, err, .. } => {
                    let v = &mut self.views[view];
                    v.loading = false;
                    v.error = Some(err);
                }
                DataMsg::LiveEnded { .. } => unreachable!("handled above"),
            }
        }
    }

    /// Start/stop the live chunk stream for the active view. One stream at a time (the active
    /// view); a healthy stream starves interval polling, a dead one lets polling take over.
    fn manage_stream(&mut self, ctx: &egui::Context) {
        let idx = self.active;
        let (want, site, base) = {
            let v = &self.views[idx];
            // Stream only while pinned to the live head; scrubbing pauses it. A live loop is
            // suppressed too — the loop shows past frames, so interval polling (not the sweep
            // stream) carries new-volume arrival. ponytail: stream resumes on pause / go_head.
            let want = v.timeline.following
                && !v.timeline.live_looping()
                && v.site.is_some()
                && v.volume.is_some();
            (want, v.site.clone(), v.volume.as_ref().map(|vol| vol.scan.clone()))
        };

        // Abort an existing stream if it no longer matches the active view/site or isn't wanted.
        if let Some((sv, ss, handle)) = &self.live_stream {
            if !want || *sv != idx || Some(ss.as_str()) != site.as_deref() {
                handle.abort();
                self.live_stream = None;
            }
        }

        if want && self.live_stream.is_none() {
            let due = self.last_stream_attempt.is_none_or(|t| t.elapsed().as_secs() >= 60);
            if due {
                self.last_stream_attempt = Some(Instant::now());
                let site = site.unwrap();
                let base = base.unwrap();
                let handle = self.spawn_stream(idx, site.clone(), base, ctx.clone());
                self.live_stream = Some((idx, site, handle));
            }
        }
    }

    /// Spawn the live chunk streamer for `site`, routing merged volumes back to `view_idx`.
    fn spawn_stream(&self, view_idx: usize, site: String, base: Scan, ctx: egui::Context) -> tokio::task::JoinHandle<()> {
        let tx = self.msg_tx.clone();
        self._rt.spawn(async move {
            let end_site = site.clone();
            let cb_tx = tx.clone();
            let cb_ctx = ctx.clone();
            let cb_site = site.clone();
            let res = live::stream(site, base, move |u| {
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
            let _ = tx.send(DataMsg::LiveEnded { view: view_idx, site: end_site });
            ctx.request_repaint();
        })
    }

    fn apply_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Product(m) => self.views[self.active].moment = m,
            Action::TiltUp => {
                let v = &mut self.views[self.active];
                if let Some(vol) = &v.volume {
                    if v.tilt + 1 < vol.elevations.len() {
                        v.tilt += 1;
                    }
                }
            }
            Action::TiltDown => {
                let v = &mut self.views[self.active];
                v.tilt = v.tilt.saturating_sub(1);
            }
            Action::OpenSiteDialog => {
                if self.site_dialog.is_none() {
                    self.site_dialog = Some(Default::default());
                }
            }
            Action::Reload => self.trigger_reload(ctx),
            Action::CycleBasemap => {
                let (mb, mt) = (!self.settings.mapbox_key.is_empty(), !self.settings.maptiler_key.is_empty());
                let v = &mut self.views[self.active];
                v.basemap = v.basemap.next(mb, mt);
            }
            Action::ToggleAlertPanel => self.show_alert_panel = !self.show_alert_panel,
            Action::ToggleObs => {
                self.obs_mode = !self.obs_mode;
                if !self.obs_mode {
                    self.obs_tour = false;
                }
            }
            Action::ToggleObsTour => {
                self.obs_tour = !self.obs_tour;
                self.obs_tour_last = None; // step immediately on enable
                if self.obs_tour {
                    self.obs_mode = true;
                }
            }
            Action::ToggleToolbox => self.show_toolbox = !self.show_toolbox,
            Action::InstantReplay => self.instant_replay(),
        }
    }

    /// Streamer/OBS auto-tour: every ~12 s, fly the active camera to the next active-warning
    /// centroid (highest-severity first), cycling. No-op with no warnings in the feed.
    fn drive_obs_tour(&mut self) {
        if !self.obs_tour {
            return;
        }
        if self.obs_tour_last.is_some_and(|t| t.elapsed().as_secs() < 12) {
            return;
        }
        // Centroids of active warning polygons, tornado/severe first.
        let mut targets: Vec<(u8, f64, f64)> = self
            .alert_features
            .iter()
            .filter(|f| f.kind == overlay::FeatureKind::Warning)
            .filter_map(|f| {
                let (w, s, e, n) = f.bbox()?;
                let sev = if f.title.to_lowercase().contains("tornado") { 0 } else { 1 };
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

    /// Per-frame per-pane: react to site changes, keep the timeline current, and (for the active
    /// pane) manage the live stream. Each pane fetches its own volume via its view index.
    fn sync_pane(&mut self, idx: usize, ctx: &egui::Context) {
        // Site change: clear the old volume, recenter, and (if a real site) refetch.
        let site_changed = self.views[idx].site != self.views[idx].loaded_site;
        if site_changed {
            let v = &mut self.views[idx];
            v.loaded_site = v.site.clone();
            v.volume = None;
            v.error = None;
            match &v.site {
                Some(s) => ui::site_dialog::center_on_site(&mut v.camera, s),
                None => {
                    self.pane_shown.remove(&idx);
                }
            }
            // Storm cells follow the active pane's site: drop the old ones and refetch.
            if idx == self.active {
                self.storm_cells.clear();
                self.cells_site = None;
                if let Some(site) = self.views[idx].site.clone() {
                    let http = self.http.clone();
                    let tx = self.overlay_tx.clone();
                    let ctx2 = ctx.clone();
                    self._rt.spawn(async move {
                        let cells = level3::fetch_cells(&http, &site).await;
                        let _ = tx.send(OverlayMsg::Cells(site, cells));
                        ctx2.request_repaint();
                    });
                }
            }
        }

        // Advance playback (if playing) then reconcile the displayed volume with the timeline.
        self.views[idx].timeline.live_window = self.settings.live_loop_frames.max(1);
        self.views[idx].timeline.tick();
        self.sync_timeline(idx, ctx, site_changed);

        // Live streaming is limited to the active pane; others poll their head.
        if idx == self.active {
            self.manage_stream(ctx);
        }
    }

    /// Reconcile the frame listing and the displayed volume with the timeline: keep the
    /// listing current, poll the live head while following, or load the scrubbed frame.
    fn sync_timeline(&mut self, idx: usize, ctx: &egui::Context, site_changed: bool) {
        // (Re)list volumes when the site or selected date changed.
        let (site, date, following, need_list, listing) = {
            let v = &self.views[idx];
            let key = v.site.clone().map(|s| (s, v.timeline.date));
            let need = v.site.is_some() && v.timeline.frames_key != key;
            (v.site.clone(), v.timeline.date, v.timeline.following, need, v.timeline.listing)
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
                    .is_none_or(|t| t.elapsed().as_secs() >= self.settings.poll_interval_secs);
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
            let target = self.views[idx]
                .timeline
                .current()
                .map(|id| (id.name().to_string(), id.date_time().unwrap_or_else(Utc::now), id.clone()));
            if let Some((name, time, id)) = target {
                let shown = self.views[idx].volume.as_ref().map(|v| v.name.clone());
                if shown.as_deref() != Some(name.as_str()) {
                    if let Some(scan) = self.scan_cache.get(&name).cloned() {
                        let v = &mut self.views[idx];
                        v.volume = Some(Volume::new(scan, name, time));
                        v.loading = false;
                        v.error = None;
                        v.clamp_tilt();
                        self.pane_shown.remove(&idx);
                    } else if !self.views[idx].loading {
                        let s = self.views[idx].site.clone().unwrap_or_default();
                        self.views[idx].loading = true;
                        self.spawn_frame_fetch(idx, s, id, ctx.clone());
                    }
                }
            }
        }
    }

    /// Download a specific archive volume (a scrubbed timeline frame), routed to `view_idx`.
    fn spawn_frame_fetch(&self, view_idx: usize, site: String, id: Identifier, ctx: egui::Context) {
        let tx = self.msg_tx.clone();
        self._rt.spawn(async move {
            let name = id.name().to_string();
            let time = id.date_time().unwrap_or_else(Utc::now);
            let msg = match level2::download_scan(id).await {
                Ok(scan) => DataMsg::Volume { view: view_idx, site, name, time, scan },
                Err(e) => DataMsg::Error { view: view_idx, site, err: e.to_string() },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    /// List the archive volumes for `site` on `date` (timeline frames).
    fn spawn_list_frames(&self, view_idx: usize, site: String, date: NaiveDate, ctx: egui::Context) {
        let tx = self.msg_tx.clone();
        self._rt.spawn(async move {
            match level2::list_volumes(&site, date).await {
                Ok(frames) => {
                    let _ = tx.send(DataMsg::Frames { view: view_idx, site, date, frames });
                    ctx.request_repaint();
                }
                Err(e) => log::warn!("list frames {site} {date}: {e}"),
            }
        });
    }

    /// Radar upload for pane `idx`, binning the shared volume in `data` (usually the active pane)
    /// with pane `idx`'s product/tilt. Returns `(upload_when_changed, draw_radar)`; the pane's GPU
    /// buffer persists, so `None` means "reuse what's uploaded". Caches per pane via `pane_shown`.
    fn pane_radar(&mut self, idx: usize, data: usize) -> (Option<RadarUpload>, bool) {
        let has_volume = self.views[data].volume.is_some();
        if !self.views[idx].show_radar || !has_volume {
            self.pane_shown.remove(&idx);
            return (None, false);
        }
        let count = self.views[data].elevation_count();
        self.views[idx].clamp_tilt_to(&count);
        let (moment, tilt, threshold, smooth, storm_uv) = {
            let v = &self.views[idx];
            (v.moment, v.tilt, v.active_threshold(), v.smooth, v.storm_motion_uv())
        };
        let name = self.views[data].volume.as_ref().unwrap().name.clone();
        let uv_key = storm_uv.map(|(e, n)| (e.to_bits(), n.to_bits()));
        // Dealiasing only applies to Doppler velocity.
        let dealias = self.settings.dealias_velocity && moment == Moment::Velocity;
        let key: ShownKey = (name, moment, tilt, threshold, smooth, self.palettes.gen, uv_key, dealias);
        if self.pane_shown.get(&idx) == Some(&key) {
            return (None, true);
        }
        let table = self.palettes.table(moment);
        let upload = {
            let vol = self.views[data].volume.as_mut().unwrap();
            vol.binned(moment, tilt, dealias).map(|s| to_upload(s, table, threshold, smooth, storm_uv))
        };
        match upload {
            Ok(up) => {
                self.pane_shown.insert(idx, key);
                (Some(up), true)
            }
            Err(e) => {
                self.views[idx].error = Some(e.to_string());
                (None, false)
            }
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
        is_vector: bool,
        is_raster: bool,
        clear_tiles: bool,
        clear_vector: bool,
        first: bool,
        placefile_labels: &[(egui::Color32, [f64; 2], String, String, bool)],
    ) {
        use crate::tiles::BasemapStyle;
        let vp = (prect.width(), prect.height());
        let response = ui.interact(prect, egui::Id::new(("pane", idx)), egui::Sense::click_and_drag());

        // --- Input (mutates this pane's camera / selects it active) ---
        // During a multi-touch gesture the first finger still drives the egui pointer, so a pinch
        // would ALSO register as a drag and fight the zoom — the gesture block below owns both
        // pan and zoom while two fingers are down.
        let gesture = ui.input(|i| i.multi_touch());
        if response.dragged() && gesture.is_none() {
            self.active = idx;
            let d = response.drag_delta();
            self.views[idx].camera.pan_pixels(d.x, d.y);
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            if let Some(pos) = response.hover_pos() {
                if prect.contains(pos) {
                    self.active = idx;
                    let cursor = (pos.x - prect.left(), pos.y - prect.top());
                    self.views[idx].camera.zoom_at(scroll as f64 * 0.005, cursor, vp);
                }
            }
        }
        // Two-finger gesture (touchscreens): pan by the gesture's translation, and zoom by the
        // pinch. `zoom_delta` is a scale factor, so its log2 is the change in the camera's log2
        // zoom level; anchor it at the gesture center so the pinched point stays put. Fires for
        // the pane the gesture centers over. No-op with no touch.
        if let Some(mt) = gesture {
            if prect.contains(mt.center_pos) {
                self.active = idx;
                let t = mt.translation_delta;
                if t != egui::Vec2::ZERO {
                    self.views[idx].camera.pan_pixels(t.x, t.y);
                }
                if (mt.zoom_delta - 1.0).abs() > f32::EPSILON {
                    let cursor = (mt.center_pos.x - prect.left(), mt.center_pos.y - prect.top());
                    self.views[idx].camera.zoom_at((mt.zoom_delta as f64).log2(), cursor, vp);
                }
            }
        }
        if response.hover_pos().is_some_and(|p| prect.contains(p)) {
            let cam = self.views[idx].camera;
            self.cursor_ll = response.hover_pos().map(|pos| {
                let px = (pos.x - prect.left(), pos.y - prect.top());
                let w = cam.screen_to_world(px, vp);
                crate::render::mercator::world_to_lonlat(w.0, w.1)
            });
        }

        if response.clicked() {
            self.active = idx;
            if let Some(pos) = response.interact_pointer_pos() {
                let cam = self.views[idx].camera;
                let px = (pos.x - prect.left(), pos.y - prect.top());
                let w = cam.screen_to_world(px, vp);
                let (lon, lat) = crate::render::mercator::world_to_lonlat(w.0, w.1);
                // Interrogate + a click on a radar-site ring switches radars (storm features win,
                // handled inside try_pick_site). Consumes the click so no popup opens underneath.
                let picked_site = self.tool == MapTool::Interrogate
                    && self.show_radar_sites
                    && self.try_pick_site(idx, pos, cam, prect, vp);
                match self.tool {
                    _ if picked_site => {}
                    MapTool::Measure => {
                        if self.measure.len() >= 2 {
                            self.measure.clear();
                        }
                        self.measure.push([lon, lat]);
                    }
                    MapTool::Marker => {
                        let n = self.settings.markers.len() + 1;
                        self.settings.markers.push(crate::settings::Marker {
                            name: format!("Marker {n}"),
                            lat,
                            lon,
                            icon: None,
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
                    MapTool::Chase => {
                        self.chase_mode = true;
                        self.chase_pos = Some((lon, lat));
                    }
                    MapTool::Climatology => self.query_climatology(lon, lat),
                    MapTool::Interrogate => {
                        // Storm reports sit on top: a click near a report dot opens its detail.
                        let report = self.show_storm_reports.then(|| {
                            self.active_storm_reports().iter().find(|r| {
                                let w = crate::render::mercator::lonlat_to_world(r.lon, r.lat);
                                let (sx, sy) = cam.world_to_screen(w, vp);
                                let (dx, dy) = (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
                                dx * dx + dy * dy <= tap_r2(12.0)
                            })
                        }).flatten().cloned();
                        if let Some(r) = report {
                            self.cell_popup = None;
                            self.warning_popup = None;
                            self.detail = Some(Detail {
                                title: format!("{} Report — {}", r.kind.label(), r.magnitude),
                                body: format!(
                                    "{}, {}\nCounty: {}\nTime: {}Z\n\n{}",
                                    r.location, r.state, r.county, r.time, r.comments
                                ),
                                color: report_color(r.kind),
                            });
                        } else {
                        let cell_hit = self.filters.show_cells
                            && self.cells_site.as_deref() == self.views[idx].site.as_deref()
                            && !self.storm_cells.is_empty();
                        let picked = cell_hit
                            .then(|| {
                                self.storm_cells.iter().find(|c| {
                                    let w = crate::render::mercator::lonlat_to_world(c.lon, c.lat);
                                    let (sx, sy) = cam.world_to_screen(w, vp);
                                    let (dx, dy) = (prect.left() + sx - pos.x, prect.top() + sy - pos.y);
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
                                        Some(ui::warning_window::WarningPopup { cards, selected: Some(0) });
                                } else {
                                    self.warning_popup = None;
                                    self.detail = hits.first().map(|f| Detail {
                                        title: f.title.clone(),
                                        body: f.detail.clone(),
                                        color: f.stroke,
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
        let visible = if is_raster {
            let vis = self.tiles.visible(&cam, vp);
            self.tiles.request_missing(&vis);
            vis
        } else {
            Vec::new()
        };
        let (visible_vector, vlabels) = if is_vector {
            let vis = self.vtiles.visible(&cam, vp);
            self.vtiles.request_missing(&vis);
            let ids: Vec<crate::render::TileId> = vis.iter().map(|v| v.id).collect();
            let labels: Vec<crate::vector_tiles::PlaceLabel> =
                self.vtiles.labels_for(ids.iter()).into_iter().cloned().collect();
            (ids, labels)
        } else {
            (Vec::new(), Vec::new())
        };
        // Drain finished fetches once (on the first pane) — they upload into the shared cache.
        let (new_tiles, new_vector_tiles) = if first {
            let nt = self.tiles.drain_ready();
            let nv = self.vtiles.drain_ready();
            if !nt.is_empty() || !nv.is_empty() {
                ctx.request_repaint();
            }
            (nt, nv)
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
        let field_uploads: Vec<(crate::render::FieldLayer, crate::render::MrmsUpload)> = if first {
            self.fields
                .iter_mut()
                .filter_map(|(k, s)| s.pending.take().map(|u| (*k, u)))
                .collect()
        } else {
            Vec::new()
        };
        let field_draws: Vec<crate::render::FieldLayer> =
            self.fields.iter().filter(|(_, s)| s.show).map(|(k, _)| *k).collect();

        let cam = self.views[idx].camera;
        let (center, scale) = cam.world_to_clip_uniform(vp);
        let cb = MapCallback {
            pane: idx as u32,
            camera_center: center,
            camera_scale: scale,
            new_tiles,
            visible,
            radar_upload,
            draw_radar,
            overlay_upload: if first { self.pending_overlay.take() } else { None },
            draw_overlay: self.overlay_ready,
            field_uploads,
            field_draws,
            clear_tiles,
            new_vector_tiles,
            visible_vector,
            clear_vector,
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(prect, cb));

        // Per-pane product picker (multi-pane only): set THIS pane's moment directly, without
        // clicking to activate it first. Single-pane keeps using the toolbox Level 2 section.
        if self.views.len() > 1 && !self.obs_mode {
            let cur = self.views[idx].moment;
            egui::Area::new(egui::Id::new(("pane_product", idx)))
                .order(egui::Order::Foreground)
                .fixed_pos(prect.left_top() + egui::vec2(6.0, 6.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::symmetric(4, 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for m in Moment::ALL {
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
        let tds_hits = if self.filters.show_tds && idx == self.active {
            self.compute_tds(idx)
        } else {
            Vec::new()
        };

        // --- Painter overlays (clipped to this pane) ---
        let painter = ui.painter_at(prect);
        let view = &self.views[idx];
        let basemap = view.basemap;

        // Vector city/town labels.
        if is_vector && !vlabels.is_empty() {
            let st = crate::basemap_style::style(basemap == BasemapStyle::Dark);
            let text_col = egui::Color32::from_rgb(st.label[0], st.label[1], st.label[2]);
            let halo_col = egui::Color32::from_rgb(st.label_halo[0], st.label_halo[1], st.label_halo[2]);
            let z = cam.zoom;
            let mut labels: Vec<&crate::vector_tiles::PlaceLabel> =
                vlabels.iter().filter(|l| l.city || z >= 9.0).collect();
            labels.sort_by_key(|l| (!l.city, l.rank));
            let mut placed: Vec<egui::Rect> = Vec::new();
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for l in labels {
                if !seen.insert(l.name.as_str()) {
                    continue;
                }
                let (sx, sy) = cam.world_to_screen((l.world[0] as f64, l.world[1] as f64), vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let font = egui::FontId::proportional(if l.city { 13.0 } else { 11.0 });
                let galley = painter.layout_no_wrap(l.name.clone(), font.clone(), text_col);
                let r = egui::Rect::from_min_size(p, galley.size()).expand(4.0);
                if placed.iter().any(|q| q.intersects(r)) {
                    continue;
                }
                placed.push(r);
                for off in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(0.0, -1.0)] {
                    painter.text(p + off, egui::Align2::LEFT_TOP, &l.name, font.clone(), halo_col);
                }
                painter.text(p, egui::Align2::LEFT_TOP, &l.name, font.clone(), text_col);
            }
            painter.text(
                egui::pos2(prect.left() + 6.0, prect.bottom() - 4.0),
                egui::Align2::LEFT_BOTTOM,
                "© OpenMapTiles © OpenStreetMap",
                egui::FontId::proportional(10.0),
                text_col.gamma_multiply(0.7),
            );
        }

        // Raster basemap attribution (provider styles + USGS satellite).
        if view.basemap.is_raster() {
            let col = egui::Color32::from_gray(200).gamma_multiply(0.6);
            painter.text(
                egui::pos2(prect.left() + 6.0, prect.bottom() - 4.0),
                egui::Align2::LEFT_BOTTOM,
                view.basemap.attribution(),
                egui::FontId::proportional(10.0),
                col,
            );
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
                let mut etas: Vec<(f64, String)> = Vec::new();
                for c in &self.storm_cells {
                    let (Some(dir), Some(kt)) = (c.mvt_deg, c.mvt_kt) else { continue };
                    if kt <= 1.0 {
                        continue;
                    }
                    let lead_km = kt as f64 * 1.852 * (LEAD_MIN / 60.0);
                    let left = crate::geo::destination_point([c.lon, c.lat], dir as f64 - HALF_ANGLE, lead_km);
                    let right = crate::geo::destination_point([c.lon, c.lat], dir as f64 + HALF_ANGLE, lead_km);
                    let apex = to_screen(c.lon, c.lat);
                    let lp = to_screen(left[0], left[1]);
                    let rp = to_screen(right[0], right[1]);
                    let col = cell_color(c.kind);
                    let fill = egui::Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 40);
                    painter.add(egui::Shape::convex_polygon(vec![apex, lp, rp], fill, egui::Stroke::NONE));
                    // Center line toward the projected 60-min position.
                    let tip = crate::geo::destination_point([c.lon, c.lat], dir as f64, lead_km);
                    painter.line_segment([apex, to_screen(tip[0], tip[1])],
                        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(col[0], col[1], col[2], 160)));
                    // ETA to each watched marker inside this cone.
                    for m in &self.settings.markers {
                        if let Some(min) = crate::geo::arrival_eta_min([c.lon, c.lat], dir, kt, [m.lon, m.lat], HALF_ANGLE, LEAD_MIN) {
                            etas.push((min, format!("{} — {} in {:.0} min", m.name, c.id, min)));
                        }
                    }
                }
                if idx == self.active && !etas.is_empty() {
                    etas.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    let font = egui::FontId::proportional(12.0);
                    let mut y = prect.top() + 40.0;
                    for (_, line) in etas.iter().take(6) {
                        let text = format!("⏱ {line}");
                        let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
                        let anchor = egui::pos2(prect.left() + 8.0, y);
                        let bg = egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                        painter.rect_filled(bg, 3.0, egui::Color32::from_rgba_unmultiplied(150, 30, 30, 210));
                        painter.text(anchor + egui::vec2(5.0, 2.0), egui::Align2::LEFT_TOP, &text, font.clone(), egui::Color32::WHITE);
                        y += galley.size().y + 6.0;
                    }
                }
            }

            // Optical-flow nowcast: advected echo ghost + a lead-time banner.
            if !nowcast_pts.is_empty() {
                for (lon, lat, col) in &nowcast_pts {
                    let p = to_screen(*lon, *lat);
                    if prect.contains(p) {
                        painter.circle_filled(p, 2.5, *col);
                    }
                }
                if idx == self.active {
                    let text = format!("◈ NOWCAST +{} min — echo extrapolated from storm motion", self.filters.nowcast_lead_min);
                    let font = egui::FontId::proportional(12.0);
                    let anchor = egui::pos2(prect.left() + 8.0, prect.top() + 20.0);
                    let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
                    let bg = egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                    painter.rect_filled(bg, 3.0, egui::Color32::from_rgba_unmultiplied(60, 60, 150, 200));
                    painter.text(anchor + egui::vec2(5.0, 2.0), egui::Align2::LEFT_TOP, &text, font, egui::Color32::WHITE);
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
                    vec![p + egui::vec2(-s, -s), p + egui::vec2(s, -s), p + egui::vec2(0.0, s)],
                    egui::Color32::from_rgba_unmultiplied(240, 40, 210, 60),
                    egui::Stroke::new(2.0, m),
                ));
                painter.text(p + egui::vec2(0.0, -s - 2.0), egui::Align2::CENTER_BOTTOM,
                    format!("TDS ρ{:.2}", h.min_cc), egui::FontId::proportional(11.0), m);
            }

            let label_tracks = self.filters.show_tracks && cam.zoom >= 7.0;
            for c in &self.storm_cells {
                let p = to_screen(c.lon, c.lat);
                // Past track (packet 23): faint gray polyline leading up to the current position.
                if self.filters.show_tracks && c.past_track.len() >= 2 {
                    let gray = egui::Color32::from_gray(150).gamma_multiply(0.7);
                    let pts: Vec<egui::Pos2> = c.past_track.iter().map(|&(lon, lat)| to_screen(lon, lat)).collect();
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
                                painter.text(lp + off, egui::Align2::LEFT_CENTER, &txt,
                                    egui::FontId::proportional(10.0), egui::Color32::from_black_alpha(180));
                            }
                            painter.text(lp, egui::Align2::LEFT_CENTER, &txt,
                                egui::FontId::proportional(10.0), egui::Color32::from_rgb(255, 90, 90));
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
                if c.kind == CellKind::Storm && !c.id.is_empty() {
                    painter.text(p + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM,
                        &c.id, egui::FontId::proportional(11.0), color);
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
                    vec![p + egui::vec2(0.0, -d), p + egui::vec2(d, 0.0), p + egui::vec2(0.0, d), p + egui::vec2(-d, 0.0)],
                    color,
                    egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
                ));
            }
        }

        // Spotter Network positions, filtered to within Level-II range of this pane's site.
        if self.show_spotters {
            if let Some(site_pos) = self.views[idx]
                .site
                .as_deref()
                .and_then(wxdata::sites::site_by_id)
                .map(|s| [s.longitude as f64, s.latitude as f64])
            {
                let now = Utc::now();
                let show_labels = cam.zoom >= 9.0;
                for sp in &self.spotters {
                    // ponytail: ~1400 haversines/frame is microseconds; cache per site if it profiles.
                    if crate::geo::great_circle(site_pos, [sp.lon, sp.lat]).0 > 230.0 {
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
                        if stale { g.gamma_multiply(0.35) } else { g }
                    };
                    painter.circle_filled(p, 3.0, color);
                    painter.circle_stroke(p, 3.0, egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)));
                    // Movement arrow tick, heading clockwise from north.
                    if let Some(h) = sp.heading {
                        let r = h.to_radians();
                        let dir = egui::vec2(r.sin(), -r.cos());
                        painter.line_segment([p, p + dir * 8.0], egui::Stroke::new(1.5, color));
                    }
                    if show_labels {
                        painter.text(p + egui::vec2(5.0, -5.0), egui::Align2::LEFT_BOTTOM,
                            &sp.name, egui::FontId::proportional(10.0), color);
                    }
                    let hit = egui::Rect::from_center_size(p, egui::vec2(14.0, 14.0));
                    if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                        let hover = format!("{}\n{}\n{}", sp.name, sp.time.format("%Y-%m-%d %H:%M UTC"), sp.status);
                        response.clone().show_tooltip_text(hover);
                    }
                }
            }
        }

        // ProbSevere per-storm probability badges (polygons draw via the overlay pipeline).
        if self.show_probsevere {
            for f in &self.probsevere {
                let Some(ring) = f.rings.first() else { continue };
                if ring.is_empty() {
                    continue;
                }
                let (mut clon, mut clat) = (0.0, 0.0);
                for p in ring {
                    clon += p[0];
                    clat += p[1];
                }
                let cw = crate::render::mercator::lonlat_to_world(clon / ring.len() as f64, clat / ring.len() as f64);
                let (sx, sy) = cam.world_to_screen(cw, vp);
                let c = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(c) {
                    continue;
                }
                let color = egui::Color32::from_rgb(f.stroke[0], f.stroke[1], f.stroke[2]);
                let font = egui::FontId::proportional(11.0);
                let galley = painter.layout_no_wrap(f.title.clone(), font.clone(), egui::Color32::BLACK);
                let rect = egui::Rect::from_center_size(c, galley.size() + egui::vec2(8.0, 4.0));
                painter.rect_filled(rect, 3.0, color);
                painter.text(c, egui::Align2::CENTER_CENTER, &f.title, font, egui::Color32::BLACK);
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
            let mut etas: Vec<(f64, String)> = Vec::new();
            let time = ctx.input(|i| i.time);
            // Viewport-center lon/lat: a polygon with every vertex off-screen can still fill the
            // whole pane (zoomed inside it) — the primary chase case for an escalated warning.
            let (center_lon, center_lat) = {
                let w = cam.screen_to_world((vp.0 * 0.5, vp.1 * 0.5), vp);
                crate::render::mercator::world_to_lonlat(w.0, w.1)
            };
            for f in self.active_alert_features() {
                let Some(a) = &f.alert else { continue };
                // Pulsing outline for escalated warnings only — watches can carry PDS wording,
                // but pulsing a state-sized watch polygon would drown the map (and `escalation`
                // uppercases the whole bulletin, too heavy to run for every alert every frame).
                if f.kind == overlay::FeatureKind::Warning && wxdata::alerts::escalation(a) >= 2 {
                    let visible = f.rings.first().is_some_and(|r| {
                        r.iter().any(|p| prect.contains(to_screen(p[0], p[1])))
                    }) || f.contains(center_lon, center_lat);
                    if visible {
                        any_escalated = true;
                        let w = 2.0 + 2.0 * (time * 4.0).sin().abs() as f32;
                        let col = egui::Color32::from_rgb(255, 40, 40);
                        for ring in &f.rings {
                            let pts: Vec<egui::Pos2> = ring.iter().map(|p| to_screen(p[0], p[1])).collect();
                            if pts.len() >= 2 {
                                painter.add(egui::Shape::line(pts, egui::Stroke::new(w, col)));
                            }
                        }
                    }
                }
                // Motion vector + projected path (heading = FROM + 180).
                let Some(m) = &a.motion else { continue };
                let Some(&origin) = m.points.first() else { continue };
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
                        painter.text(p + egui::vec2(5.0, -2.0), egui::Align2::LEFT_CENTER,
                            format!("+{min:.0}m"), egui::FontId::proportional(10.0), col);
                    }
                    prev = p;
                }
                // ETA to any watched marker along the storm's heading.
                for mk in &self.settings.markers {
                    if let Some(t) = crate::geo::arrival_eta_min(origin, heading as f32, m.kt, [mk.lon, mk.lat], 22.5, 90.0) {
                        etas.push((t, format!("⚠ {} — {} in {:.0} min", mk.name, a.event, t)));
                    }
                }
            }
            if idx == self.active && !etas.is_empty() {
                etas.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let font = egui::FontId::proportional(12.0);
                let mut y = prect.top() + 64.0;
                for (_, line) in etas.iter().take(6) {
                    let galley = painter.layout_no_wrap(line.clone(), font.clone(), egui::Color32::WHITE);
                    let anchor = egui::pos2(prect.left() + 8.0, y);
                    let bg = egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                    painter.rect_filled(bg, 3.0, egui::Color32::from_rgba_unmultiplied(150, 30, 30, 210));
                    painter.text(anchor + egui::vec2(5.0, 2.0), egui::Align2::LEFT_TOP, line, font.clone(), egui::Color32::WHITE);
                    y += galley.size().y + 6.0;
                }
            }
            if any_escalated {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
        }

        // Surface obs (METAR station plots): fltCat-colored circle, wind barb, T/Td in °F.
        if self.show_metar && cam.zoom >= 6.0 {
            let show_labels = cam.zoom >= 7.0;
            let flt_color = |c: &str| match c {
                "VFR" => egui::Color32::from_rgb(60, 200, 90),
                "MVFR" => egui::Color32::from_rgb(80, 150, 240),
                "IFR" => egui::Color32::from_rgb(230, 60, 60),
                "LIFR" => egui::Color32::from_rgb(220, 60, 200),
                _ => egui::Color32::from_gray(180),
            };
            // Windiest-first so the strongest stations survive decluttering.
            let mut obs: Vec<&wxdata::metar::SurfaceOb> = self.metars.iter().collect();
            obs.sort_by(|a, b| b.wspd_kt.partial_cmp(&a.wspd_kt).unwrap_or(std::cmp::Ordering::Equal));
            let mut placed: Vec<egui::Rect> = Vec::new();
            for ob in obs {
                let w = crate::render::mercator::lonlat_to_world(ob.lon, ob.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                // Greedy declutter: skip stations whose plot cell overlaps one already drawn.
                let cell = egui::Rect::from_center_size(p, egui::vec2(44.0, 34.0));
                if placed.iter().any(|r| r.intersects(cell)) {
                    continue;
                }
                placed.push(cell);
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
                        painter.text(p + egui::vec2(-6.0, -6.0), egui::Align2::RIGHT_BOTTOM,
                            format!("{:.0}", t * 9.0 / 5.0 + 32.0), f.clone(), egui::Color32::from_rgb(240, 90, 90));
                    }
                    if let Some(d) = ob.dewp_c {
                        painter.text(p + egui::vec2(-6.0, 6.0), egui::Align2::RIGHT_TOP,
                            format!("{:.0}", d * 9.0 / 5.0 + 32.0), f, egui::Color32::from_rgb(90, 220, 120));
                    }
                }
                // Hover → the raw METAR text.
                let hit = egui::Rect::from_center_size(p, egui::vec2(16.0, 16.0));
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) && !ob.raw.is_empty() {
                    response.clone().show_tooltip_text(&ob.raw);
                }
            }
        }
        // ponytail: °F hardcoded (US station-plot convention); wire to the Units setting if asked.

        // NHC tropical suite: forecast track polyline + category-colored points + storm name.
        if self.show_tropical {
            if let Some(t) = &self.tropical {
                let to_screen = |lon: f64, lat: f64| {
                    let w = crate::render::mercator::lonlat_to_world(lon, lat);
                    let (sx, sy) = cam.world_to_screen(w, vp);
                    egui::pos2(prect.left() + sx, prect.top() + sy)
                };
                for storm in &t.storms {
                    // Forecast track: white polyline through the points.
                    if storm.points.len() >= 2 {
                        let pts: Vec<egui::Pos2> = storm.points.iter().map(|p| to_screen(p.lon, p.lat)).collect();
                        painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, egui::Color32::from_rgb(235, 235, 235))));
                    }
                    for p in &storm.points {
                        let sp = to_screen(p.lon, p.lat);
                        if !prect.contains(sp) {
                            continue;
                        }
                        let (cat, rgb) = wxdata::tropical::saffir_simpson(p.kt);
                        let col = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        painter.circle_filled(sp, 4.0, col);
                        if cam.zoom >= 5.0 {
                            painter.text(sp + egui::vec2(6.0, -2.0), egui::Align2::LEFT_CENTER,
                                cat, egui::FontId::proportional(10.0), col);
                        }
                    }
                    // Current position: bold storm name with a dark halo.
                    let cp = to_screen(storm.lon, storm.lat);
                    if prect.contains(cp) {
                        let (_, rgb) = wxdata::tropical::saffir_simpson(storm.intensity_kt);
                        let col = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        painter.circle_filled(cp, 5.0, col);
                        painter.circle_stroke(cp, 5.0, egui::Stroke::new(1.5, egui::Color32::BLACK));
                        let font = egui::FontId::proportional(13.0);
                        for off in [egui::vec2(1.0, 1.0), egui::vec2(-1.0, -1.0), egui::vec2(1.0, -1.0), egui::vec2(-1.0, 1.0)] {
                            painter.text(cp + egui::vec2(8.0, -8.0) + off, egui::Align2::LEFT_BOTTOM,
                                &storm.name, font.clone(), egui::Color32::BLACK);
                        }
                        painter.text(cp + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM,
                            &storm.name, font, egui::Color32::WHITE);
                    }
                }
            }
        }

        // HRRR "future radar" banner — unmistakable that this is model forecast, not observation.
        if idx == self.active && self.fields.get(&crate::render::FieldLayer::Hrrr).is_some_and(|s| s.show) {
            let valid = self
                .hrrr_valid
                .map(|v| v.format("%a %H:%MZ").to_string())
                .unwrap_or_else(|| "loading…".to_string());
            let text = format!("⚠ FORECAST +{}h — HRRR MODEL, NOT OBSERVED — valid {}", self.hrrr_fcst_hour, valid);
            let font = egui::FontId::proportional(13.0);
            let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::BLACK);
            let pad = egui::vec2(10.0, 4.0);
            let center = egui::pos2(prect.center().x, prect.top() + 16.0);
            let rect = egui::Rect::from_center_size(center, galley.size() + pad * 2.0);
            painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(255, 170, 60));
            painter.text(center, egui::Align2::CENTER_CENTER, &text, font, egui::Color32::BLACK);
        }

        // Placefile labels/icons.
        for (color, [lon, lat], text, hover, is_icon) in placefile_labels {
            let w = crate::render::mercator::lonlat_to_world(*lon, *lat);
            let (sx, sy) = cam.world_to_screen(w, vp);
            let p = egui::pos2(prect.left() + sx, prect.top() + sy);
            if !prect.contains(p) {
                continue;
            }
            if *is_icon {
                painter.circle_stroke(p, 5.0, egui::Stroke::new(1.5, *color));
                painter.circle_filled(p, 1.5, *color);
            } else {
                painter.text(p, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.0), *color);
            }
            if !hover.is_empty() {
                let hit = egui::Rect::from_center_size(p, egui::vec2(16.0, 16.0));
                if response.hover_pos().is_some_and(|hp| hit.contains(hp)) {
                    response.clone().show_tooltip_text(hover);
                }
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
                        painter.text(to_screen(top[0], top[1]), egui::Align2::CENTER_BOTTOM,
                            format!("{km:.0} km"), egui::FontId::proportional(10.0), col);
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

        // Radar sites: a ring per NEXRAD site; the active site in accent, others muted. IDs only
        // when zoomed in so the CONUS view isn't cluttered. Click handled in the Interrogate tool.
        if self.show_radar_sites {
            let accent = crate::theme::accent(self.settings.theme);
            let current = self.views[idx].site.as_deref();
            let show_labels = cam.zoom >= 5.0;
            for s in wxdata::sites::sites() {
                let w = crate::render::mercator::lonlat_to_world(s.longitude as f64, s.latitude as f64);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                let is_current = current == Some(s.id);
                let col = if is_current { accent } else { egui::Color32::from_rgb(120, 190, 255) };
                let r = if is_current { 5.0 } else { 3.5 };
                painter.circle_stroke(p, r, egui::Stroke::new(1.5, col));
                painter.circle_filled(p, 1.5, col);
                if show_labels {
                    painter.text(p + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, s.id,
                        egui::FontId::monospace(10.0), col);
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
            // Uploaded icon if one is loaded; otherwise the default accent dot.
            let tex = m.icon.as_ref().and_then(|n| self.marker_icon_tex.get(n)).and_then(|t| t.as_ref());
            let label_dx = if let Some(tex) = tex {
                let r = egui::Rect::from_center_size(p, egui::vec2(24.0, 24.0));
                painter.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                14.0
            } else {
                painter.circle_filled(p, 4.0, col);
                painter.circle_stroke(p, 4.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                7.0
            };
            painter.text(p + egui::vec2(label_dx, 0.0), egui::Align2::LEFT_CENTER, &m.name, egui::FontId::proportional(12.0), col);
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
                let txt = format!("{:.1} nmi / {:.1} km  @ {:.0}°", crate::geo::km_to_nmi(km), km, brg);
                let mid = a + (b - a) * 0.5;
                painter.text(mid + egui::vec2(0.0, -10.0), egui::Align2::CENTER_BOTTOM, txt, egui::FontId::proportional(12.0), col);
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
                painter.text(a, egui::Align2::RIGHT_BOTTOM, "A", egui::FontId::proportional(12.0), col);
                painter.text(b, egui::Align2::LEFT_BOTTOM, "B", egui::FontId::proportional(12.0), col);
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

        if view.show_legend && view.volume.is_some() {
            let table = self.palettes.table(view.moment);
            let (df, dl) = display_units(view.moment, &self.settings);
            // On Android the floating top bar sits over the map's top edge; drop the legend below
            // it so the two don't overlap.
            let lrect = if cfg!(target_os = "android") {
                let mut r = prect;
                r.min.y += 70.0;
                r
            } else {
                prect
            };
            ui::legend::draw(&painter, lrect, view.moment, table, view.active_threshold(), df, dl);
        }
    }

    /// Resize the pane grid to `n` (1/2/4). New panes copy the active pane's site/camera but
    /// default to a distinct product, so a 4-panel shows REF/VEL/ZDR/RHO out of the box.
    fn set_pane_count(&mut self, n: usize) {
        let n = n.clamp(1, 4);
        while self.views.len() < n {
            let src = &self.views[self.active];
            let (site, camera, basemap, tilt, date) =
                (src.site.clone(), src.camera, src.basemap, src.tilt, src.timeline.date);
            let mut v = MapView::new(site, camera);
            v.basemap = basemap;
            v.tilt = tilt;
            v.timeline.date = date;
            v.moment = Moment::ALL[self.views.len() % Moment::ALL.len()];
            self.views.push(v);
        }
        self.views.truncate(n);
        if self.active >= n {
            self.active = n - 1;
        }
        self.pane_shown.clear();
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            // Touch affordance for the toolbox drawer (F7 has no key on a phone).
            if cfg!(target_os = "android") && ui.button("☰").clicked() {
                self.show_toolbox = !self.show_toolbox;
            }
            ui.menu_button("File", |ui| {
                if ui.button("Settings…").clicked() {
                    self.settings_window.open = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Export Settings…")
                    .on_hover_text("Save settings + color tables to a portable bundle")
                    .clicked()
                {
                    self.export_settings_bundle();
                    ui.close();
                }
                if ui.button("Import Settings…")
                    .on_hover_text("Load a settings bundle from another machine")
                    .clicked()
                {
                    self.import_settings_bundle();
                    ui.close();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                let v = &mut self.views[self.active];
                let mut on = v.basemap != crate::tiles::BasemapStyle::None;
                if ui.checkbox(&mut on, "Basemap").changed() {
                    v.basemap = if on { crate::tiles::BasemapStyle::Dark } else { crate::tiles::BasemapStyle::None };
                }
                ui.checkbox(&mut v.show_radar, "Radar");
                ui.checkbox(&mut v.show_legend, "Legend");
                ui.checkbox(&mut self.show_toolbox, "Toolbox (F7)")
                    .on_hover_text("Collapse the left panel for a full-width radar view");
                ui.separator();
                if ui.checkbox(&mut self.obs_mode, "Streamer / OBS mode (F8)")
                    .on_hover_text("Hide all panels, leaving only the map — clean capture for streaming")
                    .changed() && !self.obs_mode
                {
                    self.obs_tour = false;
                }
                if ui.checkbox(&mut self.obs_tour, "Auto-tour active warnings (F9)")
                    .on_hover_text("Cycle the camera through active warning polygons every ~12 s")
                    .changed()
                {
                    self.obs_tour_last = None;
                    if self.obs_tour {
                        self.obs_mode = true;
                    }
                }
            });
            ui.menu_button("Tools", |ui| {
                ui.label("Map tool");
                ui.selectable_value(&mut self.tool, MapTool::Interrogate, "Interrogate");
                ui.selectable_value(&mut self.tool, MapTool::Measure, "Measure");
                ui.selectable_value(&mut self.tool, MapTool::Marker, "Drop marker");
                ui.selectable_value(&mut self.tool, MapTool::CrossSection, "Cross-section")
                    .on_hover_text("Click two points to slice a vertical reflectivity panel");
                ui.selectable_value(&mut self.tool, MapTool::Sounding, "Sounding")
                    .on_hover_text("Click a point for an HRRR Skew-T / hodograph");
                ui.selectable_value(&mut self.tool, MapTool::Chase, "Set chase location")
                    .on_hover_text("Click your position; chase mode follows it to the nearest radar");
                ui.selectable_value(&mut self.tool, MapTool::Climatology, "Tornado climatology")
                    .on_hover_text("Click a point to list historical tornadoes within 25 mi (SPC 1950–2022)");
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.chase_mode, "Chase mode (follow me)").changed() && !self.chase_mode {
                        self.chase_applied = None;
                    }
                    if self.chase_mode {
                        if let Some((lon, lat)) = self.chase_pos {
                            if let Some(s) = crate::geo::nearest_site_id(lon, lat) {
                                ui.weak(format!("→ {s}"));
                            }
                        } else {
                            ui.weak("pick a location");
                        }
                    }
                });
                // gpsd is a desktop daemon; Android has no local gpsd (native location is a v2 JNI
                // job), so this connect button only appears off-Android.
                if !cfg!(target_os = "android") && self.gps_rx.is_none() {
                    if ui.button("Connect GPS (gpsd)")
                        .on_hover_text("Stream your live position from a local gpsd on :2947")
                        .clicked()
                    {
                        match crate::gps::spawn() {
                            Some(rx) => {
                                self.gps_rx = Some(rx);
                                self.chase_mode = true;
                            }
                            None => log::warn!("gpsd not reachable on 127.0.0.1:2947"),
                        }
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.weak("📡 GPS connected");
                        if ui.button("Disconnect").clicked() {
                            self.gps_rx = None;
                        }
                    });
                }
                ui.separator();
                if ui.button("3D View").on_hover_text("Raymarch the volume in 3D (active pane)").clicked() {
                    self.build_volume3d();
                    ui.close();
                }
                if ui.button("CAPPI slice…").on_hover_text("Constant-altitude reflectivity slice (active pane)").clicked() {
                    self.show_cappi = true;
                    self.cappi_key = None; // force a re-slice on open
                    ui.close();
                }
                if ui.button("Clear measurement").clicked() {
                    self.measure.clear();
                }
                ui.separator();
                if ui.button("Location Markers…").clicked() {
                    self.marker_window.open = true;
                    ui.close();
                }
                if ui.button("Event Library…")
                    .on_hover_text("Jump to famous storms or your saved bookmarks")
                    .clicked()
                {
                    self.event_window.open = true;
                    ui.close();
                }
                if ui.button("Storm Digest…")
                    .on_hover_text("Plain-language briefing of the in-view weather")
                    .clicked()
                {
                    self.digest_window.open = true;
                    self.generate_digest();
                    ui.close();
                }
                if ui.button("Forecast Discussion (AFD)…")
                    .on_hover_text("The active site's WFO Area Forecast Discussion — the forecaster's reasoning")
                    .clicked()
                {
                    self.afd_open = true;
                    self.fetch_afd();
                    ui.close();
                }
                if ui.button("Placefile Manager…").clicked() {
                    self.placefile_window.open = true;
                    ui.close();
                }
                if ui.button("Color-Table Editor…")
                    .on_hover_text("Edit radar palettes with live preview; import/export .pal")
                    .clicked()
                {
                    self.palette_editor.open = true;
                    ui.close();
                }
                ui.add_enabled(false, egui::Button::new("Layer Manager (U7)"));
                ui.separator();
                if ui.button("Save Screenshot…").clicked() {
                    if let Some(path) = crate::dialog::save_path("hookecho.png", "png") {
                        self.screenshot_pending = Some(ShotDest::File(path));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(
                            egui::UserData::default(),
                        ));
                    }
                    ui.close();
                }
                if ui.button("Copy View").clicked() {
                    self.screenshot_pending = Some(ShotDest::Clipboard);
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                    ui.close();
                }
                if ui.add_enabled(self.loop_export.is_none(), egui::Button::new("Export Loop (GIF)…"))
                    .on_hover_text("Capture the archive timeline as a looping animation")
                    .clicked()
                {
                    self.start_loop_export(crate::loopexport::LoopFormat::Gif);
                    ui.close();
                }
                // MP4 export shells out to the `ffmpeg` CLI, which isn't present on Android; GIF
                // export (pure Rust) stays. Hide the MP4 item there rather than fail on click.
                if !cfg!(target_os = "android")
                    && ui.add_enabled(self.loop_export.is_none(), egui::Button::new("Export Loop (MP4)…"))
                        .on_hover_text("Capture the archive timeline as an MP4 (requires ffmpeg)")
                        .clicked()
                {
                    self.start_loop_export(crate::loopexport::LoopFormat::Mp4);
                    ui.close();
                }
            });
            ui.menu_button("Panes", |ui| {
                for count in [1usize, 2, 4] {
                    let label = if count == 1 { "1 pane".to_string() } else { format!("{count} panes") };
                    if ui.selectable_label(self.views.len() == count, label).clicked() {
                        self.set_pane_count(count);
                        ui.close();
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.link_cameras, "Link cameras");
            });
            ui.menu_button("Help", |ui| {
                ui.label("Hook Echo-WX — NEXRAD radar viewer");
                ui.label("github.com/d4vid87/hookecho");
                ui.separator();
                if ui.button("Setup wizard…").clicked() {
                    self.wizard.start();
                    ui.close();
                }
            });
        });
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
        v.site = Some(site.to_ascii_uppercase());
        v.camera = crate::render::mercator::Camera { center: lonlat_to_world(lon, lat), zoom };
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
    pub(crate) fn add_bookmark(&mut self, name: String) {
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
        });
    }

    /// Export settings + referenced color tables to a portable JSON bundle (rfd save dialog).
    fn export_settings_bundle(&mut self) {
        let Some(path) = crate::dialog::save_path("hookecho-settings.json", "json") else {
            return;
        };
        match self.settings.export_bundle() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("settings export failed: {e}");
                }
            }
            Err(e) => log::warn!("settings export failed: {e}"),
        }
    }

    /// Import a settings bundle (rfd open dialog). The next-frame dirty-diff reloads palettes
    /// and persists, and the UI (theme, layers, markers…) updates live from the new settings.
    fn import_settings_bundle(&mut self) {
        let Some(path) = crate::dialog::open_path("JSON", &["json"]) else {
            return;
        };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| crate::settings::Settings::import_bundle(&s)) {
            Ok(settings) => self.settings = settings,
            Err(e) => log::warn!("settings import failed: {e}"),
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
            return;
        }
        v.timeline.go_begin();
        self.loop_export = Some(LoopExport {
            dest: path,
            format,
            frames: Vec::with_capacity(slots),
            remaining: slots,
            settle: LOOP_SETTLE_FRAMES,
            capturing: false,
        });
    }

    /// Advance the loop export: wait for the stepped radar to settle, then request a screenshot.
    fn drive_loop_export(&mut self, ctx: &egui::Context) {
        let Some(le) = &mut self.loop_export else { return };
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
        let Some(le) = &mut self.loop_export else { return };
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
                LoopFormat::Gif => crate::loopexport::encode_gif(&le.frames, 200, &le.dest),
                LoopFormat::Mp4 => crate::loopexport::encode_mp4(&le.frames, 5, &le.dest),
            };
            match res {
                Ok(()) => log::info!("loop saved: {} ({} frames)", le.dest.display(), le.frames.len()),
                Err(e) => log::warn!("loop encode failed: {e}"),
            }
        }
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
                        Ok(()) => log::info!("screenshot saved: {}", path.display()),
                        Err(e) => log::warn!("screenshot save failed: {e}"),
                    }
                }
                ShotDest::Clipboard => {
                    // `image` here is already an egui ColorImage; hand it straight to the clipboard.
                    ctx.copy_image((*image).clone());
                    log::info!("view copied to clipboard");
                }
                ShotDest::Loop => self.record_loop_frame(&image),
            }
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let v = &self.views[self.active];
        ui.horizontal(|ui| {
            ui.strong(v.site.as_deref().unwrap_or("no site"));
            ui.separator();
            if let Some(vol) = &v.volume {
                let age = (Utc::now() - vol.time).num_seconds().max(0);
                ui.label(format!("{} ({} ago)", vol.time.format("%H:%M:%SZ"), humanize(age)));
            } else if v.loading {
                ui.label("loading…");
            }
            if let Some(e) = &v.error {
                ui.colored_label(egui::Color32::from_rgb(230, 100, 100), e);
            }
            if self.tool != MapTool::Interrogate {
                ui.separator();
                let name = match self.tool {
                    MapTool::Measure => "Measure: click 2 points",
                    MapTool::Marker => "Drop marker: click map",
                    MapTool::CrossSection => "Cross-section: click 2 points",
                    MapTool::Sounding => "Sounding: click a point",
                    MapTool::Chase => "Chase: click your location",
                    MapTool::Climatology => "Climatology: click a point",
                    MapTool::Interrogate => "",
                };
                ui.colored_label(crate::theme::accent(self.settings.theme), name);
            }
            // Right-aligned segment: zoom, then cursor lat/lon, then DVR buffer depth.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("z{:.1}", v.camera.zoom));
                if let Some((lon, lat)) = self.cursor_ll {
                    ui.separator();
                    ui.monospace(format!("{lat:.3}, {lon:.3}"));
                }
                let depth = self.dvr_depth();
                if depth > 1 {
                    ui.separator();
                    ui.weak(format!("⟲ DVR {depth}")).on_hover_text("Frames buffered in memory for instant replay (press R)");
                }
            });
        });
    }
}

/// Convert a binned sweep into a GPU upload with its world-space bounding box.
///
/// `threshold` (physical units) is baked into the color LUT; `None` shows all values.
/// `smooth` enables bilinear sampling in the shader. `table` selects the colormap.
/// `storm_uv` is the storm motion (east, north) in m/s for storm-relative velocity, or
/// `None` for ground-relative.
pub(crate) fn to_upload(
    s: &BinnedSweep,
    table: &ColorTable,
    threshold: Option<f32>,
    smooth: bool,
    storm_uv: Option<(f32, f32)>,
) -> RadarUpload {
    use crate::render::mercator::lonlat_to_world;
    let max_range_km = s.first_gate_km + s.gate_count as f32 * s.gate_interval_km;
    let dlat = (max_range_km / 111.32) as f64;
    let coslat = (s.radar_lat as f64 * std::f64::consts::PI / 180.0).cos().max(0.01);
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
    RadarUpload {
        az_bins: s.az_bins as u32,
        gate_count: s.gate_count as u32,
        data: s.data.clone(),
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
            0.0,
            0.0,
        ],
        lut: crate::colormap::bake_lut(table, (s.value_min, s.value_max), threshold).to_vec(),
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
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
            0.0,
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
fn field_index_upload(
    f: &wxdata::mrms::MrmsField,
    map: impl Fn(f32) -> u8,
    lut: Vec<u8>,
) -> crate::render::MrmsUpload {
    use crate::render::mercator::lonlat_to_world;
    let data: Vec<u8> = f.values.iter().map(|&v| if v.is_nan() { 0 } else { map(v) }).collect();
    let (wx0, wy0) = lonlat_to_world(f.lon_west, f.lat_north);
    let (wx1, wy1) = lonlat_to_world(f.lon_east, f.lat_south);
    crate::render::MrmsUpload {
        data,
        nx: f.nx as u32,
        ny: f.ny as u32,
        world_min: [wx0 as f32, wy0 as f32],
        world_max: [wx1 as f32, wy1 as f32],
        uniform: [
            f.lon_west as f32, f.lat_north as f32, f.lon_east as f32, f.lat_south as f32,
            f.nx as f32, f.ny as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
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
    let map = |v: f32| if v <= 0.0 { 0 } else { (2.0 + ((v.log10() + 1.7) / 2.0).clamp(0.0, 1.0) * 253.0) as u8 };
    field_index_upload(f, map, ramp_lut(&[(0.0, [255, 255, 255]), (0.35, [255, 240, 120]), (0.65, [255, 160, 40]), (1.0, [230, 60, 200])]))
}

/// Build the GPU upload for an index-mapped field layer (everything except the reflectivity
/// mosaic, which needs the app's color table). Kept public for the headless harness.
pub(crate) fn field_upload_indexed(layer: crate::render::FieldLayer, f: &wxdata::mrms::MrmsField) -> crate::render::MrmsUpload {
    use crate::render::FieldLayer as FL;
    match layer {
        FL::Lightning => lightning_upload(f),
        // Azimuthal shear (MRMS units ~0..45, ×10⁻³ s⁻¹), accumulated (rotation track) or
        // instantaneous: threshold ~4, saturate ~40 → blue→cyan→yellow→red.
        FL::Rotation | FL::AzShear => {
            let map = |v: f32| {
                let a = v.abs();
                if a < 4.0 { 0 } else { (2.0 + ((a - 4.0) / 36.0).clamp(0.0, 1.0) * 253.0) as u8 }
            };
            field_index_upload(f, map, ramp_lut(&[(0.0, [40, 90, 200]), (0.4, [40, 200, 200]), (0.7, [240, 230, 60]), (1.0, [230, 40, 40])]))
        }
        // MESH max hail size (mm): 10..75 mm → green→yellow→orange→magenta.
        FL::Mesh => {
            let map = |v: f32| if v < 10.0 { 0 } else { (2.0 + ((v - 10.0) / 65.0).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut(&[(0.0, [60, 200, 90]), (0.4, [240, 230, 60]), (0.7, [240, 150, 30]), (1.0, [230, 60, 200])]))
        }
        // QPE accumulation (mm): 0.25..100 mm on a log ramp, green→yellow→red→magenta→white.
        FL::Qpe1h | FL::Qpe24h => {
            let full: f32 = if layer == FL::Qpe24h { 250.0 } else { 100.0 };
            let map = move |v: f32| {
                if v < 0.25 { 0 } else {
                    let t = ((v.log10() - (0.25f32).log10()) / (full.log10() - (0.25f32).log10())).clamp(0.0, 1.0);
                    (2.0 + t * 253.0) as u8
                }
            };
            field_index_upload(f, map, ramp_lut(&[
                (0.0, [40, 180, 90]), (0.3, [230, 220, 60]), (0.55, [230, 110, 40]),
                (0.8, [220, 40, 60]), (1.0, [230, 220, 240]),
            ]))
        }
        // Surface CAPE (J/kg): 100..5000 → cyan→green→yellow→orange→magenta, translucent.
        FL::Cape => {
            let map = |v: f32| if v < 100.0 { 0 } else { (2.0 + ((v - 100.0) / 4900.0).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut_a(&[
                (0.0, [0, 200, 200]), (0.25, [40, 200, 90]), (0.5, [240, 230, 60]),
                (0.75, [240, 150, 30]), (1.0, [230, 60, 200]),
            ], 150))
        }
        // Storm-relative helicity (m²/s²): 50..500 → blue→yellow→red, translucent.
        FL::Srh => {
            let map = |v: f32| if v < 50.0 { 0 } else { (2.0 + ((v - 50.0) / 450.0).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut_a(&[
                (0.0, [40, 90, 200]), (0.5, [240, 230, 60]), (1.0, [230, 40, 40]),
            ], 150))
        }
        // MRMS surface precip type flag: discrete categories via a categorical LUT.
        FL::PrecipType => {
            let map = |v: f32| v as u8; // categorical index, not a ramp
            field_index_upload(f, map, categorical_lut(&[
                (1, [60, 200, 90]),   // warm stratiform rain — green
                (3, [90, 150, 240]),  // snow — blue
                (6, [240, 230, 60]),  // convective — yellow
                (7, [230, 40, 40]),   // hail — red
                (10, [40, 200, 200]), // cold stratiform rain — teal
                (91, [80, 220, 120]), // tropical/stratiform rain — green
                (96, [80, 220, 120]), // tropical/convective rain — green
            ], 200))
        }
        // MRMS FLASH flash-flood ARI (years): 1..100 log ramp yellow→orange→red→purple→white.
        FL::FlashFlood => {
            let map = |v: f32| if v < 1.0 { 0 } else { (2.0 + (v.log10() / 2.0).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut(&[
                (0.0, [240, 230, 60]), (0.3, [240, 150, 30]), (0.6, [230, 40, 40]),
                (0.85, [150, 40, 200]), (1.0, [240, 240, 240]),
            ]))
        }
        // Digital VIL (kg/m²): 0.1..80 → green→yellow→orange→magenta→white.
        FL::Vil => {
            let map = |v: f32| if v < 0.1 { 0 } else { (2.0 + ((v - 0.1) / 79.9).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut(&[
                (0.0, [60, 200, 90]), (0.35, [240, 230, 60]), (0.6, [240, 150, 30]),
                (0.85, [230, 60, 200]), (1.0, [240, 240, 240]),
            ]))
        }
        // Enhanced Echo Tops (kft): 5..70 → blue→green→yellow→white.
        FL::EchoTops => {
            let map = |v: f32| if v < 5.0 { 0 } else { (2.0 + ((v - 5.0) / 65.0).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut(&[
                (0.0, [40, 90, 200]), (0.4, [40, 200, 90]), (0.75, [240, 230, 60]), (1.0, [240, 240, 240]),
            ]))
        }
        // 24-h hail swaths: same MESH scale, but starting at severe-ish sizes so old small hail
        // doesn't blanket the map (19 mm ≈ 0.75 in).
        FL::HailSwath => {
            let map = |v: f32| if v < 19.0 { 0 } else { (2.0 + ((v - 19.0) / 56.0).clamp(0.0, 1.0) * 253.0) as u8 };
            field_index_upload(f, map, ramp_lut(&[
                (0.0, [60, 200, 90]), (0.4, [240, 230, 60]), (0.7, [240, 150, 30]), (1.0, [230, 60, 200]),
            ]))
        }
        // Hybrid Hydrometeor Classification: raw HCA class codes → categorical colors
        // (per the product's class table; 140 = unknown, 150 = range-folded).
        FL::Hca => {
            let map = |v: f32| v as u8;
            field_index_upload(f, map, categorical_lut(&[
                (10, [140, 110, 90]),   // biological
                (20, [95, 95, 95]),     // ground clutter / AP
                (30, [185, 220, 255]),  // ice crystals
                (40, [110, 160, 240]),  // dry snow
                (50, [0, 200, 255]),    // wet snow
                (60, [90, 200, 90]),    // light/moderate rain
                (70, [25, 145, 50]),    // heavy rain
                (80, [240, 200, 60]),   // big drops
                (90, [200, 120, 220]),  // graupel
                (100, [230, 50, 50]),   // hail (possibly with rain)
                (110, [170, 0, 0]),     // large hail
                (120, [120, 0, 60]),    // giant hail
                (140, [160, 160, 160]), // unknown
                (150, [240, 150, 200]), // range folded
            ], 200))
        }
        // The reflectivity-palette layers (mosaic, HRRR) route through the app method instead.
        FL::Mrms | FL::Hrrr => field_index_upload(f, |_| 0, vec![0u8; 256 * 4]),
    }
}

impl HookEchoApp {
    /// Build the GPU upload for `layer` from its freshly-fetched grid, picking the value→index
    /// mapping and color LUT that suit the product's units.
    fn field_upload(&self, layer: crate::render::FieldLayer, f: &wxdata::mrms::MrmsField) -> crate::render::MrmsUpload {
        use crate::render::FieldLayer as FL;
        match layer {
            // Mosaic + HRRR forecast are both dBZ → the reflectivity palette.
            FL::Mrms | FL::Hrrr => mrms_upload(f, self.palettes.table(Moment::Reflectivity)),
            other => field_upload_indexed(other, f),
        }
    }
}

/// Marker color for a storm-cell kind (sRGB).
/// `[r,g,b,a]` -> egui `Color32` (unmultiplied).
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

/// Marker color for an SPC storm-report kind.
fn report_color(kind: wxdata::spc::ReportKind) -> [u8; 4] {
    use wxdata::spc::ReportKind as R;
    match kind {
        R::Tornado => [230, 40, 40, 255],  // red
        R::Wind => [70, 130, 240, 255],    // blue
        R::Hail => [70, 210, 110, 255],    // green
        R::Flood => [0, 150, 90, 255],     // dark green
        R::Other => [180, 180, 180, 255],  // gray
    }
}

/// Display-unit factor and label for a moment: velocity/spectrum-width honor the Units
/// setting (internal data stays m/s), everything else uses its native unit.
pub(crate) fn display_units(moment: Moment, settings: &Settings) -> (f32, &'static str) {
    match moment {
        Moment::Velocity | Moment::SpectrumWidth => {
            (settings.velocity_unit.factor_from_ms(), settings.velocity_unit.label())
        }
        _ => (1.0, moment.units()),
    }
}

/// A coarse "N ago" string for volume age.
fn humanize(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// True if the feature's bounding box overlaps `box = (min_lon, min_lat, max_lon, max_lat)`.
/// Features with no geometry (no bbox) are treated as not overlapping.
fn feature_in_box(f: &GeoFeature, bx: (f64, f64, f64, f64)) -> bool {
    let Some((x0, y0, x1, y1)) = f.bbox() else { return false };
    let (bx0, by0, bx1, by1) = bx;
    x1 >= bx0 && x0 <= bx1 && y1 >= by0 && y0 <= by1
}

impl eframe::App for HookEchoApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Android: feed the status-bar / gesture-bar insets so no UI draws under system chrome.
        crate::platform::apply_safe_area(ctx, raw_input);
        // Android: clipboard text fetched by the paste bar lands as a real egui Paste event, so
        // the focused text field inserts it exactly like Ctrl+V would.
        if let Some(text) = self.pending_paste.take() {
            raw_input.events.push(egui::Event::Paste(text));
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;

        // Android paste: re-focus the text field that lost focus to the Paste-button tap, before
        // any window draws, so the queued Paste event (see `raw_input_hook`) lands in it.
        if let Some(id) = self.paste_target.take() {
            ctx.memory_mut(|m| m.request_focus(id));
        }

        // Tray menu commands (Linux StatusNotifier): restore the window or quit for real.
        if let Some(rx) = &self.tray_rx {
            while let Ok(cmd) = rx.try_recv() {
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
            let cmd = if self.tray_rx.is_some() {
                egui::ViewportCommand::Visible(false) // hide fully; the tray restores it
            } else {
                egui::ViewportCommand::Minimized(true) // no tray → keep a taskbar entry
            };
            ctx.send_viewport_cmd(cmd);
        }

        // "Modern dark pro" styling (palette/spacing/rounding/accent), applied every frame so
        // it survives runtime theme switches.
        let system_dark = ctx.input(|i| i.raw.system_theme) != Some(egui::Theme::Light);
        crate::theme::apply(ctx, self.settings.theme, system_dark);

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

        self.save_pending_screenshot(ctx);
        self.load_marker_icons(ctx);
        self.drive_loop_export(ctx);
        self.apply_chase();
        self.sync_forecast_scrub();
        self.poll_messages();
        self.poll_overlays();
        // Time-machine warnings + storm reports: swap in archived sets while scrubbed.
        self.sync_archive_warnings(ctx);
        self.sync_archive_lsr(ctx);
        // Surface obs (METAR station plots).
        self.sync_metar(ctx);
        // NHC tropical suite: refresh every 15 min while enabled.
        if self.show_tropical
            && self.tropical_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 900)
        {
            self.tropical_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Tropical);
        }
        // Periodic overlay refresh (~2 min), honoring live weather cadence.
        if self.overlay_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 120) {
            self.fetch_overlays(ctx);
        }
        // MRMS national mosaic: fetch when enabled, refresh at the ~2-min product cadence.
        // National field layers: fetch each enabled layer at its product cadence.
        use crate::render::FieldLayer as FL;
        for layer in FL::DRAW_ORDER {
            // HRRR forecast, HRRR environment, and per-site L3 grids each fetch in their own block.
            if matches!(layer, FL::Hrrr | FL::Cape | FL::Srh | FL::Vil | FL::EchoTops | FL::Hca) {
                continue;
            }
            let stale = self
                .fields
                .get(&layer)
                .is_some_and(|s| s.show && s.last_fetch.is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer)));
            if stale {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                let product = match layer {
                    FL::Mrms => wxdata::mrms::REFLECTIVITY.to_string(),
                    FL::Lightning => wxdata::mrms::LIGHTNING.to_string(),
                    FL::Mesh => wxdata::mrms::MESH.to_string(),
                    FL::AzShear => wxdata::mrms::AZSHEAR.to_string(),
                    FL::Rotation => wxdata::mrms::rotation_track(self.rotation_minutes).to_string(),
                    FL::Qpe1h => wxdata::mrms::QPE_01H.to_string(),
                    FL::Qpe24h => wxdata::mrms::QPE_24H.to_string(),
                    FL::PrecipType => wxdata::mrms::PRECIP_TYPE.to_string(),
                    FL::FlashFlood => wxdata::mrms::FLASH_ARI30.to_string(),
                    FL::HailSwath => wxdata::mrms::MESH_1440.to_string(),
                    FL::Hrrr | FL::Cape | FL::Srh | FL::Vil | FL::EchoTops | FL::Hca => unreachable!(),
                };
                self.spawn_overlay(ctx, OverlaySource::Field(layer, product));
            }
        }
        // Environment suite (HRRR CAPE/SRH): fetch each enabled layer at f00, refresh ~15 min.
        for layer in [FL::Cape, FL::Srh] {
            let stale = self
                .fields
                .get(&layer)
                .is_some_and(|s| s.show && s.last_fetch.is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer)));
            if stale {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.spawn_overlay(ctx, OverlaySource::Env(layer, self.env_cape_ml, self.env_srh_km));
            }
        }
        // Gridded L3 products (DVL/EET): per-site, refetch on the L3 cadence or a site change.
        let l3_site = self.views[self.active].site.clone();
        let site_changed = self.l3grid_site != l3_site;
        for layer in [FL::Vil, FL::EchoTops, FL::Hca] {
            let on = self.fields.get(&layer).is_some_and(|s| s.show);
            if !on {
                continue;
            }
            let stale = self
                .fields
                .get(&layer)
                .is_some_and(|s| s.last_fetch.is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer)));
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
                .any(|l| self.fields.get(l).is_some_and(|s| s.show))
        {
            self.l3grid_site = l3_site;
        }
        // HRRR future radar: fetch when enabled and the forecast hour changed or the run refreshed
        // (~10-min throttle; a new run posts hourly).
        let hrrr_on = self.fields.get(&FL::Hrrr).is_some_and(|s| s.show);
        if hrrr_on {
            let hour_changed = self.hrrr_fetched_hour != Some(self.hrrr_fcst_hour);
            let stale = self.hrrr_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 600);
            if hour_changed || stale {
                self.hrrr_fetched_hour = Some(self.hrrr_fcst_hour);
                self.hrrr_last_fetch = Some(Instant::now());
                self.spawn_overlay(ctx, OverlaySource::Hrrr(self.hrrr_fcst_hour));
            }
        }
        // Live LSR refresh (~2-min cadence; the IEM feed is minutes-fresh).
        if self.show_storm_reports
            && self.reports_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 120)
        {
            self.reports_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::StormReports(None));
        }
        // Aviation SIGMET/AIRMET refresh (10-min cadence).
        if self.show_aviation
            && self.aviation_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 600)
        {
            self.aviation_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Aviation);
        }
        // Spotter Network refresh (feed's own 1-min cadence).
        if self.show_spotters
            && self.spotters_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 60)
        {
            self.spotters_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::Spotters);
        }
        // ProbSevere refresh (~2-min product cadence).
        if self.show_probsevere
            && self.probsevere_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 120)
        {
            self.probsevere_last_fetch = Some(Instant::now());
            self.spawn_overlay(ctx, OverlaySource::ProbSevere);
        }
        // Sensors: fetch when the window is open and the site changed or the 10-min clock elapsed.
        if self.show_sensors {
            if let Some(site) = self.views[self.active].site.clone() {
                let stale = self.sensor_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 600);
                let site_changed = self.sensor_site.as_deref() != Some(site.as_str());
                if stale || site_changed {
                    if let Some(s) = wxdata::sites::site_by_id(&site) {
                        if site_changed {
                            self.sensor_data = None; // show "loading" until the new site returns
                        }
                        self.sensor_last_fetch = Some(Instant::now());
                        self.spawn_overlay(ctx, OverlaySource::Obs {
                            site: site.clone(),
                            lat: s.latitude as f64,
                            lon: s.longitude as f64,
                        });
                    }
                }
            }
        }
        // VAD hodograph: fetch when open and the site changed or the 5-min clock elapsed.
        if self.show_hodo {
            if let Some(site) = self.views[self.active].site.clone() {
                let stale = self.hodo_last_fetch.is_none_or(|t| t.elapsed().as_secs() >= 300);
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
        for action in hotkeys::poll(ctx) {
            self.apply_action(action, ctx);
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

        // Chrome: touch-first on Android (top bar + dock + slide-up sheets), desktop otherwise
        // (menu bar + left toolbox). Both funnel into the same `ToolboxActions` handling below.
        let mut actions = ui::toolbox::ToolboxActions::default();
        if cfg!(target_os = "android") {
            if !self.obs_mode {
                actions = self.mobile_chrome(root, ctx);
            }
        } else {
            if !self.obs_mode {
                egui::Panel::top("menu_bar").show(root, |ui| self.menu_bar(ui));
            }
            if !self.obs_mode && self.show_toolbox {
                egui::Panel::left("toolbox")
                    .resizable(true)
                    .default_size(240.0)
                    .show(root, |ui| {
                        let l3_site = self.l3grid_site.clone();
                        actions = ui::toolbox::show(
                            ui,
                            &mut self.views[self.active],
                            &mut self.settings,
                            &mut self.filters,
                            &mut self.fields,
                            &mut self.rotation_minutes,
                            &mut self.hrrr_fcst_hour,
                            self.hrrr_valid,
                            &mut self.env_cape_ml,
                            &mut self.env_srh_km,
                            l3_site.as_deref(),
                            &mut self.show_sensors,
                            &mut self.show_hodo,
                            &mut self.show_alert_panel,
                            &mut self.show_storm_reports,
                            &mut self.show_spotters,
                            &mut self.show_probsevere,
                            &mut self.show_radar_sites,
                            &mut self.show_metar,
                            &mut self.show_tropical,
                            &mut self.show_aviation,
                            &mut self.show_range_rings,
                        );
                    });
            }
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
        if actions.outlook_kind_changed && self.filters.outlook_day == 1 {
            // Hazard switched: drop the stale Day-1 features so the empty-check refetches it.
            self.outlook_features[0].clear();
        }
        if actions.overlays_changed {
            // Selecting an outlook day/kind that hasn't been fetched yet pulls it on demand.
            let day = self.filters.outlook_day;
            if (1..=3).contains(&day) && self.outlook_features[(day - 1) as usize].is_empty() {
                self.spawn_overlay(ctx, OverlaySource::Outlook(day, self.outlook_kind_for_day()));
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

        // First-run setup wizard.
        let active = self.active;
        if let Some(site) = ui::wizard::show(
            ctx,
            &mut self.wizard,
            &mut self.settings,
            &mut self.views[active].basemap,
            &self.marker_icon_tex,
        ) {
            self.settings.setup_done = true;
            self.settings.save();
            let v = &mut self.views[self.active];
            v.site = Some(site.clone());
            ui::site_dialog::center_on_site(&mut v.camera, &site);
        }
        if !self.wizard.open && !self.settings.setup_done {
            // Dismissed without finishing: don't nag every frame, but keep for next launch.
            self.settings.setup_done = true;
            self.settings.save();
        }

        // Floating windows.
        if let Some(dialog) = &mut self.site_dialog {
            let keep = ui::site_dialog::show(ctx, dialog, &mut self.views[self.active], &mut self.settings);
            if !keep {
                self.site_dialog = None;
            }
        }
        self.settings_window.show(ctx, &mut self.settings, &self.palettes);
        let pf_status: Vec<ui::placefile_window::PlacefileStatus> = self
            .placefiles
            .iter()
            .map(|lp| ui::placefile_window::PlacefileStatus {
                url: lp.url.clone(),
                loaded: lp.loaded,
                items: lp.pf.items.len(),
                title: lp.pf.title.clone(),
            })
            .collect();
        self.placefile_window.show(ctx, &mut self.settings, &pf_status);
        self.marker_window.show(ctx, &mut self.settings, &self.marker_icon_tex);
        self.palette_editor.show(ctx, &mut self.settings, &self.palettes);
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
        if let Some(ui::digest_window::DigestAction::Generate) = self.digest_window.show(ctx) {
            self.generate_digest();
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
            );
            if refresh {
                self.fetch_afd();
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
        self.sounding_window.show(ctx);
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
        self.show_climatology_window(ctx);
        // GOES frame times arrived → keep the scrub at latest until the user moves it.
        if let Some(rx) = &self.goes_times_rx {
            if let Ok(times) = rx.try_recv() {
                self.goes_times = times;
                self.goes_times_rx = None;
            }
        }
        self.goes_time_bar(ctx);
        if let Some(act) = self.event_window.show(ctx, &mut self.settings) {
            use ui::event_window::EventAction;
            match act {
                EventAction::Goto { site, lon, lat, zoom, time } => {
                    self.goto_view(&site, lon, lat, zoom, time);
                }
                EventAction::AddBookmark => {
                    let n = self.settings.bookmarks.len() + 1;
                    self.add_bookmark(format!("Bookmark {n}"));
                }
            }
        }

        if let Some(detail) = &self.detail {
            if !ui::detail_window::show(ctx, detail) {
                self.detail = None;
            }
        }
        if let Some(cell) = &self.cell_popup {
            let trend = self.cell_trends.get(&cell.id).map(Vec::as_slice).unwrap_or(&[]);
            if !ui::cell_window::show(ctx, cell, trend) {
                self.cell_popup = None;
            }
        }
        if let Some(popup) = &mut self.warning_popup {
            if !ui::warning_window::show(ctx, popup) {
                self.warning_popup = None;
            }
        }
        if self.show_sensors && !ui::sensor_window::show(ctx, self.sensor_data.as_ref()) {
            self.show_sensors = false;
        }
        if self.show_hodo && !ui::hodograph_window::show(ctx, self.hodo_site.as_deref(), &self.hodo_data) {
            self.show_hodo = false;
        }
        if let (Some(xs), Some(tex)) = (&self.xsection, &self.xsection_tex) {
            if !ui::xsection_window::show(ctx, xs, tex) {
                self.xsection = None;
                self.xsection_tex = None;
                self.xsection_pts.clear();
            }
        }
        if self.show_3d {
            let mut open = true;
            ui::volume3d_window::show(
                ctx, &mut open, &mut self.vol3d_az, &mut self.vol3d_el, &mut self.vol3d_dist,
                &mut self.vol3d_pending, 192, 48,
            );
            self.show_3d = open;
        }
        if self.show_cappi {
            self.update_cappi(ctx);
            let mut open = true;
            if let Some(tex) = self.cappi_tex.clone() {
                open = ui::cappi_window::show(ctx, &tex, &mut self.cappi_alt_km, 300.0);
            } else {
                crate::ui::fit_phone(ctx, egui::Window::new("CAPPI slice")).open(&mut open).show(ctx, |ui| {
                    ui.weak("No volume loaded in the active pane.");
                });
            }
            self.show_cappi = open;
        }
        self.show_warning_banners(ctx);

        // Turn this frame's UI mutations into uploads/fetches before painting the map.
        for idx in 0..self.views.len() {
            self.sync_pane(idx, ctx);
        }
        self.sync_overlay();

        // Desktop status bar + right-dock alert panel. On Android the top bar shows the volume
        // age and the alerts live in the bell's slide-up sheet (see `app::mobile`).
        if !cfg!(target_os = "android") {
            if !self.obs_mode {
                egui::Panel::bottom("status_bar").show(root, |ui| self.status_bar(ui));
            }
            if self.show_alert_panel && !self.obs_mode {
                let bounds = self.view_bounds();
                if let Some((id, lon, lat)) = ui::alert_panel::show(root, self.active_alert_features(), bounds) {
                    // Fly the active camera to the alert and open its bulletin.
                    let cam = &mut self.views[self.active].camera;
                    cam.center = crate::render::mercator::lonlat_to_world(lon, lat);
                    cam.zoom = cam.zoom.max(8.0);
                    self.open_alert_popup(&id);
                }
            }
        }

        // OBS-mode hint so the chrome-free view is still escapable.
        if self.obs_mode {
            egui::Area::new("obs_hint".into())
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
                .interactable(false)
                .show(root, |ui| {
                    let txt = if self.obs_tour { "OBS · tour (F8 exit · F9 stop tour)" } else { "OBS mode (F8 exit · F9 tour)" };
                    egui::Frame::new()
                        .fill(egui::Color32::from_black_alpha(150))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| ui.colored_label(egui::Color32::from_white_alpha(200), txt));
                });
        }

        let placefile_labels = self.placefile_labels();
        egui::CentralPanel::default().show(root, |ui| {
            let full = ui.available_rect_before_wrap();
            let n = self.views.len();
            let rects = pane_rects(full, n);

            // If cameras are linked, mirror the active pane's camera to the others.
            if self.link_cameras {
                let cam = self.views[self.active.min(n - 1)].camera;
                for v in &mut self.views {
                    v.camera = cam;
                }
            }

            // Global basemap style is driven by the active pane.
            use crate::tiles::BasemapStyle;
            let style = self.views[self.active.min(n - 1)].basemap;
            let is_vector = matches!(style, BasemapStyle::Dark | BasemapStyle::Light);
            let is_raster = style.is_raster();
            let raster_style = if is_raster { style } else { BasemapStyle::None };
            self.tiles.set_keys(&self.settings.mapbox_key, &self.settings.maptiler_key);
            let mut clear_tiles = self.tiles.set_style(raster_style);
            // GOES sub-hourly scrub: fetch the available frame times when a GOES style becomes
            // active, and apply the selected frame (None = latest).
            if raster_style.goes_layer().is_some() {
                if self.goes_times_style != Some(raster_style) {
                    self.goes_times_style = Some(raster_style);
                    self.goes_times.clear();
                    self.goes_time_idx = None;
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.goes_times_rx = Some(rx);
                    let http = self.http.clone();
                    self._rt.spawn(async move {
                        let times = crate::tiles::fetch_goes_times(&http, raster_style, 8, 48).await;
                        let _ = tx.send(times);
                    });
                }
                let selected = self.goes_time_idx.and_then(|i| self.goes_times.get(i).copied());
                clear_tiles |= self.tiles.set_goes_time(selected);
            } else if self.goes_times_style.is_some() {
                self.goes_times_style = None;
                self.goes_times.clear();
                clear_tiles |= self.tiles.set_goes_time(None);
            }
            let mut clear_vector = false;
            if is_vector {
                clear_vector |= self.vtiles.set_style(style == BasemapStyle::Dark);
                clear_vector |= self.vtiles.note_zoom(self.views[self.active.min(n - 1)].camera.zoom);
            }
            self.last_viewport = rects.get(self.active).map_or((full.width(), full.height()), |r| (r.width(), r.height()));

            for (i, prect) in rects.iter().enumerate() {
                let first = i == 0;
                self.render_pane(
                    ui,
                    ctx,
                    i,
                    *prect,
                    is_vector,
                    is_raster,
                    clear_tiles && first,
                    clear_vector && first,
                    first,
                    &placefile_labels,
                );
            }

            // Pane borders; the active pane gets an accent outline.
            if n > 1 {
                for (i, prect) in rects.iter().enumerate() {
                    let (w, col) = if i == self.active {
                        (2.0, crate::theme::accent(self.settings.theme))
                    } else {
                        (1.0, egui::Color32::from_gray(60))
                    };
                    ui.painter().rect_stroke(*prect, 0.0, egui::Stroke::new(w, col), egui::StrokeKind::Inside);
                }
            }
        });

        // Dirty-diff persistence: one write per actual change, from any mutation site.
        if self.settings != self.saved {
            // A palette-map change reloads the color tables (bumps gen -> LUT re-bake).
            if self.settings.palettes != self.saved.palettes {
                self.palettes.reload(&self.settings.palette_paths());
            }
            self.settings.save();
            self.saved = self.settings.clone();
        }

        // Android text input: summon/dismiss the soft keyboard as egui focus moves in/out of
        // text fields, and float a Paste button (the system clipboard is unreachable from a
        // NativeActivity keyboard otherwise — egui gets the text as a Paste event next frame).
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

        // Idle heartbeat so clocks (volume age, countdowns) tick without input. Data arrivals and
        // animations (pulse, banners) request faster repaints on their own. Slower on Android to
        // spare the battery — nothing on screen changes faster than this between frames.
        let idle = if cfg!(target_os = "android") { 250 } else { 100 };
        ctx.request_repaint_after(std::time::Duration::from_millis(idle));
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
    use super::{categorical_lut, ramp_lut, ramp_lut_a};

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
