//! Small crash/reload checkpoint, kept separately from portable user settings.
const MAX_BYTES: usize = 256 * 1024;

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Option<String> {
    let path = crate::paths::config_dir()?.join("recovery.json");
    if std::fs::metadata(&path).ok()?.len() > MAX_BYTES as u64 { return None; }
    std::fs::read_to_string(path).ok()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save(json: &str) -> Result<(), String> {
    if json.len() > MAX_BYTES { return Err("recovery snapshot too large".into()); }
    let dir = crate::paths::config_dir().ok_or("no recovery directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::settings::atomic_write(&dir.join("recovery.json"), json.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn load() -> Option<String> {
    let json = web_sys::window()?.local_storage().ok()??.get_item("hookecho.recovery.v1").ok()??;
    (json.len() <= MAX_BYTES).then_some(json)
}

#[cfg(target_arch = "wasm32")]
pub fn save(json: &str) -> Result<(), String> {
    if json.len() > MAX_BYTES { return Err("recovery snapshot too large".into()); }
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or("recovery storage unavailable")?;
    storage.set_item("hookecho.recovery.v1", json).map_err(|e| format!("recovery storage: {e:?}"))
}
