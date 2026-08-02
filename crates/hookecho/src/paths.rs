//! App storage roots. Desktop resolves the OS config/data/cache dirs via `directories`; Android
//! has no such split, so `android_main` sets one app-private base dir and the three roots become
//! `<base>/config`, `<base>/data`, `<base>/cache`.
//!
//! Every persistent path in the app (settings, color tables, marker icons, tile + climatology
//! caches) goes through here, so the platform difference lives in exactly one place.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Storage base override (Android sets this once from the activity's internal data path).
static BASE: OnceLock<PathBuf> = OnceLock::new();

/// Point every storage root at `base` (Android). One-shot: later calls are ignored.
pub fn set_base(base: PathBuf) {
    let _ = BASE.set(base);
}

/// Resolve a named root: the override's subfolder when set (Android), else the matching OS dir.
fn root(kind: &str) -> Option<PathBuf> {
    if let Some(base) = BASE.get() {
        return Some(base.join(kind));
    }
    // In a browser there is no filesystem. Every caller already treats `None` as "no cache and no
    // saved settings", so the web build runs on defaults and keeps everything in memory.
    //
    // ponytail: no persistence on the web; `localStorage` for settings is the upgrade path.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = kind;
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let pd = directories::ProjectDirs::from("", "", "hookecho")?;
    #[cfg(not(target_arch = "wasm32"))]
    Some(match kind {
        "config" => pd.config_dir().to_path_buf(),
        "data" => pd.data_dir().to_path_buf(),
        _ => pd.cache_dir().to_path_buf(),
    })
}

/// Config root (settings.json lives here).
pub fn config_dir() -> Option<PathBuf> {
    root("config")
}

/// Data root (color tables, marker icons).
pub fn data_dir() -> Option<PathBuf> {
    root("data")
}

/// Deep-link drop box: the Android alert service's notification tap writes `SITE,lon,lat,zoom`
/// here (see `MainActivity.kt`) and the app consumes it at startup and on resume. `None` on
/// desktop, where the env var does the same job.
pub fn goto_file() -> Option<PathBuf> {
    BASE.get().map(|b| b.join("goto.txt"))
}

/// Cache root (tiles, vector tiles, climatology CSV).
pub fn cache_dir() -> Option<PathBuf> {
    root("cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_roots_resolve_without_override() {
        // With no override set (the desktop path), all three resolve via `directories`.
        // (On CI's headless Linux these still return Some — ProjectDirs needs no real HOME write.)
        assert!(config_dir().is_some());
        assert!(data_dir().is_some());
        assert!(cache_dir().is_some());
    }
}
