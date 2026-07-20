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
    let pd = directories::ProjectDirs::from("", "", "hookecho")?;
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
