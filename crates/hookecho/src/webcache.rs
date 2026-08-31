//! Offline chase packs: raw radar volumes kept in IndexedDB so a saved loop plays with no network.
//!
//! The service worker already caches basemap tiles (`web/sw.src.js`, `tiles-v1`) and deliberately
//! excludes radar — a volume is tens of megabytes and the newest one changes every few minutes,
//! which is exactly what a cache-first HTTP cache handles worst. But a chaser about to lose
//! signal wants the opposite trade: pin *these specific* archived volumes, which never change,
//! and read them back with no request at all. That is a pack.
//!
//! Raw Archive II bytes are stored rather than decoded scans, for the same reason the native disk
//! cache stores them: they are smaller, they need no serialization of their own, and they go back
//! through the same decode path either way.
//!
//! ponytail: no eviction inside a pack and no sharing accounting between packs — a volume in two
//! packs is stored once and freed when the last pack referencing it goes. Packs are deleted whole,
//! oldest first, once the store passes [`BYTE_CAP`].

#[cfg(target_arch = "wasm32")]
use anyhow::anyhow;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;
#[cfg(target_arch = "wasm32")]
use web_sys::{IdbDatabase, IdbObjectStore, IdbRequest, IdbTransactionMode};

#[cfg(target_arch = "wasm32")]
const DB_NAME: &str = "hookecho-packs";
#[cfg(target_arch = "wasm32")]
const VOLUMES: &str = "volumes";
#[cfg(target_arch = "wasm32")]
const PACKS: &str = "packs";

/// How much radar may sit in IndexedDB before saving a pack evicts the oldest one.
///
/// A browser's quota is a fraction of free disk and is not knowable up front, so this is a
/// self-imposed ceiling well under any plausible quota: about ten loops of a dozen volumes.
#[cfg(target_arch = "wasm32")]
const BYTE_CAP: f64 = 250.0 * 1024.0 * 1024.0;

/// One saved loop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Pack {
    /// Radar site the loop was saved from.
    pub site: String,
    /// UTC day of the loop, `YYYY-MM-DD`.
    pub date: String,
    /// Volume object names, oldest first — the timeline as it stood when saved.
    pub volumes: Vec<String>,
    /// Unix seconds when it was saved.
    pub saved_at: i64,
    /// Total size of the volumes, for the eviction accounting and the picker's readout.
    pub bytes: f64,
}

impl Pack {
    /// Stable key: one pack per site and day, so re-saving a loop replaces it.
    pub fn key(&self) -> String {
        format!("{}-{}", self.site, self.date)
    }

    /// One line for the picker.
    pub fn label(&self) -> String {
        format!(
            "{} {} \u{2014} {} volume{}, {:.0} MB",
            self.site,
            self.date,
            self.volumes.len(),
            if self.volumes.len() == 1 { "" } else { "s" },
            self.bytes / 1024.0 / 1024.0
        )
    }
}

#[cfg(target_arch = "wasm32")]
/// Await an `IDBRequest`, resolving to its result.
#[cfg(target_arch = "wasm32")]
async fn await_request(req: IdbRequest) -> anyhow::Result<JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let r = req.clone();
        let ok = Closure::once_into_js(move |_: JsValue| {
            let _ = resolve.call1(&JsValue::NULL, &r.result().unwrap_or(JsValue::NULL));
        });
        req.set_onsuccess(Some(ok.unchecked_ref()));
        let err = Closure::once_into_js(move |_: JsValue| {
            let _ = reject.call1(&JsValue::NULL, &"indexeddb request failed".into());
        });
        req.set_onerror(Some(err.unchecked_ref()));
    });
    JsFuture::from(promise).await.map_err(|e| anyhow!("{e:?}"))
}

/// Open the database, creating the two stores on first use or version bump.
#[cfg(target_arch = "wasm32")]
async fn open() -> anyhow::Result<IdbDatabase> {
    let factory = web_sys::window()
        .and_then(|w| w.indexed_db().ok().flatten())
        .ok_or_else(|| anyhow!("no IndexedDB in this browser"))?;
    let req = factory
        .open_with_u32(DB_NAME, 1)
        .map_err(|e| anyhow!("{e:?}"))?;
    let upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let Some(req) = ev.target().and_then(|t| t.dyn_into::<IdbRequest>().ok()) else {
            return;
        };
        let Ok(db) = req.result().and_then(|v| v.dyn_into::<IdbDatabase>()) else {
            return;
        };
        let _ = db.create_object_store(VOLUMES);
        let _ = db.create_object_store(PACKS);
    });
    req.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let db = await_request(req.clone().unchecked_into()).await?;
    // The closure has to outlive the open, and the database outlives everything: leaking one
    // closure per open beats a lifetime dance for an object that lives as long as the page.
    upgrade.forget();
    db.dyn_into::<IdbDatabase>().map_err(|e| anyhow!("{e:?}"))
}

/// The object store for `name`, in a transaction of `mode`.
#[cfg(target_arch = "wasm32")]
fn store(db: &IdbDatabase, name: &str, mode: IdbTransactionMode) -> anyhow::Result<IdbObjectStore> {
    db.transaction_with_str_and_mode(name, mode)
        .and_then(|tx| tx.object_store(name))
        .map_err(|e| anyhow!("{e:?}"))
}

/// Raw bytes of one volume, if a pack holds it.
#[cfg(target_arch = "wasm32")]
pub async fn volume(name: &str) -> Option<Vec<u8>> {
    let db = open().await.ok()?;
    let s = store(&db, VOLUMES, IdbTransactionMode::Readonly).ok()?;
    let v = await_request(s.get(&name.into()).ok()?).await.ok()?;
    let arr = v.dyn_into::<js_sys::Uint8Array>().ok()?;
    Some(arr.to_vec())
}

/// Store one volume's bytes.
#[cfg(target_arch = "wasm32")]
async fn put_volume(db: &IdbDatabase, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let s = store(db, VOLUMES, IdbTransactionMode::Readwrite)?;
    let arr = js_sys::Uint8Array::from(bytes);
    let req = s
        .put_with_key(&arr, &name.into())
        .map_err(|e| anyhow!("{e:?}"))?;
    await_request(req).await.map(|_| ())
}

/// Every saved pack, newest first.
#[cfg(target_arch = "wasm32")]
pub async fn packs() -> Vec<Pack> {
    let Ok(db) = open().await else {
        return Vec::new();
    };
    let Ok(s) = store(&db, PACKS, IdbTransactionMode::Readonly) else {
        return Vec::new();
    };
    let Ok(req) = s.get_all() else {
        return Vec::new();
    };
    let Ok(v) = await_request(req).await else {
        return Vec::new();
    };
    let mut out: Vec<Pack> = js_sys::Array::from(&v)
        .iter()
        .filter_map(|e| e.as_string())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect();
    out.sort_by_key(|p: &Pack| std::cmp::Reverse(p.saved_at));
    out
}

/// Save a pack: store every volume's bytes, then the manifest, then evict oldest packs until the
/// store is back under [`BYTE_CAP`].
///
/// `volumes` is `(object name, raw bytes)` in timeline order. Returns the pack as stored.
#[cfg(target_arch = "wasm32")]
pub async fn save_pack(
    site: &str,
    date: &str,
    volumes: Vec<(String, Vec<u8>)>,
) -> anyhow::Result<Pack> {
    if volumes.is_empty() {
        anyhow::bail!("nothing in the loop to save");
    }
    let db = open().await?;
    let mut bytes = 0.0;
    let mut names = Vec::new();
    for (name, data) in &volumes {
        put_volume(&db, name, data).await?;
        bytes += data.len() as f64;
        names.push(name.clone());
    }
    let pack = Pack {
        site: site.to_string(),
        date: date.to_string(),
        volumes: names,
        saved_at: chrono::Utc::now().timestamp(),
        bytes,
    };
    let s = store(&db, PACKS, IdbTransactionMode::Readwrite)?;
    let json = serde_json::to_string(&pack)?;
    let req = s
        .put_with_key(&json.as_str().into(), &pack.key().into())
        .map_err(|e| anyhow!("{e:?}"))?;
    await_request(req).await?;
    evict(&db).await;
    Ok(pack)
}

/// Delete oldest packs until the store fits under the cap.
#[cfg(target_arch = "wasm32")]
async fn evict(db: &IdbDatabase) {
    let mut all = packs().await;
    let mut total: f64 = all.iter().map(|p| p.bytes).sum();
    all.sort_by_key(|p| p.saved_at);
    for p in all {
        if total <= BYTE_CAP {
            break;
        }
        total -= p.bytes;
        let _ = delete_pack(db, &p).await;
    }
}

/// Remove a pack and any volume of its that no surviving pack still lists.
#[cfg(target_arch = "wasm32")]
async fn delete_pack(db: &IdbDatabase, pack: &Pack) -> anyhow::Result<()> {
    let others: Vec<Pack> = packs()
        .await
        .into_iter()
        .filter(|p| p.key() != pack.key())
        .collect();
    let s = store(db, VOLUMES, IdbTransactionMode::Readwrite)?;
    for name in &pack.volumes {
        if others.iter().any(|p| p.volumes.contains(name)) {
            continue;
        }
        if let Ok(req) = s.delete(&name.as_str().into()) {
            let _ = await_request(req).await;
        }
    }
    let s = store(db, PACKS, IdbTransactionMode::Readwrite)?;
    let req = s
        .delete(&pack.key().as_str().into())
        .map_err(|e| anyhow!("{e:?}"))?;
    await_request(req).await.map(|_| ())
}

/// Delete one pack by key, for the picker's remove button.
#[cfg(target_arch = "wasm32")]
pub async fn remove(key: String) {
    let Ok(db) = open().await else { return };
    if let Some(p) = packs().await.into_iter().find(|p| p.key() == key) {
        let _ = delete_pack(&db, &p).await;
    }
}

/// What the UI reads: the packs on hand and the last progress line. Filled by the async saves and
/// loads, which have nowhere else to put a result — the picker is drawn from a `&mut self` the
/// spawned task cannot hold.
#[cfg(target_arch = "wasm32")]
static STATE: std::sync::Mutex<(Vec<Pack>, Option<String>, bool)> =
    std::sync::Mutex::new((Vec::new(), None, false));

/// Packs known to the UI. The first call kicks off the read that fills them.
#[cfg(target_arch = "wasm32")]
pub fn known_packs() -> Vec<Pack> {
    let asked = {
        let Ok(mut s) = STATE.lock() else {
            return Vec::new();
        };
        std::mem::replace(&mut s.2, true)
    };
    if !asked {
        wasm_bindgen_futures::spawn_local(async {
            refresh().await;
        });
    }
    STATE.lock().map(|s| s.0.clone()).unwrap_or_default()
}

/// The last progress or error line, for the picker.
#[cfg(target_arch = "wasm32")]
pub fn status() -> Option<String> {
    STATE.lock().ok().and_then(|s| s.1.clone())
}

#[cfg(target_arch = "wasm32")]
fn set_status(msg: Option<String>) {
    if let Ok(mut s) = STATE.lock() {
        s.1 = msg;
    }
}

#[cfg(target_arch = "wasm32")]
async fn refresh() {
    let list = packs().await;
    if let Ok(mut s) = STATE.lock() {
        s.0 = list;
    }
}

/// Fetch every volume in `ids` — from the pack store when it is already there, from the bucket
/// otherwise — and save them as one pack.
#[cfg(target_arch = "wasm32")]
pub async fn save_timeline(site: String, date: String, ids: Vec<wxdata::level2::Identifier>) {
    let total = ids.len();
    let mut out = Vec::new();
    for (i, id) in ids.into_iter().enumerate() {
        let name = id.name().to_string();
        set_status(Some(format!("saving {} of {total}\u{2026}", i + 1)));
        let bytes = match volume(&name).await {
            Some(b) => b,
            None => match wxdata::level2::volume_bytes(id).await {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("pack: skipping {name}: {e}");
                    continue;
                }
            },
        };
        out.push((name, bytes));
    }
    match save_pack(&site, &date, out).await {
        Ok(p) => set_status(Some(format!("saved {}", p.label()))),
        Err(e) => set_status(Some(format!("could not save the pack: {e}"))),
    }
    refresh().await;
}

/// Delete a pack from the picker.
#[cfg(target_arch = "wasm32")]
pub fn spawn_remove(key: String) {
    wasm_bindgen_futures::spawn_local(async move {
        remove(key).await;
        refresh().await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_round_trips_through_its_manifest() {
        let p = Pack {
            site: "KTLX".into(),
            date: "2026-05-20".into(),
            volumes: vec!["KTLX20260520_231502_V06".into()],
            saved_at: 1_780_000_000,
            bytes: 32.0 * 1024.0 * 1024.0,
        };
        let back: Pack = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.key(), "KTLX-2026-05-20");
        assert_eq!(back.label(), "KTLX 2026-05-20 — 1 volume, 32 MB");
    }
}
