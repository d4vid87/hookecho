//! App storage roots. Desktop resolves the OS config/data/cache dirs via `directories`; Android
//! has no such split, so `android_main` sets one app-private base dir and the three roots become
//! `<base>/config`, `<base>/data`, `<base>/cache`.
//!
//! Every persistent path in the app (settings, color tables, marker icons, tile + climatology
//! caches) goes through here, so the platform difference lives in exactly one place.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Storage base override (Android sets this once from the activity's internal data path).
static BASE: OnceLock<PathBuf> = OnceLock::new();

// Selected by the standalone desktop launcher before any settings, crash reporter or
// background thread is initialized. Its storage never aliases the ordinary HookEcho profile.
static RADAR_WORKSPACE: OnceLock<bool> = OnceLock::new();

pub fn use_radar_workspace() {
    let _ = RADAR_WORKSPACE.set(true);
}

pub fn is_radar_workspace() -> bool {
    RADAR_WORKSPACE.get().copied().unwrap_or(false)
}

pub fn application_id() -> &'static str {
    application_id_for_profile(is_radar_workspace())
}

fn application_id_for_profile(radar_workspace: bool) -> &'static str {
    if radar_workspace {
        "hookecho-radar-workspace"
    } else {
        "hookecho"
    }
}

pub fn application_name() -> &'static str {
    if is_radar_workspace() {
        "HookEcho Radar Workspace"
    } else {
        "HookEcho"
    }
}

/// Point every storage root at `base` (Android). One-shot: later calls are ignored.
pub fn set_base(base: PathBuf) {
    let _ = BASE.set(base);
}

/// Resolve a named root: the override's subfolder when set (Android), else the matching OS dir.
fn root(kind: &str) -> Option<PathBuf> {
    root_for_profile(kind, BASE.get().map(PathBuf::as_path), is_radar_workspace())
}

// Explicit inputs let profile isolation be verified without changing process-wide OnceLocks.
fn root_for_profile(kind: &str, base: Option<&Path>, radar_workspace: bool) -> Option<PathBuf> {
    if let Some(base) = base {
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
        let _ = (kind, radar_workspace);
        return None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let pd = directories::ProjectDirs::from("", "", application_id_for_profile(radar_workspace))?;
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn desktop_profiles_have_separate_config_data_and_cache() {
        for kind in ["config", "data", "cache"] {
            let ordinary = root_for_profile(kind, None, false).expect("ordinary OS root");
            let standalone = root_for_profile(kind, None, true).expect("standalone OS root");
            assert_ne!(ordinary, standalone, "{kind} profiles must not alias");
            assert!(!ordinary.starts_with(&standalone));
            assert!(!standalone.starts_with(&ordinary));
        }
        assert_eq!(application_id_for_profile(false), "hookecho");
        assert_eq!(application_id_for_profile(true), "hookecho-radar-workspace");
    }

    #[test]
    fn android_override_keeps_its_existing_private_layout() {
        let base = Path::new("app-private");
        for kind in ["config", "data", "cache"] {
            assert_eq!(
                root_for_profile(kind, Some(base), false),
                Some(base.join(kind))
            );
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn browser_profiles_have_no_filesystem_roots() {
        for standalone in [false, true] {
            for kind in ["config", "data", "cache"] {
                assert!(root_for_profile(kind, None, standalone).is_none());
            }
        }
    }
}
