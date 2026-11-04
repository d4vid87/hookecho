//! Saved pane layouts.
//!
//! Rebuilding "KTLX reflectivity beside KDMX velocity, storm-relative, with these overlays on"
//! costs a dozen clicks, and it's the layout people rebuild every time they sit down. A workspace
//! is a snapshot of that arrangement, restored in one command.
//!
//! It stores state, not actions: the command palette's actions all target the active pane, so a
//! recorded action list has no way to say "and pane two looks like this". What it deliberately
//! doesn't capture is anything tied to the moment rather than the arrangement — the archive
//! playhead, open windows, per-moment thresholds.
//!
//! Per-pane display thresholds and field layers ride along, since a pane that shows one field
//! above 50 dBZ beside another that shows a different one is exactly the arrangement worth saving.

use crate::view::MapView;

pub const MAX_PANES: usize = 4;

pub fn bounded_pane_count(requested: usize) -> usize {
    requested.clamp(1, MAX_PANES)
}

/// One saved pane arrangement.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Workspace {
    pub name: String,
    pub panes: Vec<PaneSnap>,
    /// Which pane was focused.
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub link_cameras: bool,
    #[serde(default)]
    pub link_times: bool,
    /// Overlay toggles that were on, by slug — the same names `Settings::overlays_on` uses, so an
    /// unknown one from a newer build is skipped rather than fatal.
    #[serde(default)]
    pub overlays_on: Vec<String>,
    /// Fill panes that have no site from whatever site is on screen when the workspace is
    /// applied. Set on the starter workspaces, which describe an arrangement rather than a place:
    /// "two panes of the same radar" has to mean *your* radar, not one shipped in a default file.
    #[serde(default)]
    pub adopt_site: bool,
    /// National field layers that were on, by slug. Same forward-compatibility rule as the
    /// overlays; `default` so a workspace written before this field existed still loads.
    #[serde(default)]
    pub fields_on: Vec<String>,
    /// What the chrome looked like: which panel was showing, which drawer page was open. `None`
    /// on a workspace written before this field existed, and on the starters — both mean "leave
    /// the chrome where the user left it" rather than "close everything", which is what a
    /// defaulted struct would have meant.
    #[serde(default)]
    pub chrome: Option<Chrome>,
}


/// Public scene links carry map state only; settings, markers and source credentials never enter
/// this schema. Keep the uncompressed URL bounded before allocating or parsing JSON.
pub const MAX_SCENE_TOKEN_BYTES: usize = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneTimeMode { Live, Exact }


/// Public selectors required to interpret national/model field IDs consistently.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneProducts {
    pub global_model: wxdata::global::GlobalModel,
    pub global_fcst_hour: u16,
    pub analysis_source: wxdata::rtma::Source,
    pub hrrr_fcst_hour: u8,
    pub refs_fcst_hour: u8,
    pub refs_dbz_threshold: u8,
    pub env_model: wxdata::hrrr::Model,
    pub env_cape_ml: bool,
    pub env_srh_km: u8,
    pub snow_hours: u16,
    pub lightning_minutes: u16,
    pub hail_minutes: u16,
    pub rotation_minutes: u16,
    pub wind_level: wxdata::hrrr::WindLevel,
    pub diff_field: crate::fielddiff::DiffField,
    pub tropical_wind_kt: Option<u8>,
    pub tropical_surge: bool,
    pub storm_motion: Vec<(f32, f32)>,
}

impl SceneProducts {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.global_fcst_hour <= 384 && self.hrrr_fcst_hour <= 48
            && (1..=60).contains(&self.refs_fcst_hour) && self.refs_dbz_threshold <= 80
            && matches!(self.env_srh_km, 1 | 3) && (1..=168).contains(&self.snow_hours)
            && (1..=1440).contains(&self.lightning_minutes)
            && (1..=1440).contains(&self.hail_minutes) && (1..=1440).contains(&self.rotation_minutes)
            && self.tropical_wind_kt.is_none_or(|kt| matches!(kt, 34 | 50 | 64)),
            "Invalid public product selection");
        anyhow::ensure!(!matches!(self.global_model, wxdata::global::GlobalModel::GefsMember(member) if member > 30),
            "Invalid GEFS member");
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedScene {
    version: u8,
    pub workspace: Workspace,
    pub time_mode: SceneTimeMode,
    pub times: Vec<Option<chrono::DateTime<chrono::Utc>>>,
    pub filters: crate::app::OverlayFilters,
    pub products: SceneProducts,
    pub lock_source_frame: bool,
    pub link_satellite_times: bool,
}

impl SharedScene {
    pub fn new(
        mut workspace: Workspace,
        time_mode: SceneTimeMode,
        mut times: Vec<Option<chrono::DateTime<chrono::Utc>>>,
        filters: crate::app::OverlayFilters,
        products: SceneProducts,
        lock_source_frame: bool,
        link_satellite_times: bool,
    ) -> anyhow::Result<Self> {
        // Saved workspace titles and drawer names may contain personal information.
        workspace.name = "Shared scene".into();
        workspace.adopt_site = false;
        workspace.chrome = workspace.chrome.map(|chrome| Chrome {
            analyst_open: chrome.analyst_open,
            analyst_inspector_open: chrome.analyst_inspector_open,
            ..Default::default()
        });
        workspace.overlays_on.retain(|slug| public_scene_overlay(slug));
        for pane in &mut workspace.panes {
            anyhow::ensure!(pane.custom_product.is_none(),
                "Switch user-defined products to a built-in radar product before sharing");
            // Palettes are public color tables, but can make a chat URL enormous. Named custom
            // products and private/provider-key maps are not portable between installations.
            pane.appearance.palettes.clear();
            let style = crate::tiles::BasemapStyle::from_slug(&pane.basemap);
            if style == crate::tiles::BasemapStyle::CustomXyz
                || style.provider_kind() != crate::tiles::Provider::Builtin
            {
                pane.basemap = "dark".into();
            }
            if time_mode == SceneTimeMode::Live {
                pane.satellite.time = None;
                pane.satellite.follow_radar = true;
            }
        }
        if time_mode == SceneTimeMode::Live { times.fill(None); }
        let scene = Self { version: 1, workspace, time_mode, times, filters, products,
            lock_source_frame, link_satellite_times };
        scene.validate()?;
        Ok(scene)
    }

    pub fn encode(&self) -> anyhow::Result<String> {
        use base64::Engine;
        self.validate()?;
        let json = serde_json::to_vec(self)?;
        anyhow::ensure!(json.len() <= MAX_SCENE_TOKEN_BYTES / 4 * 3,
            "This scene is too large for a share link");
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
    }

    pub fn decode(token: &str) -> anyhow::Result<Self> {
        use base64::Engine;
        anyhow::ensure!(!token.is_empty() && token.len() <= MAX_SCENE_TOKEN_BYTES,
            "Scene link is empty or too large");
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token)?;
        let scene: Self = serde_json::from_slice(&json)?;
        scene.validate()?;
        Ok(scene)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let ws = &self.workspace;
        self.products.validate()?;
        anyhow::ensure!(self.products.storm_motion.len() == ws.panes.len()
            && self.products.storm_motion.iter().all(|(direction, speed)|
                direction.is_finite() && (0.0..=360.0).contains(direction)
                && speed.is_finite() && (0.0..=300.0).contains(speed)),
            "Invalid storm-relative motion");
        anyhow::ensure!(self.version == 1, "Unsupported scene link version");
        anyhow::ensure!((1..=MAX_PANES).contains(&ws.panes.len())
            && ws.active < ws.panes.len() && self.times.len() == ws.panes.len(),
            "Invalid scene pane count");
        anyhow::ensure!(ws.name == "Shared scene" && !ws.adopt_site,
            "Invalid public workspace");
        if let Some(chrome) = &ws.chrome {
            anyhow::ensure!(!chrome.panel_open && !chrome.alerts_tab && !chrome.basemap_open
                && chrome.drawer.is_none() && chrome.drawer_id.is_none(),
                "Scene links cannot open settings or private drawers");
        }
        anyhow::ensure!(ws.overlays_on.len() <= 64
            && ws.overlays_on.iter().all(|slug| public_scene_overlay(slug)),
            "Invalid public overlay");
        validate_scene_fields(&ws.fields_on)?;
        for pane in &ws.panes {
            anyhow::ensure!(pane.site.as_ref().is_none_or(|site|
                site.len() <= 8 && wxdata::sites::site_by_id(site).is_some()),
                "Unknown radar station in scene");
            anyhow::ensure!(pane.lon.is_finite() && (-180.0..=180.0).contains(&pane.lon)
                && pane.lat.is_finite() && (-85.051_129..=85.051_129).contains(&pane.lat)
                && pane.zoom.is_finite() && (-2.0..=crate::render::mercator::MAX_ZOOM).contains(&pane.zoom)
                && pane.pitch_deg.is_finite() && (0.0..=60.0).contains(&pane.pitch_deg)
                && pane.bearing_deg.is_finite() && (0.0..=360.0).contains(&pane.bearing_deg)
                && pane.tilt <= 64, "Invalid scene camera or radar tilt");
            let style = crate::tiles::BasemapStyle::from_slug(&pane.basemap);
            anyhow::ensure!(style.slug() == pane.basemap
                && style != crate::tiles::BasemapStyle::CustomXyz
                && style.provider_kind() == crate::tiles::Provider::Builtin,
                "Scene basemap must be a public built-in source");
            anyhow::ensure!(pane.custom_product.is_none() && pane.appearance.palettes.is_empty(),
                "Scene links do not import custom products or palettes");
            if let Some(fields) = &pane.fields_on { validate_scene_fields(fields)?; }
            anyhow::ensure!(pane.thresholds.len() <= wxdata::level2::Moment::ALL.len()
                && pane.thresholds.iter().all(|(moment, value)| {
                    let (lo, hi) = moment.value_range();
                    value.is_finite() && (lo..=hi).contains(value)
                }), "Invalid scene threshold");
            let mut appearance = pane.appearance.clone();
            appearance.sanitize();
            anyhow::ensure!(appearance == pane.appearance, "Invalid scene display settings");
            anyhow::ensure!(pane.satellite.source_slug.as_ref().is_none_or(|slug|
                slug.len() <= 80 && crate::tiles::BasemapStyle::from_slug(slug).slug() == slug),
                "Invalid imagery source");
            if let Some(time) = pane.satellite.time { validate_scene_time(time)?; }
            if self.time_mode == SceneTimeMode::Live {
                anyhow::ensure!(pane.satellite.time.is_none(), "Live scene contains a fixed imagery time");
            }
        }
        match self.time_mode {
            SceneTimeMode::Live => anyhow::ensure!(self.times.iter().all(Option::is_none),
                "Live scene contains an archive time"),
            SceneTimeMode::Exact => {
                anyhow::ensure!(self.times.iter().any(Option::is_some)
                    && ws.panes.iter().zip(&self.times).all(|(pane, time)|
                        time.is_some() || (!pane.show_radar && pane.fields_on.as_ref().is_none_or(Vec::is_empty))),
                    "Wait for every visible data pane to load before sharing this exact time");
            },
        }
        for time in self.times.iter().flatten() { validate_scene_time(*time)?; }
        let f = &self.filters;
        anyhow::ensure!(f.outlook_day <= 8 && f.fire_weather_day <= 8
            && f.wssi_day <= 4 && f.winter_snow_day <= 3 && f.winter_ice_day <= 3
            && f.ero_day <= 5 && f.cpc_opacity_pct <= 100 && f.ero_opacity_pct <= 100
            && f.ero_border_px.is_finite() && (0.1..=10.0).contains(&f.ero_border_px)
            && f.nowcast_lead_min <= 120, "Invalid outlook settings");
        anyhow::ensure!(f.ero_archive.as_ref().is_none_or(|key|
            key.len() <= 40 && key.ends_with(".zip")
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')),
            "Invalid WPC archive key");
        Ok(())
    }

    /// Discard any previous playback cursor before applying a public scene to an existing pane.
    pub fn apply_times(&self, views: &mut [MapView]) {
        for ((view, time), (direction, speed)) in views.iter_mut().zip(&self.times).zip(&self.products.storm_motion) {
            view.storm_dir_deg = *direction;
            view.storm_speed_kt = *speed;
            view.timeline = Default::default();
            if let Some(time) = time {
                view.timeline.date = time.date_naive();
                view.timeline.following = false;
                view.timeline.playing = false;
                view.timeline.seek_target = Some(*time);
            }
        }
    }
}

fn public_scene_overlay(slug: &str) -> bool {
    crate::app::OverlayToggle::from_slug(slug).is_some_and(|toggle|
        !toggle.session_only() && !matches!(toggle,
            crate::app::OverlayToggle::Sensors | crate::app::OverlayToggle::Strikes))
}

fn validate_scene_fields(fields: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(fields.len() <= 256 && fields.iter().all(|slug|
        slug.len() <= 96 && crate::render::FieldLayer::from_stable_id(slug).is_some()),
        "Invalid scene field");
    Ok(())
}

fn validate_scene_time(time: chrono::DateTime<chrono::Utc>) -> anyhow::Result<()> {
    use chrono::Datelike;
    anyhow::ensure!((1970..=2100).contains(&time.year()), "Invalid scene time");
    Ok(())
}


#[cfg(test)]
mod scene_tests {
    use super::*;
    use base64::Engine;

    fn at(hour: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
            .and_hms_opt(hour, 0, 0).unwrap().and_utc()
    }

    fn scene(mode: SceneTimeMode) -> SharedScene {
        let mut workspace = starters().remove(2);
        for (pane, site) in workspace.panes.iter_mut().zip(["KTLX", "KFWS", "KDMX", "KLOT"]) {
            pane.site = Some(site.into());
        }
        workspace.panes[1].moment = wxdata::level2::Moment::Velocity;
        workspace.panes[1].appearance.opacity = 0.5;
        workspace.panes[2].fields_on = Some(vec!["mrms".into()]);
        workspace.active = 1;
        workspace.chrome = Some(Chrome { analyst_open: true, ..Default::default() });
        let products = SceneProducts {
            global_model: wxdata::global::GlobalModel::Ecmwf,
            global_fcst_hour: 12,
            analysis_source: wxdata::rtma::Source::Urma,
            hrrr_fcst_hour: 3,
            refs_fcst_hour: 6,
            refs_dbz_threshold: 40,
            env_model: wxdata::hrrr::Model::Rap,
            env_cape_ml: true,
            env_srh_km: 3,
            snow_hours: 24,
            lightning_minutes: 5,
            hail_minutes: 60,
            rotation_minutes: 60,
            wind_level: wxdata::hrrr::WindLevel::Steering,
            diff_field: crate::fielddiff::DiffField::Cape,
            tropical_wind_kt: Some(34),
            tropical_surge: true,
            storm_motion: vec![(240.0, 25.0); 4],
        };
        let filters = crate::app::OverlayFilters { outlook_day: 2, ero_day: 3,
            cpc_period: Some(wxdata::cpc::Period::Day8To14), ..Default::default() };
        SharedScene::new(workspace, mode, vec![Some(at(12)), Some(at(13)), Some(at(14)), Some(at(15))],
            filters, products, true, false).unwrap()
    }

    #[test]
    fn shared_scene_roundtrip_retains_four_sites_products_layers_and_time_links() {
        let scene = scene(SceneTimeMode::Exact);
        let token = scene.encode().unwrap();
        assert!(token.len() < MAX_SCENE_TOKEN_BYTES);
        let round = SharedScene::decode(&token).unwrap();
        assert_eq!(round, scene);
        assert_eq!(round.workspace.panes[1].site.as_deref(), Some("KFWS"));
        assert_eq!(round.workspace.panes[1].appearance.opacity, 0.5);
        assert!(round.workspace.link_times && round.workspace.link_cameras && round.lock_source_frame);
        assert!(!round.link_satellite_times);
        assert_eq!(round.filters.outlook_day, 2);
    }

    #[test]
    fn shared_scene_live_drops_all_times_and_fixed_resets_previous_playback() {
        let live = scene(SceneTimeMode::Live);
        assert!(live.times.iter().all(Option::is_none));
        let mut views: Vec<_> = live.workspace.panes.iter().map(|pane| {
            let mut view = MapView::new(pane.site.clone(),
                crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 2.0));
            view.timeline.seek_target = Some(at(1));
            view.timeline.following = false;
            view.timeline.playing = true;
            view.timeline.replay = Some((3, 8));
            view
        }).collect();
        live.apply_times(&mut views);
        assert!(views.iter().all(|v| v.timeline.following && v.timeline.seek_target.is_none()
            && v.timeline.replay.is_none()));
        let fixed = scene(SceneTimeMode::Exact);
        fixed.apply_times(&mut views);
        assert_eq!(views[1].timeline.seek_target, Some(at(13)));
        assert!(views.iter().all(|v| !v.timeline.following && !v.timeline.playing));
        assert_eq!(views[1].storm_speed_kt, 25.0);
        let mut invalid = fixed;
        invalid.times[0] = None;
        assert!(invalid.encode().is_err(), "An unloaded radar pane must not silently become live");
    }

    #[test]
    fn shared_scene_restored_camera_preserves_independent_station_intent() {
        let scene = scene(SceneTimeMode::Exact);
        let views: Vec<_> = scene.workspace.panes.iter().map(|pane| {
            let mut view = MapView::new(Some("KTLX".into()),
                crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 2.0));
            pane.apply(&mut view);
            view
        }).collect();
        // sync_pane propagates explicit station selection only when camera_placed is false.
        assert!(views.iter().all(|view| view.camera_placed));
        assert_eq!(views.iter().filter_map(|v| v.site.as_deref()).collect::<Vec<_>>(),
            ["KTLX", "KFWS", "KDMX", "KLOT"]);
    }

    #[test]
    fn shared_scene_rejects_malformed_oversized_unknown_and_unsafe_values() {
        assert!(SharedScene::decode("not json").is_err());
        assert!(SharedScene::decode(&"a".repeat(MAX_SCENE_TOKEN_BYTES + 1)).is_err());
        let baseline = serde_json::to_value(scene(SceneTimeMode::Exact)).unwrap();
        for (pointer, value) in [
            ("/version", serde_json::json!(9)),
            ("/workspace/active", serde_json::json!(99)),
            ("/workspace/panes/0/site", serde_json::json!("../../evil")),
            ("/workspace/panes/0/lon", serde_json::json!(999)),
            ("/workspace/panes/0/zoom", serde_json::json!(90)),
            ("/workspace/panes/0/basemap", serde_json::json!("custom")),
            ("/workspace/panes/0/custom_product", serde_json::json!("private")),
            ("/workspace/panes/0/fields_on", serde_json::json!(["https://evil.invalid"])),
            ("/workspace/overlays_on", serde_json::json!(["Sensors"])),
            ("/filters/outlook_day", serde_json::json!(250)),
            ("/filters/ero_archive", serde_json::json!("../../secret.zip")),
            ("/products/global_model", serde_json::json!({"GefsMember": 255})),
            ("/time_mode", serde_json::json!("live")),
        ] {
            let mut invalid = baseline.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            let token = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&invalid).unwrap());
            assert!(SharedScene::decode(&token).is_err(), "{pointer}");
        }
        let mut invalid = baseline;
        invalid["mapbox_key"] = serde_json::json!("do-not-import");
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&invalid).unwrap());
        assert!(SharedScene::decode(&token).is_err());
    }

    #[test]
    fn shared_scene_scrubs_private_workspace_names_and_keyed_basemaps() {
        let baseline = scene(SceneTimeMode::Exact);
        let mut workspace = baseline.workspace;
        workspace.name = "My secret home".into();
        workspace.chrome = Some(Chrome { drawer: Some("Secret".into()), drawer_id: Some("Settings".into()),
            panel_open: true, ..Default::default() });
        workspace.overlays_on.push("Sensors".into());
        workspace.panes[0].basemap = "mapbox-dark".into();
        let public = SharedScene::new(workspace, baseline.time_mode, baseline.times,
            baseline.filters, baseline.products, baseline.lock_source_frame, baseline.link_satellite_times).unwrap();
        let json = serde_json::to_string(&public).unwrap();
        assert!(!json.contains("secret") && !json.contains("Secret") && !json.contains("mapbox"));
        assert_eq!(public.workspace.panes[0].basemap, "dark");
        assert!(public.workspace.chrome.unwrap().drawer.is_none());
    }
}

impl Workspace {
    pub fn adopted_site(&self, current: Option<&str>, default: &str) -> Option<String> {
        self.adopt_site
            .then(|| current.unwrap_or(default).to_owned())
    }
}

/// The floating chrome's state, as far as it is worth restoring: which surface was showing, not
/// how far it was scrolled.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Chrome {
    #[serde(default)]
    pub panel_open: bool,
    /// Optional workbench docks; older workspaces continue to open the plain map.
    #[serde(default)]
    pub analyst_open: bool,
    #[serde(default)]
    pub analyst_inspector_open: bool,
    /// The panel's Alerts tab rather than Data.
    #[serde(default)]
    pub alerts_tab: bool,
    #[serde(default)]
    pub basemap_open: bool,
    /// Title of the drawer page on top, if any — matched back to a window by
    /// `app::chrome::window_for_page`. A title this build no longer has is skipped, same rule as
    /// the overlay slugs.
    #[serde(default)]
    pub drawer: Option<String>,
    /// Stable window variant for new builds; `drawer` remains for old workspace readers.
    #[serde(default)]
    pub drawer_id: Option<String>,
}

/// One pane's state. Camera as lon/lat/zoom, basemap as its slug: both survive a file written by
/// a different build.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PaneSnap {
    pub site: Option<String>,
    /// Keep source selection independent of the remembered radar station/product.
    #[serde(default = "visible_by_default")]
    pub show_radar: bool,
    #[serde(default = "visible_by_default")]
    pub show_legend: bool,
    pub moment: wxdata::level2::Moment,
    #[serde(default)]
    pub custom_product: Option<String>,
    pub tilt: usize,
    #[serde(default)]
    pub srv: bool,
    pub basemap: String,
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
    /// Field layers this pane drew, by slug. Same forward-compatibility rule as the overlays: a
    /// slug this build does not have is skipped.
    ///
    /// `None` is a snapshot written before per-pane layers existed, which falls back to the
    /// workspace-wide list so an old file still restores what it meant. `Some([])` is a pane that
    /// deliberately had no layers on, and has to come back that way — as a plain `Vec` the two
    /// were the same value, so a pane you had cleared came back wearing the union of every other
    /// pane's layers.
    #[serde(default)]
    pub fields_on: Option<Vec<String>>,
    /// Enabled display thresholds, as `(moment, physical value)`. Only the enabled ones are
    /// stored: a threshold that was set but switched off is not part of the arrangement.
    #[serde(default)]
    pub thresholds: Vec<(wxdata::level2::Moment, f32)>,
    #[serde(default)]
    pub appearance: crate::radar_appearance::RadarAppearance,
    #[serde(default)]
    pub satellite: crate::satellite_time::Selection,
    #[serde(default)]
    pub abi_scene: Option<wxdata::abi::Scene>,
    /// Older layouts inherit the app default. New layouts retain the pane choice.
    #[serde(default)]
    pub smooth: Option<bool>,
    #[serde(default)]
    pub globe: bool,
    #[serde(default)]
    pub pitch_deg: f64,
    #[serde(default)]
    pub bearing_deg: f64,
}

fn visible_by_default() -> bool { true }

impl PaneSnap {
    /// Snapshot a live pane.
    pub fn capture(v: &MapView) -> Self {
        let (lon, lat) =
            crate::render::mercator::world_to_lonlat(v.camera.center.0, v.camera.center.1);
        Self {
            site: v.site.clone(),
            show_radar: v.show_radar,
            show_legend: v.show_legend,
            moment: v.moment,
            custom_product: v.custom_product.clone(),
            tilt: v.tilt,
            srv: v.srv,
            basemap: v.basemap.slug().to_string(),
            lon,
            lat,
            zoom: v.camera.zoom,
            fields_on: Some(
                crate::render::FieldLayer::draw_order()
                    .filter(|l| v.fields_on.contains(l))
                    .map(|l| l.stable_id().to_string())
                    .collect(),
            ),
            appearance: v.appearance.clone(),
            satellite: v.satellite.clone(),
            abi_scene: v.abi_scene,
            smooth: Some(v.smooth),
            globe: v.camera.globe,
            pitch_deg: v.camera.pitch_deg,
            bearing_deg: v.camera.bearing_deg,
            thresholds: wxdata::level2::Moment::ALL
                .iter()
                .enumerate()
                .filter(|&(i, _)| v.threshold_enabled[i])
                .filter_map(|(i, m)| v.thresholds[i].map(|t| (*m, t)))
                .collect(),
        }
    }

    /// Apply this snapshot to a pane. The volume itself isn't restored — a pane with a site and no
    /// data fetches through the normal poll path, which is also what a fresh pane does.
    pub fn apply(&self, v: &mut MapView) {
        v.site = self.site.clone();
        v.show_radar = self.show_radar;
        v.show_legend = self.show_legend;
        v.moment = self.moment;
        v.custom_product = self.custom_product.clone();
        v.tilt = self.tilt;
        v.follow_low_cut = false; // a saved tilt takes precedence over this session-only live mode
        v.srv = self.srv;
        v.satellite = self.satellite.clone();
        v.abi_scene = self.abi_scene;
        v.appearance = self.appearance.clone();
        v.appearance.sanitize();
        if let Some(smooth) = self.smooth { v.smooth = smooth; }
        v.basemap = crate::tiles::BasemapStyle::from_slug(&self.basemap);
        v.camera = crate::render::mercator::Camera::at_lonlat(self.lon, self.lat, self.zoom);
        v.camera.globe = self.globe;
        v.camera.set_orientation(self.pitch_deg, self.bearing_deg);
        // The camera came from the saved layout, not from a site recenter — hold it through the
        // site change the restore just triggered.
        v.camera_placed = true;
        // Thresholds are a per-moment array; clear it first so restoring an arrangement without
        // one does not leave the previous pane's filter in place.
        v.thresholds = Default::default();
        v.threshold_enabled = Default::default();
        for (m, t) in &self.thresholds {
            v.thresholds[m.index()] = Some(*t);
            v.threshold_enabled[m.index()] = true;
        }
    }
}

/// Pane at a site-less default camera: the starters describe arrangements, and `adopt_site`
/// fills the site in when one is applied.
fn pane(moment: wxdata::level2::Moment, tilt: usize, srv: bool) -> PaneSnap {
    PaneSnap {
        site: None,
        show_radar: true,
        show_legend: true,
        moment,
        custom_product: None,
        tilt,
        srv,
        basemap: "dark".into(),
        // Centered on the southern plains at a single-radar zoom; overwritten the moment the
        // pane adopts a site and recenters on it.
        lon: -97.5,
        lat: 35.5,
        zoom: 7.5,
        // `None`, not an empty list: a starter describes an arrangement, and its panes take the
        // workspace-wide layers rather than asserting that every pane is bare.
        fields_on: None,
        thresholds: vec![(wxdata::level2::Moment::Reflectivity, 16.0)],
        appearance: Default::default(),
        satellite: Default::default(),
                abi_scene: None,
        smooth: None,
        globe: false,
        pitch_deg: 0.0,
        bearing_deg: 0.0,
    }
}

/// The three arrangements worth having before you have built any of your own. Seeded once, on
/// first run; deleting them is final (see `Settings::seeded_workspaces`).
pub fn starters() -> Vec<Workspace> {
    use wxdata::level2::Moment;
    vec![
        Workspace {
            name: "Chase".into(),
            // Reflectivity beside storm-relative velocity, same radar, cameras locked: the
            // couplet and the hook in one glance.
            panes: vec![
                pane(Moment::Reflectivity, 0, false),
                pane(Moment::Velocity, 0, true),
            ],
            active: 0,
            link_cameras: true,
            link_times: true,
            overlays_on: vec![
                "Alerts".into(),
                "Cells".into(),
                "RadarSites".into(),
                "Spotters".into(),
                "StormReports".into(),
                "Tds".into(),
            ],
            adopt_site: true,
            fields_on: Vec::new(),
            chrome: None,
        },
        Workspace {
            name: "National overview".into(),
            // One pane, no site, the MRMS mosaic under the warnings — what is happening anywhere.
            panes: vec![PaneSnap {
                site: None,
                show_radar: true,
                show_legend: true,
                moment: Moment::Reflectivity,
                custom_product: None,
                tilt: 0,
                srv: false,
                basemap: "dark".into(),
                lon: -97.0,
                lat: 38.5,
                zoom: 4.0,
                fields_on: Some(vec!["mrms".into()]),
                thresholds: vec![(Moment::Reflectivity, 16.0)],
                appearance: Default::default(),
                satellite: Default::default(),
                abi_scene: None,
                smooth: None,
        globe: false,
        pitch_deg: 0.0,
        bearing_deg: 0.0,
            }],
            active: 0,
            link_cameras: false,
            link_times: false,
            overlays_on: vec![
                "Alerts".into(),
                "RadarSites".into(),
                "StormReports".into(),
                "Fronts".into(),
            ],
            adopt_site: false,
            fields_on: vec!["mrms".into()],
            chrome: None,
        },
        Workspace {
            name: "Analysis".into(),
            // The same storm at four heights: how a couplet leans with height, which is the
            // layout people rebuild by hand every time.
            panes: (0..4)
                .map(|t| pane(Moment::Reflectivity, t, false))
                .collect(),
            active: 0,
            link_cameras: true,
            link_times: true,
            overlays_on: vec![
                "Alerts".into(),
                "Cells".into(),
                "RadarSites".into(),
                "RangeRings".into(),
            ],
            adopt_site: true,
            fields_on: Vec::new(),
            chrome: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn abi_scene_survives_workspace_without_becoming_global() {
        let mut view = crate::view::MapView::new(None, crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 4.0));
        view.abi_scene = Some(wxdata::abi::Scene::Mesoscale2);
        let snapshot = super::PaneSnap::capture(&view);
        let restored: super::PaneSnap = serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        view.abi_scene = Some(wxdata::abi::Scene::Conus);
        restored.apply(&mut view);
        assert_eq!(view.abi_scene, Some(wxdata::abi::Scene::Mesoscale2));
    }

    use super::*;

    #[test]
    fn independent_satellite_selections_survive_workspace_roundtrip() {
        let mut view = MapView::new(Some("KTLX".into()), crate::render::mercator::Camera::at_lonlat(-97.0, 35.0, 7.0));
        view.satellite = crate::satellite_time::Selection { follow_radar: false, time: chrono::DateTime::from_timestamp(1_700_000_000, 0), source_slug: None };
        let snapshot = PaneSnap::capture(&view);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: PaneSnap = serde_json::from_str(&encoded).unwrap();
        let mut second = MapView::new(None, view.camera);
        decoded.apply(&mut second);
        assert_eq!(second.satellite, view.satellite);
        second.satellite.time = None;
        assert_ne!(second.satellite, view.satellite);
    }

    #[test]
    fn starters_can_always_reach_radar() {
        let starters = starters();
        assert!(starters
            .iter()
            .all(|ws| ws.overlays_on.iter().any(|overlay| overlay == "RadarSites")));
        assert_eq!(starters[0].adopted_site(None, "KTLX").as_deref(), Some("KTLX"));
        assert_eq!(starters[2].adopted_site(Some("KFWS"), "KTLX").as_deref(), Some("KFWS"));
        assert_eq!(starters[1].adopted_site(None, "KTLX"), None);
    }

    #[test]
    fn pane_count_has_a_fixed_resource_ceiling() {
        assert_eq!(bounded_pane_count(0), 1);
        assert_eq!(bounded_pane_count(2), 2);
        assert_eq!(bounded_pane_count(99), MAX_PANES);
    }

    #[test]
    fn pane_roundtrips_through_a_view() {
        let mut v = MapView::new(
            Some("KTLX".into()),
            crate::render::mercator::Camera::at_lonlat(-97.28, 35.33, 8.5),
        );
        v.moment = wxdata::level2::Moment::Velocity;
        v.tilt = 2;
        v.srv = true;
        v.basemap = crate::tiles::BasemapStyle::from_slug("satellite");

        let snap = PaneSnap::capture(&v);
        let mut fresh = MapView::new(
            None,
            crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0),
        );
        snap.apply(&mut fresh);

        assert_eq!(fresh.site.as_deref(), Some("KTLX"));
        assert_eq!(fresh.moment, wxdata::level2::Moment::Velocity);
        assert_eq!(fresh.tilt, 2);
        assert!(fresh.srv && fresh.camera_placed);
        assert_eq!(fresh.basemap, v.basemap);
        // Camera survives the lon/lat round trip to within a pixel at this zoom.
        assert!((fresh.camera.center.0 - v.camera.center.0).abs() < 1e-9);
        assert!((fresh.camera.center.1 - v.camera.center.1).abs() < 1e-9);
        assert_eq!(fresh.camera.zoom, 8.5);
    }

    #[test]
    fn per_pane_thresholds_and_layers_survive_the_round_trip() {
        use wxdata::level2::Moment;
        let mut v = MapView::new(
            None,
            crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0),
        );
        v.fields_on.insert(crate::render::FieldLayer::Mrms);
        v.thresholds[Moment::Reflectivity.index()] = Some(35.0);
        v.threshold_enabled[Moment::Reflectivity.index()] = true;
        v.appearance.upper_bounds[Moment::Reflectivity.index()] = Some(65.0);
        v.appearance.blend_width = 0.8;
        v.appearance.precision = true;
        v.appearance.multipass = true;
        v.camera.globe = true;
        v.appearance.palettes.insert("REF".into(), crate::colormap::default_table(Moment::Reflectivity).clone());
        v.smooth = false;
        // Set but switched off: part of the session, not part of the arrangement.
        v.thresholds[Moment::Velocity.index()] = Some(20.0);

        let snap = PaneSnap::capture(&v);
        assert_eq!(
            snap.fields_on.as_deref(),
            Some(["mrms.composite-reflectivity".to_string()].as_slice())
        );
        assert_eq!(snap.thresholds, vec![(Moment::Reflectivity, 35.0)]);

        let mut fresh = MapView::new(
            None,
            crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0),
        );
        // A leftover filter from whatever this pane was showing before must not survive a restore.
        fresh.thresholds[Moment::SpectrumWidth.index()] = Some(4.0);
        fresh.threshold_enabled[Moment::SpectrumWidth.index()] = true;
        snap.apply(&mut fresh);
        assert_eq!(fresh.thresholds[Moment::Reflectivity.index()], Some(35.0));
        assert!(fresh.threshold_enabled[Moment::Reflectivity.index()]);
        assert!(!fresh.threshold_enabled[Moment::SpectrumWidth.index()]);
        assert_eq!(fresh.thresholds[Moment::Velocity.index()], None);
        assert_eq!(fresh.appearance, v.appearance);
        assert!(!fresh.smooth);
        assert!(fresh.camera.globe);
    }

    #[test]
    fn a_pane_with_no_layers_says_so_rather_than_saying_nothing() {
        // The distinction the `Option` exists for. Capturing a bare pane must record "no layers",
        // not "no opinion" — as a plain `Vec` both were `[]`, and a pane you had cleared came back
        // wearing the union of every other pane's layers when the workspace was applied.
        let v = MapView::new(None, crate::render::mercator::Camera::at_lonlat(-97.3, 35.3, 8.0));
        let snap = PaneSnap::capture(&v);
        assert_eq!(snap.fields_on, Some(Vec::new()));
    }

    #[test]
    fn a_pane_written_before_per_pane_layers_still_loads() {
        // No `fields_on`, no `thresholds` — exactly what an older build wrote.
        let snap: PaneSnap = serde_json::from_str(
            r#"{"site":"KTLX","moment":"Reflectivity","tilt":0,"basemap":"dark",
                "lon":-97.3,"lat":35.3,"zoom":8.0}"#,
        )
        .expect("old pane snapshots still parse");
        // `None`, not an empty list: this file predates per-pane layers and says nothing about
        // them, so the restore falls back to the workspace-wide list. A pane that really had its
        // layers off captures `Some([])`, which is a different instruction.
        assert_eq!(snap.fields_on, None);
        assert!(snap.thresholds.is_empty());
    }

    #[test]
    fn workspace_roundtrips_through_json() {
        let ws = Workspace {
            name: "Two-site chase".into(),
            panes: vec![PaneSnap {
                site: Some("KDMX".into()),
                show_radar: true,
                show_legend: true,
                moment: wxdata::level2::Moment::CorrelationCoefficient,
                custom_product: Some("Debris score".into()),
                tilt: 1,
                srv: false,
                basemap: "dark".into(),
                lon: -93.72,
                lat: 41.73,
                zoom: 7.25,
                fields_on: None,
                thresholds: Vec::new(),
                appearance: Default::default(),
                satellite: Default::default(),
                abi_scene: None,
                smooth: None,
        globe: false,
        pitch_deg: 0.0,
        bearing_deg: 0.0,
            }],
            active: 0,
            link_cameras: true,
            link_times: true,
            overlays_on: vec!["Alerts".into(), "Cells".into()],
            adopt_site: false,
            fields_on: vec!["mrms".into()],
            chrome: Some(Chrome {
                panel_open: true,
                analyst_open: true,
                analyst_inspector_open: true,
                alerts_tab: false,
                basemap_open: false,
                drawer: Some("Settings".into()),
                drawer_id: Some("Settings".into()),
            }),
        };
        let json = serde_json::to_string(&ws).unwrap();
        assert_eq!(serde_json::from_str::<Workspace>(&json).unwrap(), ws);
    }

    #[test]
    fn old_workspace_files_still_load() {
        // Written before `fields_on` and `adopt_site` existed.
        let json = r#"{"name":"old","panes":[],"active":0,"link_cameras":false,
            "overlays_on":["Alerts"]}"#;
        let ws: Workspace = serde_json::from_str(json).unwrap();
        assert!(
            ws.fields_on.is_empty()
                && !ws.adopt_site
                && !ws.link_times
                && ws.chrome.is_none()
        );
    }

    #[test]
    fn old_chrome_keeps_plain_map_as_default() {
        let chrome: Chrome = serde_json::from_str(
            r#"{"panel_open":true,"alerts_tab":false,"drawer":"Settings"}"#,
        ).unwrap();
        assert!(chrome.panel_open);
        assert!(!chrome.analyst_open);
        assert!(!chrome.analyst_inspector_open);
        assert!(chrome.drawer_id.is_none());
    }

    #[test]
    fn every_starter_names_things_this_build_has() {
        for ws in starters() {
            assert!(!ws.panes.is_empty(), "{} has no panes", ws.name);
            assert!(ws.active < ws.panes.len());
            for slug in &ws.fields_on {
                assert!(
                    crate::render::FieldLayer::from_stable_id(slug).is_some(),
                    "{}: unknown field layer {slug}",
                    ws.name
                );
            }
            for p in &ws.panes {
                // `from_slug` falls back to Dark on an unknown name, so compare round trips.
                let style = crate::tiles::BasemapStyle::from_slug(&p.basemap);
                assert_eq!(style.slug(), p.basemap, "{}: bad basemap", ws.name);
            }
        }
    }
    #[test]
    fn pitched_camera_workspace_roundtrip_and_legacy_defaults() {
        let mut v=MapView::new(None,crate::render::mercator::Camera::at_lonlat(-97.0,35.0,8.0));
        v.camera.set_orientation(58.0,275.0);
        let encoded=serde_json::to_string(&PaneSnap::capture(&v)).unwrap();
        let decoded:PaneSnap=serde_json::from_str(&encoded).unwrap();
        let mut restored=MapView::new(None,crate::render::mercator::Camera::at_lonlat(-97.0,35.0,8.0));decoded.apply(&mut restored);
        assert_eq!((restored.camera.pitch_deg,restored.camera.bearing_deg),(58.0,275.0));
        let mut old:serde_json::Value=serde_json::from_str(&encoded).unwrap();
        old.as_object_mut().unwrap().remove("pitch_deg");old.as_object_mut().unwrap().remove("bearing_deg");
        let old:PaneSnap=serde_json::from_value(old).unwrap();old.apply(&mut restored);
        assert_eq!((restored.camera.pitch_deg,restored.camera.bearing_deg),(0.0,0.0));
    }

    #[test]
    fn hidden_radar_and_legend_survive_source_snapshot_without_losing_product() {
        use crate::render::{FieldLayer, mercator::Camera};
        use wxdata::level2::Moment;
        let mut pane = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.28, 35.33, 8.0));
        pane.moment = Moment::Velocity;
        pane.custom_product = Some("Storm motion".into());
        pane.tilt = 2;
        pane.show_radar = false;
        pane.show_legend = false;
        pane.fields_on.clear();
        pane.fields_on.insert(FieldLayer::GoesC13);
        let encoded = serde_json::to_string(&PaneSnap::capture(&pane)).unwrap();
        let snapshot: PaneSnap = serde_json::from_str(&encoded).unwrap();
        // The app restores these field IDs after applying the pane, so source selection must
        // retain both halves: the chosen field and the hidden native-radar flag.
        assert_eq!(snapshot.fields_on, Some(vec![FieldLayer::GoesC13.stable_id().to_string()]));
        let mut restored = MapView::new(Some("KFWS".into()), Camera::at_lonlat(0.0, 0.0, 3.0));
        assert!(restored.show_radar && restored.show_legend);
        snapshot.apply(&mut restored);
        assert!(!restored.show_radar);
        assert!(!restored.show_legend);
        assert_eq!(restored.site.as_deref(), Some("KTLX"));
        assert_eq!(restored.moment, Moment::Velocity);
        assert_eq!(restored.custom_product.as_deref(), Some("Storm motion"));
        assert_eq!(restored.tilt, 2);
    }

    #[test]
    fn legacy_snapshot_restores_visible_radar_and_legend_defaults() {
        let legacy = r#"{"site":"KTLX","moment":"Reflectivity","tilt":0,"basemap":"dark","lon":-97.28,"lat":35.33,"zoom":8.0}"#;
        let snapshot: PaneSnap = serde_json::from_str(legacy).unwrap();
        assert!(snapshot.show_radar && snapshot.show_legend);
        let mut restored = MapView::new(None, crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0));
        restored.show_radar = false;
        restored.show_legend = false;
        snapshot.apply(&mut restored);
        assert!(restored.show_radar && restored.show_legend);
        assert_eq!(restored.site.as_deref(), Some("KTLX"));
    }

}
