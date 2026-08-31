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
    // In a browser there is no filesystem. Every caller already treats `None` as "no cache", so
    // the web build keeps everything in memory. Settings are the exception: they persist through
    // `localStorage` instead, entirely inside `settings.rs` (nothing here returns a path for it).
    //
    // Tiles are the exception, and they are cached outside Rust entirely: the service worker
    // keeps them in Cache Storage (`web/sw.src.js`, the `tiles-v1` bucket). Radar volumes are
    // deliberately left out of that bucket — they are large, per-scan, and would evict the map.
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
/// here (see `MainActivity.kt`) and the app consumes it at startup and on resume. On desktop a
/// second launch carrying a `hookecho://` link writes the same file for the running instance
/// (see `main.rs`), so the handover shape is one thing on both platforms.
pub fn goto_file() -> Option<PathBuf> {
    match BASE.get() {
        Some(b) => Some(b.join("goto.txt")),
        None => cache_dir().map(|c| c.join("goto.txt")),
    }
}

/// Where the last panic's report is written, next to the settings so it survives a restart and
/// works the same on Android (see [`crate::crash`]).
pub fn crash_file() -> Option<PathBuf> {
    match BASE.get() {
        Some(b) => Some(b.join("last-panic.txt")),
        None => config_dir().map(|c| c.join("last-panic.txt")),
    }
}

/// Where the activity drops a file the user picked through the Storage Access Framework, in the
/// same handover shape as [`goto_file`]: the picker result arrives in Kotlin, and the app may not
/// be resumed to receive it.
pub fn import_file() -> Option<PathBuf> {
    BASE.get().map(|b| b.join("import.txt"))
}

/// Where the app writes the picture the home-screen radar widget shows (Android only). Sits at
/// the files-dir root, which is what `Context.filesDir` resolves to on the Kotlin side.
pub fn widget_snapshot() -> Option<PathBuf> {
    BASE.get().map(|b| b.join("widget-radar.png"))
}

/// Where the app writes the one-line storm caption the radar widget shows under its picture.
/// Beside the PNG, in the same files dir the Kotlin side reads.
pub fn widget_caption() -> Option<PathBuf> {
    BASE.get().map(|b| b.join("widget-radar.txt"))
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
