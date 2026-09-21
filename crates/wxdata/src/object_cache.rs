//! Automatic browser cache for immutable source objects.

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Meta {
    key: String,
    family: String,
    bytes: usize,
    #[serde(default)]
    checksum: u64,
    received_at: i64,
    accessed_at: i64,
    #[serde(default)]
    pinned: bool,
}

pub struct CachedObject {
    pub bytes: Vec<u8>,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

fn range_key(url: &str, start: u64, end: Option<u64>) -> String {
    end.map_or_else(
        || format!("{url}#bytes={start}-"),
        |end| format!("{url}#bytes={start}-{}", end - 1),
    )
}

/// Read one immutable model message, reusing the exact source-object byte range in browsers.
pub async fn fetch_range(
    http: &reqwest::Client,
    url: &str,
    start: u64,
    end: Option<u64>,
) -> anyhow::Result<CachedObject> {
    let key = range_key(url, start, end);
    if let Some(cached) = get("model", &key).await {
        return Ok(cached);
    }
    let range = end.map_or_else(
        || format!("bytes={start}-"),
        |end| format!("bytes={start}-{}", end - 1),
    );
    let bytes = http
        .get(crate::net::fetch_url(url))
        .timeout(crate::net::FEED_TIMEOUT)
        .header("User-Agent", crate::alerts::USER_AGENT)
        .header("Range", range)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let received_at = chrono::Utc::now();
    put("model", &key, &bytes, received_at).await?;
    Ok(CachedObject {
        bytes: bytes.to_vec(),
        received_at,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStats {
    pub family: String,
    pub objects: usize,
    pub bytes: usize,
    pub cap: usize,
}

#[cfg(target_arch = "wasm32")]
fn family_cap(family: &str) -> usize {
    match family {
        "mrms" => 128 * 1024 * 1024,
        "model" | "satellite" | "radar" => 256 * 1024 * 1024,
        _ => 64 * 1024 * 1024,
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn eviction_keys(mut entries: Vec<Meta>, cap: usize) -> Vec<String> {
    entries.retain(|entry| !entry.pinned);
    let mut total: usize = entries.iter().map(|entry| entry.bytes).sum();
    entries.sort_by_key(|entry| entry.accessed_at);
    entries
        .into_iter()
        .filter_map(|entry| {
            if total <= cap {
                return None;
            }
            total = total.saturating_sub(entry.bytes);
            Some(entry.key)
        })
        .collect()
}

fn checksum(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    hash.finish()
}

#[cfg(any(target_arch = "wasm32", test))]
fn newest_key(entries: &[Meta], family: &str, prefix: &str) -> Option<String> {
    entries
        .iter()
        .filter(|entry| entry.family == family && entry.key.starts_with(prefix))
        .map(|entry| entry.key.as_str())
        .max()
        .map(str::to_string)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{CacheStats, CachedObject, Meta, checksum, eviction_keys, family_cap, newest_key};
    use anyhow::anyhow;
    use wasm_bindgen::{JsCast, prelude::*};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{IdbDatabase, IdbObjectStore, IdbRequest, IdbTransactionMode};

    const DB: &str = "hookecho-objects";
    const DATA: &str = "data";
    const META: &str = "meta";
    static MEMORY: std::sync::Mutex<Vec<(Meta, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());

    fn memory_get(family: &str, key: &str) -> Option<CachedObject> {
        let mut cache = MEMORY.lock().ok()?;
        let (meta, bytes) = cache
            .iter_mut()
            .find(|(meta, _)| meta.family == family && meta.key == key)?;
        meta.accessed_at = chrono::Utc::now().timestamp();
        Some(CachedObject {
            bytes: bytes.clone(),
            received_at: chrono::DateTime::from_timestamp(meta.received_at, 0)?,
        })
    }

    fn memory_put(
        family: &str,
        key: &str,
        bytes: &[u8],
        received_at: chrono::DateTime<chrono::Utc>,
    ) {
        let Ok(mut cache) = MEMORY.lock() else { return };
        cache.retain(|(meta, _)| meta.key != key);
        cache.push((
            Meta {
                key: key.to_string(),
                family: family.to_string(),
                bytes: bytes.len(),
                checksum: checksum(bytes),
                received_at: received_at.timestamp(),
                accessed_at: chrono::Utc::now().timestamp(),
                pinned: false,
            },
            bytes.to_vec(),
        ));
        let entries = cache
            .iter()
            .filter(|(meta, _)| meta.family == family)
            .map(|(meta, _)| meta.clone())
            .collect();
        let evict = eviction_keys(entries, family_cap(family));
        cache.retain(|(meta, _)| !evict.contains(&meta.key));
    }

    fn memory_remove(key: &str) {
        if let Ok(mut cache) = MEMORY.lock() {
            cache.retain(|(meta, _)| meta.key != key);
        }
    }

    async fn await_request(req: IdbRequest) -> anyhow::Result<JsValue> {
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let result = req.clone();
            let ok = Closure::once_into_js(move |_: JsValue| {
                let _ = resolve.call1(&JsValue::NULL, &result.result().unwrap_or(JsValue::NULL));
            });
            req.set_onsuccess(Some(ok.unchecked_ref()));
            let err = Closure::once_into_js(move |_: JsValue| {
                let _ = reject.call1(&JsValue::NULL, &"indexeddb request failed".into());
            });
            req.set_onerror(Some(err.unchecked_ref()));
        });
        JsFuture::from(promise)
            .await
            .map_err(|error| anyhow!("{error:?}"))
    }

    async fn open() -> anyhow::Result<IdbDatabase> {
        let factory = web_sys::window()
            .and_then(|window| window.indexed_db().ok().flatten())
            .ok_or_else(|| anyhow!("IndexedDB unavailable"))?;
        let request = factory
            .open_with_u32(DB, 1)
            .map_err(|error| anyhow!("{error:?}"))?;
        let upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(request) = event
                .target()
                .and_then(|target| target.dyn_into::<IdbRequest>().ok())
            else {
                return;
            };
            let Ok(db) = request
                .result()
                .and_then(|value| value.dyn_into::<IdbDatabase>())
            else {
                return;
            };
            let _ = db.create_object_store(DATA);
            let _ = db.create_object_store(META);
        });
        request.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
        let db = await_request(request.clone().unchecked_into()).await?;
        upgrade.forget();
        db.dyn_into::<IdbDatabase>()
            .map_err(|error| anyhow!("{error:?}"))
    }

    fn store(
        db: &IdbDatabase,
        name: &str,
        mode: IdbTransactionMode,
    ) -> anyhow::Result<IdbObjectStore> {
        db.transaction_with_str_and_mode(name, mode)
            .and_then(|transaction| transaction.object_store(name))
            .map_err(|error| anyhow!("{error:?}"))
    }

    async fn put_meta(db: &IdbDatabase, meta: &Meta) -> anyhow::Result<()> {
        let json = serde_json::to_string(meta)?;
        let request = store(db, META, IdbTransactionMode::Readwrite)?
            .put_with_key(&json.into(), &meta.key.as_str().into())
            .map_err(|error| anyhow!("{error:?}"))?;
        await_request(request).await.map(|_| ())
    }

    async fn delete(db: &IdbDatabase, key: &str) {
        for name in [DATA, META] {
            if let Ok(store) = store(db, name, IdbTransactionMode::Readwrite) {
                if let Ok(request) = store.delete(&key.into()) {
                    let _ = await_request(request).await;
                }
            }
        }
    }

    async fn metadata(db: &IdbDatabase) -> Vec<Meta> {
        let Ok(store) = store(db, META, IdbTransactionMode::Readonly) else {
            return Vec::new();
        };
        let Ok(request) = store.get_all() else {
            return Vec::new();
        };
        let Ok(values) = await_request(request).await else {
            return Vec::new();
        };
        js_sys::Array::from(&values)
            .iter()
            .filter_map(|value| value.as_string())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect()
    }

    async fn persistent_get(family: &str, key: &str) -> Option<CachedObject> {
        let db = open().await.ok()?;
        let value = await_request(
            store(&db, DATA, IdbTransactionMode::Readonly)
                .ok()?
                .get(&key.into())
                .ok()?,
        )
        .await
        .ok()?;
        if value.is_null() || value.is_undefined() {
            return None;
        }
        let bytes = value.dyn_into::<js_sys::Uint8Array>().ok()?.to_vec();
        let meta_value = await_request(
            store(&db, META, IdbTransactionMode::Readonly)
                .ok()?
                .get(&key.into())
                .ok()?,
        )
        .await
        .ok()?;
        let mut meta: Meta = serde_json::from_str(&meta_value.as_string()?).ok()?;
        if meta.family != family || meta.bytes != bytes.len() || meta.checksum != checksum(&bytes) {
            delete(&db, key).await;
            return None;
        }
        meta.accessed_at = chrono::Utc::now().timestamp();
        let _ = put_meta(&db, &meta).await;
        Some(CachedObject {
            bytes,
            received_at: chrono::DateTime::from_timestamp(meta.received_at, 0)?,
        })
    }

    async fn persistent_put(
        family: &str,
        key: &str,
        bytes: &[u8],
        received_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        let db = open().await?;
        let array = js_sys::Uint8Array::from(bytes);
        let request = store(&db, DATA, IdbTransactionMode::Readwrite)?
            .put_with_key(&array, &key.into())
            .map_err(|error| anyhow!("{error:?}"))?;
        await_request(request).await?;
        put_meta(
            &db,
            &Meta {
                key: key.to_string(),
                family: family.to_string(),
                bytes: bytes.len(),
                checksum: checksum(bytes),
                received_at: received_at.timestamp(),
                accessed_at: chrono::Utc::now().timestamp(),
                pinned: false,
            },
        )
        .await?;
        let family_entries: Vec<_> = metadata(&db)
            .await
            .into_iter()
            .filter(|entry| entry.family == family)
            .collect();
        for old in eviction_keys(family_entries, family_cap(family)) {
            delete(&db, &old).await;
        }
        refresh().await;
        Ok(())
    }

    pub async fn get(family: &str, key: &str) -> Option<CachedObject> {
        persistent_get(family, key)
            .await
            .or_else(|| memory_get(family, key))
    }

    pub async fn latest_key(family: &str, prefix: &str) -> Option<String> {
        let mut entries = match open().await {
            Ok(db) => metadata(&db).await,
            Err(_) => Vec::new(),
        };
        if let Ok(cache) = MEMORY.lock() {
            entries.extend(cache.iter().map(|(meta, _)| meta.clone()));
        }
        newest_key(&entries, family, prefix)
    }

    pub async fn put(
        family: &str,
        key: &str,
        bytes: &[u8],
        received_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        if let Err(error) = persistent_put(family, key, bytes, received_at).await {
            memory_put(family, key, bytes, received_at);
            if let Ok(mut state) = STATE.lock() {
                state.3 = Some(format!(
                    "Browser storage unavailable; using memory cache: {error}"
                ));
            }
            refresh().await;
        } else {
            memory_remove(key);
            if let Ok(mut state) = STATE.lock() {
                state.3 = None;
            }
        }
        Ok(())
    }

    async fn stats() -> Vec<CacheStats> {
        let mut totals = std::collections::BTreeMap::<String, (usize, usize)>::new();
        if let Ok(db) = open().await {
            for entry in metadata(&db).await {
                let total = totals.entry(entry.family).or_default();
                total.0 += 1;
                total.1 += entry.bytes;
            }
        }
        if let Ok(cache) = MEMORY.lock() {
            for (entry, _) in cache.iter() {
                let total = totals.entry(entry.family.clone()).or_default();
                total.0 += 1;
                total.1 += entry.bytes;
            }
        }
        totals
            .into_iter()
            .map(|(family, (objects, bytes))| CacheStats {
                cap: family_cap(&family),
                family,
                objects,
                bytes,
            })
            .collect()
    }

    async fn clear_family(family: &str) -> anyhow::Result<()> {
        if let Ok(mut cache) = MEMORY.lock() {
            cache.retain(|(entry, _)| entry.family != family || entry.pinned);
        }
        let db = open().await?;
        for entry in metadata(&db)
            .await
            .into_iter()
            .filter(|entry| entry.family == family && !entry.pinned)
        {
            delete(&db, &entry.key).await;
        }
        Ok(())
    }

    pub async fn set_pinned(family: &str, key: &str, pinned: bool) -> anyhow::Result<()> {
        if let Ok(mut cache) = MEMORY.lock() {
            if let Some((meta, _)) = cache
                .iter_mut()
                .find(|(meta, _)| meta.family == family && meta.key == key)
            {
                meta.pinned = pinned;
            }
        }
        let db = open().await?;
        let Some(mut meta) = metadata(&db)
            .await
            .into_iter()
            .find(|entry| entry.family == family && entry.key == key)
        else {
            anyhow::bail!("cached object not found");
        };
        meta.pinned = pinned;
        put_meta(&db, &meta).await
    }

    static STATE: std::sync::Mutex<(Vec<CacheStats>, bool, bool, Option<String>)> =
        std::sync::Mutex::new((Vec::new(), false, false, None));

    async fn refresh() {
        let rows = stats().await;
        if let Ok(mut state) = STATE.lock() {
            state.0 = rows;
            state.2 = true;
        }
    }

    pub fn known_stats() -> (Vec<CacheStats>, bool, Option<String>) {
        let asked = STATE
            .lock()
            .map(|mut state| {
                let asked = state.1;
                state.1 = true;
                asked
            })
            .unwrap_or(true);
        if !asked {
            wasm_bindgen_futures::spawn_local(refresh());
        }
        STATE
            .lock()
            .map(|state| (state.0.clone(), state.2, state.3.clone()))
            .unwrap_or_default()
    }

    pub fn spawn_clear(family: String) {
        wasm_bindgen_futures::spawn_local(async move {
            let result = clear_family(&family).await;
            if let Ok(mut state) = STATE.lock() {
                state.3 = result.err().map(|error| error.to_string());
            }
            refresh().await;
        });
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{get, known_stats, latest_key, put, set_pinned, spawn_clear};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{CacheStats, CachedObject, checksum};
    use chrono::{DateTime, Utc};
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    const MAGIC: &[u8; 4] = b"HEO1";
    static ROOT: OnceLock<PathBuf> = OnceLock::new();

    pub fn set_root(root: PathBuf) {
        let _ = ROOT.set(root);
    }

    pub fn root() -> Option<&'static Path> {
        ROOT.get().map(PathBuf::as_path)
    }

    pub fn known_stats() -> (Vec<CacheStats>, bool, Option<String>) {
        let Some(root) = root() else {
            return (Vec::new(), false, None);
        };
        let mut rows = Vec::new();
        let Ok(families) = std::fs::read_dir(root) else {
            return (rows, false, None);
        };
        for family in families.flatten().filter(|entry| entry.path().is_dir()) {
            let mut objects = 0;
            let mut bytes = 0;
            if let Ok(entries) = std::fs::read_dir(family.path()) {
                for entry in entries.flatten().filter(|entry| entry.path().is_file()) {
                    objects += 1;
                    bytes += entry.metadata().map(|meta| meta.len() as usize).unwrap_or(0);
                }
            }
            rows.push(CacheStats {
                family: family.file_name().to_string_lossy().into_owned(),
                objects,
                bytes,
                cap: 0,
            });
        }
        (rows, false, None)
    }

    fn key_hash(key: &str) -> u64 {
        key.as_bytes()
            .iter()
            .fold(0xcbf29ce484222325, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            })
    }

    fn path(root: &Path, family: &str, key: &str) -> PathBuf {
        root.join(family)
            .join(format!("{:016x}.obj", key_hash(key)))
    }

    fn read(path: &Path, expected_key: Option<&str>) -> Option<(String, CachedObject)> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut fixed = [0u8; 24];
        file.read_exact(&mut fixed).ok()?;
        if &fixed[..4] != MAGIC {
            return None;
        }
        let received = i64::from_le_bytes(fixed[4..12].try_into().ok()?);
        let expected_checksum = u64::from_le_bytes(fixed[12..20].try_into().ok()?);
        let key_len = u32::from_le_bytes(fixed[20..24].try_into().ok()?) as usize;
        if key_len > 64 * 1024 {
            return None;
        }
        let mut key = vec![0; key_len];
        file.read_exact(&mut key).ok()?;
        let key = String::from_utf8(key).ok()?;
        if expected_key.is_some_and(|expected| expected != key) {
            return None;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        if checksum(&bytes) != expected_checksum {
            let _ = std::fs::remove_file(path);
            return None;
        }
        Some((
            key,
            CachedObject {
                bytes,
                received_at: DateTime::from_timestamp(received, 0)?,
            },
        ))
    }

    fn read_key(path: &Path) -> Option<String> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut fixed = [0u8; 24];
        file.read_exact(&mut fixed).ok()?;
        if &fixed[..4] != MAGIC {
            return None;
        }
        let key_len = u32::from_le_bytes(fixed[20..24].try_into().ok()?) as usize;
        if key_len > 64 * 1024 {
            return None;
        }
        let mut key = vec![0; key_len];
        file.read_exact(&mut key).ok()?;
        String::from_utf8(key).ok()
    }

    fn write(
        root: &Path,
        family: &str,
        key: &str,
        bytes: &[u8],
        received_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let path = path(root, family, key);
        let parent = path.parent().expect("cache object has parent");
        std::fs::create_dir_all(parent)?;
        let temp = path.with_extension("tmp");
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(MAGIC)?;
        file.write_all(&received_at.timestamp().to_le_bytes())?;
        file.write_all(&checksum(bytes).to_le_bytes())?;
        file.write_all(&(key.len() as u32).to_le_bytes())?;
        file.write_all(key.as_bytes())?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(temp, path)?;
        Ok(())
    }

    pub async fn get(family: &str, key: &str) -> Option<CachedObject> {
        let root = root()?.to_path_buf();
        let family = family.to_string();
        let key = key.to_string();
        crate::task::blocking(move || read(&path(&root, &family, &key), Some(&key)))
            .await
            .ok()?
            .map(|(_, object)| object)
    }

    pub async fn put(
        family: &str,
        key: &str,
        bytes: &[u8],
        received_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let Some(root) = root().map(Path::to_path_buf) else {
            return Ok(());
        };
        let family = family.to_string();
        let key = key.to_string();
        let bytes = bytes.to_vec();
        crate::task::blocking(move || write(&root, &family, &key, &bytes, received_at)).await??;
        Ok(())
    }

    pub async fn latest_key(family: &str, prefix: &str) -> Option<String> {
        let root = root()?.join(family);
        let prefix = prefix.to_string();
        crate::task::blocking(move || {
            std::fs::read_dir(root)
                .ok()?
                .filter_map(Result::ok)
                .filter_map(|entry| read_key(&entry.path()))
                .filter(|key| key.starts_with(&prefix))
                .max()
        })
        .await
        .ok()?
    }

    pub async fn set_pinned(_family: &str, _key: &str, _pinned: bool) -> anyhow::Result<()> {
        // Native chase packs retain their own source copies; automatic-cache eviction cannot
        // remove them, so no pin marker is needed here.
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{get, known_stats, latest_key, put, set_pinned};

#[cfg(not(target_arch = "wasm32"))]
pub fn set_native_root(root: std::path::PathBuf) {
    native::set_root(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_is_lru_and_stops_at_the_family_cap() {
        assert_ne!(checksum(b"complete"), checksum(b"corrupt"));
        let entry = |key: &str, bytes, accessed_at| Meta {
            key: key.into(),
            family: "mrms".into(),
            bytes,
            checksum: 0,
            received_at: 0,
            accessed_at,
            pinned: false,
        };
        let keys = eviction_keys(
            vec![
                entry("new", 40, 30),
                entry("old", 40, 10),
                entry("middle", 40, 20),
            ],
            80,
        );
        assert_eq!(keys, ["old"]);
        let mut pinned = entry("pinned", 400, 0);
        pinned.pinned = true;
        assert!(eviction_keys(vec![pinned], 1).is_empty());
    }

    #[test]
    fn newest_key_is_scoped_to_family_and_prefix() {
        let entry = |family: &str, key: &str| Meta {
            key: key.into(),
            family: family.into(),
            bytes: 1,
            checksum: 0,
            received_at: 0,
            accessed_at: 0,
            pinned: false,
        };
        let entries = [
            entry("mrms", "CONUS/MESH/20260919/a"),
            entry("mrms", "CONUS/MESH/20260919/b"),
            entry("model", "CONUS/MESH/20260919/z"),
        ];
        assert_eq!(
            newest_key(&entries, "mrms", "CONUS/MESH/"),
            Some("CONUS/MESH/20260919/b".into())
        );
        assert_eq!(newest_key(&entries, "mrms", "CONUS/QPE/"), None);
    }

    #[test]
    fn range_identity_includes_object_and_exact_interval() {
        assert_eq!(range_key("object", 10, Some(20)), "object#bytes=10-19");
        assert_ne!(
            range_key("object", 10, Some(20)),
            range_key("object", 10, None)
        );
        assert_ne!(range_key("object", 10, None), range_key("other", 10, None));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn native_cache_round_trips_bytes_receipt_and_latest_key() {
        let root = std::env::temp_dir().join(format!(
            "hookecho-object-cache-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        set_native_root(root.clone());
        let received = chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp(), 0)
            .expect("valid timestamp");
        put("mrms", "product/20260921/a", b"first", received)
            .await
            .unwrap();
        put("mrms", "product/20260921/b", b"second", received)
            .await
            .unwrap();
        let cached = get("mrms", "product/20260921/a").await.unwrap();
        assert_eq!(cached.bytes, b"first");
        assert_eq!(cached.received_at, received);
        assert_eq!(
            latest_key("mrms", "product/").await.as_deref(),
            Some("product/20260921/b")
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
