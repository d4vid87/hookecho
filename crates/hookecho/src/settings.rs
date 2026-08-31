//! Persisted user settings (JSON at the platform config dir).
//!
//! Mirrors Supercell WX's settings tabs; only the General tab is wired in U1, the rest
//! land in later milestones. `#[serde(default)]` makes old config files forward-compatible
//! — new fields fill from `Default`, unknown fields are ignored.

use crate::ui::m3::Density;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// egui theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    #[serde(alias = "Magma", alias = "Redline", alias = "AcidStorm")]
    Dark,
    #[serde(alias = "Glacier")]
    Light,
    System,
    #[serde(alias = "Ultraviolet", alias = "Bubblegum", alias = "Voltage")]
    Synthwave,
    #[serde(alias = "Riptide")]
    Aurora,
    HighContrast,
    Oled,
}

impl Theme {
    /// All themes in menu order.
    pub const ALL: [Theme; 7] = [
        Theme::Dark,
        Theme::Light,
        Theme::System,
        Theme::Synthwave,
        Theme::Aurora,
        Theme::HighContrast,
        Theme::Oled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::System => "System",
            Theme::Synthwave => "Synthwave",
            Theme::Aurora => "Aurora",
            Theme::HighContrast => "High contrast",
            Theme::Oled => "OLED black",
        }
    }
}

/// Alert sound choice. Built-ins are synthesized in `audio.rs` (no asset files); `Custom` plays a
/// user file (wav/mp3/ogg/flac). Serializes as `"Chime"` or `{"Custom":"/path/f.wav"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum AlertSound {
    #[default]
    Chime,
    Ding,
    Siren,
    Alarm,
    Pulse,
    /// The EAS/NWS Attention Signal: 853 Hz and 960 Hz together, the sound that precedes an
    /// emergency broadcast. Synthesized from the two published frequencies like every other
    /// built-in — no recording is bundled.
    Eas,
    Custom(String),
}

impl AlertSound {
    /// The synthesized built-ins, for sound-picker combos.
    pub const BUILTINS: [AlertSound; 6] = [
        AlertSound::Chime,
        AlertSound::Ding,
        AlertSound::Siren,
        AlertSound::Alarm,
        AlertSound::Pulse,
        AlertSound::Eas,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            AlertSound::Chime => "Chime",
            AlertSound::Ding => "Ding",
            AlertSound::Siren => "Siren",
            AlertSound::Alarm => "Alarm",
            AlertSound::Pulse => "Pulse",
            AlertSound::Eas => "Emergency (EAS tone)",
            AlertSound::Custom(_) => "Custom…",
        }
    }
}

fn default_custom_tile_max_z() -> u8 {
    19
}

fn default_volume() -> f32 {
    0.2
}

fn default_live_loop_frames() -> usize {
    10
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Default/home radar site (ICAO id).
    pub default_site: String,
    /// Seconds between live-update polls for the newest volume.
    pub poll_interval_secs: u64,
    pub theme: Theme,
    /// UI density (spacing/type token table). Comfortable by default; Compact restores the
    /// pre-0.12 pro-dense desktop metrics.
    pub density: Density,
    /// User accent override as RGB. `None` keeps the theme's own accent.
    pub accent: Option<[u8; 3]>,
    /// Hold every animation at its endpoint. The app also sets this for itself when frames get
    /// slow; this flag is only the user's half of that.
    #[serde(default)]
    pub reduce_motion: bool,
    /// Starred radar sites shown in the toolbox presets dropdown.
    pub presets: Vec<String>,
    /// Per-moment color-table override: moment short name (`REF`, `VEL`, …) -> `.pal` path.
    /// A missing key uses the built-in default table.
    pub palettes: BTreeMap<String, String>,
    /// File name -> file content, for platforms with nowhere to put a file. A browser hands an
    /// imported `.pal` over as bytes and there is no path that would survive the next reload, so
    /// the content rides along in the settings — which already persist, and already export with
    /// the settings bundle. Empty everywhere else.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub web_files: BTreeMap<String, String>,
    /// Velocity/spectrum-width display unit (internal data stays m/s).
    pub velocity_unit: VelocityUnit,
    /// Temperature display unit for the surface station plots (internal data stays Celsius).
    #[serde(default)]
    pub temp_unit: TempUnit,
    /// Whether radar timestamps read in the site's local time or in UTC.
    #[serde(default)]
    pub time_display: TimeDisplay,
    /// UI text/widget zoom factor (egui `zoom_factor`); also captures Ctrl+= / Ctrl+- / Ctrl+0.
    pub ui_scale: f32,
    /// User-added GRLevelX placefile overlays.
    pub placefiles: Vec<PlacefileConfig>,
    /// User-placed location markers.
    pub markers: Vec<Marker>,
    /// Recolour reflectivity by the MRMS surface precipitation type: blue where it is falling
    /// as snow, pink where it is freezing rain or sleet.
    #[serde(default)]
    pub precip_tint: bool,
    /// Unfold aliased Doppler velocity (region-based dealiasing) when displaying VEL.
    #[serde(default)]
    pub dealias_velocity: bool,
    /// Mapbox access token (enables the Mapbox raster basemap styles). Held locally only.
    #[serde(default)]
    pub mapbox_key: String,
    /// MapTiler API key (enables the MapTiler raster basemap styles). Held locally only.
    #[serde(default)]
    pub maptiler_key: String,
    /// `{z}/{x}/{y}` tile URL template for the "Custom (XYZ URL)" basemap. Validated at the
    /// settings boundary — see [`crate::tiles::valid_xyz_template`] — and desktop/Android only.
    #[serde(default)]
    pub custom_tile_url: String,
    /// Max zoom the custom tile source serves. Deeper views stretch the deepest tile that loaded.
    #[serde(default = "default_custom_tile_max_z")]
    pub custom_tile_max_z: u8,
    /// Attribution line to paint for the custom tile source.
    #[serde(default)]
    pub custom_tile_attribution: String,
    /// WeatherFlow Tempest personal access token — adds your Tempest stations to the live station
    /// cards. Held locally only, same as the basemap keys.
    #[serde(default)]
    pub tempest_token: String,
    /// Weather Underground API key — adds nearby PWS stations to the live station cards.
    #[serde(default)]
    pub wu_key: String,
    /// Synoptic Data token — adds the mesonets (state, university, DOT) to the station cards.
    /// Held locally only, and never sent through the web build's proxy.
    #[serde(default)]
    pub synoptic_token: String,
    /// AirNow API key — turns on the AQI station layer. Held locally, same as the other keys.
    #[serde(default)]
    pub airnow_key: String,
    /// Per-field-layer opacity 0..1 (Layer Manager sliders). A missing entry means fully opaque.
    #[serde(default)]
    pub field_opacity: std::collections::HashMap<crate::render::FieldLayer, f32>,
    /// Windy API key — adds the Windy webcam network to the keyless FAA cameras, which is what
    /// gives the layer any coverage outside the United States. Held locally, same as the rest.
    #[serde(default)]
    pub windy_key: String,
    /// A ground field mill publishing JSON (`{"time":…, "kv_per_m":…}`, or an array of those).
    /// When set, the station cards chart real kV/m instead of NOAA's ionospheric model.
    #[serde(default)]
    pub field_mill_url: String,
    /// Saved startup view (radar site + camera). `None` = open on `default_site`.
    #[serde(default)]
    pub start_view: Option<StartView>,
    /// Where the app was looking when it last closed, written on exit only while no explicit
    /// `start_view` is set. Kept apart from `start_view` so "save this view" stays a deliberate
    /// choice you can clear, rather than something the first quit freezes forever.
    #[serde(default)]
    pub last_view: Option<StartView>,
    /// Google OAuth client id + secret for settings sync (see `docs/sync.md`). You create the
    /// client; there is no shipped default, so an open-source binary carries nobody's quota.
    #[serde(default)]
    pub sync_client_id: String,
    #[serde(default)]
    pub sync_client_secret: String,
    /// Sync settings through your Google Drive app folder once signed in.
    #[serde(default)]
    pub sync_enabled: bool,
    /// Share your GPS position with other HookEcho instances (LAN broadcast, and the relay below
    /// when one is set). Off by default: your live position is not shared without asking.
    #[serde(default)]
    pub share_position: bool,
    /// The name other instances label your dot with. Empty = "me".
    #[serde(default)]
    pub share_name: String,
    /// Optional HTTP endpoint that relays positions when the devices aren't on one network
    /// (`POST` a peer, `GET` the list). Empty = LAN only. You host it; it sees your position.
    #[serde(default)]
    pub share_relay: String,
    /// A live-stream URL broadcast alongside your shared position, so chase partners can watch
    /// your feed from your dot. Empty = share position only.
    #[serde(default)]
    pub share_video_url: String,
    /// Averaging window for the MRMS cloud-to-ground strike-density layer, in minutes
    /// (1/5/15/30). Short windows show where lightning is right now; long ones show the storm's
    /// track.
    #[serde(default = "default_lightning_minutes")]
    pub lightning_minutes: u16,
    /// Also poll GOES-West (GOES-18) for GLM lightning, not only GOES-East. Costs one extra S3
    /// listing per 20-second cycle and covers the Pacific and the west coast.
    #[serde(default)]
    pub glm_goes_west: bool,
    /// How far from the active radar to draw Spotter Network dots, in km. 0 = no limit (the whole
    /// CONUS feed). Default 230 km, roughly the radar's own useful range.
    #[serde(default = "default_spotter_range_km")]
    pub spotter_range_km: f64,
    /// Play an audible chime when a new NWS warning appears.
    #[serde(default = "default_true")]
    pub alert_sound: bool,
    /// Interpolate radar gates (and the color lookup) instead of drawing hard gate squares.
    #[serde(default = "default_true")]
    pub smooth_radar: bool,
    /// ntfy.sh topic for push notifications when a warning covers a saved location (empty = off).
    #[serde(default)]
    pub ntfy_topic: String,
    /// Discord incoming-webhook URL for alert delivery (empty = off). A user secret: it lives in
    /// settings.json only and is never committed.
    #[serde(default)]
    pub discord_webhook: String,
    /// Slack incoming-webhook URL for alert delivery (empty = off). User secret, as above.
    #[serde(default)]
    pub slack_webhook: String,
    /// Matrix homeserver base URL, e.g. `https://matrix.org` (empty = off).
    #[serde(default)]
    pub matrix_homeserver: String,
    /// Matrix room ID to post alerts into, e.g. `!abc:matrix.org`.
    #[serde(default)]
    pub matrix_room: String,
    /// Matrix access token. User secret, as above.
    #[serde(default)]
    pub matrix_token: String,
    /// MQTT broker hostname for publishing alerts and status (empty = off).
    #[serde(default)]
    pub mqtt_host: String,
    /// Broker port. 1883 is the plain default, 8883 the TLS one.
    #[serde(default = "default_mqtt_port")]
    pub mqtt_port: u16,
    /// Connect over TLS. Off by default: a house broker on the same LAN usually is not.
    #[serde(default)]
    pub mqtt_tls: bool,
    /// Broker username (empty = anonymous).
    #[serde(default)]
    pub mqtt_user: String,
    /// Broker password. A user secret: settings.json only, never committed.
    #[serde(default)]
    pub mqtt_pass: String,
    /// Topic prefix everything is published under, e.g. `home/weather`.
    #[serde(default = "default_mqtt_prefix")]
    pub mqtt_prefix: String,
    /// Topic the broker republishes lightning strikes on, e.g. `blitzortung/1.1/#`. Empty is off.
    ///
    /// The app never talks to a strike network itself — Blitzortung's terms ask third-party apps
    /// to run their own relay rather than point clients at theirs, and this is the subscriber end
    /// of that arrangement (see `scripts/strikes-relay/`).
    #[serde(default)]
    pub strikes_topic: String,
    /// Publish retained Home Assistant discovery configs so it creates the device by itself.
    ///
    /// Off by default, and deliberately: retained config topics on a broker that is not running
    /// Home Assistant are litter somebody else has to clear up.
    #[serde(default)]
    pub mqtt_discovery: bool,
    /// Fetch terrain at z12 (~40 m/px) instead of z10. Sixteen times the tiles, so it is off by
    /// default and meant for chase packs you download deliberately.
    #[serde(default)]
    pub pack_hires_dem: bool,
    /// Include vector street tiles in a chase pack even when the active basemap is raster. Streets
    /// are what a chase pack is for; the raster imagery alone leaves you without road names.
    #[serde(default = "default_true")]
    pub pack_include_vector: bool,
    /// Include satellite imagery in a chase pack even when the active basemap is vector. The
    /// mirror of `pack_include_vector`: a road map offline is no help for spotting a wall cloud
    /// against terrain you have never seen.
    #[serde(default)]
    pub pack_include_satellite: bool,
    /// User-drawn zones that alert when an NWS warning polygon intersects them. The Android
    /// background service reads this file too, so the name is load-bearing across the boundary.
    #[serde(default)]
    pub alert_polygons: Vec<AlertPolygon>,
    /// User-written alert rules (see [`AlertRule`]). Empty by default: the five built-in alerts
    /// are what the app fires on until somebody says otherwise.
    #[serde(default)]
    pub alert_rules: Vec<AlertRule>,
    /// External-process plugins that emit placefiles (desktop only). Off by default and empty:
    /// each entry is a command the user chose to run.
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
    /// Android only: run a foreground service that watches `markers` for NWS alerts and posts a
    /// notification, so warnings arrive with the app closed. Opt-in — it costs a permanent
    /// notification and some battery. The switch itself reaches Kotlin over JNI
    /// (`platform::set_background_alerts`), not through this file; what the service reads here is
    /// `markers` and `alert_polygons`, so those names are load-bearing across the boundary.
    #[serde(default)]
    pub background_alerts: bool,
    /// Keep running in the background (hide to tray) instead of quitting when the window closes.
    #[serde(default)]
    pub close_to_tray: bool,
    /// User-saved view bookmarks (time-machine library, alongside the curated events).
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    /// Anthropic API key for the optional plain-language storm digest (held locally only; empty
    /// = digest uses the built-in templated summary instead of Claude).
    #[serde(default)]
    pub anthropic_key: String,
    /// Chime + push when cloud-to-ground lightning strikes within ~15 km of a saved location.
    #[serde(default)]
    pub lightning_alarm: bool,
    /// Read new warnings aloud (system speech engine). Off by default — speech is intrusive.
    #[serde(default)]
    pub speak_warnings: bool,
    /// Path to a Piper binary, or blank to look on `PATH`. Piper is a local neural voice; when it
    /// and a voice model are both present, spoken warnings go through it instead of espeak.
    #[serde(default)]
    pub piper_path: String,
    /// Path to a Piper `.onnx` voice model. Blank turns Piper off — an engine with no model has
    /// nothing to say. Never committed: it is a ~60 MB download or a file the user already has.
    #[serde(default)]
    pub piper_voice: String,
    /// Which curated voice the picker has selected for download. Only the picker reads it;
    /// `piper_voice` above is still what actually speaks.
    #[serde(default)]
    pub piper_download_voice: String,
    /// While chase mode is on, speak the nearest storm's bearing and distance as it changes.
    #[serde(default)]
    pub speak_position: bool,
    /// Alert when radar echo is heading for a saved location.
    #[serde(default)]
    pub rain_alerts: bool,
    /// Sound for the rain-arrival alert.
    #[serde(default)]
    pub rain_sound: AlertSound,
    /// One-time hints already shown, by id. A hint is a sentence somebody needs once: showing it
    /// twice is nagging, and the only way to know is to remember. Ids, not text, so rewording a
    /// hint doesn't bring it back for everyone who already read it.
    #[serde(default)]
    pub hints_seen: Vec<String>,
    /// First-run setup completed (or dismissed). `false` shows the first-run card at startup.
    #[serde(default)]
    pub setup_done: bool,
    /// Post alerts to the platform's own notification centre, so they arrive with the window
    /// behind something else. Ignored on Android, where the alert service posts its own.
    #[serde(default)]
    pub desktop_notify: bool,
    /// Battery saver: relax every cadence the app controls — the idle repaint clock, the volume
    /// poll, the widget snapshot, and the background alert alarm on Android. Warnings still
    /// arrive; they arrive on a slower schedule. Off by default, because a weather app that
    /// quietly polls less often than it says it does is the wrong kind of surprise.
    #[serde(default)]
    pub battery_saver: bool,
    /// Record a breadcrumb track of the session's GPS fixes, exportable as GPX. Off by default:
    /// where you drove is yours, and nothing records it unless you say so. The track lives in
    /// memory only until you save it.
    #[serde(default)]
    pub chase_log: bool,
    /// Attach a picture of the radar to the ntfy push when a warning fires. Desktop only: the
    /// Android background service has no GPU surface to render from, and says so in the UI.
    #[serde(default)]
    pub ntfy_snapshot: bool,
    /// Watch wherever you are, not only the places you saved: while a GPS fix is coming in it
    /// joins the marker list for the proximity alerts (lightning, rotation), under the name
    /// "my location". Nothing is written to the saved markers and nothing is shared.
    #[serde(default)]
    pub alert_follow_gps: bool,
    /// Quiet hours: between `quiet_start_hour` and `quiet_end_hour` (local, 24h), alert sounds
    /// and pushes are held back. Escalated warnings — Tornado Emergency, PDS, destructive — go
    /// through anyway: the whole point of the tier is that it is worth waking up for.
    #[serde(default)]
    pub quiet_hours: bool,
    #[serde(default = "default_quiet_start")]
    pub quiet_start_hour: u32,
    #[serde(default = "default_quiet_end")]
    pub quiet_end_hour: u32,
    /// Lowest NWS escalation tier (see `wxdata::alerts::escalation`) allowed to push and sound.
    /// 0 lets everything through, which is the default.
    #[serde(default)]
    pub alert_min_escalation: u8,
    /// Alerts inside `alert_rollup_window_min` before pushes collapse into one rolling summary.
    /// 0 turns the rollup off. Escalated alerts always push as themselves.
    #[serde(default = "default_alert_rollup_threshold")]
    pub alert_rollup_threshold: usize,
    /// Window the rollup threshold counts over, in minutes.
    #[serde(default = "default_alert_rollup_window_min")]
    pub alert_rollup_window_min: u64,
    /// Chime when a new radar volume lands on the live pane in view.
    #[serde(default)]
    pub scan_chime: bool,
    /// Sound for the new-scan chime. Ding by default — a scan every four minutes should be a tap
    /// on the shoulder, not a warning tone.
    #[serde(default = "default_scan_sound")]
    pub scan_sound: AlertSound,
    /// Sound played when a new NWS warning appears (gated by `alert_sound`).
    #[serde(default)]
    pub warn_sound: AlertSound,
    /// Sound played on tornado-debris-signature detection.
    #[serde(default)]
    pub tds_sound: AlertSound,
    /// Sound played when a rotation couplet is detected (defaults to Siren — distinct from TDS).
    #[serde(default = "default_rotation_sound")]
    pub rotation_sound: AlertSound,
    /// Sound played on the lightning proximity alarm.
    #[serde(default)]
    pub lightning_sound: AlertSound,
    /// Sound played when an escalated (Tornado Emergency / PDS / destructive) warning appears.
    #[serde(default = "default_emergency_sound")]
    pub emergency_sound: AlertSound,
    /// Playback volume for all alert sounds (0.0..=1.0).
    #[serde(default = "default_volume")]
    pub alert_volume: f32,
    /// Number of newest volumes the live loop cycles over when playing.
    #[serde(default = "default_live_loop_frames")]
    pub live_loop_frames: usize,
    /// Persisted basemap style slug for startup (empty = the pane default, [`crate::tiles::BasemapStyle::default`]).
    #[serde(default)]
    pub basemap: String,
    /// Overlay toggles that were on when the app last ran, by name (see `OverlayToggle::slug`).
    /// Names rather than the enum on purpose: a name this build doesn't know is skipped, where an
    /// unknown enum variant would fail the parse and reset every other setting with it.
    #[serde(default)]
    pub overlays_on: Vec<String>,
    /// Saved pane layouts (see `crate::workspace`), applied from the command palette. Distinct
    /// from `presets`, which is the starred-radar-site list.
    #[serde(default)]
    pub workspaces: Vec<crate::workspace::Workspace>,
    /// Whether the starter workspaces have been offered. Seeding on "the list is empty" alone
    /// would resurrect them every time someone deleted the last one, which is the opposite of
    /// what deleting the last one means.
    #[serde(default)]
    pub seeded_workspaces: bool,
    /// NOAA Weather Radio relays to listen to. Empty by default on purpose: NOAA runs no streams of
    /// its own, so every URL here is a third-party relay someone runs for their own county, and
    /// shipping a guessed list would mostly ship dead links. Add the one for your area.
    #[serde(default)]
    pub nwr_streams: Vec<NwrStream>,
    /// Silence every alert sound and spoken warning at once, without touching the per-feature
    /// sound choices — the "I'm in a meeting" switch.
    #[serde(default)]
    pub mute_alerts: bool,
    /// User keyboard bindings. Empty means "never edited" and the app uses
    /// [`crate::hotkeys::defaults`]; the Hotkeys settings tab materializes the whole table on the
    /// first edit, so a later change to the defaults doesn't silently rewrite someone's keys.
    #[serde(default)]
    pub(crate) keybinds: Vec<crate::hotkeys::Binding>,
    /// mPING API key (free, from mping.ou.edu) — enables the crowd precipitation-type reports.
    /// Held locally only, same as the other keys.
    #[serde(default)]
    pub mping_key: String,
    /// Reflectivity threshold (dBZ) defining the derived echo-top height. 18.5 matches the NWS
    /// Enhanced Echo Tops product; raise it to track the core rather than the anvil.
    #[serde(default = "default_etop_dbz")]
    pub etop_dbz: f32,
    /// Caption saved and copied images with site, product, valid time and source. On by default:
    /// a radar picture that leaves the app without those four things cannot be checked by whoever
    /// receives it.
    #[serde(default = "default_true")]
    pub share_card: bool,
    /// Registry labels in the order the user dragged them, across every category. Labels not in
    /// here keep their registry order behind the ones that are — so a reorder never hides a row,
    /// and a renamed action just falls back to its default place.
    #[serde(default)]
    pub layer_order: Vec<String>,
    /// Thresholds the signature detectors fire at (see [`DetectorTuning`]).
    #[serde(default)]
    pub detectors: DetectorTuning,
    /// Bearer token `--serve` requires on every request; empty leaves the server open, which is
    /// what loopback-only has always been. A user secret: settings.json only, never committed,
    /// and device-local so it does not travel to machines that are not this server.
    #[serde(default)]
    pub serve_token: String,
    /// Pushes quiet hours held back, kept across a restart so the catch-up summary still arrives
    /// when the window ends. Device-local (see `cloud::DEVICE_LOCAL`): they are owed to whoever
    /// is at this machine.
    #[serde(default)]
    pub quiet_pending: Vec<(String, String)>,
    /// Cap for the on-disk radar-volume cache, in MB. 0 = the platform default (2 GB desktop,
    /// 300 MB Android). Applied by the startup sweep, so a change takes effect next launch.
    #[serde(default)]
    pub volume_cache_mb: u32,
    /// Cap for each on-disk map-tile cache (raster and vector), in MB. 0 = platform default.
    #[serde(default)]
    pub tile_disk_cache_mb: u32,
}

/// Where the dual-pol signature detectors and the GLM flash-extent grid draw their lines.
///
/// Every one of these was a constant, which is fine until you point the app at a radar whose
/// calibration or season disagrees: a ZDR floor that is right in May over Oklahoma is noise in
/// December over Buffalo. The defaults are the values the detectors shipped with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectorTuning {
    /// Reflectivity (dBZ) a core must reach before a hail spike behind it is looked for.
    pub tbss_core_dbz: f32,
    /// Differential reflectivity (dB) a gate must reach to count toward a ZDR column.
    pub zdr_min_db: f32,
    /// How far a column must extend above the freezing level (km) to be reported.
    pub zdr_min_depth_km: f64,
    /// GLM flash-extent density grid cell size, in degrees (~5 km at the default).
    pub glm_fed_cell_deg: f64,
    /// How far back the flash-extent density grid counts, in minutes.
    pub glm_fed_window_min: i64,
}

impl Default for DetectorTuning {
    fn default() -> Self {
        Self {
            tbss_core_dbz: 60.0,
            zdr_min_db: 1.0,
            zdr_min_depth_km: 1.0,
            glm_fed_cell_deg: 0.05,
            glm_fed_window_min: 15,
        }
    }
}

impl Settings {
    /// Timezone to render `site`'s timestamps in — `None` means "show Zulu", either because the
    /// user picked UTC or because the site has no known zone.
    pub fn tz_for(&self, site: Option<&str>) -> Option<wxdata::tz::Tz> {
        match self.time_display {
            TimeDisplay::Utc => None,
            TimeDisplay::SiteLocal => site.and_then(wxdata::tz::site_tz),
        }
    }
}

/// A saved view: site + camera, and (for archive views) the UTC instant to seek to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub site: String,
    /// Camera center in web-mercator world space `[0,1]²`.
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    /// UTC time to seek to (Unix seconds); `None` = live/head.
    #[serde(default)]
    pub time_secs: Option<i64>,
    /// Replay window around `time_secs`, in minutes. `0` (and any bookmark written before this
    /// existed) means a still: jump there and stop.
    #[serde(default)]
    pub span_min: u16,
}

fn default_quiet_start() -> u32 {
    22
}

pub fn default_alert_rollup_threshold() -> usize {
    5
}
pub fn default_alert_rollup_window_min() -> u64 {
    10
}
fn default_quiet_end() -> u32 {
    7
}

fn default_scan_sound() -> AlertSound {
    AlertSound::Ding
}

fn default_true() -> bool {
    true
}

fn default_etop_dbz() -> f32 {
    18.5
}

fn default_rotation_sound() -> AlertSound {
    AlertSound::Siren
}

fn default_emergency_sound() -> AlertSound {
    AlertSound::Eas
}

/// A portable settings export: the full settings plus inlined `.pal` contents (by moment short
/// name), so palette overrides survive moving to a machine where the original paths don't exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsBundle {
    settings: Settings,
    #[serde(default)]
    palette_files: BTreeMap<String, String>,
}

/// A remembered startup camera: which site to load and where the map sits (world coords).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartView {
    pub site: String,
    /// Camera center in web-mercator world space `[0,1]²`.
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// One external-process plugin: a command that prints a placefile on stdout.
///
/// `refresh_secs` is the app's cadence, not the placefile's own `RefreshSeconds` — a plugin that
/// samples something live wants to be asked again on a schedule the user controls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_plugin_refresh")]
    pub refresh_secs: u32,
    pub enabled: bool,
}

fn default_plugin_refresh() -> u32 {
    60
}

/// One NOAA Weather Radio relay the user has added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NwrStream {
    /// What to call it in the picker, e.g. "KEC55 Norman".
    pub name: String,
    /// A streaming audio URL (Icecast-style MP3).
    pub url: String,
}

/// A configured placefile overlay (URL + on/off + opacity), persisted across sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacefileConfig {
    pub url: String,
    pub enabled: bool,
    /// Draw opacity 0..=1, set in the Layer Manager.
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

fn default_opacity() -> f32 {
    1.0
}

/// A user-drawn watch zone: a closed ring of `[lon, lat]` that raises an alert whenever an NWS
/// warning polygon touches it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertPolygon {
    pub name: String,
    /// Outer ring only, `[lon, lat]`.
    // ponytail: no holes, and editing is delete-and-redraw. A vertex editor is a lot of UI for a
    // shape that takes four clicks to redraw.
    pub ring: Vec<[f64; 2]>,
}

/// What a rule watches for. The scan signatures are the ones the app already computes per
/// volume; the rest ride on feeds it already polls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuleTrigger {
    /// Tornado debris signature.
    Tds,
    /// Three-body scatter spike (hail).
    Tbss,
    /// A ZDR column above the freezing level.
    ZdrColumn,
    /// A velocity couplet; the threshold is minimum Vrot in knots.
    Rotation,
    /// NOAA ProbSevere; the threshold is the minimum Severe percentage.
    ProbSevere,
    /// GLM flash-extent density; the threshold is minimum flashes per cell per window.
    GlmFed,
    /// A lightning jump: how fast flash-extent density is *rising*, in flashes per cell per
    /// minute. The threshold is that rate.
    GlmJump,
    /// An NWS warning whose event name contains this text, case-insensitively. Empty matches
    /// every warning.
    Warning { event_contains: String },
}

impl RuleTrigger {
    /// Every trigger, with the defaults a fresh rule starts on — menu order.
    pub const ALL: [RuleTrigger; 8] = [
        RuleTrigger::Tds,
        RuleTrigger::Tbss,
        RuleTrigger::ZdrColumn,
        RuleTrigger::Rotation,
        RuleTrigger::ProbSevere,
        RuleTrigger::GlmFed,
        RuleTrigger::GlmJump,
        RuleTrigger::Warning {
            event_contains: String::new(),
        },
    ];

    pub fn label(&self) -> &'static str {
        match self {
            RuleTrigger::Tds => "Debris signature (TDS)",
            RuleTrigger::Tbss => "Hail spike (TBSS)",
            RuleTrigger::ZdrColumn => "ZDR column",
            RuleTrigger::Rotation => "Rotation",
            RuleTrigger::ProbSevere => "ProbSevere",
            RuleTrigger::GlmFed => "Lightning density",
            RuleTrigger::GlmJump => "Lightning jump",
            RuleTrigger::Warning { .. } => "NWS warning",
        }
    }

    /// What the threshold means for this trigger, and its default — `None` where the trigger is
    /// its own answer (a debris signature does not come in degrees).
    pub fn threshold_hint(&self) -> Option<(&'static str, f64)> {
        match self {
            RuleTrigger::Rotation => Some(("kt Vrot", 40.0)),
            RuleTrigger::ProbSevere => Some(("% severe", 50.0)),
            RuleTrigger::GlmFed => Some(("flashes", 20.0)),
            RuleTrigger::GlmJump => Some(("flashes/min rise", 4.0)),
            _ => None,
        }
    }

    /// Whether this trigger is answered by a per-volume scan of the active pane's radar, as
    /// opposed to a national feed. Scan triggers are what the headless verifier can replay.
    pub fn is_scan(&self) -> bool {
        matches!(
            self,
            RuleTrigger::Tds | RuleTrigger::Tbss | RuleTrigger::ZdrColumn | RuleTrigger::Rotation
        )
    }
}

/// Where a rule is allowed to fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum RulePlace {
    /// Anywhere the active radar can see. Loud by design — worth pairing with a cooldown.
    #[default]
    Anywhere,
    /// Within a marker's own `alert_radius_mi`. Keyed by [`Marker::id`], so renaming the marker
    /// does not silently detach the rule.
    Marker { id: String },
    /// Touching a drawn watch zone, by [`AlertPolygon::name`].
    Zone { name: String },
}

/// One user rule: "if this signature shows up there, tell me".
///
/// The five built-in alerts are fixed — they fire on what somebody else decided was worth
/// waking up for. This is the same machinery pointed at the user's own question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRule {
    /// Stable id; the cooldown map keys on it, so renaming a rule does not re-arm it.
    pub id: String,
    /// What the user calls it. Empty falls back to the trigger's label.
    #[serde(default)]
    pub name: String,
    pub trigger: RuleTrigger,
    /// Trigger-dependent minimum (see [`RuleTrigger::threshold_hint`]); ignored where the
    /// trigger takes none.
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub place: RulePlace,
    /// Push past quiet hours. Off by default: the user decides what is worth waking up for, and
    /// the default answer is "not this".
    #[serde(default)]
    pub urgent: bool,
    /// Minutes before the same rule may fire for the same place again.
    #[serde(default = "default_rule_cooldown")]
    pub cooldown_min: u16,
    /// Rules are created switched off, and armed deliberately.
    #[serde(default)]
    pub enabled: bool,
    /// Play this sound when the rule fires, instead of nothing. `None` keeps the old behaviour:
    /// the rule banners and pushes but makes no noise of its own.
    #[serde(default)]
    pub sound: Option<AlertSound>,
    /// Attach a picture of the map to the rule's push (desktop only, same path the warning
    /// snapshot uses).
    #[serde(default)]
    pub snapshot: bool,
    /// Extra conditions on top of the trigger. Empty is the old single-condition rule, and an old
    /// rule deserializes into exactly that.
    #[serde(default)]
    pub conditions: Vec<RuleCondition>,
    /// How the extra conditions combine among themselves. The rule's own trigger is always
    /// required — it is what starts the evaluation.
    #[serde(default)]
    pub combine: RuleCombinator,
}

/// One extra thing that must (or may) also be true for a rule to fire.
///
/// Deliberately a trigger and a threshold, not an expression: one level, no nesting. A rule
/// nobody can read at 3am is a rule nobody trusts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RuleCondition {
    pub trigger: RuleTrigger,
    #[serde(default)]
    pub threshold: Option<f64>,
}

/// How a rule's extra conditions combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RuleCombinator {
    /// Every extra condition must also hold.
    #[default]
    And,
    /// At least one of them must.
    Or,
}

impl RuleCombinator {
    pub const ALL: [RuleCombinator; 2] = [RuleCombinator::And, RuleCombinator::Or];

    pub fn label(self) -> &'static str {
        match self {
            RuleCombinator::And => "and also",
            RuleCombinator::Or => "and either",
        }
    }
}

/// Ten minutes, matching the built-in proximity alerts' own cooldown.
pub fn default_rule_cooldown() -> u16 {
    10
}

impl AlertRule {
    /// A fresh, disabled rule.
    pub fn new(trigger: RuleTrigger) -> Self {
        let threshold = trigger.threshold_hint().map(|(_, d)| d);
        Self {
            id: new_marker_id(),
            name: String::new(),
            trigger,
            threshold,
            place: RulePlace::Anywhere,
            urgent: false,
            cooldown_min: default_rule_cooldown(),
            enabled: false,
            sound: None,
            snapshot: false,
            conditions: Vec::new(),
            combine: RuleCombinator::default(),
        }
    }

    /// What to call it in a notification.
    pub fn title(&self) -> String {
        if self.name.trim().is_empty() {
            self.trigger.label().to_string()
        } else {
            self.name.clone()
        }
    }
}

pub fn default_lightning_minutes() -> u16 {
    5
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_mqtt_prefix() -> String {
    "hookecho".to_string()
}

pub fn default_spotter_range_km() -> f64 {
    230.0
}

/// A named location marker at a geographic point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    /// Stable identity, independent of the name. Names are the display field and are not unique
    /// — "Marker 3" comes back after a delete — so anything that remembers a marker between
    /// frames (alert cooldowns, most of all) keys on this instead. Empty in files written before
    /// ids existed; [`Settings::load`] fills those in once.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Optional icon: a filename inside [`Settings::marker_icons_dir`] (not a full path, so the
    /// settings stay portable). `None` draws the default accent dot.
    #[serde(default)]
    pub icon: Option<String>,
    /// Alert when a warning comes within this many miles of the marker, not only when the polygon
    /// covers it. A warning three counties wide still matters when its edge is down the road.
    #[serde(default = "default_alert_radius_mi")]
    pub alert_radius_mi: f64,
    /// Optional live-video URL for this place (a yard camera, a chase stream). Direct HLS/MJPEG
    /// plays in-app; anything else opens in the browser.
    #[serde(default)]
    pub video_url: String,
    /// The one marker that is home: drawn with a ring, and the place alerts speak of by default.
    /// At most one marker has this set (the marker editor enforces it).
    #[serde(default)]
    pub home: bool,
}

/// A fresh marker id: 8 hex characters from the system's randomness, which is plenty for a list
/// a person types by hand.
pub fn new_marker_id() -> String {
    let mut b = [0u8; 4];
    // A duplicate id would only collapse two markers' alert cooldowns; falling back to a
    // time-based id beats refusing to make the marker.
    if getrandom::fill(&mut b).is_err() {
        let t = chrono::Utc::now().timestamp_subsec_nanos();
        b = t.to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Default watch radius for a marker, in miles.
pub fn default_alert_radius_mi() -> f64 {
    20.0
}

/// Whether radar timestamps read in the selected site's local time or in UTC ("Zulu").
///
/// Site-local is the default: the clock a chaser cares about is the one the storm is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeDisplay {
    #[default]
    SiteLocal,
    Utc,
}

impl TimeDisplay {
    pub const ALL: [TimeDisplay; 2] = [TimeDisplay::SiteLocal, TimeDisplay::Utc];

    pub fn label(self) -> &'static str {
        match self {
            TimeDisplay::SiteLocal => "Site local",
            TimeDisplay::Utc => "UTC (Zulu)",
        }
    }
}

/// Display unit for temperature. Observations arrive in Celsius; US surface plots read in
/// Fahrenheit, which is why that is the default here and not the one on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TempUnit {
    #[default]
    Fahrenheit,
    Celsius,
}

impl TempUnit {
    pub const ALL: [TempUnit; 2] = [TempUnit::Fahrenheit, TempUnit::Celsius];

    pub fn label(self) -> &'static str {
        match self {
            TempUnit::Fahrenheit => "°F",
            TempUnit::Celsius => "°C",
        }
    }

    /// Convert an observation's Celsius into this unit.
    pub fn from_c(self, c: f32) -> f32 {
        match self {
            TempUnit::Fahrenheit => c * 9.0 / 5.0 + 32.0,
            TempUnit::Celsius => c,
        }
    }
}

/// Display unit for velocity products. GRLevelX defaults to knots; internal math is m/s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VelocityUnit {
    #[default]
    Knots,
    MetersPerSecond,
    Mph,
}

impl VelocityUnit {
    pub const ALL: [VelocityUnit; 3] = [
        VelocityUnit::Knots,
        VelocityUnit::MetersPerSecond,
        VelocityUnit::Mph,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VelocityUnit::Knots => "kt",
            VelocityUnit::MetersPerSecond => "m/s",
            VelocityUnit::Mph => "mph",
        }
    }

    /// Factor to convert internal m/s into this unit.
    pub fn factor_from_ms(self) -> f32 {
        match self {
            VelocityUnit::Knots => 1.943_844,
            VelocityUnit::MetersPerSecond => 1.0,
            VelocityUnit::Mph => 2.236_936,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_site: "KTLX".to_string(),
            web_files: BTreeMap::new(),
            detectors: DetectorTuning::default(),
            alert_rules: Vec::new(),
            serve_token: String::new(),
            custom_tile_url: String::new(),
            custom_tile_max_z: default_custom_tile_max_z(),
            custom_tile_attribution: String::new(),
            quiet_pending: Vec::new(),
            volume_cache_mb: 0,
            tile_disk_cache_mb: 0,
            share_card: true,
            layer_order: Vec::new(),
            mping_key: String::new(),
            etop_dbz: default_etop_dbz(),
            poll_interval_secs: 30,
            theme: Theme::Dark,
            density: Density::default(),
            accent: None,
            reduce_motion: false,
            presets: Vec::new(),
            palettes: BTreeMap::new(),
            velocity_unit: VelocityUnit::default(),
            temp_unit: TempUnit::default(),
            time_display: TimeDisplay::default(),
            // 1.0 everywhere: this multiplies the native scale factor, and Android's display
            // density already sizes widgets for touch — an extra 1.3 shrank the S24's logical
            // canvas to ~277 pt wide (nothing fit).
            ui_scale: 1.0,
            placefiles: Vec::new(),
            markers: Vec::new(),
            precip_tint: false,
            dealias_velocity: false,
            mapbox_key: String::new(),
            maptiler_key: String::new(),
            tempest_token: String::new(),
            wu_key: String::new(),
            synoptic_token: String::new(),
            field_opacity: Default::default(),
            airnow_key: String::new(),
            windy_key: String::new(),
            field_mill_url: String::new(),
            start_view: None,
            sync_client_id: String::new(),
            sync_client_secret: String::new(),
            sync_enabled: false,
            share_position: false,
            share_name: String::new(),
            share_relay: String::new(),
            share_video_url: String::new(),
            lightning_minutes: default_lightning_minutes(),
            glm_goes_west: false,
            spotter_range_km: default_spotter_range_km(),
            alert_sound: true,
            smooth_radar: true,
            ntfy_topic: String::new(),
            discord_webhook: String::new(),
            slack_webhook: String::new(),
            matrix_homeserver: String::new(),
            matrix_room: String::new(),
            matrix_token: String::new(),
            mqtt_host: String::new(),
            mqtt_port: default_mqtt_port(),
            mqtt_tls: false,
            mqtt_user: String::new(),
            mqtt_pass: String::new(),
            mqtt_prefix: default_mqtt_prefix(),
            strikes_topic: String::new(),
            mqtt_discovery: false,
            background_alerts: false,
            pack_hires_dem: false,
            pack_include_vector: true,
            pack_include_satellite: false,
            alert_polygons: Vec::new(),
            plugins: Vec::new(),
            close_to_tray: false,
            bookmarks: Vec::new(),
            anthropic_key: String::new(),
            lightning_alarm: false,
            speak_warnings: false,
            piper_path: String::new(),
            piper_voice: String::new(),
            piper_download_voice: String::new(),
            speak_position: false,
            rain_alerts: false,
            rain_sound: AlertSound::default(),
            hints_seen: Vec::new(),
            setup_done: false,
            desktop_notify: false,
            chase_log: false,
            battery_saver: false,
            ntfy_snapshot: false,
            alert_follow_gps: false,
            quiet_hours: false,
            quiet_start_hour: default_quiet_start(),
            quiet_end_hour: default_quiet_end(),
            alert_min_escalation: 0,
            alert_rollup_threshold: default_alert_rollup_threshold(),
            alert_rollup_window_min: default_alert_rollup_window_min(),
            scan_chime: false,
            scan_sound: default_scan_sound(),
            warn_sound: AlertSound::default(),
            tds_sound: AlertSound::default(),
            rotation_sound: default_rotation_sound(),
            lightning_sound: AlertSound::default(),
            emergency_sound: default_emergency_sound(),
            alert_volume: default_volume(),
            live_loop_frames: default_live_loop_frames(),
            basemap: String::new(),
            overlays_on: Vec::new(),
            workspaces: Vec::new(),
            seeded_workspaces: false,
            last_view: None,
            nwr_streams: Vec::new(),
            mute_alerts: false,
            keybinds: Vec::new(),
        }
    }
}

impl Settings {
    /// Where settings.json lives. Web has no filesystem — it persists to `localStorage` instead.
    #[cfg(not(target_arch = "wasm32"))]
    fn path() -> Option<PathBuf> {
        crate::paths::config_dir().map(|d| d.join("settings.json"))
    }

    /// The auto-scanned color-tables folder (`<data_dir>/colortables`). Created on first use.
    pub fn colortables_dir() -> Option<PathBuf> {
        let dir = crate::paths::data_dir()?.join("colortables");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    }

    /// Folder holding uploaded marker icons (`<data_dir>/marker-icons`). Created on first use.
    pub fn marker_icons_dir() -> Option<PathBuf> {
        let dir = crate::paths::data_dir()?.join("marker-icons");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    }

    /// Resolve the per-moment `.pal` override paths (`None` = built-in default), indexed by
    /// [`wxdata::level2::Moment::index`].
    pub fn palette_paths(&self) -> [Option<PathBuf>; wxdata::level2::Moment::ALL.len()] {
        use wxdata::level2::Moment;
        Moment::ALL.map(|m| {
            let mut p = self.palettes.get(m.short_name());
            if p.is_none() && m == Moment::CorrelationCoefficient {
                p = self.palettes.get("RHO"); // legacy key, pre-CC rename
            }
            // A name that is in `web_files` names content, not a file. Resolved here rather than
            // in the loader so the loader keeps taking one kind of thing.
            p.map(|v| match self.web_files.get(v) {
                Some(text) => PathBuf::from(format!("{}{text}", crate::colormap::INLINE_PREFIX)),
                None => PathBuf::from(v),
            })
        })
    }

    /// Is local `hour` inside the quiet-hours window? Handles the ordinary case of a window that
    /// crosses midnight (22 → 7). Start == end means "no window", not "all day" — a 24-hour mute
    /// is what `mute_alerts` is for, and reading it the other way would silence someone by
    /// accident.
    pub fn in_quiet_hours(&self, hour: u32) -> bool {
        if !self.quiet_hours {
            return false;
        }
        let (s, e, h) = (
            self.quiet_start_hour % 24,
            self.quiet_end_hour % 24,
            hour % 24,
        );
        match s.cmp(&e) {
            std::cmp::Ordering::Equal => false,
            std::cmp::Ordering::Less => (s..e).contains(&h),
            std::cmp::Ordering::Greater => h >= s || h < e,
        }
    }

    /// The saved settings JSON, or `None` if there is none to read.
    #[cfg(not(target_arch = "wasm32"))]
    fn read_saved() -> Option<String> {
        std::fs::read_to_string(Self::path()?).ok()
    }

    /// Web: the same JSON, out of `localStorage` (no filesystem to read).
    #[cfg(target_arch = "wasm32")]
    fn read_saved() -> Option<String> {
        local_storage()?.get_item(WEB_KEY).ok()?
    }

    /// Load saved settings, falling back to defaults on any error (nothing saved, parse failure).
    /// Parse a settings file, keeping every field that reads and defaulting only the ones that
    /// do not.
    ///
    /// This used to be a plain `serde_json::from_str().unwrap_or_default()`. Every field carries
    /// `#[serde(default)]`, so a *missing* key was always fine — but one bad *value* anywhere in
    /// the file failed the whole struct, and the whole file was thrown away: every marker, every
    /// alert rule, every API key, replaced by defaults, and then written back over the file by the
    /// next `save()`. A settings file whose `theme` said `"light"` instead of `"Light"` is enough
    /// to do it, which is how this was found.
    ///
    /// The repair is per-key: rebuild the object one key at a time and drop any key that stops it
    /// parsing. That is one full deserialize per key, but only on a file that already failed, and
    /// only at startup.
    ///
    /// ponytail: quadratic in the number of keys on the error path. It runs once, on a file that
    /// is already broken.
    fn from_json_lossy(text: &str) -> Self {
        match serde_json::from_str::<Self>(text) {
            Ok(v) => return v,
            Err(e) => log::warn!("settings parse failed ({e}); salvaging what reads"),
        }
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text)
        else {
            log::warn!("settings file is not a JSON object; using defaults");
            return Self::default();
        };
        let mut good = serde_json::Map::new();
        for (k, v) in map {
            good.insert(k.clone(), v);
            if serde_json::from_value::<Self>(serde_json::Value::Object(good.clone())).is_err() {
                good.remove(&k);
                log::warn!("settings: ignoring unreadable value for `{k}`");
            }
        }
        serde_json::from_value(serde_json::Value::Object(good)).unwrap_or_default()
    }

    pub fn load() -> Self {
        let mut loaded = match Self::read_saved() {
            Some(s) => Self::from_json_lossy(&s),
            None => Self::default(),
        };
        // One-shot repair: early Android builds persisted ui_scale 1.3 as the default, which
        // multiplied on top of display density and left a ~277-pt-wide canvas. A saved exact 1.3
        // on Android is that bug, not a choice — the slider steps land there for almost no one.
        if cfg!(target_os = "android") && (loaded.ui_scale - 1.3).abs() < 0.001 {
            loaded.ui_scale = 1.0;
        }
        // Markers saved before ids existed get one now, and keep it: written straight back so the
        // Android alert service reads the same identities this process does.
        let filled = loaded.markers.iter().any(|m| m.id.is_empty());
        for m in loaded.markers.iter_mut().filter(|m| m.id.is_empty()) {
            m.id = new_marker_id();
        }
        if filled {
            loaded.save();
        }
        loaded
    }

    /// Export a portable bundle: this Settings plus the *contents* of every referenced `.pal`
    /// file (inlined by moment short name), so it restores identically on another machine where
    /// the palette paths don't exist. Returns pretty JSON.
    pub fn export_bundle(&self) -> Result<String, String> {
        let mut palette_files = BTreeMap::new();
        for (moment, path) in &self.palettes {
            // A built-in alternate is compiled in on the other machine too — nothing to inline.
            if path.starts_with(crate::colormap::BUILTIN_PREFIX) {
                continue;
            }
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    palette_files.insert(moment.clone(), text);
                }
                Err(e) => log::warn!("bundle: skipping palette {moment} ({path}): {e}"),
            }
        }
        let bundle = SettingsBundle {
            settings: self.clone(),
            palette_files,
        };
        serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())
    }

    /// Import a bundle produced by [`export_bundle`]: writes each inlined `.pal` into the local
    /// colortables dir and rewrites the palette paths to point there, so the imported palettes
    /// resolve locally. Returns the ready-to-use Settings (caller assigns + saves).
    pub fn import_bundle(json: &str) -> Result<Settings, String> {
        let bundle: SettingsBundle = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut settings = bundle.settings;
        if !bundle.palette_files.is_empty() {
            let dir = Self::colortables_dir().ok_or("no colortables dir")?;
            for (moment, text) in &bundle.palette_files {
                let path = dir.join(format!("{moment}.pal"));
                std::fs::write(&path, text).map_err(|e| e.to_string())?;
                settings
                    .palettes
                    .insert(moment.clone(), path.to_string_lossy().into_owned());
            }
        }
        Ok(settings)
    }

    /// Persist `json`: a settings.json on native, a `localStorage` entry on the web.
    #[cfg(not(target_arch = "wasm32"))]
    fn write_saved(json: &str) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = atomic_write(&path, json.as_bytes()) {
            log::warn!("settings save failed: {e}");
        }
    }

    /// Web: one `localStorage` key holding the same JSON. Quota/private-mode failures are logged.
    ///
    // ponytail: settings only. Caches and palette files stay in memory on the web; IndexedDB is the
    // upgrade path if offline-web ever becomes real.
    #[cfg(target_arch = "wasm32")]
    fn write_saved(json: &str) {
        let Some(store) = local_storage() else { return };
        if let Err(e) = store.set_item(WEB_KEY, json) {
            log::warn!("settings save failed: {e:?}");
        }
    }

    /// Write out, logging (not failing) on error.
    pub fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(json) => Self::write_saved(&json),
            Err(e) => log::warn!("settings serialize failed: {e}"),
        }
    }
}

/// `localStorage` key holding the serialized [`Settings`] on the web.
#[cfg(target_arch = "wasm32")]
const WEB_KEY: &str = "hookecho-settings";

/// The page's `localStorage`, or `None` where the browser denies it (private mode, no window).
#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Write via a sibling temp file + rename. A crash mid-write must never tear a config file:
/// a torn settings.json loses every setting on the next launch, and on Android the Kotlin
/// alert service reads this file while we write it.
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_absent_from_old_settings_and_start_disabled() {
        let old: Settings = serde_json::from_str(r#"{"default_site":"KTLX"}"#).unwrap();
        assert!(old.alert_rules.is_empty());

        let r = AlertRule::new(RuleTrigger::Rotation);
        assert!(!r.enabled, "a new rule must not start armed");
        assert_eq!(r.cooldown_min, 10);
        assert_eq!(r.threshold, Some(40.0));
        assert_eq!(r.title(), "Rotation");

        let mut s = Settings::default();
        s.alert_rules.push(r.clone());
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.alert_rules, vec![r]);
        // A trigger with no numeric meaning gets no threshold to be confused by.
        assert_eq!(AlertRule::new(RuleTrigger::Tds).threshold, None);
    }

    #[test]
    fn markers_get_distinct_ids_and_old_files_still_load() {
        // A settings file from before ids existed.
        let old: Settings = serde_json::from_str(
            r#"{"markers":[{"name":"Home","lat":35.3,"lon":-97.3},
                           {"name":"Home","lat":36.1,"lon":-96.9}]}"#,
        )
        .unwrap();
        assert_eq!(old.markers.len(), 2);
        assert!(old.markers.iter().all(|m| m.id.is_empty()));
        // Two places can share a name; ids are what tell them apart.
        let ids: Vec<String> = (0..64).map(|_| new_marker_id()).collect();
        assert!(ids.iter().all(|id| id.len() == 8));
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn quiet_pending_round_trips_and_defaults_empty() {
        let old: Settings = serde_json::from_str(r#"{"default_site":"KTLX"}"#).unwrap();
        assert!(old.quiet_pending.is_empty());
        let mut s = Settings::default();
        s.quiet_pending
            .push(("Severe Thunderstorm Warning".into(), "Cleveland Co.".into()));
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.quiet_pending, s.quiet_pending);
    }

    #[test]
    fn detector_tuning_survives_a_settings_file_that_predates_it() {
        // Settings written before the knobs existed must load with the shipped thresholds.
        let old: Settings = serde_json::from_str(r#"{"default_site":"KTLX"}"#).unwrap();
        assert_eq!(old.detectors, DetectorTuning::default());
        assert_eq!(old.detectors.tbss_core_dbz, 60.0);

        let mut s = Settings::default();
        s.detectors.zdr_min_db = 2.5;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.detectors.zdr_min_db, 2.5);
    }

    #[test]
    fn field_opacity_roundtrips_and_defaults() {
        let mut s = Settings::default();
        assert!(s.field_opacity.is_empty(), "no entry = fully opaque");
        s.field_opacity.insert(crate::render::FieldLayer::Mrms, 0.4);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.field_opacity
                .get(&crate::render::FieldLayer::Mrms)
                .copied(),
            Some(0.4)
        );
        // Old config files without the key still load.
        let bare: Settings = serde_json::from_str("{}").unwrap();
        assert!(bare.field_opacity.is_empty());
    }

    #[test]
    fn a_settings_file_from_the_coach_mark_era_still_loads() {
        // `coach_done` went away with the coach marks; every installed settings.json still has it.
        let s: Settings =
            serde_json::from_str(r#"{"coach_done": true, "setup_done": true}"#).unwrap();
        assert!(s.setup_done);
    }

    #[test]
    fn a_web_file_name_resolves_to_its_content() {
        let mut s = Settings::default();
        s.palettes.insert("REF".to_string(), "mine.pal".to_string());
        // Not in web_files yet: it is an ordinary path, whatever the platform makes of it.
        assert_eq!(
            s.palette_paths()[0].as_deref(),
            Some(std::path::Path::new("mine.pal"))
        );
        s.web_files
            .insert("mine.pal".to_string(), "Color: 5 255 0 0".to_string());
        let p = s.palette_paths()[0].clone().unwrap();
        assert_eq!(
            p.to_string_lossy(),
            format!("{}Color: 5 255 0 0", crate::colormap::INLINE_PREFIX)
        );
    }

    #[test]
    fn roundtrips() {
        let s = Settings {
            hints_seen: Vec::new(),
            web_files: BTreeMap::new(),
            reduce_motion: true,
            precip_tint: false,
            custom_tile_url: String::new(),
            custom_tile_max_z: default_custom_tile_max_z(),
            custom_tile_attribution: String::new(),
            detectors: DetectorTuning::default(),
            alert_rules: Vec::new(),
            serve_token: String::new(),
            quiet_pending: Vec::new(),
            volume_cache_mb: 0,
            tile_disk_cache_mb: 0,
            workspaces: Vec::new(),
            seeded_workspaces: false,
            smooth_radar: false,
            share_card: true,
            layer_order: Vec::new(),
            mping_key: String::new(),
            etop_dbz: 30.0,
            default_site: "KFWS".to_string(),
            density: Density::Compact,
            accent: Some([255, 0, 128]),
            poll_interval_secs: 45,
            theme: Theme::Synthwave,
            presets: vec!["KTLX".to_string(), "KOUN".to_string()],
            palettes: BTreeMap::from([("REF".to_string(), "/tmp/foo.pal".to_string())]),
            velocity_unit: VelocityUnit::Mph,
            temp_unit: TempUnit::Celsius,
            time_display: TimeDisplay::Utc,
            ui_scale: 1.2,
            sync_client_id: String::new(),
            sync_client_secret: String::new(),
            sync_enabled: false,
            share_position: true,
            share_name: "chaser".to_string(),
            share_relay: String::new(),
            share_video_url: String::new(),
            placefiles: vec![PlacefileConfig {
                url: "http://x/p.txt".to_string(),
                enabled: true,
                opacity: 1.0,
            }],
            markers: vec![Marker {
                id: new_marker_id(),
                name: "Home".to_string(),
                lat: 35.3,
                lon: -97.5,
                icon: Some("home.png".to_string()),
                alert_radius_mi: 20.0,
                video_url: String::new(),
                home: true,
            }],
            dealias_velocity: true,
            mapbox_key: "pk.test".to_string(),
            tempest_token: String::new(),
            wu_key: String::new(),
            synoptic_token: String::new(),
            field_opacity: Default::default(),
            airnow_key: String::new(),
            windy_key: String::new(),
            field_mill_url: String::new(),
            maptiler_key: "mt.test".to_string(),
            start_view: Some(StartView {
                site: "KFWS".to_string(),
                x: 0.3,
                y: 0.4,
                zoom: 8.0,
            }),
            lightning_minutes: default_lightning_minutes(),
            glm_goes_west: false,
            spotter_range_km: default_spotter_range_km(),
            alert_sound: false,
            ntfy_topic: "hookecho-test".to_string(),
            discord_webhook: String::new(),
            slack_webhook: String::new(),
            matrix_homeserver: String::new(),
            matrix_room: String::new(),
            matrix_token: String::new(),
            mqtt_host: String::new(),
            mqtt_port: default_mqtt_port(),
            mqtt_tls: false,
            mqtt_user: String::new(),
            mqtt_pass: String::new(),
            mqtt_prefix: default_mqtt_prefix(),
            strikes_topic: String::new(),
            mqtt_discovery: false,
            background_alerts: false,
            pack_hires_dem: false,
            pack_include_vector: true,
            pack_include_satellite: false,
            alert_polygons: Vec::new(),
            plugins: Vec::new(),
            close_to_tray: true,
            bookmarks: vec![Bookmark {
                name: "Storm".to_string(),
                site: "KTLX".to_string(),
                x: 0.3,
                y: 0.4,
                zoom: 9.0,
                time_secs: Some(1_600_000_000),
                span_min: 60,
            }],
            anthropic_key: "sk-test".to_string(),
            lightning_alarm: true,
            speak_warnings: true,
            piper_path: String::new(),
            piper_voice: String::new(),
            piper_download_voice: String::new(),
            speak_position: false,
            rain_alerts: true,
            rain_sound: AlertSound::Ding,
            setup_done: true,
            desktop_notify: false,
            chase_log: false,
            battery_saver: false,
            ntfy_snapshot: false,
            alert_follow_gps: false,
            quiet_hours: false,
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            alert_min_escalation: 0,
            alert_rollup_threshold: default_alert_rollup_threshold(),
            alert_rollup_window_min: default_alert_rollup_window_min(),
            scan_chime: false,
            scan_sound: AlertSound::Ding,
            warn_sound: AlertSound::Siren,
            tds_sound: AlertSound::Custom("/tmp/tds.wav".to_string()),
            rotation_sound: AlertSound::Siren,
            lightning_sound: AlertSound::Alarm,
            emergency_sound: AlertSound::Alarm,
            alert_volume: 0.7,
            live_loop_frames: 12,
            basemap: "carto-dark".to_string(),
            overlays_on: vec!["Alerts".to_string(), "Wind".to_string()],
            last_view: Some(StartView {
                site: "KOUN".to_string(),
                x: 0.2,
                y: 0.4,
                zoom: 7.5,
            }),
            nwr_streams: vec![NwrStream {
                name: "KEC55 Norman".into(),
                url: "https://example.invalid/nwr.mp3".into(),
            }],
            mute_alerts: true,
            keybinds: vec![crate::hotkeys::Binding {
                shortcut: egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::K),
                action: crate::hotkeys::BindableAction::Palette(
                    crate::app::PaletteAction::SetMoment(wxdata::level2::Moment::Velocity, true),
                ),
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn quiet_hours_window_wraps_midnight() {
        let mut s = Settings {
            quiet_hours: true,
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            ..Settings::default()
        };
        assert!(s.in_quiet_hours(23) && s.in_quiet_hours(0) && s.in_quiet_hours(6));
        assert!(!s.in_quiet_hours(7), "the end hour is already awake");
        assert!(!s.in_quiet_hours(21) && !s.in_quiet_hours(12));
        // A window inside one day.
        s.quiet_start_hour = 9;
        s.quiet_end_hour = 17;
        assert!(s.in_quiet_hours(9) && s.in_quiet_hours(16) && !s.in_quiet_hours(17));
        // Equal bounds is no window, not all day.
        s.quiet_end_hour = 9;
        assert!(!s.in_quiet_hours(9) && !s.in_quiet_hours(3));
        // And off is off.
        s.quiet_hours = false;
        s.quiet_start_hour = 0;
        s.quiet_end_hour = 23;
        assert!(!s.in_quiet_hours(5));
    }

    #[test]
    fn bundle_inlines_and_restores_palettes() {
        // A bundle with an inlined .pal should restore to a local path whose file has that text.
        let json = r#"{
            "settings": {"default_site":"KFWS","theme":"Magma","markers":[{"name":"H","lat":1.0,"lon":2.0}]},
            "palette_files": {"REF":"; test palette\nStep: 5\n"}
        }"#;
        let s = Settings::import_bundle(json).expect("import");
        assert_eq!(s.default_site, "KFWS");
        assert_eq!(s.theme, Theme::Dark); // "Magma" is aliased onto Dark
        let ref_path = s.palettes.get("REF").expect("REF palette path set");
        let text = std::fs::read_to_string(ref_path).expect("palette file written");
        assert!(text.contains("test palette"));
    }

    #[test]
    fn an_unknown_overlay_name_does_not_take_the_file_down_with_it() {
        // A file written by a newer build, carrying a layer this one has never heard of. The rest
        // of the settings must survive: `Settings::load` falls back to defaults on a parse error,
        // so a strict enum here would silently reset the user's whole configuration.
        let json = r#"{"default_site":"KDMX","overlays_on":["Alerts","Teleportation"]}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.default_site, "KDMX");
        assert_eq!(s.overlays_on, vec!["Alerts", "Teleportation"]);
    }

    #[test]
    fn temp_unit_converts_from_celsius() {
        assert_eq!(TempUnit::Celsius.from_c(21.0), 21.0);
        assert!((TempUnit::Fahrenheit.from_c(0.0) - 32.0).abs() < 1e-4);
        assert!((TempUnit::Fahrenheit.from_c(-40.0) + 40.0).abs() < 1e-4);
    }

    #[test]
    fn tolerates_unknown_and_missing_fields() {
        // An old/newer config: extra field, and a missing one that should default.
        let json = r#"{"default_site":"KDMX","future_field":true}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.default_site, "KDMX");
        assert_eq!(s.poll_interval_secs, 30, "missing field defaults");
    }

    /// The Android alert service (`android/app/src/main/kotlin/.../AlertService.kt`) parses
    /// settings.json by hand, in another language, with no compiler to tell it when a field is
    /// renamed here. This test is that compiler: rename one of these and it fails, loudly,
    /// pointing at the Kotlin that has to change with it.
    #[test]
    fn kotlin_alert_service_field_names_survive() {
        let json = serde_json::to_string(&Settings {
            markers: vec![Marker {
                id: new_marker_id(),
                name: "Home".to_string(),
                lat: 35.0,
                lon: -97.0,
                icon: None,
                alert_radius_mi: crate::settings::default_alert_radius_mi(),
                video_url: String::new(),
                home: false,
            }],
            alert_polygons: vec![AlertPolygon {
                name: "Farm".to_string(),
                ring: vec![[-97.0, 35.0], [-96.9, 35.0], [-96.9, 35.1]],
            }],
            background_alerts: true,
            ..Settings::default()
        })
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["background_alerts"], serde_json::json!(true));
        let m = &v["markers"][0];
        for key in ["name", "lat", "lon", "alert_radius_mi"] {
            assert!(
                m.get(key).is_some(),
                "AlertService.kt reads markers[].{key}"
            );
        }
        let z = &v["alert_polygons"][0];
        for key in ["name", "ring"] {
            assert!(
                z.get(key).is_some(),
                "AlertService.kt reads alert_polygons[].{key}"
            );
        }
        assert_eq!(
            z["ring"][0][0],
            serde_json::json!(-97.0),
            "Nws.kt reads ring vertices as [lon, lat]"
        );
    }

    /// The bug this was written for: one bad enum value used to discard the entire settings file.
    #[test]
    fn one_unreadable_value_does_not_discard_the_whole_file() {
        // `theme` serialises as "Light"; lowercase is not a valid variant. Everything else here
        // is perfectly good and must survive.
        let text = r#"{
            "theme": "light",
            "mapbox_key": "kept",
            "ui_scale": 1.25,
            "smooth_radar": true
        }"#;
        let s = Settings::from_json_lossy(text);
        assert_eq!(s.mapbox_key, "kept", "a good key was thrown away");
        assert!((s.ui_scale - 1.25).abs() < 1e-6);
        assert!(s.smooth_radar);
        // Only the unreadable field falls back.
        assert_eq!(s.theme, Theme::default());
    }

    #[test]
    fn a_good_file_is_unchanged_and_a_broken_one_still_loads() {
        let original = Settings {
            mapbox_key: "abc".to_string(),
            ui_scale: 1.1,
            ..Default::default()
        };
        let text = serde_json::to_string(&original).unwrap();
        let round = Settings::from_json_lossy(&text);
        assert_eq!(round.mapbox_key, "abc");
        assert!((round.ui_scale - 1.1).abs() < 1e-6);

        // Not an object at all, and not even JSON: defaults, no panic.
        assert_eq!(Settings::from_json_lossy("[1,2,3]").mapbox_key, "");
        assert_eq!(Settings::from_json_lossy("not json").mapbox_key, "");
    }

    /// Unknown keys were already tolerated and must stay that way — a settings file written by a
    /// newer build has to open in an older one.
    #[test]
    fn unknown_keys_are_still_ignored() {
        let s = Settings::from_json_lossy(r#"{"mapbox_key":"x","a_field_from_the_future":42}"#);
        assert_eq!(s.mapbox_key, "x");
    }
}
