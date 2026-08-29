//! The address bar as a permalink.
//!
//! `#goto=` has been a link you could be sent since PR #76, and one the app read exactly once, at
//! boot. Nothing ever wrote it back — so the URL of a browser tab said where the session started,
//! never where it had got to, and copying out of the address bar sent someone to the wrong place
//! and the wrong hour.
//!
//! This keeps it honest: the fragment tracks the active pane, including the archive time when the
//! timeline is scrubbed off live. `replaceState`, not `pushState` — panning a map is not
//! navigation, and forty history entries per drag would make the back button useless.

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
            let Some(frag) = self.permalink_fragment() else {
                return;
            };
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_deref() == Some(frag.as_str()) {
                return;
            }
            if !self.permalink_due() {
                return;
            }
            let Some(win) = web_sys::window() else { return };
            // The rest of the URL is left exactly as it is — the recovery reload's `?relaunched`
            // lives in the query, and stamping over it would spend the retry it is tracking.
            if let Ok(h) = win.history() {
                let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&frag));
                *last = Some(frag);
            }
        }
    }

    /// The `#goto=…` for the active pane, built from the same [`goto_link`] the share button uses
    /// so a copied address and a shared link are the same string.
    #[cfg(target_arch = "wasm32")]
    fn permalink_fragment(&self) -> Option<String> {
        let v = self.views.get(self.active)?;
        let site = v.site.clone()?;
        let c = v.camera.center;
        let (lon, lat) = crate::render::mercator::world_to_lonlat(c.0, c.1);
        // Live stays live: a permalink that pinned the current minute would turn "share what I am
        // watching" into "share the moment I happened to copy it".
        let time = (!v.timeline.following)
            .then(|| v.timeline.current().and_then(|id| id.date_time()))
            .flatten();
        let link = goto_link(&Goto {
            site,
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
        link.find('#').map(|i| link[i..].to_string())
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
