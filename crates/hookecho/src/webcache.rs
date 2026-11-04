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
    /// Immutable native GOES source objects pinned in the shared object cache.
    #[serde(default)]
    pub satellite: Vec<String>,
    /// Immutable MRMS source objects pinned in the shared object cache.
    #[serde(default)]
    pub mrms: Vec<String>,
    /// Imported placefile/GIS source text needed by this pack.
    #[serde(default)]
    pub overlays: std::collections::BTreeMap<String, String>,
    /// Overlay settings paired with [`Self::overlays`].
    #[serde(default)]
    pub placefiles: Vec<crate::settings::PlacefileConfig>,
    /// Unix seconds when it was saved.
    pub saved_at: i64,
    /// Total size of pinned weather bytes, for eviction accounting and the picker's readout.
    pub bytes: f64,
    /// Revalidated against persistent storage when the picker refreshes.
    #[serde(skip)]
    pub available: bool,
}

impl Pack {
    /// Stable key: one pack per site and day, so re-saving a loop replaces it.
    pub fn key(&self) -> String {
        format!("{}-{}", self.site, self.date)
    }

    /// One line for the picker.
    pub fn label(&self) -> String {
        format!(
            "{} {} \u{2014} {} volume{}, {} weather file{}, {:.0} MB",
            self.site,
            self.date,
            self.volumes.len(),
            if self.volumes.len() == 1 { "" } else { "s" },
            self.satellite.len() + self.mrms.len() + self.overlays.len(),
            if self.satellite.len() + self.mrms.len() + self.overlays.len() == 1 {
                ""
            } else {
                "s"
            },
            self.bytes / 1024.0 / 1024.0
        )
    }
}

#[cfg(target_arch = "wasm32")]
static SAVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(target_arch = "wasm32")]
static CANCEL_SAVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_arch = "wasm32")]
pub struct SaveGuard;
#[cfg(target_arch = "wasm32")]
impl Drop for SaveGuard {
    fn drop(&mut self) { SAVING.store(false, std::sync::atomic::Ordering::Relaxed); }
}
#[cfg(target_arch = "wasm32")]
pub fn begin_save() -> Option<SaveGuard> {
    if SAVING.swap(true, std::sync::atomic::Ordering::Relaxed) { return None; }
    CANCEL_SAVE.store(false, std::sync::atomic::Ordering::Relaxed);
    set_status(Some("Preparing saved weather…".into()));
    Some(SaveGuard)
}
#[cfg(target_arch = "wasm32")]
pub fn is_saving() -> bool { SAVING.load(std::sync::atomic::Ordering::Relaxed) }
#[cfg(target_arch = "wasm32")]
pub fn cancel_save() { CANCEL_SAVE.store(true, std::sync::atomic::Ordering::Relaxed); }
#[cfg(target_arch = "wasm32")]
pub fn save_cancelled() -> bool {
    let cancelled = CANCEL_SAVE.load(std::sync::atomic::Ordering::Relaxed);
    if cancelled { set_status(Some("Weather download cancelled; no replay saved".into())); }
    cancelled
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

#[cfg(target_arch = "wasm32")]
fn transaction_done(tx: &web_sys::IdbTransaction) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, reject| {
        let ok = Closure::once_into_js(move |_: JsValue| { let _ = resolve.call0(&JsValue::NULL); });
        tx.set_oncomplete(Some(ok.unchecked_ref()));
        let failure = Closure::once_into_js(move |_: JsValue| {
            let _ = reject.call1(&JsValue::NULL, &"Browser storage write aborted (quota or permission)".into());
        });
        tx.set_onabort(Some(failure.unchecked_ref()));
    })
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

/// Every saved pack, newest first.
#[cfg(target_arch = "wasm32")]
pub async fn packs() -> Vec<Pack> {
    let Ok(db) = open().await else { return Vec::new() };
    let Ok(mut out) = list_packs(&db).await else { return Vec::new() };
    for pack in &mut out {
        pack.available = !pack.volumes.is_empty();
        for name in &pack.volumes {
            let found = if let Ok(store) = store(&db, VOLUMES, IdbTransactionMode::Readonly) {
                if let Ok(request) = store.get_key(&name.as_str().into()) {
                    await_request(request).await.is_ok_and(|key| !key.is_undefined())
                } else { false }
            } else { false };
            if !found { pack.available = false; break; }
        }
        for key in &pack.satellite {
            if wxdata::object_cache::get("satellite", key).await.is_none() { pack.available = false; }
        }
        for key in &pack.mrms {
            if wxdata::object_cache::get("mrms", key).await.is_none() { pack.available = false; }
        }
    }
    out.sort_by_key(|p: &Pack| std::cmp::Reverse(p.saved_at));
    out
}

/// Metadata only: accounting and reference checks must not load/checksum weather blobs.
#[cfg(target_arch = "wasm32")]
async fn list_packs(db: &IdbDatabase) -> anyhow::Result<Vec<Pack>> {
    let request = store(db, PACKS, IdbTransactionMode::Readonly)?.get_all()
        .map_err(|error| anyhow!("{error:?}"))?;
    decode_manifests(await_request(request).await?)
}

#[cfg(target_arch = "wasm32")]
fn decode_manifests(value: JsValue) -> anyhow::Result<Vec<Pack>> {
    js_sys::Array::from(&value).iter().map(|value| {
        let json = value.as_string().ok_or_else(|| anyhow!("Invalid saved weather manifest"))?;
        serde_json::from_str(&json).map_err(Into::into)
    }).collect()
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, Default, PartialEq)]
struct Unreferenced {
    volumes: Vec<String>,
    satellite: Vec<String>,
    mrms: Vec<String>,
}

#[cfg(any(target_arch = "wasm32", test))]
fn unreferenced_volumes(names: &[String], surviving: &[Pack]) -> Vec<String> {
    names.iter().filter(|name| !surviving.iter().any(|other| other.volumes.contains(name)))
        .cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect()
}

#[cfg(any(target_arch = "wasm32", test))]
fn unreferenced(pack: &Pack, surviving: &[Pack]) -> Unreferenced {
    let unused = |names: &[String], select: fn(&Pack) -> &[String]| {
        names.iter().filter(|name| !surviving.iter().any(|other| select(other).contains(name)))
            .cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect()
    };
    Unreferenced {
        volumes: unreferenced_volumes(&pack.volumes, surviving),
        satellite: unused(&pack.satellite, |pack| &pack.satellite),
        mrms: unused(&pack.mrms, |pack| &pack.mrms),
    }
}

#[cfg(target_arch = "wasm32")]
async fn release_objects(db: &IdbDatabase, pack: &Pack) {
    let Ok(current) = list_packs(db).await else { return };
    let unused = unreferenced(pack, &current);
    for (family, keys) in [("satellite", &unused.satellite), ("mrms", &unused.mrms)] {
        for key in keys {
            if let Err(error) = wxdata::object_cache::set_pinned(family, key, false).await {
                log::warn!("Could not release saved weather object: {error}");
            }
        }
    }
}

/// Commit the manifest and its radar files together: an aborted write preserves the old pack.
#[cfg(target_arch = "wasm32")]
async fn commit_pack(
    db: &IdbDatabase, key: &str, pack: Option<&Pack>, volumes: Vec<(String, Vec<u8>)>,
) -> anyhow::Result<Option<Pack>> {
    let stores = js_sys::Array::of2(&VOLUMES.into(), &PACKS.into());
    let tx = db.transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)
        .map_err(|error| anyhow!("{error:?}"))?;
    let committed = transaction_done(&tx);
    let queued = async {
        let data = tx.object_store(VOLUMES).map_err(|error| anyhow!("{error:?}"))?;
        let manifests = tx.object_store(PACKS).map_err(|error| anyhow!("{error:?}"))?;
        let request = manifests.get_all().map_err(|error| anyhow!("{error:?}"))?;
        let mut current = decode_manifests(await_request(request).await?)?;
        let previous = current.iter().find(|old| old.key() == key).cloned();
        current.retain(|old| old.key() != key);
        if let Some(pack) = pack { current.push(pack.clone()); }
        anyhow::ensure!(pack.is_none() || !save_cancelled(), "Weather download cancelled; no replay saved");
        // Consume queued buffers instead of retaining a second full pack until commit completes.
        for (name, bytes) in volumes {
            data.put_with_key(&js_sys::Uint8Array::from(bytes.as_slice()), &name.as_str().into())
                .map_err(|error| anyhow!("{error:?}"))?;
        }
        let request = data.get_all_keys().map_err(|error| anyhow!("{error:?}"))?;
        let names: Vec<_> = js_sys::Array::from(&await_request(request).await?).iter().filter_map(|value| value.as_string()).collect();
        // Read references under the same write lock: other tabs' committed packs stay protected.
        for name in unreferenced_volumes(&names, &current) {
            data.delete(&name.as_str().into()).map_err(|error| anyhow!("{error:?}"))?;
        }
        anyhow::ensure!(pack.is_none() || !save_cancelled(), "Weather download cancelled; no replay saved");
        if let Some(pack) = pack {
            manifests.put_with_key(&serde_json::to_string(pack)?.as_str().into(), &key.into())
                .map_err(|error| anyhow!("{error:?}"))?;
        } else {
            manifests.delete(&key.into()).map_err(|error| anyhow!("{error:?}"))?;
        }
        Ok::<_, anyhow::Error>(previous)
    }.await;
    if queued.is_err() { let _ = tx.abort(); }
    let result = JsFuture::from(committed).await.map_err(|error| anyhow!("{error:?}"));
    let previous = queued?;
    result?;
    Ok(previous)
}

/// Save all requested files or preserve the previous replay. The caller holds SaveGuard.
#[cfg(target_arch = "wasm32")]
pub async fn save_pack(
    site: &str,
    date: &str,
    volumes: Vec<(String, Vec<u8>)>,
    satellite: Vec<String>,
    mrms: Vec<String>,
    overlays: std::collections::BTreeMap<String, String>,
    placefiles: Vec<crate::settings::PlacefileConfig>,
) -> anyhow::Result<Pack> {
    anyhow::ensure!(!volumes.is_empty(), "nothing in the loop to save");
    let mut pack = Pack {
        site: site.into(), date: date.into(),
        volumes: volumes.iter().map(|(name, _)| name.clone()).collect(),
        satellite: satellite.into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
        mrms: mrms.into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
        bytes: volumes.iter().map(|(_, bytes)| bytes.len() as f64).sum::<f64>()
            + overlays.values().map(|text| text.len() as f64).sum::<f64>(),
        overlays, placefiles, saved_at: chrono::Utc::now().timestamp(), available: true,
    };
    anyhow::ensure!(pack.bytes <= BYTE_CAP, "Weather pack exceeds 250 MiB; save fewer frames or layers");
    let db = open().await?;
    for (family, keys) in [("satellite", &pack.satellite), ("mrms", &pack.mrms)] {
        for key in keys {
            let object = wxdata::object_cache::get(family, key).await
                .ok_or_else(|| anyhow!("{family} file missing; reload the layer before saving"))?;
            pack.bytes += object.bytes.len() as f64;
        }
    }
    anyhow::ensure!(pack.bytes <= BYTE_CAP, "Weather pack exceeds 250 MiB; save fewer frames or layers");
    let result = async {
        for (family, keys) in [("satellite", &pack.satellite), ("mrms", &pack.mrms)] {
            for key in keys {
                anyhow::ensure!(!save_cancelled(), "Weather download cancelled; no replay saved");
                wxdata::object_cache::set_pinned(family, key, true).await?;
            }
        }
        anyhow::ensure!(!save_cancelled(), "Weather download cancelled; no replay saved");
        commit_pack(&db, &pack.key(), Some(&pack), volumes).await
    }.await;
    match result {
        Err(error) => {
            release_objects(&db, &pack).await;
            return Err(error);
        }
        Ok(Some(previous)) => release_objects(&db, &previous).await,
        Ok(None) => {}
    }
    evict(&db, &pack.key()).await;
    Ok(pack)
}

/// Delete oldest packs until the manifests fit under the cap.
#[cfg(target_arch = "wasm32")]
async fn evict(db: &IdbDatabase, keep: &str) {
    let Ok(mut all) = list_packs(db).await else { return };
    let mut total: f64 = all.iter().map(|pack| pack.bytes).sum();
    all.sort_by_key(|pack| pack.saved_at);
    for pack in all {
        if total <= BYTE_CAP { break; }
        if pack.key() == keep { continue; }
        if delete_pack(db, &pack).await.is_ok() { total -= pack.bytes; }
    }
}

/// Delete the manifest and unshared radar files atomically, then release unshared weather pins.
#[cfg(target_arch = "wasm32")]
async fn delete_pack(db: &IdbDatabase, pack: &Pack) -> anyhow::Result<()> {
    if let Some(previous) = commit_pack(db, &pack.key(), None, Vec::new()).await? {
        release_objects(db, &previous).await;
    }
    Ok(())
}

/// Serialize delete with save so it cannot remove files a replacement is about to reference.
#[cfg(target_arch = "wasm32")]
pub async fn remove(key: String) {
    let Some(_guard) = begin_save() else { return };
    let result = async {
        let db = open().await?;
        if let Some(pack) = list_packs(&db).await?.into_iter().find(|pack| pack.key() == key) {
            delete_pack(&db, &pack).await?;
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    set_status(Some(match result {
        Ok(()) => "Saved weather deleted".into(),
        Err(error) => format!("Could not delete saved weather: {error}"),
    }));
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
pub async fn save_timeline(
    site: String,
    date: String,
    ids: Vec<wxdata::level2::Identifier>,
    satellite: Vec<String>,
    mrms: Vec<String>,
    overlays: std::collections::BTreeMap<String, String>,
    placefiles: Vec<crate::settings::PlacefileConfig>,
) {
    let total = ids.len();
    let mut out = Vec::new();
    for (i, id) in ids.into_iter().enumerate() {
        if save_cancelled() { return; }
        let name = id.name().to_string();
        set_status(Some(format!("saving {} of {total}\u{2026}", i + 1)));
        let bytes = match volume(&name).await {
            Some(b) => b,
            None => match wxdata::level2::volume_bytes(id).await {
                Ok(b) => b,
                Err(e) => {
                    set_status(Some(format!("Weather pack incomplete: {name} failed: {e}. No replay saved.")));
                    return;
                }
            },
        };
        out.push((name, bytes));
        if out.iter().map(|(_, b): &(String, Vec<u8>)| b.len()).sum::<usize>() as f64 > BYTE_CAP {
            set_status(Some("Weather pack exceeds 250 MiB; save fewer frames".into()));
            return;
        }
    }
    if save_cancelled() { return; }
    match save_pack(&site, &date, out, satellite, mrms, overlays, placefiles).await {
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

    fn pack(volumes: &[&str], satellite: &[&str], mrms: &[&str]) -> Pack {
        Pack { site: "KTLX".into(), date: "2026-05-20".into(),
            volumes: volumes.iter().map(|name| (*name).into()).collect(),
            satellite: satellite.iter().map(|name| (*name).into()).collect(),
            mrms: mrms.iter().map(|name| (*name).into()).collect(),
            overlays: Default::default(), placefiles: Vec::new(), saved_at: 1, bytes: 3.0, available: true }
    }

    #[test]
    fn replacement_only_releases_files_without_surviving_references() {
        let previous = pack(&["A", "B", "A"], &["sat-old", "sat-shared"], &["mrms-old"]);
        let replacement = pack(&["B", "C"], &["sat-new", "sat-shared"], &[]);
        assert_eq!(unreferenced(&previous, std::slice::from_ref(&replacement)), Unreferenced {
            volumes: vec!["A".into()], satellite: vec!["sat-old".into()], mrms: vec!["mrms-old".into()],
        });
        let other = pack(&["A"], &["sat-old"], &["mrms-old"]);
        assert_eq!(unreferenced(&previous, &[replacement, other]), Unreferenced::default());
    }

    #[test]
    fn failed_save_rollback_keeps_the_previous_packs_pins() {
        let previous = pack(&["A", "B"], &["sat-shared"], &["mrms-shared"]);
        let failed = pack(&["B", "C"], &["sat-new", "sat-shared"], &["mrms-new", "mrms-shared"]);
        assert_eq!(unreferenced(&failed, &[previous]), Unreferenced {
            volumes: vec!["C".into()], satellite: vec!["sat-new".into()], mrms: vec!["mrms-new".into()],
        });
    }

    #[test]
    fn deleting_last_pack_releases_all_its_files_once() {
        let last = pack(&["A", "A", "B"], &["sat", "sat"], &["mrms"]);
        assert_eq!(unreferenced(&last, &[]), Unreferenced {
            volumes: vec!["A".into(), "B".into()], satellite: vec!["sat".into()], mrms: vec!["mrms".into()],
        });
    }

    #[test]
    fn a_pack_round_trips_through_its_manifest() {
        let p = Pack {
            site: "KTLX".into(),
            date: "2026-05-20".into(),
            volumes: vec!["KTLX20260520_231502_V06".into()],
            satellite: vec!["ABI-L2-CMIPC/example.nc".into()],
            mrms: vec!["MergedReflectivityQCComposite/example.grib2.gz".into()],
            overlays: std::collections::BTreeMap::from([(
                "gis:damage.geojson".into(),
                r#"{"type":"FeatureCollection","features":[]}"#.into(),
            )]),
            placefiles: vec![crate::settings::PlacefileConfig {
                url: "gis:damage.geojson".into(),
                enabled: true,
                opacity: 1.0,
                gis_label_field: None,
                gis_color_field: None,
                gis_valid_start_field: None,
                gis_valid_end_field: None,
            }],
            saved_at: 1_780_000_000,
            bytes: 32.0 * 1024.0 * 1024.0,
            available: false,
        };
        let back: Pack = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.key(), "KTLX-2026-05-20");
        assert_eq!(
            back.label(),
            "KTLX 2026-05-20 — 1 volume, 3 weather files, 32 MB"
        );
        let old: Pack = serde_json::from_str(
            r#"{"site":"KTLX","date":"2026-05-20","volumes":[],"saved_at":1,"bytes":0}"#,
        )
        .unwrap();
        assert!(old.satellite.is_empty() && old.mrms.is_empty() && old.overlays.is_empty());
    }
}
