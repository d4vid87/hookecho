//! Public multi-pane scene links and the browser address-bar snapshot.
//!
//! The fragment follows the complete arrangement at a bounded cadence. replaceState avoids
//! adding a browser history entry for every map movement; incoming fragments win over snapshots.

use super::*;

/// The last fragment written, so an unchanged view costs nothing.
///
/// A static rather than a field on the app: the browser owns this, there is exactly one address
/// bar, and threading it through `HookEchoApp` would be state nobody else can use.
#[cfg(target_arch = "wasm32")]
static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// How often the URL may be rewritten. A pan is a hundred frames and one destination; the number
/// is low enough that the address bar is never stale in any way a person would notice.
#[cfg(target_arch = "wasm32")]
const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);

impl HookEchoApp {
    /// Point the address bar at what is on screen. No-op off the web, where there is no address
    /// bar to point.
    pub(crate) fn sync_permalink(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(win) = web_sys::window() else { return };
            // A pasted fragment is an incoming navigation. The one-second deep-link poll has
            // not necessarily seen it yet; rewriting the address bar from the old camera here
            // would erase the destination before that poll can apply it.
            if win.location().hash().ok().is_some_and(|hash|
                (hash.starts_with("#goto=") || hash.starts_with("#scene=")) && self.last_goto_hash.as_deref() != Some(hash.as_str())) {
                return;
            }
            // Serialize at the URL refresh cadence, never on every rendered animation frame.
            if !self.permalink_due() { return; }
            let Some(frag) = self.permalink_fragment() else {
                return;
            };
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_deref() == Some(frag.as_str()) {
                return;
            }
            // The rest of the URL is left exactly as it is — the recovery reload's `?relaunched`
            // lives in the query, and stamping over it would spend the retry it is tracking.
            if let Ok(h) = win.history() {
                if h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&frag)).is_ok() {
                    // This is our own camera snapshot, not an incoming navigation. Otherwise
                    // apply_goto_hash replays this stale zoom on its next one-second poll.
                    self.last_goto_hash = Some(frag.clone());
                    *last = Some(frag);
                }
            }
        }
    }

    /// The address bar shares the same public scene format as the Share controls.
    #[cfg(target_arch = "wasm32")]
    fn permalink_fragment(&mut self) -> Option<String> {
        let mode = self.current_scene_time_mode();
        self.shared_scene(mode).and_then(|scene| scene.encode()).ok()
            .map(|token| format!("#scene={token}"))
    }

    /// Has enough time passed since the last rewrite? Safari throttles `replaceState` and starts
    /// throwing past roughly a hundred calls a minute, so this is a correctness guard, not a
    /// politeness one.
    #[cfg(target_arch = "wasm32")]
    fn permalink_due(&self) -> bool {
        use std::sync::Mutex;
        static WHEN: Mutex<Option<f64>> = Mutex::new(None);
        let Some(now) = web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
        else {
            return false;
        };
        let mut when = WHEN.lock().unwrap_or_else(|e| e.into_inner());
        if when.is_some_and(|t| now - t < MIN_INTERVAL.as_millis() as f64) {
            return false;
        }
        *when = Some(now);
        true
    }
}


impl HookEchoApp {
    pub(crate) fn current_scene_time_mode(&self) -> crate::workspace::SceneTimeMode {
        if self.views.iter().all(|view| view.timeline.following && view.satellite.time.is_none()) {
            crate::workspace::SceneTimeMode::Live
        } else {
            crate::workspace::SceneTimeMode::Exact
        }
    }

    fn shared_scene(&mut self, mode: crate::workspace::SceneTimeMode) -> anyhow::Result<crate::workspace::SharedScene> {
        let times = self.views.iter().enumerate().map(|(pane, view)| {
            if mode == crate::workspace::SceneTimeMode::Live { return None; }
            let field_time = view.fields_on.iter().filter_map(|layer|
                self.field_state(pane, *layer).and_then(|state|
                    state.metadata.as_ref().map(|(_, stamp)| stamp.valid_time))).max();
            if view.show_radar {
                view.displayed_time().or(view.timeline.selected_time()).or(field_time).or(view.satellite.time)
            } else {
                view.satellite.time.or(field_time).or(view.displayed_time()).or(view.timeline.selected_time())
            }
        }).collect();
        let mut workspace = self.capture_workspace();
        for pane in &mut workspace.panes { pane.abi_scene = Some(pane.abi_scene.unwrap_or(self.settings.abi_scene)); }
        let products = crate::workspace::SceneProducts {
            global_model: self.global_model,
            global_fcst_hour: self.global_fcst_hour,
            analysis_source: self.analysis_source,
            hrrr_fcst_hour: self.hrrr_fcst_hour,
            refs_fcst_hour: self.refs_fcst_hour,
            refs_dbz_threshold: self.refs_dbz_threshold,
            env_model: self.env_model,
            env_cape_ml: self.env_cape_ml,
            env_srh_km: self.env_srh_km,
            snow_hours: self.snow_hours,
            lightning_minutes: self.settings.lightning_minutes,
            hail_minutes: self.hail_minutes,
            rotation_minutes: self.rotation_minutes,
            wind_level: self.wind_level,
            diff_field: self.diff_field,
            tropical_wind_kt: self.tropical_wind_kt,
            tropical_surge: self.tropical_surge,
            storm_motion: self.views.iter().map(|view| (view.storm_dir_deg, view.storm_speed_kt)).collect(),
        };
        crate::workspace::SharedScene::new(workspace, mode, times, self.filters.clone(),
            products, self.lock_source_frame, self.settings.link_satellite_times)
    }

    pub(crate) fn share_scene_link(&mut self, mode: crate::workspace::SceneTimeMode, ctx: &egui::Context) {
        match self.shared_scene(mode).and_then(|scene| scene.encode()) {
            Ok(token) => {
                let link = format!("{}/#scene={token}", scene_link_origin());
                if !crate::platform::share_link("HookEcho", &link) {
                    ctx.copy_text(link);
                    self.toast(ToastKind::Success, match mode {
                        crate::workspace::SceneTimeMode::Live => "Live scene link copied",
                        crate::workspace::SceneTimeMode::Exact => "Exact-time scene link copied",
                    });
                }
            }
            Err(error) => self.toast(ToastKind::Error, error.to_string()),
        }
    }

    /// Returns true for a scene link, including a rejected one, so it never falls into legacy goto.
    pub(crate) fn apply_scene_link(&mut self, link: &str, ctx: &egui::Context) -> bool {
        let Some(token) = scene_link_token(link) else { return false };
        let scene = match crate::workspace::SharedScene::decode(token) {
            Ok(scene) => scene,
            Err(error) => {
                self.toast(ToastKind::Error, format!("Could not open shared scene: {error}"));
                return true;
            }
        };
        self.firstrun.open = false;
        self.settings.setup_done = true;
        self.ndfd_probe_rx = None;
        self.detail = None;
        self.apply_workspace(&scene.workspace, ctx);
        scene.apply_times(&mut self.views);
        self.lock_source_frame = scene.lock_source_frame;
        self.settings.link_satellite_times = scene.link_satellite_times;
        self.filters = scene.filters;
        let products = scene.products;
        self.global_model = products.global_model;
        self.global_fcst_hour = products.global_fcst_hour;
        self.analysis_source = products.analysis_source;
        self.hrrr_fcst_hour = products.hrrr_fcst_hour;
        self.refs_fcst_hour = products.refs_fcst_hour;
        self.refs_dbz_threshold = products.refs_dbz_threshold;
        self.env_model = products.env_model;
        self.env_cape_ml = products.env_cape_ml;
        self.env_srh_km = products.env_srh_km;
        self.snow_hours = products.snow_hours;
        self.settings.lightning_minutes = products.lightning_minutes;
        self.hail_minutes = products.hail_minutes;
        self.rotation_minutes = products.rotation_minutes;
        self.wind_level = products.wind_level;
        self.diff_field = products.diff_field;
        self.tropical_wind_kt = products.tropical_wind_kt;
        self.tropical_surge = products.tropical_surge;
        self.global_layer_key.clear();
        self.refs_key = None;
        self.diff_key = None;
        self.hrrr_fetched_hour = None;
        self.hrrr_last_fetch = None;
        self.snow_fetched = None;
        self.tropical_last_fetch = None;
        for state in self.fields.values_mut() { *state = Default::default(); }
        self.outlook_features.iter_mut().for_each(Vec::clear);
        self.fire_weather_features.clear();
        self.cpc_features.clear();
        self.ero_features.clear();
        self.ero_loaded = None;
        self.wssi_features.clear();
        self.winter_snow_features.clear();
        self.winter_ice_features.clear();
        self.fetch_overlays(ctx);
        self.rebuild_overlays();
        self.autoplay_pending = false;
        self.toast(ToastKind::Success, match scene.time_mode {
            crate::workspace::SceneTimeMode::Live => "Opened live scene",
            crate::workspace::SceneTimeMode::Exact => "Opened shared time; archive availability varies by layer",
        });
        true
    }
}

fn scene_link_origin() -> String {
    #[cfg(target_arch = "wasm32")]
    if let Some(origin) = web_sys::window().and_then(|window| window.location().origin().ok()) {
        if origin.starts_with("https://") || origin.starts_with("http://") { return origin; }
    }
    "https://app.hookecho.io".into()
}

fn scene_link_token(link: &str) -> Option<&str> {
    let link = link.trim();
    link.strip_prefix("hookecho://scene/")
        .or_else(|| link.strip_prefix("#scene="))
        .or_else(|| link.strip_prefix("scene="))
        .or_else(|| {
            (link.starts_with("https://") || link.starts_with("http://"))
                .then(|| link.split_once("#scene=").map(|(_, token)| token)).flatten()
        })
}


#[cfg(test)]
mod scene_link_tests {
    use super::*;

    #[test]
    fn scene_links_accept_web_fragments_and_native_handoffs_without_capturing_goto() {
        for link in ["#scene=abc", "scene=abc", "hookecho://scene/abc",
            "https://app.hookecho.io/#scene=abc", "http://localhost:8765/#scene=abc"] {
            assert_eq!(scene_link_token(link), Some("abc"));
        }
        for link in ["KTLX,-97.3,35.3,9", "hookecho://goto/KTLX", "#goto=KTLX",
            "https://app.hookecho.io/?scene=abc", "javascript:#scene=abc"] {
            assert_eq!(scene_link_token(link), None);
        }
    }
}
