//! What the app has put on your disk, and how to get rid of it.
//!
//! Six caches accumulate under the cache root — map tiles, vector tiles, archived radar volumes,
//! zone geometry, RAOB soundings, rendered snapshots — plus a few loose files. Every one of them
//! is swept at startup against a cap, but until now nothing said how big any of them were, so the
//! only way to find out was `du`, and the only way to clear one was `rm -rf`.
//!
//! Sizes come from a directory walk, which on a full 2 GB tile cache is tens of thousands of
//! files. That runs on a background thread, once, when the tab is opened — never per frame.

use std::path::{Path, PathBuf};

/// One row of the storage report.
pub struct Entry {
    pub label: &'static str,
    pub path: PathBuf,
    pub bytes: u64,
    /// The sweep cap this is trimmed to at startup, if it has one.
    pub cap: Option<u64>,
}

/// Total bytes of every file under `root`, following subdirectories. Missing directories are 0,
/// not an error — a cache nothing has written to yet simply doesn't exist.
pub fn dir_size(root: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(md) if md.is_dir() => dir_size(&e.path()),
            Ok(md) => md.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Every cache the app writes, measured. Blocking — call it off the UI thread.
pub fn report() -> Vec<Entry> {
    let Some(root) = crate::paths::cache_dir() else {
        return Vec::new();
    };
    let tiles = crate::tiles::tile_cache_bytes();
    let small = crate::tiles::SMALL_CACHE_BYTES;
    [
        ("Map tiles", "tiles", Some(tiles)),
        ("Vector tiles", "vector", Some(tiles)),
        ("Radar volumes", "volumes", Some(crate::tiles::volume_cache_bytes())),
        ("Zone geometry", "zones", Some(small)),
        ("Soundings (RAOB)", "raob", Some(small)),
        ("Server snapshots", "snapshots", Some(small)),
    ]
    .into_iter()
    .map(|(label, sub, cap)| {
        let path = root.join(sub);
        Entry {
            label,
            bytes: dir_size(&path),
            path,
            cap,
        }
    })
    // Everything else directly in the root: the alert snapshot, the tornado climatology CSV, and
    // whatever a future feature drops there. One row rather than a name each — they are kilobytes.
    .chain(std::iter::once_with(|| {
        let loose = std::fs::read_dir(&root)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.metadata().ok())
                    .filter(|md| md.is_file())
                    .map(|md| md.len())
                    .sum()
            })
            .unwrap_or(0);
        Entry {
            label: "Other cached files",
            path: root.clone(),
            bytes: loose,
            cap: None,
        }
    }))
    .collect()
}

/// Delete a cache directory's contents. The directory itself is recreated, because the writers
/// that use it assume it exists.
pub fn clear(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Bytes as something a person reads, at three significant figures.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_size_counts_nested_files() {
        let root = std::env::temp_dir().join(format!("hookecho-storage-{}", std::process::id()));
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("one"), [0u8; 100]).unwrap();
        std::fs::write(nested.join("two"), [0u8; 200]).unwrap();
        assert_eq!(dir_size(&root), 300);

        clear(&root).unwrap();
        assert_eq!(dir_size(&root), 0, "cleared, and the directory still exists");
        assert!(root.is_dir());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_directory_is_zero_not_an_error() {
        assert_eq!(dir_size(Path::new("/nonexistent/hookecho/cache")), 0);
    }

    #[test]
    fn human_rounds_to_the_unit() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
