//! Readiness for the most recently downloaded map area. Weather packs remain dated snapshots.
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

// OpenFreeMap's user-triggered offline download limits (openfreemap.org/tos/).
pub const TILE_LIMIT: u64 = 3_000;
pub const BYTE_LIMIT: u64 = 50_000_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MapPack {
    pub label: String,
    pub bounds: (f64, f64, f64, f64),
    pub zoom: (u8, u8),
    /// URL and native cache path; browser reads use the same proxied URL as the renderer.
    pub files: Vec<(String, String)>,
    pub completed: u64,
    pub bytes: u64,
    pub finished: bool,
    pub error: Option<String>,
    pub saved_at: i64,
    #[serde(default)]
    pub vector_template: Option<String>,
    #[serde(skip)]
    pub verified: bool,
}

impl MapPack {
    pub fn ready(&self) -> bool {
        self.verified && self.finished && self.error.is_none()
            && !self.files.is_empty() && self.completed == self.files.len() as u64
    }

    pub fn coverage(&self) -> String {
        let (west, south, east, north) = self.bounds;
        format!("{south:.2}° to {north:.2}° lat · {west:.2}° to {east:.2}° lon · zoom {}–{}", self.zoom.0, self.zoom.1)
    }
}

fn state() -> &'static Mutex<Option<Arc<MapPack>>> {
    static STATE: OnceLock<Mutex<Option<Arc<MapPack>>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(read_manifest().and_then(|s| serde_json::from_str(&s).ok()).map(Arc::new)))
}

pub fn snapshot() -> Option<Arc<MapPack>> { state().lock().ok()?.clone() }

fn read_manifest() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    { std::fs::read_to_string(crate::paths::cache_dir()?.join("offline-map.json")).ok() }
    #[cfg(target_arch = "wasm32")]
    { web_sys::window()?.local_storage().ok()??.get_item("hookecho.offline-map").ok()? }
}

fn persist(pack: &MapPack) -> anyhow::Result<()> {
    let json = serde_json::to_string(&pack)?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let root = crate::paths::cache_dir().ok_or_else(|| anyhow::anyhow!("No cache directory"))?;
        std::fs::create_dir_all(&root)?;
        let pending = root.join("offline-map.json.tmp");
        std::fs::write(&pending, json)?;
        std::fs::rename(pending, root.join("offline-map.json"))?;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
            .ok_or_else(|| anyhow::anyhow!("Browser storage is unavailable"))?;
        storage.set_item("hookecho.offline-map", &json).map_err(|e| anyhow::anyhow!("Storage quota or permission: {e:?}"))?;
    }
    Ok(())
}

pub fn save(pack: MapPack) -> anyhow::Result<()> {
    let mut current = state().lock().unwrap();
    persist(&pack)?;
    *current = Some(Arc::new(pack));
    Ok(())
}

pub fn within_budget(tiles: u64, bytes: u64) -> bool {
    tiles > 0 && tiles <= TILE_LIMIT && bytes <= BYTE_LIMIT
}

/// Preserve explicitly downloaded native map files when the ordinary browsing cache is trimmed.
#[cfg(not(target_arch = "wasm32"))]
pub fn pinned_paths() -> std::collections::HashSet<std::path::PathBuf> {
    snapshot().map(|p| p.files.iter().map(|(_, path)| path.into()).collect()).unwrap_or_default()
}

pub fn verify(spawner: &crate::rt::Spawner, ctx: egui::Context) {
    let Some(previous) = snapshot() else { return };
    if !previous.finished { return; }
    spawner.spawn(async move {
        let mut pack = (*previous).clone();
        let mut present = 0;
        for (url, path) in &pack.files {
            #[cfg(target_arch = "wasm32")]
            let exists = cached_tile(url).await.is_some();
            #[cfg(not(target_arch = "wasm32"))]
            let exists = { let _ = url; std::fs::metadata(path).is_ok_and(|m| m.len() > 0) };
            #[cfg(target_arch = "wasm32")]
            let _ = path;
            if exists { present += 1; }
        }
        pack.completed = present;
        pack.verified = true;
        pack.finished = true;
        pack.error = (present != pack.files.len() as u64).then(|| format!("{} tiles missing — download this area again", pack.files.len() as u64 - present));
        if let Ok(mut current) = state().lock() {
            if current.as_ref().is_some_and(|now| Arc::ptr_eq(now, &previous)) && persist(&pack).is_ok() {
                *current = Some(Arc::new(pack));
            }
        }
        ctx.request_repaint();
    });
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
const OFFLINE = 'hookecho-offline-maps-v1';
export async function offlineRead(url) {
  const c = await caches.open(OFFLINE);
  const hit = await c.match(url);
  return hit ? new Uint8Array(await hit.arrayBuffer()) : null;
}
export async function offlineDelete(url) {
  for (const name of await caches.keys()) {
    if (name === OFFLINE || name.startsWith('tiles-')) {
      const c = await caches.open(name);
      await c.delete(url);
    }
  }
}
export async function offlinePrepare(estimate) {
  if (!globalThis.caches) throw new Error('Offline storage is unavailable');
  const info = await navigator.storage?.estimate?.();
  if (info?.quota && info.quota - (info.usage || 0) < estimate)
    throw new Error('Not enough browser storage; free space or choose a smaller area');
  await caches.delete(OFFLINE);
  await navigator.storage?.persist?.();
}
export async function offlineWrite(url, bytes) {
  const c = await caches.open(OFFLINE);
  await c.put(url, new Response(bytes));
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn offlineRead(url: &str) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn offlineDelete(url: &str) -> Result<(), wasm_bindgen::JsValue>;
    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn offlinePrepare(estimate: f64) -> Result<(), wasm_bindgen::JsValue>;
    #[wasm_bindgen::prelude::wasm_bindgen(catch)]
    async fn offlineWrite(url: &str, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue>;
}

pub async fn invalidate_tile(url: &str) {
    #[cfg(target_arch = "wasm32")]
    let _ = offlineDelete(&wxdata::net::fetch_url(url)).await;
    if let Ok(mut current) = state().lock() {
        if let Some(pack) = current.as_ref().filter(|pack| pack.files.iter().any(|(saved, _)| saved == url)) {
            let mut invalid = (**pack).clone();
            invalid.verified = false;
            invalid.error = Some("A saved map tile was corrupt; verify or download this area again".into());
            let _ = persist(&invalid);
            *current = Some(Arc::new(invalid));
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn cached_tile(url: &str) -> Option<Vec<u8>> {
    if !state().lock().ok()?.is_some() { return None; }
    use wasm_bindgen::JsCast;
    let value = offlineRead(&wxdata::net::fetch_url(url)).await.ok()?;
    let bytes = value.dyn_into::<js_sys::Uint8Array>().ok()?.to_vec();
    (!bytes.is_empty()).then_some(bytes)
}

#[cfg(target_arch = "wasm32")]
pub async fn prepare(estimate: u64) -> anyhow::Result<()> {
    offlinePrepare(estimate as f64).await.map_err(|e| anyhow::anyhow!("{e:?}"))
}

#[cfg(target_arch = "wasm32")]
pub async fn store_tile(url: &str, bytes: &[u8]) -> anyhow::Result<()> {
    offlineWrite(&wxdata::net::fetch_url(url), bytes).await.map_err(|e| anyhow::anyhow!("Could not save map tile (storage quota or permission): {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack() -> MapPack {
        MapPack { label: "Dark".into(), bounds: (-98., 32., -96., 34.), zoom: (8, 10),
            files: vec![("https://tiles.example/8/1/2.pbf".into(), "/cache/tile".into())],
            completed: 1, bytes: 20, finished: true, error: None, saved_at: 1, vector_template: None, verified: true }
    }
    #[test]
    fn only_complete_verified_packs_are_ready() {
        let mut p = pack();
        assert!(p.ready());
        p.completed = 0;
        assert!(!p.ready());
        p.completed = 1;
        p.error = Some("quota".into());
        assert!(!p.ready());
        p.error = None;
        let reloaded: MapPack = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(!reloaded.ready(), "persisted manifests must be checked after reload");
        p.finished = false;
        assert!(!p.ready());
    }
    #[test]
    fn downloads_enforce_provider_tile_and_byte_limits() {
        assert!(within_budget(3_000, 50_000_000));
        assert!(!within_budget(3_001, 1));
        assert!(!within_budget(1, 50_000_001));
        assert!(!within_budget(0, 0));
    }
}
