//! Hook Echo-WX application shell: the map view, its floating chrome, and the async data flow.
//!
//! UI code only mutates the active [`MapView`]; a single per-frame sync step turns those
//! mutations into GPU uploads and background fetches, so buttons and hotkeys share one path.

/// Touch-first Android chrome (top bar, bottom dock, slide-up sheets), replacing the desktop
/// drawer / pills / alert dock. Only the chrome differs; the map,
/// windows, and every data path are shared.
mod mobile;

use crate::colormap::{ColorTable, Palettes};
use crate::hotkeys::{self, BindableAction};
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
use std::sync::Arc;
use std::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
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
            show_cells: true,
            show_tracks: true,
            show_arrival_cones: false,
            show_nowcast: false,
            nowcast_lead_min: 15,
            show_tds: false,
            show_couplets: false,
        }
    }
}

/// Background overlay fetch results.
enum OverlayMsg {
    Alerts(Vec<GeoFeature>),
    /// WPC coded surface analysis (fronts + pressure centers).
    Fronts(wxdata::fronts::SurfaceAnalysis),
    Outlook(u8, Vec<GeoFeature>),
    Mds(Vec<GeoFeature>),
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
    Metar(Vec<wxdata::metar::SurfaceOb>),
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
    /// Aviation SIGMET/AIRMET hazard polygons (feature GG).
    Aviation(Vec<GeoFeature>),
}

/// One overlay data source to fetch.
#[derive(Clone)]
enum OverlaySource {
    /// NWS alerts; the `Option<(lat, lon)>` scopes zone-only alert resolution to the active radar.
    Alerts(Option<(f64, f64)>),
    Mds,
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
    /// Gridded L3 product (DVL/EET) for a site, projected to a lat/lon field (feature X).
    L3Grid(crate::render::FieldLayer, String),
    /// Melting-level and −20 °C heights at `(lon, lat)`, for the derived hail grids.
    FreezingLevels(f64, f64),
    /// NOHRSC observed snowfall analysis over an accumulation window (hours).
    Snow(u16),
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
    /// NHC tropical cyclones (feature V).
    /// NHC tropical suite: `(wind-field threshold in kt, include storm surge)`.
    Tropical(Option<u8>, bool),
    /// HRRR wind components for the particle layer, at a level and forecast hour.
    Wind(wxdata::hrrr::WindLevel, u8),
}

impl OverlaySource {
    async fn fetch(self, http: &reqwest::Client) -> anyhow::Result<OverlayMsg> {
        Ok(match self {
            OverlaySource::Alerts(near) => {
                OverlayMsg::Alerts(alerts::fetch_active(http, near).await?)
            }
            OverlaySource::Mds => {
                OverlayMsg::Mds(wxdata::spc::fetch_mesoscale_discussions(http).await?)
            }
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
                    Err(e) => log::warn!("ndbc buoys: {e}"),
                }
                OverlayMsg::Metar(obs)
            }
            OverlaySource::Webcams(min_lon, min_lat, max_lon, max_lat, windy_key) => {
                // Both networks, merged: the FAA is keyless but US-only, Windy covers the rest of
                // the world for anyone who has supplied a key. Nothing for the user to choose.
                let mut sites =
                    wxdata::webcams::fetch_bbox(http, min_lon, min_lat, max_lon, max_lat)
                        .await
                        .unwrap_or_default();
                if !windy_key.is_empty() {
                    // A bad or throttled key must not take the FAA cameras down with it.
                    match wxdata::webcams::fetch_windy_bbox(
                        http, &windy_key, min_lon, min_lat, max_lon, max_lat,
                    )
                    .await
                    {
                        Ok(w) => sites.extend(w),
                        Err(e) => log::warn!("windy webcams: {e}"),
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
            } => {
                // METARs come first and cost one request; the keyed networks add themselves.
                let metars = wxdata::metar::fetch_bbox(http, bbox.0, bbox.1, bbox.2, bbox.3)
                    .await
                    .unwrap_or_default();
                OverlayMsg::Stations(
                    wxdata::stations::fetch_all(http, &metars, &tempest, &wu, center.0, center.1)
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
                OverlayMsg::Aviation(wxdata::aviation::fetch_airsigmet(http).await?)
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
}

impl ContourKind {
    pub(crate) const ALL: [ContourKind; 9] = [
        ContourKind::Off,
        ContourKind::Mslp,
        ContourKind::T2m,
        ContourKind::Td2m,
        ContourKind::Cape,
        ContourKind::Srh,
        ContourKind::Stp,
        ContourKind::Scp,
        ContourKind::Ehi,
    ];

    /// The composite parameters, which combine several GRIB fields instead of drawing one.
    pub(crate) fn severe(self) -> Option<wxdata::severe::SevereKind> {
        use wxdata::severe::SevereKind as S;
        Some(match self {
            ContourKind::Stp => S::Stp,
            ContourKind::Scp => S::Scp,
            ContourKind::Ehi => S::Ehi,
            _ => return None,
        })
    }

    /// Contour interval in display units (composites only; single fields carry theirs in `params`).
    pub(crate) fn severe_interval(self) -> f32 {
        match self {
            ContourKind::Stp => 0.5,
            ContourKind::Scp => 2.0,
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
            ContourKind::Stp | ContourKind::Scp | ContourKind::Ehi => None,
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
    ProbSevere,
    Aviation,
    RangeRings,
    Sensors,
    Hodo,
    Cells,
    Tracks,
    ArrivalCones,
    Nowcast,
    Tds,
    Couplets,
    Alerts,
    Mds,
    Mping,
    Pireps,
    Recon,
    Fronts,
    GlmLightning,
    Wind,
    LinkCameras,
}

impl OverlayToggle {
    /// Every toggle, for the persistence sweep. A new variant belongs here too, or it silently
    /// stops being remembered across restarts.
    pub(crate) const ALL: [OverlayToggle; 32] = [
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
        Self::ProbSevere,
        Self::Aviation,
        Self::RangeRings,
        Self::Sensors,
        Self::Hodo,
        Self::Cells,
        Self::Tracks,
        Self::ArrivalCones,
        Self::Nowcast,
        Self::Tds,
        Self::Couplets,
        Self::Alerts,
        Self::Mds,
        Self::Mping,
        Self::Pireps,
        Self::Recon,
        Self::Fronts,
        Self::GlmLightning,
        Self::Wind,
        Self::LinkCameras,
    ];

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
    Digest,
    Afd,
    Cappi,
    Volume3d,
    StormTable,
    /// Warning verification lab (IEM Cow): how the office's warnings scored on an event day.
    Verify,
    Climatology,
    LayerManager,
    Wizard,
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
    ToggleToolbar,
    /// Show/hide the docked sidebar on the left (desktop).
    ToggleSidebar,
    Reload,
    InstantReplay,
    GoLive,
    /// Hand the current view off to windy.com in the browser.
    OpenInWindy,
    /// Copy a `hookecho://goto/…` link to this view (site, center, zoom, archive time).
    CopyViewLink,
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
        FL::Qpe1h | FL::Qpe24h => 120,
        // MRMS precip type / flash-flood ARI on the ~2-min cadence; L3 grids on the 120 s L3 cadence.
        FL::PrecipType | FL::FlashFlood | FL::Vil | FL::EchoTops | FL::Hca => 120,
        FL::UpdraftHelicity => 600,
        // Snowfall accumulates over a whole model run; it moves as slowly as the run does.
        FL::Snowfall => 600,
        // The analysis is reissued four times a day; half an hour is plenty.
        FL::SnowAnalysis => 1800,
        FL::Smoke => 900,
        // The 24-h hail-swath accumulation moves slowly.
        FL::HailSwath => 300,
        // Environment (HRRR CAPE/SRH) refreshes slowly — 15 min.
        FL::Cape | FL::Srh => 900,
        // Derived products cost no network: they recompute when the volume does, not on a clock.
        FL::VilLocal | FL::VilDensity | FL::EtopLocal | FL::HailMehs | FL::HailPosh => 60,
    }
}

/// Per-field-layer UI + fetch state (toggle, pending upload, refresh clock).
#[derive(Default)]
pub(crate) struct FieldState {
    pub show: bool,
    pub pending: Option<crate::render::MrmsUpload>,
    pub last_fetch: Option<Instant>,
}

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
type ShownKey = (
    String,
    Moment,
    usize,
    Option<f32>,
    bool,
    u64,
    Option<(u32, u32)>,
    bool,
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

/// URL scheme for a shared view. One parser serves all three ways a view arrives: the
/// `HOOKECHO_GOTO` env var, the `goto.txt` the Android notification tap writes (which uses the
/// site-less `,lon,lat,zoom` form), and a tapped `hookecho://goto/…` link.
const GOTO_SCHEME: &str = "hookecho://goto/";

/// Parse `[hookecho://goto/]SITE,lon,lat,zoom[,RFC3339]`. The site may be empty.
#[allow(clippy::type_complexity)]
fn parse_goto(v: &str) -> Option<(String, f64, f64, f64, Option<DateTime<Utc>>)> {
    let v = v.trim().strip_prefix(GOTO_SCHEME).unwrap_or(v.trim());
    let p: Vec<&str> = v.split(',').map(str::trim).collect();
    let (Some(site), Some(Ok(lon)), Some(Ok(lat)), Some(Ok(zoom))) = (
        p.first(),
        p.get(1).map(|s| s.parse()),
        p.get(2).map(|s| s.parse()),
        p.get(3).map(|s| s.parse()),
    ) else {
        return None;
    };
    let time = p.get(4).filter(|s| !s.is_empty()).and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|e| log::warn!("goto: bad time {s:?}: {e}"))
            .ok()
    });
    Some((site.to_string(), lon, lat, zoom, time))
}

/// The shareable link for a view.
fn goto_link(site: &str, lon: f64, lat: f64, zoom: f64, time: Option<DateTime<Utc>>) -> String {
    let t = time
        .map(|t| format!(",{}", t.to_rfc3339()))
        .unwrap_or_default();
    format!("{GOTO_SCHEME}{site},{lon:.4},{lat:.4},{zoom:.1}{t}")
}

/// How a lightning flash looks at `age_secs`: bright white-hot when it just happened, fading to a
/// dim orange ember by the end of the window. The brightness IS the recency cue — a map of
/// same-colored dots says where lightning has been, not where it is now.
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
    update_tx: Sender<Option<String>>,
    update_rx: Receiver<Option<String>>,
    geocode_tx: Sender<Result<(String, f64, f64), String>>,
    geocode_rx: Receiver<Result<(String, f64, f64), String>>,
    /// In-progress offline chase-pack tile download (basemap pre-cache for the current view).
    chasepack: Option<ChasePack>,
    /// Per-pane "what's uploaded" key, so each pane re-bins/re-uploads only on a real change.
    pane_shown: std::collections::HashMap<usize, ShownKey>,
    /// Last `(theme, system_dark)` handed to `theme::apply`.
    theme_applied: Option<(crate::settings::Theme, bool)>,
    /// When the settings tree was last diffed against the saved copy.
    settings_checked: Option<Instant>,
    /// Frame counter, only used to invalidate within-frame memos.
    frame_nr: u64,
    palette_cache: Option<(u64, Vec<PaletteEntry>)>,
    /// Last result of each per-volume detector, keyed by what it depends on (see `volume_key`).
    #[allow(clippy::type_complexity)]
    nowcast_cache: Option<(
        ((usize, String, usize), usize, u8, Option<(u32, u32)>, u64),
        Vec<(f64, f64, egui::Color32)>,
    )>,
    tds_cache: Option<((usize, String, usize), Vec<wxdata::tds::TdsHit>)>,
    couplet_cache: Option<((usize, String, usize), Vec<wxdata::rotation::CoupletHit>)>,
    site_dialog: Option<ui::site_dialog::SiteDialog>,
    wizard: ui::wizard::Wizard,
    settings_window: ui::settings_window::SettingsWindow,
    /// Active color tables (one per moment); reloaded when the palette settings change.
    palettes: Palettes,
    /// Live chunk stream for the active view: (view index, site, task handle).
    #[cfg(not(target_arch = "wasm32"))]
    live_stream: Option<(usize, String, tokio::task::JoinHandle<()>)>,
    last_stream_attempt: Option<Instant>,
    /// Decoded-volume LRU keyed by AWS object name, so scrubbing back and forth on the
    /// timeline doesn't re-download. ~10 volumes; each ~a few MB.
    scan_cache: LruCache<String, Arc<Scan>>,
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
    cells_window: ui::cells_window::CellsWindow,
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
    mobile_drawer_query: String,
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
    palette_editor: ui::palette_editor::PaletteEditor,
    digest_window: ui::digest_window::DigestWindow,
    digest_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    sounding_window: ui::sounding_window::SoundingWindow,
    sounding_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::sounding::Sounding, String>>>,
    /// The observed RAOB fetched alongside the HRRR profile, for the same click.
    raob_rx: Option<std::sync::mpsc::Receiver<Result<wxdata::sounding::Sounding, String>>>,
    /// Chase mode: follow a position, auto-switching the active pane to the nearest radar.
    chase_mode: bool,
    chase_pos: Option<(f64, f64)>,
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
    sync_checked: Option<std::time::Instant>,
    /// Position sharing (LAN broadcast + optional relay), started on first use.
    share: Option<crate::share::Share>,
    /// Everyone else's last known position, keyed by their device id.
    peers: std::collections::HashMap<String, crate::share::Peer>,
    /// When we last put our own fix on the wire (both transports share the cadence).
    share_sent: Option<std::time::Instant>,
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
    /// Layers panel (floating, searchable layer picker): open flag + its search text.
    /// Viewport minus the docked bars, refreshed each frame — floating `Area`s constrain to this
    /// instead of `content_rect`, which egui measures before panels take their bite.
    chrome_rect: egui::Rect,
    layers_query: String,
    /// Ctrl+K command palette: open flag, query, and the highlighted row.
    /// Set by Ctrl+K so the drawer grabs the search field on the frame it opens.
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
    /// Android only: which slide-up sheet the mobile chrome is showing (see `app::mobile`).
    mobile_sheet: mobile::MobileSheet,
    /// Android: hide all floating chrome to view the whole radar (toggled by the eye button).
    mobile_chrome_hidden: bool,
    /// Android: how far open the persistent bottom sheet is.
    mobile_snap: mobile::sheet::SheetSnap,
    /// Android: the sheet's live height while a finger is dragging it (`None` = at/easing to a
    /// snap).
    mobile_sheet_drag: Option<f32>,
    /// Android: rects the mobile chrome covers this frame. Two-finger gestures are read straight
    /// off the raw input, which has no idea egui drew a sheet over the map, so the pane input
    /// block checks the gesture center against these.
    mobile_occlusion: Vec<egui::Rect>,
    /// When the last two-finger gesture ended. Lifting one finger of a pinch leaves the other
    /// one down, which egui immediately reads as a click and a fresh drag — an interrogate popup
    /// and a jump for what was only the end of a zoom. A short cooldown eats both.
    last_gesture_end: Option<std::time::Instant>,
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
    /// Animated wind particles. The grids are shared; the particle sets are per pane, because each
    /// pane has its own camera. Nothing here persists to settings — neither do fronts or GLM.
    show_wind: bool,
    wind: Option<crate::wind_draw::WindField>,
    wind_level: wxdata::hrrr::WindLevel,
    wind_particles: std::collections::HashMap<usize, crate::wind_draw::Particles>,
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
    /// True while a rotation couplet is currently detected (rising-edge alarm latch).
    rot_active: bool,
    /// Active new-warning banners (event, area, first-seen time); expire after a while.
    warning_banners: Vec<(String, String, Instant)>,
    /// Transient results of things the user just did (export saved, encode failed). The third
    /// lane, distinct from the warning banners (weather) and the error chip (radar feed).
    toasts: Vec<Toast>,
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
        // Bundle Phosphor icon glyphs into the proportional font family so the mobile
        // RadarOmega-style chrome (tool dock, menu rows) can draw line icons — egui's default
        // face has none. Cheap; desktop uses them too (no-op if unreferenced).
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

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
        // The GPU's 2D texture-size cap: desktop/Adreno do 16384, but many mobile GPUs cap at
        // 4096. Field grids (MRMS rotation/AzShear reach 14000 px) are decimated to fit this.
        let max_texture_dim = render_state.device.limits().max_texture_dimension_2d;
        {
            let mut w = render_state.renderer.write();
            w.callback_resources.insert(RenderResources::new(
                &render_state.device,
                render_state.target_format,
            ));
            w.callback_resources
                .insert(crate::render3d::Volume3dResources::new(
                    &render_state.device,
                    render_state.target_format,
                ));
        }

        let settings = Settings::load();
        let mut tiles = TileManager::new(spawner.clone());
        let mut vtiles = crate::vector_tiles::VectorTileManager::new(spawner.clone());
        // Tile workers wake the UI the moment a tile is ready; without this a finished tile waits
        // for the next repaint the app happens to want.
        tiles.set_ctx(cc.egui_ctx.clone());
        vtiles.set_ctx(cc.egui_ctx.clone());
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        let (overlay_tx, overlay_rx) = std::sync::mpsc::channel();
        let (update_tx, update_rx) = std::sync::mpsc::channel();
        let (geocode_tx, geocode_rx) = std::sync::mpsc::channel();
        let (pf_icon_tx, pf_icon_rx) = std::sync::mpsc::channel();
        // Every app-level fetch (alerts, overlays, placefiles, radar index) goes through this one.
        // A hung request with no timeout leaves whatever it was loading stuck loading forever.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        // Open on the saved startup view if set (and its site still resolves), else where the app
        // was last looking, else the default site.
        let resume = settings.start_view.as_ref().or(settings.last_view.as_ref());
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
            update_tx,
            update_rx,
            geocode_tx,
            geocode_rx,
            chasepack: None,
            pane_shown: std::collections::HashMap::new(),
            theme_applied: None,
            settings_checked: None,
            frame_nr: 0,
            palette_cache: None,
            nowcast_cache: None,
            tds_cache: None,
            couplet_cache: None,
            site_dialog: None,
            wizard: {
                let mut w = ui::wizard::Wizard::default();
                if !settings_setup_done {
                    w.start();
                }
                w
            },
            settings_window: Default::default(),
            palettes: Palettes::default(),
            #[cfg(not(target_arch = "wasm32"))]
            live_stream: None,
            last_stream_attempt: None,
            // DVR: retain a deep buffer of decoded volumes so instant replay serves recent frames
            // from RAM without re-downloading (~30 volumes ≈ 2.5 h at a 5-min cadence).
            // Phones can't hold a 2.5 h DVR buffer of decoded volumes — each is tens of MB and
            // Android kills the process long before the LRU fills.
            scan_cache: LruCache::new(
                NonZeroUsize::new(if cfg!(target_os = "android") { 6 } else { 30 }).unwrap(),
            ),
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
            pending_overlay: None,
            overlay_ready: false,
            overlay_last_fetch: None,
            detail: None,
            cell_popup: None,
            marker_popup: None,
            cells_window: Default::default(),
            verify_window: Default::default(),
            verify_rx: None,
            xsection_moment: Moment::Reflectivity,
            follow_cell: None,
            follow_notice: None,
            warning_popup: None,
            error_chip: None,
            mobile_drawer_query: String::new(),
            hrrr_layer_hour: std::collections::HashMap::new(),
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
            strokes: Vec::new(),
            draw_color: DRAW_COLORS[0],
            marker_window: Default::default(),
            event_window: Default::default(),
            palette_editor: Default::default(),
            digest_window: Default::default(),
            digest_rx: None,
            sounding_window: Default::default(),
            sounding_rx: None,
            raob_rx: None,
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
            derived_key: None,
            snow_hours: 24,
            snow_fetched: None,
            freezing: None,
            freezing_last_fetch: None,
            show_metar: false,
            metars: Vec::new(),
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
            // Map-first by default on both platforms: the floating chrome covers the common paths,
            // and the full toolbox is one "Advanced" tap away.
            chrome_rect: egui::Rect::EVERYTHING,
            layers_query: String::new(),
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
            mobile_sheet: mobile::MobileSheet::None,
            mobile_chrome_hidden: false,
            mobile_snap: Default::default(),
            mobile_sheet_drag: None,
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
            rain_eta: Vec::new(),
            show_fronts: false,
            fronts: None,
            fronts_last_fetch: None,
            show_glm: false,
            glm: std::sync::Arc::new(std::sync::Mutex::new(wxdata::glm::GlmFeed::new(15))),
            glm_last_poll: None,
            glm_polling: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            show_wind: false,
            wind: None,
            wind_level: wxdata::hrrr::WindLevel::Surface,
            wind_particles: std::collections::HashMap::new(),
            wind_fetched: None,
            wind_last_fetch: None,
            wind_inflight: None,
            wind_last_frame: None,
            wind_dt: 0.0,
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
            rot_active: false,
            warning_banners: Vec::new(),
            toasts: Vec::new(),
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
        // Restore the overlays that were on last time. Unknown names (an older build reading a
        // newer file) are skipped rather than treated as an error.
        let restore: Vec<OverlayToggle> = app
            .settings
            .overlays_on
            .iter()
            .filter_map(|s| OverlayToggle::from_slug(s))
            .collect();
        let needs_rebuild = restore.iter().any(|t| {
            use OverlayToggle as T;
            matches!(
                t,
                T::Tropical | T::ProbSevere | T::Aviation | T::Alerts | T::Mds | T::Fires
            )
        });
        for t in restore {
            *app.overlay_flag(t) = true;
        }
        if needs_rebuild {
            app.rebuild_overlays();
        }
        app.palettes.reload(&app.settings.palette_paths());
        app.apply_goto_env();
        app.drain_goto_file();
        crate::platform::set_background_alerts(app.settings.background_alerts);
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

    /// `SITE,lon,lat,zoom[,RFC3339]`, with or without the `hookecho://goto/` prefix.
    fn apply_goto(&mut self, v: &str) {
        let Some((site, lon, lat, zoom, time)) = parse_goto(v) else {
            log::warn!("HOOKECHO_GOTO: want SITE,lon,lat,zoom[,RFC3339], got {v:?}");
            return;
        };
        self.goto_view(&site, lon, lat, zoom, time);
    }

    /// Volume poll cadence, doubled on a metered link. A phone on mobile data pulls a multi-MB
    /// volume every interval; halving that rate costs at most a couple of minutes of latency on
    /// the live head, which the chunk stream covers anyway when it is running.
    fn poll_interval_secs(&self) -> u64 {
        let base = self.settings.poll_interval_secs;
        if crate::platform::is_metered() {
            base * 2
        } else {
            base
        }
    }

    /// Largest edge a national field grid may keep. Bounded by what the GPU will accept, and
    /// then hard-capped: at 8192 a single f32 grid is ~268 MB of RAM before it is ever indexed,
    /// which no phone should be asked to hold — and the Adreno 750 reports 16384, so the device
    /// limit alone never bit. 4096 on Android is still finer than the screen can show.
    fn field_texture_cap(&self) -> usize {
        let ceiling = if cfg!(target_os = "android") {
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
    fn available_moments(&self) -> [bool; 6] {
        // The pane's remembered union, not this instant's volume: a half-arrived live volume
        // carries fewer moments than the radar sends, and the rows must not blink.
        self.views[self.active].moments()
    }

    /// Recompute the locally derived products (VIL, VIL density, echo tops) when the active pane's
    /// volume, the echo-top threshold, or the set of enabled derived layers changed.
    ///
    /// Unlike every other field layer this costs no network — the volume is already decoded here —
    /// so it has no cadence: it recomputes exactly when its inputs move, which is what makes it
    /// work in archive replay and on each live tilt.
    fn recompute_derived(&mut self, ctx: &egui::Context) {
        use crate::render::FieldLayer as FL;
        const LAYERS: [FL; 5] = [
            FL::VilLocal,
            FL::VilDensity,
            FL::EtopLocal,
            FL::HailMehs,
            FL::HailPosh,
        ];
        /// Bit positions in the mask for the two hail grids.
        const HAIL_BITS: u8 = 0b11000;
        let mask = LAYERS.iter().enumerate().fold(0u8, |m, (i, l)| {
            m | u8::from(self.fields.get(l).is_some_and(|s| s.show)) << i
        });
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
            match source.fetch(&http).await {
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
        // Scope zone-only alert resolution (heat, advisories) to the active radar so the site's own
        // alerts always resolve — see `alerts::fetch_active`.
        let near = self.views[self.active]
            .site
            .as_deref()
            .and_then(wxdata::sites::site_by_id)
            .map(|s| (s.latitude as f64, s.longitude as f64));
        self.spawn_overlay(ctx, OverlaySource::Alerts(near));
        self.spawn_overlay(ctx, OverlaySource::Mds);
        if (1..=3).contains(&self.filters.wssi_day) {
            self.spawn_overlay(ctx, OverlaySource::Wssi(self.filters.wssi_day));
        }
        if (1..=3).contains(&self.filters.ero_day) {
            self.spawn_overlay(ctx, OverlaySource::Ero(self.filters.ero_day));
        }
        // Only fetch the SPC outlook the user has selected (off = day 0 fetches nothing).
        if (1..=3).contains(&self.filters.outlook_day) {
            self.spawn_overlay(
                ctx,
                OverlaySource::Outlook(self.filters.outlook_day, self.outlook_kind_for_day()),
            );
        }
        // Storm cells for the active view's site (Level 3 products are per-site). Terminal
        // radars don't run the storm-cell algorithms under their four-letter id.
        if let Some(site) = self.views[self.active]
            .site
            .clone()
            .filter(|s| !wxdata::tdwr::is_tdwr(s))
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

    /// Ask GitHub for the newest tagged release, once per session. `/releases/latest` skips
    /// prereleases, which is exactly right here: the rolling `latest` build is a prerelease.
    fn check_for_update(&mut self, ctx: &egui::Context) {
        if self.update_state != ui::about_window::UpdateState::Idle {
            return;
        }
        self.update_state = ui::about_window::UpdateState::Checking;
        let http = self.http.clone();
        let tx = self.update_tx.clone();
        let ctx2 = ctx.clone();
        self.spawner.spawn(async move {
            let url = "https://api.github.com/repos/d4vid87/hookecho/releases/latest";
            // GitHub rejects requests without a User-Agent.
            let tag = async {
                let text = http
                    .get(url)
                    .header("User-Agent", "hookecho")
                    .send()
                    .await
                    .ok()?
                    .text()
                    .await
                    .ok()?;
                let body: serde_json::Value = serde_json::from_str(&text).ok()?;
                body.get("tag_name")?.as_str().map(str::to_string)
            }
            .await;
            let _ = tx.send(tag);
            ctx2.request_repaint();
        });
    }

    /// Every alert sound goes through here, so one mute switch covers all of them (and any that
    /// get added later) instead of a guard per call site.
    fn play_alert(&self, sound: &crate::settings::AlertSound) {
        if self.settings.mute_alerts {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        crate::audio::play(sound, self.settings.alert_volume);
    }

    /// Say something happened. Operation results used to reach the user only through the log.
    fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
    }

    /// Toast stack, below the right-hand control column. Expires after ~4 s, fading out over the
    /// last second; click to dismiss.
    /// ponytail: clock-based fade, no per-toast animation ids.
    fn show_toasts(&mut self, ctx: &egui::Context) {
        const LIFE: f32 = 4.0;
        self.toasts.retain(|t| t.at.elapsed().as_secs_f32() < LIFE);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let accent = crate::theme::accent(self.settings.theme);
        let mut dismiss = None;
        egui::Area::new(egui::Id::new("toasts"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(
                    crate::ui::style::LANE_RIGHT_BADGE_X,
                    crate::ui::style::lane_right_badge_y(6),
                ),
            )
            .show(ctx, |ui| {
                for (i, t) in self.toasts.iter().enumerate() {
                    let left = LIFE - t.at.elapsed().as_secs_f32();
                    let alpha = left.min(1.0).clamp(0.0, 1.0);
                    let stripe = match t.kind {
                        ToastKind::Info => accent,
                        ToastKind::Success => crate::ui::style::OMEGA_GREEN,
                        ToastKind::Error => egui::Color32::from_rgb(220, 90, 90),
                    };
                    let resp = crate::ui::style::glass(ui, (220.0 * alpha) as u8)
                        .stroke(egui::Stroke::new(1.0, stripe.gamma_multiply(alpha)))
                        .show(ui, |ui| {
                            ui.set_max_width(320.0);
                            ui.label(
                                egui::RichText::new(&t.text)
                                    .size(crate::ui::style::FONT_BASE)
                                    .color(egui::Color32::from_gray(235).gamma_multiply(alpha)),
                            );
                        })
                        .response;
                    if resp.interact(egui::Sense::click()).clicked() {
                        dismiss = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }

    /// Draw new-warning banners at top-center (auto-expire ~45s; click to dismiss all).
    fn show_warning_banners(&mut self, ctx: &egui::Context) {
        self.warning_banners
            .retain(|(_, _, at)| at.elapsed().as_secs() < 45);
        if self.warning_banners.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("warning_banners"))
            .constrain_to(self.chrome_rect)
            // Read-only banner that self-expires in 45 s. Interactable layers occlude the map's
            // pinch test (`layer_id_at`), and a banner across the top of a phone screen is exactly
            // where a two-finger gesture lands, so it must not take input.
            .interactable(false)
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(0.0, crate::ui::style::LANE_TOP_BANNER),
            )
            .show(ctx, |ui| {
                for (event, area, at) in &self.warning_banners {
                    // Fade out over the last two seconds instead of vanishing mid-read.
                    let a = ((45.0 - at.elapsed().as_secs_f32()) / 2.0).clamp(0.0, 1.0);
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(150, 20, 20).gamma_multiply(a))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(255, 120, 120).gamma_multiply(a),
                        ))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(egui_phosphor::regular::WARNING)
                                        .size(16.0)
                                        .color(egui::Color32::WHITE.gamma_multiply(a)),
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("New {event}"))
                                            .strong()
                                            .color(egui::Color32::WHITE.gamma_multiply(a)),
                                    );
                                    if !area.is_empty() {
                                        ui.label(egui::RichText::new(area).small().color(
                                            egui::Color32::from_gray(230).gamma_multiply(a),
                                        ));
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
            });
        // Fast enough for the fade-out; the banner is only up for 45 s.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
                // A watched location always alerts + pushes: inside the polygon, or within that
                // marker's radius of it. Home first, then the closest — a warning that clips two
                // saved places should name the one you sleep in.
                let hit = self
                    .settings
                    .markers
                    .iter()
                    .filter_map(|m| {
                        let km = f.distance_km(m.lon, m.lat);
                        (km <= m.alert_radius_mi * crate::geo::KM_PER_MILE).then_some((m, km))
                    })
                    .min_by(|(a, ka), (b, kb)| {
                        b.home
                            .cmp(&a.home)
                            .then(ka.partial_cmp(kb).unwrap_or(std::cmp::Ordering::Equal))
                    });
                let (label, area) = match hit {
                    Some((m, km)) => {
                        // Watched location covered → push to the phone (opt-in ntfy topic).
                        self.push_ntfy(
                            &format!("⚠ {} — {}", a.event, m.name),
                            if a.headline.is_empty() {
                                &a.area
                            } else {
                                &a.headline
                            },
                            urgent,
                        );
                        let where_ = if km <= 0.05 {
                            format!("covers {}", m.name)
                        } else {
                            format!("{:.0} mi from {}", km / crate::geo::KM_PER_MILE, m.name)
                        };
                        (format!("⚠ {}", a.event), where_)
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
                // Read it out before the banner text is moved into the queue: chasing is an
                // eyes-on-the-road activity, and a warning you have to read is one you read late.
                if self.settings.speak_warnings {
                    let until = a
                        .expires
                        .map(|t| {
                            format!(
                                " until {}",
                                crate::timefmt::fmt_clock(
                                    t,
                                    self.settings
                                        .tz_for(self.views[self.active].site.as_deref()),
                                    false,
                                )
                            )
                        })
                        .unwrap_or_default();
                    if !self.settings.mute_alerts {
                        crate::speech::speak(&format!("{} for {}{}", a.event, area, until));
                    }
                }
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
                self.play_alert(sound);
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
            self.lightning_alerted
                .insert(m.name.clone(), Instant::now());
            self.push_ntfy(
                &format!("⚡ Lightning near {}", m.name),
                &format!(
                    "Cloud-to-ground strikes within {RADIUS_KM:.0} km of {}",
                    m.name
                ),
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
        // Watched points: every saved marker, plus where you are if chase mode knows.
        let mut points: Vec<(String, [f64; 2])> = self
            .settings
            .markers
            .iter()
            .map(|m| (m.name.clone(), [m.lon, m.lat]))
            .collect();
        if let Some((lon, lat)) = self.chase_pos {
            points.push(("your location".to_string(), [lon, lat]));
        }
        if points.is_empty() {
            return;
        }
        let names: Vec<String> = points.iter().map(|(n, _)| n.clone()).collect();
        self.rain_detector.retain(&names);

        let tilt = self.views[idx].tilt;
        let Some(sweep) = self.views[idx]
            .volume
            .as_mut()
            .and_then(|v| v.binned(Moment::Reflectivity, tilt, false).ok())
            .cloned()
        else {
            return;
        };
        let sample = refl_sampler(&sweep);

        let mut fired = false;
        self.rain_eta.clear();
        for (name, at) in &points {
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
            if let Verdict::Fire(min) = self.rain_detector.update(name, eta) {
                self.push_ntfy(
                    &format!("\u{1f327} Rain reaching {name}"),
                    &format!("About {min:.0} minutes out"),
                    false,
                );
                self.warning_banners.push((
                    format!("\u{1f327} Rain reaching {name}"),
                    format!("~{min:.0} min"),
                    Instant::now(),
                ));
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
        let Some(vol) = self.views[self.active].volume.as_mut() else {
            return;
        };
        let sweeps = vol.reflectivity_tilts();
        if sweeps.is_empty() {
            return;
        }
        let Some(v3) = wxdata::volume3d::build(&sweeps, N, NZ, 150.0, 18.0) else {
            return;
        };
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
        let img = ui::xsection_window::to_image(&xs, self.palettes.table(moment));
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
        self.spawner.spawn(async move {
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
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -34.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let n = self.goes_times.len();
                        // Effective index (None = latest = n-1).
                        let cur = self.goes_time_idx.unwrap_or(n - 1);
                        ui.label("🛰 GOES:");
                        if ui
                            .add_enabled(
                                cur > 0,
                                egui::Button::new(egui_phosphor::regular::CARET_LEFT),
                            )
                            .clicked()
                        {
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
                            let ni = cur + 1;
                            self.goes_time_idx = if ni >= n - 1 { None } else { Some(ni) };
                        }
                        if ui
                            .add_enabled(self.goes_time_idx.is_some(), egui::Button::new("Latest"))
                            .clicked()
                        {
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
        self.sync_checked = Some(std::time::Instant::now());
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
        self.share_sent = Some(std::time::Instant::now());
        let name = if self.settings.share_name.is_empty() {
            "me"
        } else {
            &self.settings.share_name
        };
        let me = share.me(name, lon, lat);
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
        let (tx, rx) = std::sync::mpsc::channel();
        self.sounding_rx = Some(rx);
        self.sounding_window.open = true;
        self.sounding_window.busy = true;
        self.sounding_window.sounding = None;
        let http = self.http.clone();
        self.spawner.spawn(async move {
            let res = wxdata::sounding::fetch(&http, lon, lat)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
        self.fetch_raob(lon, lat);
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
                    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 150),
                ));
            }
        }
        out
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
            self.warning_banners.push((
                "⚠ TDS detected".to_string(),
                format!("{} debris signature(s) — possible tornado", hits.len()),
                Instant::now(),
            ));
            self.push_ntfy(
                "⚠ Tornado Debris Signature",
                "Low CC + high reflectivity detected on radar",
                true,
            );
            if self.settings.alert_sound {
                self.play_alert(&self.settings.tds_sound.clone());
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
            self.warning_banners.push((
                "⟳ Rotation detected".to_string(),
                format!("{kt:.0} kt couplet — {where_}"),
                Instant::now(),
            ));
            self.push_ntfy(
                "⟳ Rotation couplet",
                &format!("{kt:.0} kt rotational velocity — {where_}"),
                true,
            );
            if self.settings.alert_sound {
                self.play_alert(&self.settings.rotation_sound.clone());
            }
        }
        self.rot_active = now_active;
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
        crate::ui::phone_surface(ctx, egui::Window::new("Tornado climatology"))
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
                } else if let Some(e) = &self.climo_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 90, 90),
                        format!("Load failed: {e}"),
                    );
                } else {
                    ui.strong(format!("{} tornadoes on record", self.climo_hits.len()));
                    let hist = wxdata::torclimo::mag_histogram(&self.climo_hits);
                    ui.horizontal_wrapped(|ui| {
                        for (i, label) in ["EF0", "EF1", "EF2", "EF3", "EF4", "EF5", "Unk"]
                            .iter()
                            .enumerate()
                        {
                            crate::theme::stat_card(ui, label, &hist[i].to_string());
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for t in self.climo_hits.iter().take(50) {
                                let mag = if t.mag < 0 {
                                    "EF?".to_string()
                                } else {
                                    format!("EF{}", t.mag)
                                };
                                ui.label(format!(
                                    "{}  {}  start {:.2},{:.2}",
                                    t.year, mag, t.slat, t.slon
                                ));
                            }
                            if self.climo_hits.len() > 50 {
                                ui.weak(format!("… and {} more", self.climo_hits.len() - 50));
                            }
                        });
                }
                ui.separator();
                ui.strong("Warning history");
                ui.weak("How often this spot's county has been warned (IEM, 1986–present).");
                match (&self.climo_warn, self.climo_warn_rx.is_some()) {
                    (Some(s), _) if s.total == 0 => {
                        ui.label("No warnings on record here.");
                    }
                    (Some(s), _) => {
                        ui.horizontal_wrapped(|ui| {
                            crate::theme::stat_card(ui, "Warnings", &s.total.to_string());
                            if let Some(y) = s.first_year {
                                crate::theme::stat_card(ui, "Since", &y.to_string());
                            }
                            if let Some((y, n)) = s.busiest_year {
                                crate::theme::stat_card(ui, "Busiest year", &format!("{y} ({n})"));
                            }
                            if let Some((d, n)) = s.worst_day {
                                crate::theme::stat_card(ui, "Worst day", &format!("{d} ({n})"));
                            }
                        });
                        for (name, n) in s.by_name.iter().take(8) {
                            ui.label(format!("{n} × {name}"));
                        }
                    }
                    (None, true) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading warning history…");
                        });
                    }
                    (None, false) => {
                        ui.weak("Warning history unavailable.");
                    }
                }
            });
        self.climo_open = open;
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
        let style = self.views[self.active].basemap;
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
        let style = self.views[self.active].basemap;
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
            crate::tiles::pack_tile_count(min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
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
        let style = self.views[self.active].basemap;
        let (z_lo, z_hi) = self.chasepack_zoom();
        let (min_lon, min_lat, max_lon, max_lat) = self.view_bounds();
        let jobs = if style.is_raster() {
            self.tiles
                .pack_jobs(style, min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
        } else if matches!(style, BasemapStyle::Dark | BasemapStyle::Light) {
            self.vtiles
                .pack_jobs(min_lon, min_lat, max_lon, max_lat, z_lo, z_hi)
        } else {
            Vec::new()
        };
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
            if (1..=3).contains(&day) && self.outlook_features[(day - 1) as usize].is_empty() {
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
        use crate::tiles::BasemapStyle;
        let chasepack = self.chasepack_ui();
        let (mb_key, mt_key) = (
            !self.settings.mapbox_key.is_empty(),
            !self.settings.maptiler_key.is_empty(),
        );
        // Split the borrow: the combo writes both the pane's style and the persisted default.
        let (view, settings) = (&mut self.views[self.active], &mut self.settings);
        egui::ComboBox::from_label("Background")
            .selected_text(view.basemap.label())
            .show_ui(ui, |ui| {
                // Only styles whose provider key is set are selectable.
                for s in BasemapStyle::ALL
                    .into_iter()
                    .filter(|s| s.available(mb_key, mt_key))
                {
                    if ui
                        .selectable_value(&mut view.basemap, s, s.label())
                        .clicked()
                    {
                        settings.basemap = s.slug().to_string(); // persist across restarts
                    }
                }
            });
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

    /// The sidebar: everything that isn't the map.
    ///
    /// A docked left column holding the whole action registry (products, layers, tools,
    /// windows — searchable) with the app's own commands below it. It replaces a wall of parallel
    /// entry points; the map keeps nothing but transient status.
    fn sidebar(&mut self, root: &mut egui::Ui, ctx: &egui::Context) {
        let accent = crate::theme::accent(self.settings.theme);
        let entries = self.palette_entries();
        let mut query = std::mem::take(&mut self.layers_query);
        let (mut chosen, mut fly_to) = (None, None);
        let mut opts = ui::layer_options::UiActions::default();
        let mut focus_search = std::mem::take(&mut self.sidebar_focus_search);
        let mut alerts_tab = self.show_alert_panel;
        let (alert_count, _) = self.alert_badge();
        let bounds = self.view_bounds();
        let feats = self.active_alert_features().to_vec();
        let mut muted = self.settings.mute_alerts;
        let mut alert_hit = None;
        // Read before the panel closure: the Layer options callback runs inside a `&mut self`
        // borrow and can only touch plain fields, not `&self` methods.
        let l3_site = self.l3grid_site.clone();
        let tz = self.active_tz();
        let mosaic = self.mosaic_status();
        let mut etop_dbz = self.settings.etop_dbz;
        let mut hide = false;
        egui::Panel::left("sidebar")
            .exact_size(264.0)
            .show(root, |ui| {
                // Title on the same line as the tabs: the name is branding, not a section, and
                // its own row plus separator cost 30 px of every screen height.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Hook Echo-WX")
                            .size(13.0)
                            .strong()
                            .color(accent),
                    );
                    ui.add_space(4.0);
                    // Data | Alerts. `show_alert_panel` is the same flag the bell and the A hotkey
                    // flip, so every entry point lands on the same tab.
                    if ui.selectable_label(!alerts_tab, "Data").clicked() {
                        alerts_tab = false;
                    }
                    let label = if alert_count == 0 {
                        "Alerts".to_string()
                    } else {
                        format!("Alerts ({alert_count})")
                    };
                    if ui.selectable_label(alerts_tab, label).clicked() {
                        alerts_tab = true;
                    }
                    // Collapse, on the row it collapses. The floating button that brings the
                    // panel back lands in the same corner this one sits in.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(egui_phosphor::regular::CARET_LEFT)
                                        .size(14.0),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                            )
                            .on_hover_text("Hide the sidebar")
                            .clicked()
                        {
                            hide = true;
                        }
                    });
                });
                ui.separator();
                if alerts_tab {
                    alert_hit = ui::alert_panel::body(ui, &feats, bounds, &mut muted);
                    return;
                }
                self.product_section(ui, &mut opts);
                ui.separator();
                // A drag rewrites the order in place, so persist it when it moves.
                let order_was = self.settings.layer_order.clone();
                chosen = ui::layers_panel::body(
                    ui,
                    &entries,
                    &mut query,
                    accent,
                    // Leave room for the disclosures under the tree, whatever the window height.
                    (ui.available_height() - 110.0).max(120.0),
                    std::mem::take(&mut focus_search),
                    &mut self.settings.layer_order,
                    |ui| {
                        // Knobs for the layers that are already on, drawn between the Radar group
                        // and the rest. Collapsed by default: the list is still the panel's job.
                        egui::CollapsingHeader::new("Layer options")
                            .default_open(false)
                            .show(ui, |ui| {
                                crate::ui::layer_options::show(
                                    ui,
                                    &mut self.filters,
                                    &mut self.fields,
                                    &mut self.rotation_minutes,
                                    &mut self.hrrr_fcst_hour,
                                    self.hrrr_valid,
                                    tz,
                                    &mut self.env_cape_ml,
                                    &mut self.env_srh_km,
                                    &mut self.env_model,
                                    &mut self.contour_kind,
                                    &mut etop_dbz,
                                    &mut self.snow_hours,
                                    &self.show_tropical,
                                    &mut self.tropical_wind_kt,
                                    &mut self.tropical_surge,
                                    l3_site.as_deref(),
                                    Some(mosaic.as_str()),
                                    &mut opts,
                                );
                            });
                    },
                );
                self.settings.etop_dbz = etop_dbz;
                if self.settings.layer_order != order_was {
                    self.settings.save();
                }
                // Place search folds in here rather than keeping a pill of its own: action
                // matches rank first, and this row is the explicit "I meant a place" answer.
                if !query.trim().is_empty() {
                    ui.add_space(4.0);
                    let w = ui.available_width();
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!(
                                    "{}  Fly to \u{201c}{}\u{201d}",
                                    egui_phosphor::regular::MAP_PIN,
                                    query.trim()
                                ))
                                .size(13.0),
                            )
                            .min_size(egui::vec2(w, 34.0))
                            .corner_radius(10.0),
                        )
                        .on_hover_text("Search the place name and move the map there")
                        .clicked()
                    {
                        fly_to = Some(query.trim().to_string());
                    }
                }
                ui.add_space(4.0);
                // The set-once map knobs, and the app's own commands.
                egui::CollapsingHeader::new("Map")
                    .default_open(false)
                    .show(ui, |ui| self.map_rows(ui, &mut opts));
                egui::CollapsingHeader::new("App")
                    .default_open(false)
                    .show(ui, |ui| self.app_rows(ui));
            });
        self.show_alert_panel = alerts_tab;
        self.settings.mute_alerts = muted;
        if hide {
            self.settings.hide_sidebar = true;
            self.settings.save();
        }
        if let Some((id, lon, lat)) = alert_hit {
            // Fly the active camera to the alert and open its bulletin.
            let cam = &mut self.views[self.active].camera;
            cam.center = crate::render::mercator::lonlat_to_world(lon, lat);
            cam.zoom = cam.zoom.max(8.0);
            self.open_alert_popup(&id);
        }
        // `query` is the live text; `self.layers_query` was taken from at the top of the frame.
        let searched = !query.trim().is_empty();
        self.layers_query = query;
        self.apply_ui_actions(opts, ctx);
        if let Some(a) = chosen {
            // Picking a search hit means you're done searching: clear the query so the tree comes
            // back. Browsing the list without a query is the opposite — you're flipping layers on
            // and off, so it stays put.
            if searched || matches!(a, PaletteAction::OpenWindow(_)) {
                self.layers_query.clear();
            }
            self.apply_palette(a, ctx);
        }
        if let Some(place) = fly_to {
            self.geocode_nav = true;
            self.save_offer = None; // a new search retires the previous offer
            self.place_status = Some(("Searching…".to_string(), Instant::now()));
            let http = self.http.clone();
            let tx = self.geocode_tx.clone();
            let ctx2 = ctx.clone();
            self.spawner.spawn(async move {
                let _ = tx.send(wxdata::geocode::search(&http, &place).await);
                ctx2.request_repaint();
            });
        }
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

    fn timeline_bar(&mut self, root: &mut egui::Ui) {
        use egui_phosphor::regular as ph;
        let accent = crate::theme::accent(self.settings.theme);
        let tz = self.active_tz();
        let fresh = self.views[self.active]
            .volume
            .as_ref()
            .is_some_and(|v| (chrono::Utc::now() - v.time).num_seconds() < 900);
        // Site and data age used to live in the docked status bar; the clock belongs with the clock.
        let site = self.views[self.active]
            .site
            .clone()
            .unwrap_or_else(|| "no site".to_string());
        let age = self.views[self.active].volume.as_ref().map(|v| {
            let secs = (Utc::now() - v.time).num_seconds().max(0);
            format!("({} ago)", humanize(secs))
        });
        let loading = self.views[self.active].loading;
        let mut go_head = false;
        // Soonest rain arrival and the DVR buffer depth ride the pill: both are about time, and
        // both used to sit in an always-on chip in the opposite corner.
        let rain = self
            .rain_eta
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(name, min)| format!("\u{1f327} {name} ~{min:.0} min"));
        let dvr = self.dvr_depth();
        // Edited through a local so the pill closure keeps its single `&mut self.views` borrow.
        let mut loop_frames = self.settings.live_loop_frames;
        egui::Panel::bottom("timeline_bar")
            .exact_size(44.0)
            .show(root, |ui| {
                let t = &mut self.views[self.active].timeline;
                ui.horizontal_centered(|ui| {
                    ui.label(
                        egui::RichText::new(&site)
                            .size(crate::ui::style::FONT_BASE)
                            .strong()
                            .color(egui::Color32::from_gray(238)),
                    );
                    let btn = |ui: &mut egui::Ui, glyph: &str, on: bool| {
                        let fg = if on {
                            accent
                        } else {
                            egui::Color32::from_gray(225)
                        };
                        ui.add(
                            egui::Button::new(egui::RichText::new(glyph).size(18.0).color(fg))
                                .min_size(egui::vec2(30.0, 30.0))
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    };
                    if btn(ui, ph::SKIP_BACK, false) {
                        t.step(-1);
                    }
                    let playing = t.playing;
                    if btn(ui, if playing { ph::PAUSE } else { ph::PLAY }, playing) {
                        t.toggle_play();
                    }
                    if btn(ui, ph::SKIP_FORWARD, false) {
                        t.step(1);
                    }
                    // Live / archive badge: click to re-pin to the newest volume.
                    let (col, text) = if t.following && fresh {
                        (mobile::OMEGA_GREEN, "LIVE".to_string())
                    } else if t.following {
                        (egui::Color32::from_rgb(220, 180, 0), "LIVE".to_string())
                    } else {
                        (
                            egui::Color32::from_gray(150),
                            format!("ARCHIVE {}", t.date.format("%m/%d")),
                        )
                    };
                    let badge = ui.add(
                        egui::Button::new(
                            egui::RichText::new(text)
                                .size(12.0)
                                .strong()
                                .color(egui::Color32::BLACK),
                        )
                        .fill(col)
                        .corner_radius(9.0),
                    );
                    if badge.clicked() {
                        go_head = true;
                    }
                    // Right-click the badge for the knobs that used to sit in the toolbox's
                    // Timeline section: which archive day, and how playback loops.
                    egui::Popup::context_menu(&badge)
                        .align(egui::RectAlign::TOP_START)
                        .show(|ui| {
                            ui.set_min_width(240.0);
                            ui.horizontal(|ui| {
                                ui.label("Date:");
                                if ui.button(egui_phosphor::regular::CARET_LEFT).clicked() {
                                    if let Some(d) = t.date.pred_opt() {
                                        t.date = d;
                                        t.following = false;
                                    }
                                }
                                ui.monospace(t.date.format("%Y-%m-%d").to_string())
                                    .on_hover_text(
                                        "Archive days are UTC days — the S3 buckets are \
                                             bucketed that way",
                                    );
                                let is_today = t.date >= chrono::Utc::now().date_naive();
                                if ui
                                    .add_enabled(
                                        !is_today,
                                        egui::Button::new(egui_phosphor::regular::CARET_RIGHT),
                                    )
                                    .clicked()
                                {
                                    if let Some(d) = t.date.succ_opt() {
                                        t.date = d;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.button("⏮").on_hover_text("First frame").clicked() {
                                    t.go_begin();
                                }
                                ui.checkbox(&mut t.loop_enabled, "Loop");
                            });
                            ui.add(
                                egui::Slider::new(&mut t.speed, 1.0..=15.0)
                                    .suffix(" fps")
                                    .show_value(true),
                            );
                            ui.add(
                                egui::DragValue::new(&mut loop_frames)
                                    .range(2..=30)
                                    .suffix(" frames"),
                            )
                            .on_hover_text(
                                "How many of the newest volumes ▶ cycles through when live",
                            );
                        });
                    // Scrub bar + readout fill the rest of the pill.
                    let observed = t.frames.len();
                    if observed == 0 {
                        ui.weak(if t.listing {
                            "listing volumes…"
                        } else {
                            "(no volumes)"
                        });
                        return;
                    }
                    let readout = match t.forecast_hour() {
                        Some(h) => format!("F+{h}h"),
                        None => t
                            .current()
                            .and_then(|id| id.date_time())
                            .map(|d| crate::timefmt::fmt_clock(d, tz, true))
                            .unwrap_or_default(),
                    };
                    let last = t.slot_count().saturating_sub(1);
                    let mut ph_idx = t.playhead;
                    let slider_w = (ui.available_width() - 92.0).max(80.0);
                    let resp = ui.add_sized(
                        [slider_w, 20.0],
                        egui::Slider::new(&mut ph_idx, 0..=last).show_value(false),
                    );
                    if resp.changed() {
                        t.playhead = ph_idx;
                        t.playing = false;
                        t.following = ph_idx + 1 == observed;
                    }
                    // Mark where observed radar ends and the HRRR forecast tail begins, so a
                    // scrub past the head reads as a model run and not as more radar.
                    if t.slot_count() > observed {
                        let frac = observed as f32 / t.slot_count() as f32;
                        let r = resp.rect;
                        let x = r.left() + frac * r.width();
                        let p = ui.painter_at(r);
                        p.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(x, r.top()),
                                egui::pos2(r.right(), r.bottom()),
                            ),
                            0.0,
                            // Faint enough to stay behind the slider, solid enough to read on
                            // the dark bar — at alpha 36 it was invisible in a screenshot.
                            egui::Color32::from_rgba_unmultiplied(120, 170, 240, 90),
                        );
                        p.line_segment(
                            [egui::pos2(x, r.top()), egui::pos2(x, r.bottom())],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 170, 240)),
                        );
                    }
                    ui.label(
                        egui::RichText::new(readout)
                            .size(12.0)
                            .monospace()
                            .color(egui::Color32::from_gray(215)),
                    );
                    if let Some(r) = &rain {
                        ui.label(
                            egui::RichText::new(r)
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_rgb(110, 180, 240)),
                        )
                        .on_hover_text(
                            "Estimated from storm motion \u{2014} rough for backbuilding storms",
                        );
                    }
                    if dvr > 1 {
                        ui.label(
                            egui::RichText::new(format!("\u{27f2} {dvr}"))
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(150)),
                        )
                        .on_hover_text("Frames buffered in memory for instant replay (R)");
                    }
                    if let Some(age) = &age {
                        ui.label(
                            egui::RichText::new(age)
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(150)),
                        );
                    } else if loading {
                        ui.label(
                            egui::RichText::new("loading…")
                                .size(crate::ui::style::FONT_SM)
                                .color(egui::Color32::from_gray(150)),
                        );
                    }
                });
            });
        self.settings.live_loop_frames = loop_frames;
        if go_head {
            self.views[self.active].timeline.go_head();
        }
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
        ui.horizontal(|ui| {
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
        });
        // Always one line, always present: a wrapping row reflowed between 9- and 14-tilt VCPs and
        // vanished entirely between volumes, which slid the whole layer tree below it up and down.
        // A fixed-height horizontal scroll keeps the sidebar still and scrolls the extra tilts.
        ui.scope(|ui| {
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
        });
        egui::CollapsingHeader::new("Product options")
            .default_open(false)
            .show(ui, |ui| {
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
        let mi = |km: f64| km * 0.621_371;
        // Urgent when the storm will be on top of you soon.
        let urgent = mi(close_km) < 5.0 && close_min < 20.0;
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
                        format!("{:.1} mi {} ({:.0}°)", mi(km), cardinal(bearing), bearing),
                    );
                    if kt > 1.0 {
                        row(
                            ui,
                            "Closest",
                            format!("{:.1} mi in {close_min:.0} min", mi(close_km)),
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
            T::ProbSevere => &mut self.show_probsevere,
            T::Aviation => &mut self.show_aviation,
            T::RangeRings => &mut self.show_range_rings,
            T::Fronts => &mut self.show_fronts,
            T::GlmLightning => &mut self.show_glm,
            T::Wind => &mut self.show_wind,
            T::Sensors => &mut self.show_sensors,
            T::Hodo => &mut self.show_hodo,
            T::Cells => &mut self.filters.show_cells,
            T::Tracks => &mut self.filters.show_tracks,
            T::ArrivalCones => &mut self.filters.show_arrival_cones,
            T::Nowcast => &mut self.filters.show_nowcast,
            T::Tds => &mut self.filters.show_tds,
            T::Couplets => &mut self.filters.show_couplets,
            T::Alerts => &mut self.filters.show_alerts,
            T::Mds => &mut self.filters.show_mds,
            T::Mping => &mut self.show_mping,
            T::Pireps => &mut self.show_pireps,
            T::Recon => &mut self.show_recon,
            T::LinkCameras => &mut self.link_cameras,
        }
    }

    /// Every layer/product/tool/window as a searchable, categorized row. Consumed by the layers
    /// panel (desktop slide-in + mobile sheet) and the Ctrl+K command palette.
    /// The action registry, rebuilt at most once a frame.
    ///
    /// Building it allocates ~150 owned strings, and up to five places render from it in the same
    /// frame (drawer, mobile sheets, legend, palette). Nothing can change it mid-frame — an
    /// action dispatched from one of those lists takes effect on the next one — so a per-frame
    /// memo is both free and honest.
    pub(crate) fn palette_entries(&mut self) -> Vec<PaletteEntry> {
        if let Some((frame, entries)) = &self.palette_cache {
            if *frame == self.frame_nr {
                return entries.clone();
            }
        }
        let entries = self.palette_entries_build();
        self.palette_cache = Some((self.frame_nr, entries.clone()));
        entries
    }

    fn palette_entries_build(&mut self) -> Vec<PaletteEntry> {
        crate::prof_scope!("palette_entries");
        use crate::render::FieldLayer as FL;
        use AppWindow as W;
        use OverlayToggle as T;
        let mut out = Vec::new();
        // Reverse index from the live bindings, so a rebind relabels every row that shows a chip.
        let keys: Vec<(PaletteAction, String)> = crate::hotkeys::active(&self.settings)
            .iter()
            .filter_map(|b| match b.action {
                crate::hotkeys::BindableAction::Palette(p) => {
                    Some((p, crate::hotkeys::pretty(&b.shortcut)))
                }
                _ => None,
            })
            .collect();
        let mut push = |label: &str, category, desc, common, action, on| {
            out.push(PaletteEntry {
                label: label.to_string(),
                category,
                action,
                on,
                desc,
                common,
                key: keys
                    .iter()
                    .find(|(a, _)| *a == action)
                    .map(|(_, k)| k.clone()),
            })
        };

        // --- Radar products (the active pane's moment). ---
        let (cur_moment, cur_srv) = {
            let v = &self.views[self.active];
            (v.moment, v.srv)
        };
        // Every product, plus the storm-relative variant of velocity.
        let rows = crate::products::PRODUCTS
            .iter()
            .map(|p| (p.moment, false, p.name, p.blurb))
            .chain([(
                Moment::Velocity,
                true,
                "Storm-Relative Velocity",
                "Velocity with the storm's own motion subtracted out",
            )]);
        let have = self.available_moments();
        for (m, srv, label, desc) in rows {
            // A product this radar doesn't send is absent, not a row that paints nothing.
            if !have[m.index()] {
                continue;
            }
            let on = cur_moment == m && (m != Moment::Velocity || cur_srv == srv);
            push(
                label,
                "Radar",
                desc,
                true,
                PaletteAction::SetMoment(m, srv),
                Some(on),
            );
        }

        // --- National / model grids. ---
        for (layer, category, label, desc, common) in [
            (
                FL::Mrms,
                "National",
                "MRMS Mosaic",
                "Every radar in the country stitched into one picture",
                true,
            ),
            (
                FL::Mosaic,
                "National",
                "Radar Mosaic",
                "Every nearby radar's own base reflectivity, stitched seamlessly",
                true,
            ),
            (
                FL::Rotation,
                "National",
                "Rotation tracks",
                "Where rotation has passed over the last hour — the tornado-track map",
                true,
            ),
            (
                FL::Mesh,
                "National",
                "MESH hail",
                "Estimated largest hail size each storm is producing",
                true,
            ),
            (
                FL::Lightning,
                "National",
                "Lightning density",
                "How much lightning is striking, and where",
                true,
            ),
            (
                FL::AzShear,
                "National",
                "AzShear (0–2 km)",
                "Low-level rotation strength, right now",
                false,
            ),
            (
                FL::Qpe1h,
                "National",
                "QPE 1-hour",
                "How much rain has fallen in the last hour",
                false,
            ),
            (
                FL::Qpe24h,
                "National",
                "QPE 24-hour",
                "How much rain has fallen in the last day",
                false,
            ),
            (
                FL::PrecipType,
                "National",
                "Precip type",
                "Rain, snow, sleet or freezing rain at the surface",
                false,
            ),
            (
                FL::FlashFlood,
                "National",
                "FLASH flood ARI",
                "How rare this much rain is here — flash-flood risk",
                false,
            ),
            (
                FL::HailSwath,
                "National",
                "Hail swaths (24 h)",
                "Where hail has fallen over the past day",
                false,
            ),
            (
                FL::Vil,
                "National",
                "Digital VIL (L3)",
                "How much water the storm is holding aloft",
                false,
            ),
            (
                FL::EchoTops,
                "National",
                "Echo tops (L3)",
                "How tall the storm is",
                false,
            ),
            (
                FL::Hca,
                "National",
                "Hydrometeor class (L3)",
                "What the radar thinks it's seeing: rain, hail, debris",
                false,
            ),
            (
                FL::VilLocal,
                "Radar",
                "VIL (derived)",
                "Water held aloft, computed from this volume \u{2014} works in archive replay",
                false,
            ),
            (
                FL::VilDensity,
                "Radar",
                "VIL density (derived)",
                "Water aloft per unit storm depth \u{2014} high values mean large hail",
                false,
            ),
            (
                FL::EtopLocal,
                "Radar",
                "Echo tops (derived)",
                "Storm-top height at a threshold you pick, from this volume",
                false,
            ),
            (
                FL::HailMehs,
                "Radar",
                "Max hail size (derived)",
                "Largest hail this storm can be making, from the volume aloft (live only)",
                false,
            ),
            (
                FL::HailPosh,
                "Radar",
                "Severe hail probability (derived)",
                "Odds this storm is producing hail an inch or larger (live only)",
                false,
            ),
            (
                FL::Hrrr,
                "Models",
                "HRRR future radar",
                "Forecast radar picture for the next 18 hours (not observed)",
                true,
            ),
            (
                FL::UpdraftHelicity,
                "Models",
                "Future rotation tracks",
                "Where storms are forecast to rotate \u{2014} scrub the timeline to extend the swath",
                true,
            ),
            (
                FL::SnowAnalysis,
                "National",
                "Snowfall analysis",
                "How much snow actually fell \u{2014} pick the window in layer options",
                false,
            ),
            (
                FL::Snowfall,
                "Models",
                "Forecast snowfall",
                "How much snow is forecast to pile up \u{2014} scrub the timeline to add hours",
                false,
            ),
            (
                FL::Smoke,
                "Models",
                "Wildfire smoke",
                "Forecast smoke near the ground, from active fires",
                false,
            ),
            (
                FL::Cape,
                "Models",
                "CAPE",
                "How much fuel the atmosphere has for storms",
                false,
            ),
            (
                FL::Srh,
                "Models",
                "Storm-relative helicity",
                "How much spin the wind profile can feed a storm",
                false,
            ),
        ] {
            let on = self.fields.get(&layer).is_some_and(|s| s.show);
            push(
                label,
                category,
                desc,
                common,
                PaletteAction::ToggleField(layer),
                Some(on),
            );
        }
        for k in ContourKind::ALL {
            let label = format!("Contours: {}", k.label());
            let on = self.contour_kind == k;
            push(
                &label,
                "Models",
                "Draw this forecast field as labeled contour lines",
                false,
                PaletteAction::SetContours(k),
                Some(on),
            );
        }

        // --- Severe / obs / reference toggles. ---
        for (t, category, label, desc, common) in [
            (
                T::Cells,
                "Severe",
                "Storm cells",
                "Mark each storm the radar is tracking",
                true,
            ),
            (
                T::Alerts,
                "Severe",
                "NWS alerts",
                "Official warning and watch polygons",
                true,
            ),
            (
                T::Couplets,
                "Severe",
                "Rotation couplets",
                "Flag tight rotation that could produce a tornado",
                true,
            ),
            (
                T::Tds,
                "Severe",
                "TDS detection",
                "Flag lofted debris — a tornado is likely on the ground",
                true,
            ),
            (
                T::StormReports,
                "Severe",
                "Storm reports (LSR)",
                "What people on the ground actually reported today",
                true,
            ),
            (
                T::AlertPanel,
                "Severe",
                "Active alerts list",
                "Every alert in view, worst first (the sidebar's Alerts tab)",
                true,
            ),
            (
                T::Tracks,
                "Severe",
                "SCIT forecast tracks",
                "Where each tracked storm is projected to go",
                false,
            ),
            (
                T::ArrivalCones,
                "Severe",
                "Arrival-time cones",
                "When a storm is expected to reach points downstream",
                false,
            ),
            (
                T::Nowcast,
                "Severe",
                "Nowcast (echo extrapolation)",
                "Short-range radar forecast by sliding echoes forward",
                false,
            ),
            (
                T::Mds,
                "Severe",
                "Mesoscale discussions",
                "SPC's notes on where watches may be issued next",
                false,
            ),
            (
                T::ProbSevere,
                "Severe",
                "ProbSevere",
                "Per-storm probability of severe weather, from NOAA/CIMSS",
                false,
            ),
            (
                T::Metar,
                "Obs",
                "Surface obs (METAR)",
                "Temperature, dewpoint and wind at airports",
                true,
            ),
            (
                T::Webcams,
                "Obs",
                "Webcams (FAA + Windy)",
                "Look at the sky through a real camera \u{2014} FAA airports, plus the Windy \
                 network worldwide with a key in Settings",
                false,
            ),
            (
                T::Fires,
                "Severe",
                "Wildfires (WFIGS)",
                "Active fire perimeters and incident points from the interagency fire feed",
                false,
            ),
            (
                T::Aqi,
                "Obs",
                "Air quality (AirNow)",
                "EPA AQI at every monitor in view \u{2014} needs a free AirNow key in Settings",
                false,
            ),
            (
                T::Stations,
                "Obs",
                "Live station cards",
                "Cameras and live telemetry from surface stations, one floating card each",
                false,
            ),
            (
                T::Dat,
                "Obs",
                "Damage surveys (NWS DAT)",
                "What the survey crews found on the ground, rated point by point",
                false,
            ),
            (
                T::Spotters,
                "Obs",
                "Spotter Network",
                "Live positions of storm spotters near the radar",
                true,
            ),
            (
                T::Gauges,
                "Obs",
                "River gauges (NWPS)",
                "River levels and flood stage",
                false,
            ),
            (
                T::Sensors,
                "Obs",
                "Sensor dashboard",
                "Current conditions and 24-hour trends at the nearest station",
                false,
            ),
            (
                T::Hodo,
                "Obs",
                "VAD hodograph",
                "How the wind turns with height above the radar",
                false,
            ),
            (
                T::Tropical,
                "Obs",
                "Tropical (NHC)",
                "Hurricane tracks and forecast cones",
                false,
            ),
            (
                T::Mping,
                "Obs",
                "Crowd reports (mPING)",
                "What people outside say is falling: rain, snow, sleet, freezing rain",
                false,
            ),
            (
                T::Pireps,
                "Obs",
                "Pilot reports (PIREPs)",
                "What pilots actually flew through: turbulence, icing, cloud tops",
                false,
            ),
            (
                T::Recon,
                "Obs",
                "Recon flight track",
                "Hurricane-hunter observations: flight-level and surface wind, measured",
                false,
            ),
            (
                T::Aviation,
                "Obs",
                "Aviation (SIGMET/AIRMET)",
                "Hazard areas for pilots: turbulence, icing, low ceilings",
                false,
            ),
            (
                T::RadarSites,
                "Reference",
                "Radar sites",
                "Show every NEXRAD site; click one to switch radars",
                true,
            ),
            (
                T::Wind,
                "Models",
                "Wind (animated)",
                "HRRR 10 m wind as drifting particles \u{2014} forecast output, CONUS only",
                true,
            ),
            (
                T::GlmLightning,
                "Severe",
                "Satellite lightning (GLM)",
                "Individual flashes from the GOES lightning mapper, fading as they age",
                true,
            ),
            (
                T::Fronts,
                "Reference",
                "Surface fronts (H/L)",
                "The cold, warm and stationary fronts from the national weather map",
                true,
            ),
            (
                T::RangeRings,
                "Reference",
                "Range rings",
                "Distance rings around the radar, every 50 km",
                true,
            ),
            (
                T::LinkCameras,
                "Reference",
                "Link pane cameras",
                "Pan and zoom every pane together",
                false,
            ),
        ] {
            let on = *self.overlay_flag(t);
            push(
                label,
                category,
                desc,
                common,
                PaletteAction::ToggleOverlay(t),
                Some(on),
            );
        }
        push(
            "Cycle basemap",
            "Reference",
            "Switch the map underneath the radar",
            true,
            PaletteAction::CycleBasemap,
            None,
        );
        push(
            "Open this view in Windy",
            "Reference",
            "Open windy.com in your browser, looking at the same place",
            true,
            PaletteAction::OpenInWindy,
            None,
        );
        push(
            "Copy link to this view",
            "Reference",
            "A hookecho:// link to this site, place, zoom and time \u{2014} opens the app here",
            true,
            PaletteAction::CopyViewLink,
            None,
        );
        push(
            "Mute audio alerts",
            "Alerts",
            "Silence every chime and spoken warning without changing your sound choices",
            true,
            PaletteAction::ToggleMute,
            Some(self.settings.mute_alerts),
        );
        push(
            "Sidebar",
            "Reference",
            "This panel \u{2014} hide it and the map runs to the left edge",
            false,
            PaletteAction::ToggleSidebar,
            Some(!self.settings.hide_sidebar),
        );
        push(
            "Timeline bar",
            "Reference",
            "The transport strip under the map \u{2014} hide it for an edge-to-edge map",
            false,
            PaletteAction::ToggleToolbar,
            Some(!self.settings.hide_toolbar),
        );
        push(
            "About Hook Echo-WX",
            "Reference",
            "Version, links, and whether a newer release is out",
            false,
            PaletteAction::OpenWindow(AppWindow::About),
            None,
        );

        // --- Tools, windows, panes. ---
        let tool = self.tool;
        for (t, label, desc, common) in [
            (
                MapTool::Interrogate,
                "Tool: Interrogate",
                "Click anywhere to read the exact radar value",
                true,
            ),
            (
                MapTool::Measure,
                "Tool: Measure",
                "Drag to measure distance and bearing",
                true,
            ),
            (
                MapTool::Marker,
                "Tool: Drop marker",
                "Save a place — home, work, where you're headed",
                true,
            ),
            (
                MapTool::Forecast,
                "Tool: Point forecast",
                "Tap anywhere for that spot's 7-day and hourly forecast",
                true,
            ),
            (
                MapTool::Sounding,
                "Tool: Sounding",
                "Click a point for the model profile plus the nearest balloon sounding",
                false,
            ),
            (
                MapTool::CrossSection,
                "Tool: Cross-section",
                "Drag a line to slice the storm vertically",
                false,
            ),
            (
                MapTool::Chase,
                "Tool: Set chase location",
                "Tell the app where you are, for the chase readout",
                false,
            ),
            (
                MapTool::Climatology,
                "Tool: Tornado climatology",
                "How often tornadoes have hit this spot historically",
                false,
            ),
            (
                MapTool::Draw,
                "Tool: Draw",
                "Scribble on the map — circle the storm you're talking about",
                false,
            ),
        ] {
            push(
                label,
                "Tools",
                desc,
                common,
                PaletteAction::Tool(t),
                Some(tool == t),
            );
        }
        for (w, label, desc, common) in [
            (
                W::Site,
                "Radar site…",
                "Pick which radar you're watching",
                true,
            ),
            (
                W::Settings,
                "Settings…",
                "Theme, units, time display, alert sounds",
                true,
            ),
            (
                W::Markers,
                "Location markers…",
                "Manage your saved places and their alerts",
                false,
            ),
            (
                W::Events,
                "Event library…",
                "Jump to a famous storm and watch it replay",
                false,
            ),
            (
                W::Digest,
                "Storm digest…",
                "A plain-language summary of what's happening now",
                false,
            ),
            (
                W::Afd,
                "Forecast discussion (AFD)…",
                "What the local forecast office is writing",
                false,
            ),
            (
                W::Placefiles,
                "Placefile manager…",
                "Add GRLevelX placefile overlays",
                false,
            ),
            (
                W::LayerManager,
                "Layer manager…",
                "Reorder and set opacity for every layer",
                false,
            ),
            (
                W::Palettes,
                "Color-table editor…",
                "Change the colors a product is drawn with",
                false,
            ),
            (
                W::StormTable,
                "Storm attributes…",
                "Every tracked storm in one sortable table \u{2014} hail size, tops, rotation",
                true,
            ),
            (
                W::Verify,
                "Warning verification…",
                "Score an office's warnings against what actually happened",
                false,
            ),
            (
                W::Cappi,
                "CAPPI slice…",
                "See the storm at one constant altitude",
                false,
            ),
            (
                W::Volume3d,
                "3D volume…",
                "Rotate the storm in three dimensions",
                false,
            ),
            (
                W::Climatology,
                "Tornado climatology…",
                "Historical tornado tracks for this area",
                false,
            ),
            (W::Wizard, "Setup wizard…", "Re-run first-time setup", false),
        ] {
            let on = None;
            push(
                label,
                "Tools",
                desc,
                common,
                PaletteAction::OpenWindow(w),
                on,
            );
        }
        push(
            "Compare 4 tilts",
            "Tools",
            "Four panes of this product at four heights, cameras linked",
            true,
            PaletteAction::AllTilts,
            None,
        );
        let panes = self.views.len();
        for n in [1usize, 2, 4] {
            push(
                &format!("{n} pane{}", if n == 1 { "" } else { "s" }),
                "Tools",
                "Split the window to watch several radars or products at once",
                false,
                PaletteAction::SetPanes(n),
                Some(panes == n),
            );
        }
        push(
            "Reload",
            "Tools",
            "Fetch the latest data again",
            true,
            PaletteAction::Reload,
            None,
        );
        push(
            "Jump to live",
            "Tools",
            "Snap back to the newest scan",
            true,
            PaletteAction::GoLive,
            None,
        );
        push(
            "Instant replay (DVR)",
            "Tools",
            "Replay the scans already in memory",
            false,
            PaletteAction::InstantReplay,
            None,
        );
        out
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
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.show = !s.show;
                }
            }
            PaletteAction::ToggleOverlay(t) => {
                let f = self.overlay_flag(t);
                *f = !*f;
                // These feed the assembled feature set rather than a painter flag.
                use OverlayToggle as T;
                if matches!(
                    t,
                    T::Tropical | T::ProbSevere | T::Aviation | T::Alerts | T::Mds | T::Fires
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
            PaletteAction::SetPanes(n) => self.set_pane_count(n),
            PaletteAction::AllTilts => self.apply_all_tilts(),
            PaletteAction::CycleBasemap => {
                let (mb, mt) = (
                    !self.settings.mapbox_key.is_empty(),
                    !self.settings.maptiler_key.is_empty(),
                );
                let v = &mut self.views[self.active];
                v.basemap = v.basemap.next(mb, mt);
            }
            PaletteAction::ToggleMute => self.apply_action(BindableAction::ToggleMute, ctx),
            PaletteAction::ToggleToolbar => {
                self.settings.hide_toolbar = !self.settings.hide_toolbar;
                self.settings.save();
            }
            PaletteAction::ToggleSidebar => {
                self.settings.hide_sidebar = !self.settings.hide_sidebar;
                self.settings.save();
            }
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
                let link = goto_link(
                    v.site.as_deref().unwrap_or(""),
                    lon,
                    lat,
                    v.camera.zoom,
                    time,
                );
                ctx.copy_text(link.clone());
                self.warning_banners
                    .push(("Link copied".to_string(), link, Instant::now()));
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
                } else if self
                    .fields
                    .get(&crate::render::FieldLayer::Cape)
                    .is_some_and(|s| s.show)
                {
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
                W::Verify => self.open_verify(),
                W::Volume3d => self.build_volume3d(),
                W::Climatology => {
                    self.climo_open = true;
                    self.load_climatology();
                }
                W::LayerManager => self.layer_window_open = true,
                W::Wizard => self.wizard.start(),
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
                OverlayMsg::Alerts(f) => {
                    self.detect_new_warnings(&f);
                    self.alert_features = f;
                }
                OverlayMsg::Mds(f) => self.md_features = f,
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
                OverlayMsg::Fronts(a) => self.fronts = Some(a),
                OverlayMsg::FreezingLevels(h0, hm20) => {
                    if let Some(site) = self.views[self.active].site.clone() {
                        self.freezing = Some((site, h0, hm20));
                    }
                }
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
                OverlayMsg::Metar(obs) => self.metars = obs,
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
        if !self.fields.get(&FL::Mosaic).is_some_and(|s| s.show) {
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
        if (1..=3).contains(&self.filters.outlook_day) {
            v.extend(
                self.outlook_features[(self.filters.outlook_day - 1) as usize]
                    .iter()
                    .cloned(),
            );
        }
        if self.filters.show_mds {
            v.extend(self.md_features.iter().cloned());
        }
        if (1..=3).contains(&self.filters.wssi_day) {
            v.extend(self.wssi_features.iter().cloned());
        }
        if (1..=3).contains(&self.filters.ero_day) {
            v.extend(self.ero_features.iter().cloned());
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
        let range = self.view_range_nmi();
        let now = Utc::now();
        let mut out = Vec::new();
        // Configured placefiles in Layer-Manager order, then plugin output on top of them: a
        // plugin is something the user wrote for this session, so it should not be buried.
        let sources = self
            .settings
            .placefiles
            .iter()
            .map(|c| (c.url.clone(), c.opacity))
            .chain(
                self.settings
                    .plugins
                    .iter()
                    .map(|p| (format!("plugin:{}", p.name), 1.0)),
            );
        for (url, opacity) in sources {
            let Some((li, lp)) = self
                .placefiles
                .iter()
                .enumerate()
                .find(|(_, lp)| lp.url == url)
            else {
                continue;
            };
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
                out.push((it, opacity, li));
            }
        }
        out
    }

    /// Owned labels/markers for the visible placefile items (drawn by the egui painter).
    /// Icons resolve their sheet cell here, so the painter just blits a quad.
    fn placefile_labels(&self) -> Vec<PlaceLabel> {
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
                    Err(e) => log::warn!("icon sheet {url} failed: {e}"),
                }
            });
        }
    }

    /// Re-tessellate the overlay when its set or the zoom bucket changed.
    fn sync_overlay(&mut self) {
        crate::prof_scope!("sync_overlay");
        let items = self.visible_placefile_items();
        if self.overlays.is_empty() && items.is_empty() {
            self.overlay_ready = false;
            return;
        }
        let zoom = self.views[self.active].camera.zoom;
        let bucket = (zoom * 2.0).round() as i32;
        if self.overlay_gen != self.built_gen || bucket != self.built_zoom_bucket {
            let mut geom = overlay_build::build(&self.overlays, zoom);
            let pf: Vec<(&wxdata::placefile::PlaceItem, f32)> =
                items.iter().map(|(it, op, _)| (*it, *op)).collect();
            overlay_build::append_placefiles(&mut geom, &pf, zoom);
            self.overlay_ready = !geom.indices.is_empty();
            self.pending_overlay = Some(OverlayUpload {
                vertices: geom.vertices,
                indices: geom.indices,
            });
            self.built_gen = self.overlay_gen;
            self.built_zoom_bucket = bucket;
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
        if wxdata::tdwr::is_tdwr(&site) {
            // Terminal radars have no Level 2 feed and no archive: one synthesized volume per
            // poll, from the newest Level 3 tilt products.
            let http = self.http.clone();
            self.spawner.spawn(async move {
                let msg = match wxdata::tdwr::fetch_volume(&http, &site).await {
                    Ok((name, _, _)) if current_name.as_deref() == Some(name.as_str()) => {
                        DataMsg::UpToDate {
                            view: view_idx,
                            site,
                        }
                    }
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
                        let fetched = match level2::download_scan(id).await {
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
                                    level2::download_scan(p)
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
        while let Ok(msg) = self.msg_rx.try_recv() {
            let idx = msg.view();
            // LiveEnded must be handled even after a site change (to drop the stream handle).
            if matches!(msg, DataMsg::LiveEnded { .. }) {
                #[cfg(not(target_arch = "wasm32"))]
                if let DataMsg::LiveEnded { view, .. } = msg {
                    if self
                        .live_stream
                        .as_ref()
                        .is_some_and(|(v, _, _)| *v == view)
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
                        && !v.site.as_deref().is_some_and(wxdata::tdwr::is_tdwr)
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
                        v.volume = Some(Volume::new(scan, name, time));
                    }
                    v.loading = false;
                    v.error = None;
                    v.clamp_tilt();
                    v.clamp_moment();
                    self.pane_shown.remove(&view);
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
                }
                DataMsg::UpToDate { view, .. } => self.views[view].loading = false,
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
    /// The web build has no chunk streamer (see `wxdata::live`); it polls for whole volumes.
    #[cfg(target_arch = "wasm32")]
    fn manage_stream(&mut self, _ctx: &egui::Context) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn manage_stream(&mut self, ctx: &egui::Context) {
        let idx = self.active;
        let (want, site, base) = {
            let v = &self.views[idx];
            // Stream only while pinned to the live head; scrubbing pauses it. A live loop is
            // suppressed too — the loop shows past frames, so interval polling (not the sweep
            // stream) carries new-volume arrival. ponytail: stream resumes on pause / go_head.
            // TDWRs have no Level 2 chunk stream to merge — asking for one downloads a
            // WSR-88D-shaped file that isn't there and decodes garbage.
            let want = v.timeline.following
                && !v.timeline.live_looping()
                && v.site.as_deref().is_some_and(|s| !wxdata::tdwr::is_tdwr(s))
                && v.volume.is_some();
            (
                want,
                v.site.clone(),
                // The streamer needs its own mutable base to merge chunks into, so this is the
                // one place a decoded volume is still deep-copied — once per stream start.
                v.volume.as_ref().map(|vol| (*vol.scan).clone()),
            )
        };

        // Abort an existing stream if it no longer matches the active view/site or isn't wanted.
        if let Some((sv, ss, handle)) = &self.live_stream {
            if !want || *sv != idx || Some(ss.as_str()) != site.as_deref() {
                handle.abort();
                self.live_stream = None;
            }
        }

        if want && self.live_stream.is_none() {
            let due = self
                .last_stream_attempt
                .is_none_or(|t| t.elapsed().as_secs() >= 60);
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
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_stream(
        &self,
        view_idx: usize,
        site: String,
        base: Scan,
        ctx: egui::Context,
    ) -> tokio::task::JoinHandle<()> {
        let tx = self.msg_tx.clone();
        // The one spawn whose handle is kept: the live streamer is aborted on a site change.
        self._rt.spawn(async move {
            let end_site = site.clone();
            let cb_tx = tx.clone();
            let cb_ctx = ctx.clone();
            let cb_site = site.clone();
            let res = live::stream(site, base, crate::platform::activity::is_active, move |u| {
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
            });
            ctx.request_repaint();
        })
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
            A::ToggleAlertPanel => self.show_alert_panel = !self.show_alert_panel,
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
                if self.settings.hide_sidebar {
                    self.settings.hide_sidebar = false;
                    self.settings.save();
                }
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

    /// Per-frame per-pane: react to site changes, keep the timeline current, and (for the active
    /// pane) manage the live stream. Each pane fetches its own volume via its view index.
    fn sync_pane(&mut self, idx: usize, ctx: &egui::Context) {
        // Site change: clear the old volume, recenter, and (if a real site) refetch.
        let site_changed = self.views[idx].site != self.views[idx].loaded_site;
        if site_changed {
            let v = &mut self.views[idx];
            v.loaded_site = v.site.clone();
            v.volume = None;
            v.moments_seen = [false; 6];
            v.error = None;
            // Clear a stuck in-flight flag: if the previous site's fetch is still running when the
            // site changes, its result is dropped on arrival (site mismatch) without clearing
            // `loading`, which would then block the new site's fetch forever ("no volume").
            v.loading = false;
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
        self.views[idx].timeline.live_window = self.settings.live_loop_frames.max(1);
        self.views[idx].timeline.tick();
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
            // TDWRs have no archive to list; their timeline stays empty and always live.
            let need = v.site.as_deref().is_some_and(|s| !wxdata::tdwr::is_tdwr(s))
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
                        let v = &mut self.views[idx];
                        v.volume = Some(Volume::new(scan, name, time));
                        v.loading = false;
                        v.error = None;
                        v.clamp_tilt();
                        v.clamp_moment();
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
        self.spawner.spawn(async move {
            let name = id.name().to_string();
            let time = id.date_time().unwrap_or_else(Utc::now);
            let msg = match level2::download_scan(id).await {
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
                Err(e) => log::warn!("list frames {site} {date}: {e}"),
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
        let have = self.views[data].volume.as_ref().unwrap().moments;
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
        let name = self.views[data].volume.as_ref().unwrap().name.clone();
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
            self.palettes.gen,
            uv_key,
            dealias,
        );
        if self.pane_shown.get(&idx) == Some(&key) {
            return (None, true);
        }
        let table = self.palettes.table(moment);
        let upload = {
            let vol = self.views[data].volume.as_mut().unwrap();
            // No tilts yet (a volume that has only just started arriving) is a "wait", not a
            // failure: erroring here put "tilt 0 out of range" on the map once per volume.
            if vol.elevations.is_empty() {
                return (None, true);
            }
            vol.binned(moment, tilt, dealias)
                .map(|s| to_upload(s, table, threshold, smooth, storm_uv))
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
        placefile_labels: &[PlaceLabel],
    ) {
        use crate::tiles::BasemapStyle;
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
            self.last_gesture_end = Some(std::time::Instant::now());
        }
        // A finger still down after the other lifted is the tail of a pinch, not a new drag or a
        // tap on the map. 150 ms is long enough to cover a normal two-finger lift and short
        // enough to be invisible when you really did mean to tap.
        let gesture_tail = self
            .last_gesture_end
            .is_some_and(|t| t.elapsed().as_millis() < 150);
        let quiet = gesture.is_none() && !gesture_tail;
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
            self.views[idx].camera.pan_pixels(d.x, d.y);
            self.follow_cell = None; // a manual pan takes over the camera
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            if let Some(pos) = response.hover_pos() {
                if prect.contains(pos) {
                    self.active = idx;
                    let cursor = (pos.x - prect.left(), pos.y - prect.top());
                    self.views[idx]
                        .camera
                        .zoom_at(scroll as f64 * 0.005, cursor, vp);
                }
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
                    self.views[idx].camera.pan_pixels(t.x, t.y);
                    self.follow_cell = None; // a manual pan takes over the camera (pinch-zoom does not)
                }
            }
        }
        if response.clicked() && quiet {
            self.active = idx;
            if let Some(pos) = response.interact_pointer_pos() {
                let cam = self.views[idx].camera;
                let px = (pos.x - prect.left(), pos.y - prect.top());
                let w = cam.screen_to_world(px, vp);
                let (lon, lat) = crate::render::mercator::world_to_lonlat(w.0, w.1);
                // Your own markers win over everything else on the map: you put them at a place you
                // chose, so a tap there means that pin, not whatever the radar drew underneath.
                // Nearest wins, so clustered markers stay individually reachable.
                let marker_hit = matches!(self.tool, MapTool::Interrogate | MapTool::Marker)
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
                // Interrogate + a click on a radar-site ring switches radars (storm features win,
                // handled inside try_pick_site). Consumes the click so no popup opens underneath.
                let picked_site = marker_hit.is_none()
                    && self.tool == MapTool::Interrogate
                    && self.show_radar_sites
                    && self.try_pick_site(idx, pos, cam, prect, vp);
                // A camera site under an interrogate click wins over everything below it: the
                // markers are sparse, so a tap on one is never ambiguous.
                let cam_site = (marker_hit.is_none()
                    && !picked_site
                    && self.tool == MapTool::Interrogate
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
                    && self.tool == MapTool::Interrogate
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
                    && self.tool == MapTool::Interrogate
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
                match self.tool {
                    // Also catches the drop tool: a second marker within a finger's width of an
                    // existing one is never what someone meant, and this makes a stray drop undoable.
                    _ if marker_hit.is_some() => {
                        self.marker_popup = marker_hit;
                        self.cell_popup = None;
                        self.detail = None;
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
                            name: format!("Marker {n}"),
                            lat,
                            lon,
                            icon: None,
                            alert_radius_mi: crate::settings::default_alert_radius_mi(),
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
                    MapTool::Interrogate => {
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
        let raster_bias = if self.views[idx].basemap.tiles_are_512() {
            // 512-px providers already carry the extra detail in the tile itself; biasing on top
            // would fetch four of them per screen tile for nothing.
            0.0
        } else {
            ctx.pixels_per_point()
                .max(1.0)
                .log2()
                .round()
                .clamp(0.0, bias_cap) as f64
        };
        let visible = if is_raster {
            let vis = self.tiles.visible(&cam, vp, raster_bias);
            self.tiles.request_missing(&vis);
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
            let labels: Vec<crate::vector_tiles::PlaceLabel> = self
                .vtiles
                .labels_for(ids.iter())
                .into_iter()
                .cloned()
                .collect();
            (if is_vector { ids } else { Vec::new() }, labels, vis)
        };
        // Drain finished fetches once (on the first pane) — they upload into the shared cache.
        // Eviction lives with the tile manager (it also owns `requested`/`uploaded`), and only
        // the first pane runs it so a multi-pane frame doesn't evict what a later pane needs.
        let drop_tiles = if first && is_raster {
            self.tiles.touch_visible(&visible)
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
        let field_uploads: Vec<(crate::render::FieldLayer, crate::render::MrmsUpload)> = if first {
            self.fields
                .iter_mut()
                .filter_map(|(k, s)| s.pending.take().map(|u| (*k, u)))
                .collect()
        } else {
            Vec::new()
        };
        let field_draws: Vec<(crate::render::FieldLayer, f32)> = self
            .fields
            .iter()
            .filter(|(_, s)| s.show)
            .map(|(k, _)| {
                (
                    *k,
                    self.settings.field_opacity.get(k).copied().unwrap_or(1.0),
                )
            })
            .collect();

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
            overlay_upload: if first {
                self.pending_overlay.take()
            } else {
                None
            },
            draw_overlay: self.overlay_ready,
            field_uploads,
            field_draws,
            clear_tiles,
            drop_tiles,
            new_vector_tiles,
            visible_vector,
            clear_vector,
            drop_vector_tiles,
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
        let tds_hits = if self.filters.show_tds && idx == self.active {
            self.compute_tds(idx)
        } else {
            Vec::new()
        };
        let couplets = if self.filters.show_couplets && idx == self.active {
            self.compute_couplets(idx)
        } else {
            Vec::new()
        };
        if idx == self.active {
            self.check_rain_arrival();
        }

        // --- Painter overlays (clipped to this pane) ---
        let painter = ui.painter_at(prect);
        let view = &self.views[idx];
        let basemap = view.basemap;

        // City/town labels, overlaid on every basemap. On raster (satellite) the baked-in labels
        // are faint over imagery + echoes, so we draw crisp white text with a solid black halo;
        // vector basemaps use their palette's label colors. Bigger fonts + an 8-way halo read well.
        if !vlabels.is_empty() {
            let (text_col, halo_col, big) = if is_vector {
                let st = crate::basemap_style::style(basemap == BasemapStyle::Dark);
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
                vlabels.iter().filter(|l| l.city || z >= 9.0).collect();
            labels.sort_by_key(|l| (!l.city, l.rank));
            let mut placed: Vec<egui::Rect> = Vec::new();
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
                if placed.iter().any(|q| q.intersects(r)) {
                    continue;
                }
                placed.push(r);
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

        // Animated wind particles.
        //
        // Painter shapes land ABOVE the radar and there is no cheap way under it — `record_pane`
        // finishes the whole `MapCallback` before this runs, so getting beneath would need a new
        // GPU draw slot. Thin lines and a low alpha instead, halved again over a reflectivity
        // product; Windy layers over its own radar exactly this way.
        if self.show_wind {
            // Past zoom 12 the 0.04 degree regrid goes visibly piecewise-linear, so fade out
            // rather than present interpolation artifacts as if they were eddies.
            let zoom_fade = (13.0 - cam.zoom).clamp(0.0, 1.0) as f32;
            let v = &self.views[idx];
            let over_radar = v.show_radar
                && matches!(v.moment, wxdata::level2::Moment::Reflectivity)
                && v.volume.is_some();
            // Scrubbed to a past frame, while these grids are for the current hour: the two layers
            // now disagree about what time it is. Dim rather than lie confidently — the same
            // treatment the HRRR field layers already get.
            let off_live = !v.timeline.following;
            let alpha =
                zoom_fade * if over_radar { 0.7 } else { 1.0 } * if off_live { 0.4 } else { 1.0 };
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
                let mut etas: Vec<(f64, String)> = Vec::new();
                for c in self.active_storm_cells() {
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
                    for m in &self.settings.markers {
                        if let Some(min) = crate::geo::arrival_eta_min(
                            [c.lon, c.lat],
                            dir,
                            kt,
                            [m.lon, m.lat],
                            HALF_ANGLE,
                            LEAD_MIN,
                        ) {
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
                        let galley = painter.layout_no_wrap(
                            text.clone(),
                            font.clone(),
                            egui::Color32::WHITE,
                        );
                        let anchor = egui::pos2(prect.left() + 8.0, y);
                        let bg = egui::Rect::from_min_size(
                            anchor,
                            galley.size() + egui::vec2(10.0, 4.0),
                        );
                        painter.rect_filled(
                            bg,
                            3.0,
                            egui::Color32::from_rgba_unmultiplied(150, 30, 30, 210),
                        );
                        painter.text(
                            anchor + egui::vec2(5.0, 2.0),
                            egui::Align2::LEFT_TOP,
                            &text,
                            font.clone(),
                            egui::Color32::WHITE,
                        );
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
                    let text = format!(
                        "◈ NOWCAST +{} min — echo extrapolated from storm motion",
                        self.filters.nowcast_lead_min
                    );
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
                if c.kind == CellKind::Storm && !c.id.is_empty() {
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
                        Some(t) => format!("{:.0}F", t * 9.0 / 5.0 + 32.0),
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
                // 230 km is at most ~2.1° of latitude and, at CONUS latitudes, under 3° of
                // longitude — a cheap box rejects almost every spotter before the haversine runs.
                let (max_dlon, max_dlat) = (3.0, 2.1);
                for sp in &self.spotters {
                    if (sp.lon - site_pos[0]).abs() > max_dlon
                        || (sp.lat - site_pos[1]).abs() > max_dlat
                    {
                        continue;
                    }
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
                }
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
                for mk in &self.settings.markers {
                    if let Some(t) = crate::geo::arrival_eta_min(
                        origin,
                        heading as f32,
                        m.kt,
                        [mk.lon, mk.lat],
                        22.5,
                        90.0,
                    ) {
                        etas.push((t, format!("⚠ {} — {} in {:.0} min", mk.name, a.event, t)));
                    }
                }
            }
            if idx == self.active && !etas.is_empty() {
                etas.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let font = egui::FontId::proportional(12.0);
                let mut y = prect.top() + 64.0;
                for (_, line) in etas.iter().take(6) {
                    let galley =
                        painter.layout_no_wrap(line.clone(), font.clone(), egui::Color32::WHITE);
                    let anchor = egui::pos2(prect.left() + 8.0, y);
                    let bg =
                        egui::Rect::from_min_size(anchor, galley.size() + egui::vec2(10.0, 4.0));
                    painter.rect_filled(
                        bg,
                        3.0,
                        egui::Color32::from_rgba_unmultiplied(150, 30, 30, 210),
                    );
                    painter.text(
                        anchor + egui::vec2(5.0, 2.0),
                        egui::Align2::LEFT_TOP,
                        line,
                        font.clone(),
                        egui::Color32::WHITE,
                    );
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
            obs.sort_by(|a, b| {
                b.wspd_kt
                    .partial_cmp(&a.wspd_kt)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
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
                        painter.text(
                            p + egui::vec2(-6.0, -6.0),
                            egui::Align2::RIGHT_BOTTOM,
                            format!("{:.0}", t * 9.0 / 5.0 + 32.0),
                            f.clone(),
                            egui::Color32::from_rgb(240, 90, 90),
                        );
                    }
                    if let Some(d) = ob.dewp_c {
                        painter.text(
                            p + egui::vec2(-6.0, 6.0),
                            egui::Align2::RIGHT_TOP,
                            format!("{:.0}", d * 9.0 / 5.0 + 32.0),
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
                    response.clone().show_tooltip_text(&ob.raw);
                }
            }
        }
        // ponytail: °F hardcoded (US station-plot convention); wire to the Units setting if asked.

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
            let mut placed: Vec<egui::Rect> = Vec::new();
            for g in &self.gauges {
                let w = crate::render::mercator::lonlat_to_world(g.lon, g.lat);
                let (sx, sy) = cam.world_to_screen(w, vp);
                let p = egui::pos2(prect.left() + sx, prect.top() + sy);
                if !prect.contains(p) {
                    continue;
                }
                // Greedy declutter: skip droplets that would overlap one already placed.
                let cell = egui::Rect::from_center_size(p, egui::vec2(15.0, 15.0));
                if placed.iter().any(|r| r.intersects(cell)) {
                    continue;
                }
                placed.push(cell);
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
                    let mut tip = format!("{} ({})\n{stage} — {}", g.name, g.lid, glabel(g.cat));
                    if let Some(f) = g.forecast_ft {
                        tip.push_str(&format!("\nFcst: {f:.1} ft ({})", glabel(g.forecast_cat)));
                    }
                    response.clone().show_tooltip_text(tip);
                }
            }
        }

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
                        let pts: Vec<egui::Pos2> = storm
                            .points
                            .iter()
                            .map(|p| to_screen(p.lon, p.lat))
                            .collect();
                        painter.add(egui::Shape::line(
                            pts,
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(235, 235, 235)),
                        ));
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
                            painter.text(
                                sp + egui::vec2(6.0, -2.0),
                                egui::Align2::LEFT_CENTER,
                                cat,
                                egui::FontId::proportional(10.0),
                                col,
                            );
                        }
                    }
                    // Current position: bold storm name with a dark halo.
                    let cp = to_screen(storm.lon, storm.lat);
                    if prect.contains(cp) {
                        let (_, rgb) = wxdata::tropical::saffir_simpson(storm.intensity_kt);
                        let col = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                        painter.circle_filled(cp, 5.0, col);
                        painter.circle_stroke(
                            cp,
                            5.0,
                            egui::Stroke::new(1.5, egui::Color32::BLACK),
                        );
                        let font = egui::FontId::proportional(13.0);
                        for off in [
                            egui::vec2(1.0, 1.0),
                            egui::vec2(-1.0, -1.0),
                            egui::vec2(1.0, -1.0),
                            egui::vec2(-1.0, 1.0),
                        ] {
                            painter.text(
                                cp + egui::vec2(8.0, -8.0) + off,
                                egui::Align2::LEFT_BOTTOM,
                                &storm.name,
                                font.clone(),
                                egui::Color32::BLACK,
                            );
                        }
                        painter.text(
                            cp + egui::vec2(8.0, -8.0),
                            egui::Align2::LEFT_BOTTOM,
                            &storm.name,
                            font,
                            egui::Color32::WHITE,
                        );
                    }
                }
            }
        }

        // HRRR "future radar" banner — unmistakable that this is model forecast, not observation.
        if idx == self.active
            && self
                .fields
                .get(&crate::render::FieldLayer::Hrrr)
                .is_some_and(|s| s.show)
        {
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
            let show_labels = cam.zoom >= 5.0;
            for s in wxdata::sites::all() {
                let w =
                    crate::render::mercator::lonlat_to_world(s.longitude as f64, s.latitude as f64);
                let (sx, sy) = cam.world_to_screen(w, vp);
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
                if show_labels {
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
                let mi = km * 0.621_371; // statute miles
                let txt = format!("{mi:.1} mi  @ {brg:.0}°");
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
            let mut y = 0.0;
            // Whichever gridded layer the user actually sees on top — the last enabled one in
            // paint order — gets its scale keyed underneath. Without this, MESH/QPE/VIL and the
            // categorical classifications were unlabeled color.
            if let Some(top) = crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .rev()
                .find(|l| self.fields.get(l).is_some_and(|s| s.show))
            {
                y += ui::legend::draw_field(&painter, prect, *top, y);
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
        {
            ui.label(egui::RichText::new("View").strong());
            {
                let v = &mut self.views[self.active];
                let mut on = v.basemap != crate::tiles::BasemapStyle::None;
                if ui.checkbox(&mut on, "Basemap").changed() {
                    v.basemap = if on {
                        crate::tiles::BasemapStyle::Dark
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
            // Desktop streams from a local gpsd; Android polls the system LocationManager
            // over JNI (see platform.rs). Both feed the same `gps_rx` channel.
            if self.gps_rx.is_none() {
                let (label, tip) = if cfg!(target_os = "android") {
                    (
                        "Enable GPS (chase)",
                        "Follow your device's position (asks for the location permission)",
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
                    "Broadcast your GPS fix to other Hook Echo instances, and show theirs",
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
                    self.screenshot_pending = Some(ShotDest::File(path));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Screenshot(
                            egui::UserData::default(),
                        ));
                }
            }
            if ui.button("Copy view to clipboard").clicked() {
                self.screenshot_pending = Some(ShotDest::Clipboard);
                ui.ctx()
                    .send_viewport_cmd(
                        egui::ViewportCommand::Screenshot(egui::UserData::default()),
                    );
            }
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
            ui.weak("Hook Echo-WX — NEXRAD radar viewer");
            ui.weak("github.com/d4vid87/hookecho");
            if ui.button("Setup wizard…").clicked() {
                self.wizard.start();
            }
            if ui.button("Exit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
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
        match self
            .settings
            .export_bundle()
            .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()))
        {
            Ok(()) => self.toast(
                ToastKind::Success,
                format!("Settings saved to {}", path.display()),
            ),
            Err(e) => {
                log::warn!("settings export failed: {e}");
                self.toast(ToastKind::Error, format!("Settings export failed: {e}"));
            }
        }
    }

    /// Import a settings bundle (rfd open dialog). The next-frame dirty-diff reloads palettes
    /// and persists, and the UI (theme, layers, markers…) updates live from the new settings.
    fn import_settings_bundle(&mut self) {
        let Some(path) = crate::dialog::open_path("JSON", &["json"]) else {
            return;
        };
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
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
                LoopFormat::Gif => crate::loopexport::encode_gif(&le.frames, 200, &le.dest),
                LoopFormat::Mp4 => crate::loopexport::encode_mp4(&le.frames, 5, &le.dest),
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
            }
        }
    }

    /// One-time callouts pointing at the three floating surfaces a newcomer has to find: the
    /// search pill, the Layers button, and the timeline.
    ///
    /// Existing users see these once too — the chrome is new to them as well, which is the point.
    /// ponytail: three hardcoded callouts, no tour framework; a real tour can come if the set grows.
    fn coach_marks(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        if !self.settings.setup_done || self.settings.coach_done || self.wizard.open {
            return;
        }
        let accent = crate::theme::accent(self.settings.theme);
        // Any real interaction — a click, or a touch gesture — means the cards have been read.
        let done = ctx.input(|i| i.pointer.any_click() || i.multi_touch().is_some());
        // Mobile has its own chrome (dock + sheets) and its own gestures, so it gets its own
        // three callouts rather than pointing at pills that aren't there.
        let marks: &[(&str, egui::Align2, egui::Vec2, &str)] = if cfg!(target_os = "android") {
            &[
                (
                    "coach_m_dock",
                    egui::Align2::CENTER_BOTTOM,
                    egui::vec2(0.0, -110.0),
                    "Everything lives down here: play the loop, pick layers and products, change radar site.",
                ),
                (
                    "coach_m_map",
                    egui::Align2::CENTER_CENTER,
                    egui::vec2(0.0, -60.0),
                    "Pinch to zoom, drag to pan. Long-press anywhere on the map for what's there — alerts, storms, distance.",
                ),
                (
                    "coach_m_time",
                    egui::Align2::CENTER_BOTTOM,
                    egui::vec2(0.0, -240.0),
                    "This row is the timeline: tap the frame count to scrub back through the storm, or jump to live.",
                ),
            ]
        } else {
            &[
            (
                "coach_sidebar",
                egui::Align2::LEFT_TOP,
                egui::vec2(340.0, 60.0),
                "Everything lives in this sidebar: every product, layer and tool, each with a plain-English description. Ctrl+K jumps to its search.",
            ),
            (
                "coach_product",
                egui::Align2::LEFT_BOTTOM,
                egui::vec2(340.0, -60.0),
                "The current product, its tilt, and its expert knobs live at the top of the sidebar.",
            ),
            (
                "coach_timeline",
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -14.0),
                "This is the timeline — play the loop, scrub back through the storm, jump to live.",
            ),
            ]
        };
        for (id, align, offset, text) in marks {
            let (id, align, offset, text) = (*id, *align, *offset, *text);
            egui::Area::new(egui::Id::new(id))
                .constrain_to(self.chrome_rect)
                .anchor(align, offset)
                .order(egui::Order::Foreground)
                // The map card sits dead center on a phone, and an interactable layer there makes
                // every pinch a no-op until it's dismissed. The cards take no input at all; any
                // tap or gesture anywhere dismisses the whole set (below).
                .interactable(false)
                .show(ctx, |ui| {
                    style::glass(ui, 240)
                        .stroke(egui::Stroke::new(1.5, accent))
                        .show(ui, |ui| {
                            ui.set_max_width(300.0);
                            ui.label(
                                egui::RichText::new(text)
                                    .size(style::FONT_BASE)
                                    .color(egui::Color32::from_gray(238)),
                            );
                            ui.label(
                                egui::RichText::new("Tap anywhere to dismiss")
                                    .size(style::FONT_SM)
                                    .color(accent),
                            );
                        });
                });
        }
        if done {
            self.settings.coach_done = true;
        }
    }

    /// Bottom-right info chip: zoom, cursor position, DVR depth, and the active tool's hint.
    ///
    /// This is what the docked status bar used to hold. A full-width bar for four short readouts
    /// cost a strip of map on every frame; the chip floats over the map instead, and the site and
    /// volume time it also carried now live in the timeline pill where the clock belongs.
    /// The way back to a hidden sidebar: one floating button in the corner it used to occupy.
    /// Nothing else on the map reaches the layer list, so this is not optional chrome.
    fn sidebar_button(&mut self, ctx: &egui::Context) {
        if !self.settings.hide_sidebar {
            return;
        }
        egui::Area::new(egui::Id::new("sidebar_button"))
            .constrain_to(self.chrome_rect)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(10.0, 10.0))
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 238).show(ui, |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(egui_phosphor::regular::LIST).size(18.0),
                            )
                            .min_size(egui::vec2(30.0, 30.0))
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        )
                        .on_hover_text("Show the sidebar")
                        .clicked()
                    {
                        self.settings.hide_sidebar = false;
                        self.settings.save();
                    }
                });
            });
    }

    fn info_chip(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use egui_phosphor::regular as ph;
        // Icon, tool name, and what to do with it — an armed tool should look armed, not leave a
        // sentence of instructions as the only sign anything changed.
        let (glyph, name, hint) = match self.tool {
            MapTool::Measure => (ph::RULER, "Measure", "click two points"),
            MapTool::Marker => (ph::MAP_PIN, "Drop marker", "click the map"),
            MapTool::CrossSection => (ph::CHART_LINE, "Cross-section", "click two points"),
            MapTool::Sounding => (ph::THERMOMETER_SIMPLE, "Sounding", "click a point"),
            MapTool::Forecast => (ph::CLOUD_SUN, "Forecast", "click a point"),
            MapTool::Chase => (ph::CROSSHAIR, "Chase", "click your location"),
            MapTool::Climatology => (ph::TORNADO, "Climatology", "click a point"),
            MapTool::Draw => (ph::PENCIL_SIMPLE, "Draw", "drag to scribble"),
            MapTool::Interrogate => ("", "", ""),
        };
        // Nothing armed, nothing to say. The chip used to be a permanent readout of the cursor's
        // lat/lon and the zoom level — numbers nobody was reading, in the corner where the map is.
        if hint.is_empty() {
            return;
        }
        let accent = crate::theme::accent(self.settings.theme);
        // An armed tool changes what a click means, so the cursor says so over the whole map.
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        // Escape disarms, the same way it closes every other transient thing.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tool = MapTool::Interrogate;
            return;
        }
        egui::Area::new(egui::Id::new("info_chip"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-14.0, style::LANE_BOTTOM_CHIP),
            )
            // The draw tool's chip carries buttons; the rest are read-only hints that must never
            // swallow a click meant for the map underneath.
            .interactable(self.tool == MapTool::Draw)
            .show(ctx, |ui| {
                style::glass(ui, 238).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(glyph).size(15.0).color(accent));
                        ui.label(
                            egui::RichText::new(name)
                                .size(style::FONT_BASE)
                                .color(accent)
                                .strong(),
                        );
                        ui.label(egui::RichText::new(hint).size(style::FONT_SM).weak());
                        ui.label(
                            egui::RichText::new("· Esc cancels")
                                .size(style::FONT_SM)
                                .weak(),
                        );
                        if self.tool != MapTool::Draw {
                            return;
                        }
                        ui.separator();
                        for c in DRAW_COLORS {
                            let sel = self.draw_color == c;
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                            let p = ui.painter_at(rect);
                            p.circle_filled(rect.center(), 7.0, c);
                            if sel {
                                p.circle_stroke(
                                    rect.center(),
                                    8.5,
                                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                                );
                            }
                            if resp.clicked() {
                                self.draw_color = c;
                            }
                        }
                        ui.separator();
                        if ui
                            .add_enabled(!self.strokes.is_empty(), egui::Button::new("Undo"))
                            .clicked()
                        {
                            self.strokes.pop();
                        }
                        if ui
                            .add_enabled(!self.strokes.is_empty(), egui::Button::new("Clear"))
                            .clicked()
                        {
                            self.strokes.clear();
                        }
                    });
                });
            });
    }

    /// Bottom-center error chip: the active pane's fetch error, auto-hiding after ~6 seconds.
    ///
    /// ponytail: one chip, newest error wins, no toast queue — add a queue if overlapping errors
    /// from different panes turn out to matter.
    fn error_chip(&mut self, ctx: &egui::Context) {
        const HOLD_SECS: f64 = 6.0;
        let now = ctx.input(|i| i.time);
        if let Some(e) = self.views[self.active].error.clone() {
            if self.error_chip.as_ref().is_none_or(|(prev, _)| *prev != e) {
                self.error_chip = Some((e, now));
            }
        }
        let Some((msg, since)) = self.error_chip.clone() else {
            return;
        };
        if now - since > HOLD_SECS {
            self.error_chip = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        egui::Area::new(egui::Id::new("error_chip"))
            .constrain_to(self.chrome_rect)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, crate::ui::style::LANE_BOTTOM_CHASE),
            )
            // Self-expires after HOLD_SECS, so it needs no dismiss click — and an interactable
            // layer over the map blanks pinch wherever it sits.
            .interactable(false)
            .show(ctx, |ui| {
                crate::ui::style::glass(ui, 246)
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(230, 100, 100).gamma_multiply(0.8),
                    ))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&msg)
                                .size(crate::ui::style::FONT_BASE)
                                .color(egui::Color32::from_rgb(240, 150, 150)),
                        );
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
fn field_index_upload(
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

/// Download and decode a placefile icon sheet (PNG/GIF) into an egui image.
async fn fetch_icon_sheet(http: &reqwest::Client, url: &str) -> anyhow::Result<egui::ColorImage> {
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

        // "Modern dark pro" styling (palette/spacing/rounding/accent). Re-applied when the theme
        // or the system light/dark preference changes — it rebuilds and installs a whole
        // `egui::Style`, which is wasted work on every other frame.
        let system_dark = ctx.input(|i| i.raw.system_theme) != Some(egui::Theme::Light);
        if self.theme_applied != Some((self.settings.theme, system_dark)) {
            crate::theme::apply(ctx, self.settings.theme, system_dark);
            self.theme_applied = Some((self.settings.theme, system_dark));
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
        if crate::platform::activity::is_active()
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
            // HRRR forecast, HRRR environment, and per-site L3 grids each fetch in their own block.
            if matches!(
                layer,
                FL::Hrrr
                    | FL::Cape
                    | FL::Srh
                    | FL::Vil
                    | FL::EchoTops
                    | FL::Hca
                    | FL::UpdraftHelicity
                    | FL::Smoke
                    | FL::Mosaic
                    | FL::VilLocal
                    | FL::VilDensity
                    | FL::EtopLocal
                    | FL::HailMehs
                    | FL::HailPosh
                    | FL::Snowfall
                    | FL::SnowAnalysis
            ) {
                continue;
            }
            let stale = self.fields.get(&layer).is_some_and(|s| {
                s.show
                    && s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
            });
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
                    FL::Hrrr
                    | FL::Cape
                    | FL::Srh
                    | FL::Vil
                    | FL::EchoTops
                    | FL::Hca
                    | FL::UpdraftHelicity
                    | FL::Smoke
                    | FL::Mosaic
                    | FL::VilLocal
                    | FL::VilDensity
                    | FL::EtopLocal
                    | FL::HailMehs
                    | FL::HailPosh
                    | FL::Snowfall
                    | FL::SnowAnalysis => unreachable!(),
                };
                self.spawn_overlay(ctx, OverlaySource::Field(layer, product));
            }
        }
        // Environment suite (HRRR CAPE/SRH): fetch each enabled layer at f00, refresh ~15 min.
        for layer in [FL::Cape, FL::Srh] {
            let stale = self.fields.get(&layer).is_some_and(|s| {
                s.show
                    && s.last_fetch
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
        // HRRR rotation tracks + smoke: same forecast-hour scrub as future radar, own cadences.
        for layer in [FL::UpdraftHelicity, FL::Smoke, FL::Snowfall] {
            let fh = self.hrrr_fcst_hour;
            let stale = self.fields.get(&layer).is_some_and(|s| {
                s.show
                    && s.last_fetch
                        .is_none_or(|t| t.elapsed().as_secs() >= field_refresh_secs(layer))
            });
            // Scrubbing the forecast tail must refetch immediately, not wait out the cadence.
            let hour_changed = self.fields.get(&layer).is_some_and(|s| s.show)
                && self.hrrr_layer_hour.get(&layer) != Some(&fh);
            if stale || hour_changed {
                if let Some(s) = self.fields.get_mut(&layer) {
                    s.last_fetch = Some(Instant::now());
                }
                self.hrrr_layer_hour.insert(layer, fh);
                self.spawn_overlay(ctx, OverlaySource::HrrrLayer(layer, fh));
            }
        }
        // GOES lightning: granules land every 20 s, so poll about that often. One in flight at a
        // time — a slow fetch must not queue up behind itself.
        if self.show_glm
            && self
                .glm_last_poll
                .is_none_or(|t| t.elapsed().as_secs() >= 20)
            && !self.glm_polling.load(std::sync::atomic::Ordering::Relaxed)
        {
            self.glm_last_poll = Some(Instant::now());
            self.glm_polling
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let feed = self.glm.clone();
            let busy = self.glm_polling.clone();
            let http = self.http.clone();
            let ctx2 = ctx.clone();
            self.spawner.spawn(async move {
                // Decode outside the lock: holding it across an await would stall the painter.
                let mut local = wxdata::glm::GlmFeed::new(15);
                if let Some(last) = feed.lock().ok().and_then(|f| f.last_key().cloned()) {
                    local.set_last_key(last);
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
            if crate::platform::activity::is_active() {
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
            let on = self.fields.get(&FL::SnowAnalysis).is_some_and(|s| s.show);
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
        // Gridded L3 products (DVL/EET): per-site, refetch on the L3 cadence or a site change.
        let l3_site = self.views[self.active].site.clone();
        let site_changed = self.l3grid_site != l3_site;
        for layer in [FL::Vil, FL::EchoTops, FL::Hca] {
            let on = self.fields.get(&layer).is_some_and(|s| s.show);
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
                .any(|l| self.fields.get(l).is_some_and(|s| s.show))
        {
            self.l3grid_site = l3_site;
        }
        // HRRR future radar: fetch when enabled and the forecast hour changed or the run refreshed
        // (~10-min throttle; a new run posts hourly).
        let hrrr_on = self.fields.get(&FL::Hrrr).is_some_and(|s| s.show);
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

        // Docked desktop chrome. Declared before every floating Area so those constrain to what's
        // left of the viewport (`self.chrome_rect`) instead of covering the bars.
        if !cfg!(target_os = "android") && !self.obs_mode {
            if !self.settings.hide_sidebar {
                self.sidebar(root, ctx);
            }
            if !self.settings.hide_toolbar {
                self.timeline_bar(root);
            }
        }

        self.chrome_rect = root.available_rect_before_wrap();

        // Chrome: touch-first on Android (top chips + bottom sheet + docked toolbar), desktop
        // otherwise (the floating map-first chrome below). Both funnel into the same `UiActions`
        // handling. The occlusion rects are rebuilt from scratch every frame; a stale rect would
        // keep swallowing gestures over a sheet that closed.
        self.mobile_occlusion.clear();
        let mut actions = ui::layer_options::UiActions::default();
        if cfg!(target_os = "android") && !self.obs_mode {
            actions = self.mobile_chrome(root, ctx);
            self.coach_marks(ctx);
        }
        self.apply_ui_actions(actions, ctx);

        // Floating map-first chrome (desktop): a hamburger, an alert bell, the two bottom pills.
        // Everything else is one drawer behind the hamburger.
        if !cfg!(target_os = "android") && !self.obs_mode {
            self.sidebar_button(ctx);
            self.info_chip(ctx);
            self.error_chip(ctx);
            self.coach_marks(ctx);
        }

        // One quiet check per session, once the app has settled — so a stale build tells you so
        // without anyone opening About.
        if ctx.input(|i| i.time) > 30.0 {
            self.check_for_update(ctx);
        }
        while let Ok(tag) = self.update_rx.try_recv() {
            self.update_state = match tag {
                Some(tag) => ui::about_window::compare(&tag),
                None => ui::about_window::UpdateState::Failed,
            };
            if let ui::about_window::UpdateState::Newer(v) = self.update_state.clone() {
                self.toast(ToastKind::Info, format!("Hook Echo-WX {v} is available"));
            }
        }
        if self.about_open {
            let accent = crate::theme::accent(self.settings.theme);
            let mut open = self.about_open;
            ui::about_window::show(ctx, &mut open, &self.update_state, accent);
            self.about_open = open;
        }

        // The `?` cheat sheet floats over everything, including the wizard.
        if self.show_cheatsheet {
            let entries = self.palette_entries();
            let bindings = hotkeys::active(&self.settings).into_owned();
            let accent = crate::theme::accent(self.settings.theme);
            self.show_cheatsheet = ui::cheatsheet::show(ctx, &bindings, &entries, accent);
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
            let keep = ui::site_dialog::show(
                ctx,
                dialog,
                &mut self.views[self.active],
                &mut self.settings,
            );
            if !keep {
                self.site_dialog = None;
            }
        }
        let entries = self.palette_entries();
        let sync_view = ui::settings_window::SyncView {
            signed_in: self.sync_tokens.is_some(),
            status: &self.sync_status,
            login_url: self.sync_login.as_ref().map(|p| p.url.as_str()),
            last_sync: self.sync_state.last_sync,
        };
        let sync_action =
            self.settings_window
                .show(ctx, &mut self.settings, &self.palettes, sync_view, &entries);
        self.capture_key = self.settings_window.capturing;
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
            .show(ctx, &mut self.settings, &pf_status);
        // Names come from the action registry, so a layer reads the same here as in the layers
        // panel — the enum's Debug spelling ("Mrms") is not a label.
        let names: std::collections::HashMap<crate::render::FieldLayer, String> = self
            .palette_entries()
            .into_iter()
            .filter_map(|e| match e.action {
                PaletteAction::ToggleField(l) => Some((l, e.label)),
                _ => None,
            })
            .collect();
        let active_fields: Vec<(crate::render::FieldLayer, String)> =
            crate::render::FieldLayer::DRAW_ORDER
                .into_iter()
                .filter(|l| self.fields.get(l).is_some_and(|s| s.show))
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
        ) {
            self.overlay_gen += 1; // paint order / opacity changed — re-tessellate
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
                        name: name.clone(),
                        lat,
                        lon,
                        icon: None,
                        alert_radius_mi: crate::settings::default_alert_radius_mi(),
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
        if let Some(query) = self
            .marker_window
            .show(ctx, &mut self.settings, &self.marker_icon_tex)
        {
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
            .show(ctx, &mut self.settings, &self.palettes);
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
        if let Some(rx) = &self.raob_rx {
            if let Ok(res) = rx.try_recv() {
                self.raob_rx = None;
                match res {
                    Ok(s) => self.sounding_window.observed = Some(s),
                    Err(e) => self.sounding_window.observed_error = Some(e),
                }
            }
        }
        self.sounding_window.show(ctx, self.active_tz());
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
        let vact = self.verify_window.show(ctx, self.active_tz());
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
            if !ui::forecast_window::show(ctx, &self.forecast_state, at, tz, minute.as_deref(), now)
            {
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
        if let Some(act) = self.event_window.show(ctx, &mut self.settings) {
            use ui::event_window::EventAction;
            match act {
                EventAction::Goto {
                    site,
                    lon,
                    lat,
                    zoom,
                    time,
                } => {
                    self.goto_view(&site, lon, lat, zoom, time);
                }
                EventAction::AddBookmark => {
                    let n = self.settings.bookmarks.len() + 1;
                    self.add_bookmark(format!("Bookmark {n}"));
                }
            }
        }

        if let Some(detail) = &self.detail {
            let tex = detail
                .image
                .as_ref()
                .and_then(|k| self.pf_icon_tex.get(k))
                .and_then(|t| t.as_ref());
            if !ui::detail_window::show(ctx, detail, tex) {
                self.detail = None;
            }
        }
        // Storm attributes table: clicking a row flies there and opens that cell's popup, the
        // same destination as clicking the dot on the map.
        let cells: &[Cell] = if self.archive_bucket().is_some() {
            &[]
        } else {
            &self.storm_cells
        };
        if let Some(id) = ui::cells_window::show(
            &mut self.cells_window,
            ctx,
            cells,
            crate::theme::accent(self.settings.theme),
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
            let (open, toggled) = ui::cell_window::show(ctx, cell, trend, following);
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
        if let Some(i) = self.marker_popup {
            match self.settings.markers.get_mut(i) {
                // The list shrank under us (the manager window deleted a row this frame).
                None => self.marker_popup = None,
                Some(m) => {
                    let r = ui::marker_popup::show(ctx, m);
                    if r.manage {
                        self.marker_window.open = true;
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
        self.follow_badge(ctx);
        if !self.obs_mode {
            self.chase_hud(ctx);
        }
        if let Some(popup) = &mut self.warning_popup {
            if !ui::warning_window::show(ctx, popup) {
                self.warning_popup = None;
            }
        }
        if self.show_sensors
            && !ui::sensor_window::show(ctx, self.sensor_data.as_ref(), self.active_tz())
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
            )
        {
            self.show_hodo = false;
        }
        if let (Some(xs), Some(tex)) = (&self.xsection, &self.xsection_tex) {
            let mut moment = self.xsection_moment;
            let open = ui::xsection_window::show(ctx, xs, tex, &mut moment);
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
            let mut open = true;
            ui::volume3d_window::show(
                ctx,
                &mut open,
                &mut self.vol3d_az,
                &mut self.vol3d_el,
                &mut self.vol3d_dist,
                &mut self.vol3d_pending,
                192,
                48,
            );
            self.show_3d = open;
        }
        if self.show_cappi {
            self.update_cappi(ctx);
            let mut open = true;
            if let Some(tex) = self.cappi_tex.clone() {
                open = ui::cappi_window::show(ctx, &tex, &mut self.cappi_alt_km, 300.0);
            } else {
                crate::ui::phone_surface(ctx, egui::Window::new("CAPPI slice"))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.weak("No volume loaded in the active pane.");
                    });
            }
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
            self.tiles
                .set_keys(&self.settings.mapbox_key, &self.settings.maptiler_key);
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
                    self.spawner.spawn(async move {
                        let times =
                            crate::tiles::fetch_goes_times(&http, raster_style, 8, 48).await;
                        let _ = tx.send(times);
                    });
                }
                let selected = self
                    .goes_time_idx
                    .and_then(|i| self.goes_times.get(i).copied());
                clear_tiles |= self.tiles.set_goes_time(selected);
            } else if self.goes_times_style.is_some() {
                self.goes_times_style = None;
                self.goes_times.clear();
                clear_tiles |= self.tiles.set_goes_time(None);
            }
            let mut clear_vector = false;
            if is_vector {
                clear_vector |= self.vtiles.set_style(style == BasemapStyle::Dark);
                clear_vector |= self
                    .vtiles
                    .note_zoom(self.views[self.active.min(n - 1)].camera.zoom);
            }
            self.last_viewport = rects
                .get(self.active)
                .map_or((full.width(), full.height()), |r| (r.width(), r.height()));

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
                .filter(|t| *t != OverlayToggle::LinkCameras && *self.overlay_flag(*t))
                .map(|t| t.slug())
                .collect();
            self.settings.overlays_on = on;
        }
        if due && self.settings != self.saved {
            // A palette-map change reloads the color tables (bumps gen -> LUT re-bake).
            if self.settings.palettes != self.saved.palettes {
                self.palettes.reload(&self.settings.palette_paths());
            }
            self.settings.save();
            self.saved = self.settings.clone();
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

        // Idle heartbeat so clocks (volume age, countdowns) tick without input. Data arrivals and
        // animations (pulse, banners) request faster repaints on their own. Slower on Android to
        // spare the battery — nothing on screen changes faster than this between frames.
        let idle = if !crate::platform::activity::is_active() {
            2_000 // backgrounded: just enough to notice coming back
        } else if cfg!(target_os = "android") {
            250
        } else {
            100
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(idle));
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
    use super::nearest_cell;
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
        let (site, lon, lat, zoom, time) = parse_goto("KTLX,-97.3,35.3,9").unwrap();
        assert_eq!((site.as_str(), lon, lat, zoom), ("KTLX", -97.3, 35.3, 9.0));
        assert!(time.is_none());
        // The URL form is the same string behind a scheme.
        assert_eq!(
            parse_goto("hookecho://goto/KTLX,-97.3,35.3,9").unwrap().0,
            "KTLX"
        );
        // AlertService writes a site-less notification link; that must keep working.
        let (site, ..) = parse_goto(",-97.3,35.3,9").unwrap();
        assert_eq!(site, "");
        // Archive links carry a time.
        let (.., time) = parse_goto("KTLX,-97.3,35.3,9,2013-05-20T20:00:00Z").unwrap();
        assert_eq!(time.unwrap().to_rfc3339(), "2013-05-20T20:00:00+00:00");
        assert!(parse_goto("").is_none());
        assert!(parse_goto("garbage").is_none());
    }

    #[test]
    fn goto_link_round_trips() {
        let link = goto_link("KFWS", -97.3031, 32.5731, 8.5, None);
        assert!(link.starts_with("hookecho://goto/KFWS,"), "{link}");
        let (site, lon, lat, zoom, _) = parse_goto(&link).unwrap();
        assert_eq!(site, "KFWS");
        assert!((lon - -97.3031).abs() < 1e-4 && (lat - 32.5731).abs() < 1e-4);
        assert_eq!(zoom, 8.5);
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
