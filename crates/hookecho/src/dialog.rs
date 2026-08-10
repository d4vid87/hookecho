//! Native file-dialog shim. Desktop uses `rfd`; Android goes through the Storage Access
//! Framework, which is asynchronous, so opening a file is a request now and a result later on
//! both platforms. Keeping both behind this shim lets the call sites stay platform-agnostic and
//! `rfd` stay off the Android build.

use std::path::PathBuf;

/// Choose a save path for `default_name` (a `<label>.<ext>` filename). Desktop pops a native save
/// dialog; Android returns `<data>/exports/<timestamp>-<default_name>` (creating the folder), so
/// screenshots / loops / exports land somewhere retrievable without a picker.
pub fn save_path(default_name: &str, ext: &str) -> Option<PathBuf> {
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    {
        rfd::FileDialog::new()
            .set_file_name(default_name)
            .add_filter(ext.to_uppercase(), &[ext])
            .save_file()
    }
    #[cfg(any(target_os = "android", target_arch = "wasm32"))]
    {
        let _ = ext;
        let dir = crate::paths::data_dir()?.join("exports");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        Some(dir.join(format!("{stamp}-{default_name}")))
    }
}

/// What an open request is for. The app needs this back with the path, because the picker returns
/// long after the button that opened it was clicked, and by then nothing else says what the file
/// was meant to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// A settings bundle written by Export settings.
    SettingsBundle,
    /// A GRLevelX `.pal` color table (both the palette editor and the Palettes tab).
    Palette,
    /// A PNG for a map marker.
    MarkerIcon,
    /// A sound file to play for alerts.
    AlertSound,
}

impl ImportKind {
    fn label(self) -> &'static str {
        match self {
            ImportKind::SettingsBundle => "Settings bundle",
            ImportKind::Palette => "GRLevelX color table",
            ImportKind::MarkerIcon => "Marker icon",
            ImportKind::AlertSound => "Alert sound",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            ImportKind::SettingsBundle => &["json"],
            ImportKind::Palette => &["pal", "pal3"],
            ImportKind::MarkerIcon => &["png"],
            ImportKind::AlertSound => &["wav", "mp3", "ogg", "flac"],
        }
    }

    /// What the Android picker filters on. SAF speaks MIME, and there is no registered type for
    /// a `.pal`, so those get the wildcard and the extension is checked on the way back.
    #[cfg(target_os = "android")]
    fn mime(self) -> &'static str {
        match self {
            ImportKind::SettingsBundle => "application/json",
            ImportKind::Palette => "*/*",
            ImportKind::MarkerIcon => "image/png",
            ImportKind::AlertSound => "audio/*",
        }
    }
}

/// The result of the last completed request, waiting to be collected: what it is, which thing
/// asked for it, and where it landed. The tag is how a result finds its way back to the row that
/// opened the picker — by the time the user has finished choosing, nothing else remembers.
static RESULT: std::sync::Mutex<Option<Import>> = std::sync::Mutex::new(None);

/// A file the user picked.
#[derive(Debug, Clone)]
pub struct Import {
    pub kind: ImportKind,
    /// Caller-chosen: a moment name, a marker index, an alert-sound row. Empty when the kind
    /// alone says everything.
    pub tag: String,
    pub path: PathBuf,
}

/// Ask the user for a file. Returns immediately on every platform; the answer arrives through
/// [`take_result`], which the app polls.
///
/// On desktop the dialog is modal and the answer is already there by the time this returns, which
/// is fine — the caller reads it a frame later either way, and one code path beats two.
pub fn request_open(kind: ImportKind, tag: impl Into<String>) {
    let tag = tag.into();
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(kind.label(), kind.extensions())
            .pick_file()
        {
            deliver(Import { kind, tag, path });
        }
    }
    #[cfg(target_os = "android")]
    {
        if let Err(e) = android_open::launch(kind, &tag) {
            log::warn!("could not open the file picker: {e:?}");
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        // No filesystem to import into. The buttons are hidden on the web anyway.
        let _ = (kind, tag);
    }
}

/// The file the user picked, once. `None` until they pick one.
pub fn take_result() -> Option<Import> {
    #[cfg(target_os = "android")]
    android_open::drain();
    RESULT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Record a picked file, for the app to route on its next frame.
#[allow(dead_code)]
fn deliver(import: Import) {
    *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(import);
}

/// Storage Access Framework, through the activity.
///
/// The picker is an activity result, so it lands in Kotlin, not here — and the app may not even
/// be resumed when it does. `MainActivity` copies the chosen content stream into the cache and
/// writes `kind<TAB>tag<TAB>path` to `filesDir/import.txt`, exactly the handover `goto.txt` already uses
/// for notification taps; this drains that file on the next frame.
#[cfg(target_os = "android")]
mod android_open {
    use super::{deliver, Import, ImportKind};
    use jni::objects::{JObject, JValue};

    fn slug(kind: ImportKind) -> &'static str {
        match kind {
            ImportKind::SettingsBundle => "settings",
            ImportKind::Palette => "palette",
            ImportKind::MarkerIcon => "marker",
            ImportKind::AlertSound => "sound",
        }
    }

    fn from_slug(s: &str) -> Option<ImportKind> {
        Some(match s {
            "settings" => ImportKind::SettingsBundle,
            "palette" => ImportKind::Palette,
            "marker" => ImportKind::MarkerIcon,
            "sound" => ImportKind::AlertSound,
            _ => return None,
        })
    }

    pub fn launch(kind: ImportKind, tag: &str) -> jni::errors::Result<()> {
        let Some(app) = crate::platform::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let kind_s = env.new_string(format!("{}\t{tag}", slug(kind)))?;
        let mime = env.new_string(kind.mime())?;
        let res = env.call_method(
            &activity,
            "openDocument",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&kind_s), JValue::Object(&mime)],
        );
        if res.is_err() {
            let _ = env.exception_clear();
        }
        res.map(|_| ())
    }

    /// Consume `import.txt` if the activity has written one.
    pub fn drain() {
        let Some(path) = crate::paths::import_file() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        // `kind<TAB>tag<TAB>path`; the tag may be empty, the path may not contain a tab.
        let mut parts = text.trim_end_matches('\n').splitn(3, '\t');
        let (Some(kind), Some(tag), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
            log::warn!("import.txt is malformed");
            return;
        };
        match from_slug(kind) {
            Some(kind) => deliver(Import {
                kind,
                tag: tag.to_string(),
                path: std::path::PathBuf::from(file),
            }),
            None => log::warn!("import.txt names an unknown kind '{kind}'"),
        }
    }
}
