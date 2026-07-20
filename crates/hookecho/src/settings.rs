//! Persisted user settings (JSON at the platform config dir).
//!
//! Mirrors Supercell WX's settings tabs; only the General tab is wired in U1, the rest
//! land in later milestones. `#[serde(default)]` makes old config files forward-compatible
//! — new fields fill from `Default`, unknown fields are ignored.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// egui theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
    Synthwave,
    AcidStorm,
    Aurora,
    Magma,
    Bubblegum,
    Riptide,
    Ultraviolet,
    Voltage,
    Redline,
    Glacier,
}

impl Theme {
    /// All themes in menu order.
    pub const ALL: [Theme; 13] = [
        Theme::Dark,
        Theme::Light,
        Theme::System,
        Theme::Synthwave,
        Theme::AcidStorm,
        Theme::Aurora,
        Theme::Magma,
        Theme::Bubblegum,
        Theme::Riptide,
        Theme::Ultraviolet,
        Theme::Voltage,
        Theme::Redline,
        Theme::Glacier,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::System => "System",
            Theme::Synthwave => "Synthwave",
            Theme::AcidStorm => "Acid Storm",
            Theme::Aurora => "Aurora",
            Theme::Magma => "Magma",
            Theme::Bubblegum => "Bubblegum",
            Theme::Riptide => "Riptide",
            Theme::Ultraviolet => "Ultraviolet",
            Theme::Voltage => "Voltage",
            Theme::Redline => "Redline",
            Theme::Glacier => "Glacier",
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
    Custom(String),
}

impl AlertSound {
    /// The synthesized built-ins, for sound-picker combos.
    pub const BUILTINS: [AlertSound; 5] =
        [AlertSound::Chime, AlertSound::Ding, AlertSound::Siren, AlertSound::Alarm, AlertSound::Pulse];

    pub fn label(&self) -> &'static str {
        match self {
            AlertSound::Chime => "Chime",
            AlertSound::Ding => "Ding",
            AlertSound::Siren => "Siren",
            AlertSound::Alarm => "Alarm",
            AlertSound::Pulse => "Pulse",
            AlertSound::Custom(_) => "Custom…",
        }
    }
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
    /// Starred radar sites shown in the toolbox presets dropdown.
    pub presets: Vec<String>,
    /// Per-moment color-table override: moment short name (`REF`, `VEL`, …) -> `.pal` path.
    /// A missing key uses the built-in default table.
    pub palettes: BTreeMap<String, String>,
    /// Velocity/spectrum-width display unit (internal data stays m/s).
    pub velocity_unit: VelocityUnit,
    /// UI text/widget zoom factor (egui `zoom_factor`); also captures Ctrl+= / Ctrl+- / Ctrl+0.
    pub ui_scale: f32,
    /// User-added GRLevelX placefile overlays.
    pub placefiles: Vec<PlacefileConfig>,
    /// User-placed location markers.
    pub markers: Vec<Marker>,
    /// Unfold aliased Doppler velocity (region-based dealiasing) when displaying VEL.
    #[serde(default)]
    pub dealias_velocity: bool,
    /// Mapbox access token (enables the Mapbox raster basemap styles). Held locally only.
    #[serde(default)]
    pub mapbox_key: String,
    /// MapTiler API key (enables the MapTiler raster basemap styles). Held locally only.
    #[serde(default)]
    pub maptiler_key: String,
    /// Saved startup view (radar site + camera). `None` = open on `default_site`.
    #[serde(default)]
    pub start_view: Option<StartView>,
    /// Play an audible chime when a new NWS warning appears.
    #[serde(default = "default_true")]
    pub alert_sound: bool,
    /// ntfy.sh topic for push notifications when a warning covers a saved location (empty = off).
    #[serde(default)]
    pub ntfy_topic: String,
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
    /// First-run setup wizard completed (or dismissed). `false` shows it at startup.
    #[serde(default)]
    pub setup_done: bool,
    /// Sound played when a new NWS warning appears (gated by `alert_sound`).
    #[serde(default)]
    pub warn_sound: AlertSound,
    /// Sound played on tornado-debris-signature detection.
    #[serde(default)]
    pub tds_sound: AlertSound,
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
    /// Persisted basemap style slug for startup (empty = pane default Dark).
    #[serde(default)]
    pub basemap: String,
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
}

fn default_true() -> bool {
    true
}

fn default_emergency_sound() -> AlertSound {
    AlertSound::Siren
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

/// A configured placefile overlay (URL + on/off), persisted across sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacefileConfig {
    pub url: String,
    pub enabled: bool,
}

/// A named location marker at a geographic point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Optional icon: a filename inside [`Settings::marker_icons_dir`] (not a full path, so the
    /// settings stay portable). `None` draws the default accent dot.
    #[serde(default)]
    pub icon: Option<String>,
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
    pub const ALL: [VelocityUnit; 3] = [VelocityUnit::Knots, VelocityUnit::MetersPerSecond, VelocityUnit::Mph];

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
            poll_interval_secs: 30,
            theme: Theme::Dark,
            presets: Vec::new(),
            palettes: BTreeMap::new(),
            velocity_unit: VelocityUnit::default(),
            // Touch UI wants larger widgets out of the box; desktop stays at 1.0. (First-run only —
            // once the user adjusts the slider, the saved value wins.)
            ui_scale: if cfg!(target_os = "android") { 1.3 } else { 1.0 },
            placefiles: Vec::new(),
            markers: Vec::new(),
            dealias_velocity: false,
            mapbox_key: String::new(),
            maptiler_key: String::new(),
            start_view: None,
            alert_sound: true,
            ntfy_topic: String::new(),
            close_to_tray: false,
            bookmarks: Vec::new(),
            anthropic_key: String::new(),
            lightning_alarm: false,
            setup_done: false,
            warn_sound: AlertSound::default(),
            tds_sound: AlertSound::default(),
            lightning_sound: AlertSound::default(),
            emergency_sound: default_emergency_sound(),
            alert_volume: default_volume(),
            live_loop_frames: default_live_loop_frames(),
            basemap: String::new(),
        }
    }
}

impl Settings {
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
    pub fn palette_paths(&self) -> [Option<PathBuf>; 6] {
        use wxdata::level2::Moment;
        Moment::ALL.map(|m| {
            let mut p = self.palettes.get(m.short_name());
            if p.is_none() && m == Moment::CorrelationCoefficient {
                p = self.palettes.get("RHO"); // legacy key, pre-CC rename
            }
            p.map(PathBuf::from)
        })
    }

    /// Load from disk, falling back to defaults on any error (missing file, parse failure).
    pub fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("settings parse failed ({e}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Export a portable bundle: this Settings plus the *contents* of every referenced `.pal`
    /// file (inlined by moment short name), so it restores identically on another machine where
    /// the palette paths don't exist. Returns pretty JSON.
    pub fn export_bundle(&self) -> Result<String, String> {
        let mut palette_files = BTreeMap::new();
        for (moment, path) in &self.palettes {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    palette_files.insert(moment.clone(), text);
                }
                Err(e) => log::warn!("bundle: skipping palette {moment} ({path}): {e}"),
            }
        }
        let bundle = SettingsBundle { settings: self.clone(), palette_files };
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
                settings.palettes.insert(moment.clone(), path.to_string_lossy().into_owned());
            }
        }
        Ok(settings)
    }

    /// Write to disk, logging (not failing) on error.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("settings save failed: {e}");
                }
            }
            Err(e) => log::warn!("settings serialize failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let s = Settings {
            default_site: "KFWS".to_string(),
            poll_interval_secs: 45,
            theme: Theme::Synthwave,
            presets: vec!["KTLX".to_string(), "KOUN".to_string()],
            palettes: BTreeMap::from([("REF".to_string(), "/tmp/foo.pal".to_string())]),
            velocity_unit: VelocityUnit::Mph,
            ui_scale: 1.2,
            placefiles: vec![PlacefileConfig { url: "http://x/p.txt".to_string(), enabled: true }],
            markers: vec![Marker {
                name: "Home".to_string(),
                lat: 35.3,
                lon: -97.5,
                icon: Some("home.png".to_string()),
            }],
            dealias_velocity: true,
            mapbox_key: "pk.test".to_string(),
            maptiler_key: "mt.test".to_string(),
            start_view: Some(StartView { site: "KFWS".to_string(), x: 0.3, y: 0.4, zoom: 8.0 }),
            alert_sound: false,
            ntfy_topic: "hookecho-test".to_string(),
            close_to_tray: true,
            bookmarks: vec![Bookmark {
                name: "Storm".to_string(),
                site: "KTLX".to_string(),
                x: 0.3,
                y: 0.4,
                zoom: 9.0,
                time_secs: Some(1_600_000_000),
            }],
            anthropic_key: "sk-test".to_string(),
            lightning_alarm: true,
            setup_done: true,
            warn_sound: AlertSound::Siren,
            tds_sound: AlertSound::Custom("/tmp/tds.wav".to_string()),
            lightning_sound: AlertSound::Alarm,
            emergency_sound: AlertSound::Alarm,
            alert_volume: 0.7,
            live_loop_frames: 12,
            basemap: "carto-dark".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
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
        assert_eq!(s.theme, Theme::Magma);
        let ref_path = s.palettes.get("REF").expect("REF palette path set");
        let text = std::fs::read_to_string(ref_path).expect("palette file written");
        assert!(text.contains("test palette"));
    }

    #[test]
    fn tolerates_unknown_and_missing_fields() {
        // An old/newer config: extra field, and a missing one that should default.
        let json = r#"{"default_site":"KDMX","future_field":true}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.default_site, "KDMX");
        assert_eq!(s.poll_interval_secs, 30, "missing field defaults");
    }
}
