//! Native file-dialog shim. Desktop uses `rfd`; Android (no `rfd`, no system picker in v1) writes
//! exports to a fixed app-private `exports/` folder and has no file-open path yet. Keeping both
//! behind this shim lets the call sites stay platform-agnostic and `rfd` stay off the Android build.

use std::path::PathBuf;

/// Choose a save path for `default_name` (a `<label>.<ext>` filename). Desktop pops a native save
/// dialog; Android returns `<data>/exports/<timestamp>-<default_name>` (creating the folder), so
/// screenshots / loops / exports land somewhere retrievable without a picker.
pub fn save_path(default_name: &str, ext: &str) -> Option<PathBuf> {
    #[cfg(not(target_os = "android"))]
    {
        rfd::FileDialog::new()
            .set_file_name(default_name)
            .add_filter(ext.to_uppercase(), &[ext])
            .save_file()
    }
    #[cfg(target_os = "android")]
    {
        let _ = ext;
        let dir = crate::paths::data_dir()?.join("exports");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        Some(dir.join(format!("{stamp}-{default_name}")))
    }
}

/// Choose an existing file to open, filtered to `exts` (labelled `label`). Desktop pops a native
/// open dialog; Android has no picker in v1, so this returns `None` (import features are inert).
/// `// ponytail: Android import via the Storage Access Framework is a JNI job — deferred to v2.`
pub fn open_path(label: &str, exts: &[&str]) -> Option<PathBuf> {
    #[cfg(not(target_os = "android"))]
    {
        rfd::FileDialog::new().add_filter(label, exts).pick_file()
    }
    #[cfg(target_os = "android")]
    {
        let _ = (label, exts);
        None
    }
}
