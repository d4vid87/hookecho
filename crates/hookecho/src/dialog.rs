//! File-dialog shim. Desktop uses `rfd`; Android goes through the Storage Access Framework; the
//! browser uses a hidden `<input type="file">` and a download link. All three are asynchronous —
//! opening a file is a request now and a result later — so the call sites stay platform-agnostic
//! and `rfd` stays off the Android and wasm builds.
//!
//! The browser has no filesystem, so a picked file arrives as bytes rather than a path and a
//! saved file leaves as a download. [`Import::text`] and [`save_bytes`] are the two call-site
//! shapes that work on every platform; a raw `path` is native-only by construction.

use std::path::PathBuf;

/// Where a save went, for the caller's toast. Cancelling is not a failure and gets no message.
pub enum Saved {
    /// The user dismissed the dialog.
    Cancelled,
    /// Written. The string is where, in whatever terms that platform has — a path, or "Downloads".
    Where(String),
    /// It did not get written. The string is why.
    Failed(String),
}

/// Save `bytes` under `default_name`. The whole-file form: the caller has the content in hand, so
/// the browser can hand it to the user as a download and no call site needs a path at all.
///
/// Browser screenshots can be PNG-encoded in memory and use this download path; native loop
/// exports still choose a destination before capture and write through [`save_path`].
pub fn save_bytes(default_name: &str, ext: &str, bytes: &[u8]) -> Saved {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ext;
        return match web_save::download(default_name, bytes) {
            Ok(()) => Saved::Where("your downloads".to_string()),
            Err(e) => Saved::Failed(e),
        };
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = save_path(default_name, ext) else {
            return Saved::Cancelled;
        };
        match std::fs::write(&path, bytes).map_err(|e| e.to_string())
            .and_then(|()| finish_save(&path)) {
            Ok(where_) => Saved::Where(where_),
            Err(e) => Saved::Failed(e),
        }
    }
}

/// Choose a save path for `default_name` (a `<label>.<ext>` filename). Desktop pops a native save
/// dialog; Android stages a private file. Call [`finish_save`] once its writer has closed to
/// offer the system Save/Share flow. Failed or incomplete files must never be handed off.
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
        std::fs::create_dir_all(&dir).ok()?;
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%f");
        Some(dir.join(format!("{stamp}-{default_name}")))
    }
}

/// Finish a path-based export; byte-based exports already call this after writing.
pub fn finish_save(path: &std::path::Path) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        android_open::offer_export(path)?;
        Ok("the Save / Share dialog".into())
    }
    #[cfg(not(target_os = "android"))]
    { Ok(path.display().to_string()) }
}

/// What an open request is for. The app needs this back with the path, because the picker returns
/// long after the button that opened it was clicked, and by then nothing else says what the file
/// was meant to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImportKind {
    /// A settings bundle written by Export settings.
    SettingsBundle,
    /// A GRLevelX `.pal` color table (both the palette editor and the Palettes tab).
    Palette,
    /// A PNG for a map marker.
    MarkerIcon,
    /// A sound file to play for alerts.
    AlertSound,
    /// A GPX track to replay (one this app wrote, or one another chase logger did).
    ChaseGpx,
    /// A user GIS overlay normalized into the map renderer.
    Gis,
    /// A portable HookEcho case manifest.
    CaseManifest,
}

impl ImportKind {
    /// Same limits as Android's bounded Storage Access Framework copy.
    pub const fn max_bytes(self) -> usize {
        let mib = match self {
            Self::Palette => 2,
            Self::SettingsBundle | Self::MarkerIcon => 8,
            Self::ChaseGpx | Self::CaseManifest => 16,
            Self::AlertSound => 32,
            Self::Gis => 64,
        };
        mib * 1024 * 1024
    }

    pub fn check_size(self, size: u64) -> Result<(), String> {
        if size > self.max_bytes() as u64 {
            Err(format!("{} exceeds the {} MiB limit", self.label(), self.max_bytes() / (1024 * 1024)))
        } else { Ok(()) }
    }

    fn label(self) -> &'static str {
        match self {
            ImportKind::SettingsBundle => "Settings bundle",
            ImportKind::Palette => "GRLevelX color table",
            ImportKind::MarkerIcon => "Marker icon",
            ImportKind::AlertSound => "Alert sound",
            ImportKind::ChaseGpx => "GPX track",
            ImportKind::Gis => "GIS overlay",
            ImportKind::CaseManifest => "HookEcho case manifest",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            ImportKind::SettingsBundle => &["json"],
            ImportKind::Palette => &["pal", "pal3"],
            ImportKind::MarkerIcon => &["png"],
            ImportKind::AlertSound => &["wav", "mp3", "ogg", "flac"],
            ImportKind::ChaseGpx => &["gpx"],
            ImportKind::Gis => &["geojson", "json", "kml", "kmz", "zip"],
            ImportKind::CaseManifest => &["hookecho-case.json", "json"],
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
            // No registered MIME for GPX that pickers agree on; the extension is checked on the
            // way back, same as a palette.
            ImportKind::ChaseGpx => "*/*",
            ImportKind::Gis => "*/*",
            ImportKind::CaseManifest => "application/json",
        }
    }
}

/// The result of the last completed request, waiting to be collected: what it is, which thing
/// asked for it, and where it landed. The tag is how a result finds its way back to the row that
/// opened the picker — by the time the user has finished choosing, nothing else remembers.
static RESULT: std::sync::Mutex<Option<Import>> = std::sync::Mutex::new(None);
static ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static IMPORTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn begin_import() -> bool {
    if IMPORTING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        fail("An import is already running. Wait for it to finish.".into());
        false
    } else { true }
}

pub fn finish_import() { IMPORTING.store(false, std::sync::atomic::Ordering::Release); }
fn fail(error: String) { *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(error); }
pub fn take_error() -> Option<String> { ERROR.lock().unwrap_or_else(|e| e.into_inner()).take() }

/// A file the user picked.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Import {
    pub kind: ImportKind,
    /// Caller-chosen: a moment name, a marker index, an alert-sound row. Empty when the kind
    /// alone says everything.
    pub tag: String,
    /// Where it landed. In a browser there is nowhere for it to land, so this is the file's own
    /// name and nothing will ever open it — read the content through [`Import::text`] instead.
    pub path: PathBuf,
    /// The content, when the platform handed it over instead of a path (the browser always does).
    pub bytes: Option<Vec<u8>>,
}

impl Import {
    /// The file's raw content, including binary GIS archives.
    pub fn content(&self) -> Result<Vec<u8>, String> {
        match &self.bytes {
            Some(bytes) => { self.kind.check_size(bytes.len() as u64)?; Ok(bytes.clone()) },
            None => read_bounded(&self.path, self.kind),
        }
    }

    pub fn into_content(mut self) -> Result<Vec<u8>, String> {
        match self.bytes.take() {
            Some(bytes) => { self.kind.check_size(bytes.len() as u64)?; Ok(bytes) },
            None => read_bounded(&self.path, self.kind),
        }
    }

    /// The file's content as text. The one read that works everywhere: native and Android reopen
    /// the path, the browser already has the bytes.
    pub fn text(&self) -> Result<String, String> {
        String::from_utf8(self.content()?).map_err(|e| e.to_string())
    }

    /// The name to show, and on the web the only handle there is.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

fn read_bounded(path: &std::path::Path, kind: ImportKind) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() { return Err("Choose a regular file".into()); }
    kind.check_size(metadata.len())?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() { return Err("Choose a regular file".into()); }
    kind.check_size(metadata.len())?;
    let mut bytes = Vec::new();
    file.take(kind.max_bytes() as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    kind.check_size(bytes.len() as u64)?;
    Ok(bytes)
}

/// Ask the user for a file. Returns immediately on every platform; the answer arrives through
/// [`take_result`], which the app polls.
///
/// On desktop the dialog is modal and the answer is already there by the time this returns, which
/// is fine — the caller reads it a frame later either way, and one code path beats two.
pub fn request_open(kind: ImportKind, tag: impl Into<String>) {
    if IMPORTING.load(std::sync::atomic::Ordering::Acquire) {
        fail("An import is already running. Wait for it to finish.".into());
        return;
    }
    let tag = tag.into();
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(kind.label(), kind.extensions())
            .pick_file()
        {
            deliver(Import {
                kind,
                tag,
                path,
                bytes: None,
            });
        }
    }
    #[cfg(target_os = "android")]
    {
        if let Err(e) = android_open::launch(kind, &tag) {
            fail(format!("Could not open the file picker: {e}"));
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_open::pick(kind, tag);
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
    if begin_import() { deliver_reserved(import); }
}

fn deliver_reserved(import: Import) { *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(import); }

/// Storage Access Framework, through the activity.
///
/// The picker is an activity result, so it lands in Kotlin, not here — and the app may not even
/// be resumed when it does. `MainActivity` copies the chosen content stream into app storage and
/// writes `kind<TAB>tag<TAB>path` to `filesDir/import.txt`, exactly the handover `goto.txt` already uses
/// for notification taps; this drains that file on the next frame.
#[cfg(target_os = "android")]
mod android_open {
    use super::{deliver, Import, ImportKind};
    use jni::objects::{JObject, JValue};

    pub fn offer_export(path: &std::path::Path) -> Result<(), String> {
        let app = crate::platform::android::app().ok_or("Android activity unavailable")?;
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }
            .map_err(|error| error.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|error| error.to_string())?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let path = env.new_string(path.to_string_lossy()).map_err(|error| error.to_string())?;
        let result = env.call_method(&activity, "offerExport", "(Ljava/lang/String;)V", &[JValue::Object(&path)]);
        if result.is_err() { let _ = env.exception_clear(); }
        result.map(|_| ()).map_err(|error| error.to_string())
    }

    fn slug(kind: ImportKind) -> &'static str {
        match kind {
            ImportKind::SettingsBundle => "settings",
            ImportKind::Palette => "palette",
            ImportKind::MarkerIcon => "marker",
            ImportKind::AlertSound => "sound",
            ImportKind::ChaseGpx => "gpx",
            ImportKind::Gis => "gis",
            ImportKind::CaseManifest => "case",
        }
    }

    fn from_slug(s: &str) -> Option<ImportKind> {
        Some(match s {
            "settings" => ImportKind::SettingsBundle,
            "palette" => ImportKind::Palette,
            "marker" => ImportKind::MarkerIcon,
            "sound" => ImportKind::AlertSound,
            "gpx" => ImportKind::ChaseGpx,
            "gis" => ImportKind::Gis,
            "case" => ImportKind::CaseManifest,
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
                bytes: None,
            }),
            None => log::warn!("import.txt names an unknown kind '{kind}'"),
        }
    }
}

/// The browser's own file picker: a hidden `<input type="file">`, clicked from here.
///
/// It has to be an element in the document — a detached input's `click()` is ignored by every
/// browser's popup blocker unless it is reachable — and it has to survive the call, because the
/// `change` event fires long after `pick` returns. So it is appended, hidden, and removes itself
/// once it has answered.
#[cfg(target_arch = "wasm32")]
mod web_open {
    use super::{begin_import, deliver_reserved, fail, finish_import, Import, ImportKind};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    pub fn pick(kind: ImportKind, tag: String) {
        if let Err(e) = try_pick(kind, tag) {
            fail(format!("File picker failed: {e:?}"));
        }
    }

    fn try_pick(kind: ImportKind, tag: String) -> Result<(), JsValue> {
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let input: web_sys::HtmlInputElement = doc.create_element("input")?.unchecked_into();
        input.set_type("file");
        // Extensions, not MIME: a `.pal` has no registered type, and every browser accepts a
        // dotted-extension accept list.
        let accept: Vec<String> = kind.extensions().iter().map(|e| format!(".{e}")).collect();
        input.set_accept(&accept.join(","));
        input.style().set_property("display", "none")?;
        doc.body()
            .ok_or_else(|| JsValue::from_str("no body"))?
            .append_child(&input)?;

        let el = input.clone();
        let on_change = Closure::once(Box::new(move || {
            let file = el.files().and_then(|f| f.get(0));
            el.remove();
            let Some(file) = file else { return };
            let name = file.name();
            if let Err(error) = kind.check_size(file.size() as u64) { fail(error); return; }
            if !begin_import() { return; }
            wasm_bindgen_futures::spawn_local(async move {
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        deliver_reserved(Import {
                            kind,
                            tag,
                            path: std::path::PathBuf::from(&name),
                            bytes: Some(bytes),
                        });
                    }
                    Err(e) => { fail(format!("Could not read {name}: {e:?}")); finish_import(); },
                }
            });
        }) as Box<dyn FnOnce()>);
        input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        // The closure outlives this function by design — it fires when the user picks, which may
        // be a minute from now. `forget` is the honest way to say "the DOM owns this".
        on_change.forget();
        input.click();
        Ok(())
    }
}

/// Saving in a browser: a Blob, an object URL, and a synthetic click on a download link. There is
/// no dialog to cancel — the browser either takes it or the user's download settings intervene.
#[cfg(target_arch = "wasm32")]
mod web_save {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    pub fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
        inner(name, bytes).map_err(|e| format!("{e:?}"))
    }

    fn inner(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| JsValue::from_str("no document"))?;
        // `Uint8Array::from` copies into the JS heap, which the Blob then owns — the Rust slice is
        // free to go the moment this returns.
        let array = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let blob = web_sys::Blob::new_with_u8_array_sequence(&array)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let a: web_sys::HtmlAnchorElement = doc.create_element("a")?.unchecked_into();
        a.set_href(&url);
        a.set_download(name);
        a.click();
        // A large GIF is generated after the originating click and the browser may not take the
        // Blob until a later task. Revoking here can silently cancel the download while `click()`
        // still returns success. Release it after a bounded handoff window instead.
        let revoke = Closure::once_into_js(move || {
            let _ = web_sys::Url::revoke_object_url(&url);
        });
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("no window"))?
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                revoke.unchecked_ref(),
                60_000,
            )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_limits_apply_before_native_read_and_to_browser_bytes() {
        let path = std::env::temp_dir().join(format!("hookecho-import-limit-{}", std::process::id()));
        let kinds = [ImportKind::SettingsBundle, ImportKind::Palette, ImportKind::MarkerIcon,
            ImportKind::AlertSound, ImportKind::ChaseGpx, ImportKind::Gis, ImportKind::CaseManifest];
        let file = std::fs::File::create(&path).unwrap();
        for kind in kinds {
            assert!(kind.check_size(kind.max_bytes() as u64).is_ok());
            file.set_len(kind.max_bytes() as u64 + 1).unwrap();
            let import = Import { kind, tag: String::new(), path: path.clone(), bytes: None };
            assert!(import.content().unwrap_err().contains("limit"));
        }
        drop(file);
        std::fs::remove_file(path).unwrap();
        let import = Import { kind: ImportKind::Palette, tag: String::new(), path: "table.pal".into(),
            bytes: Some(vec![0; ImportKind::Palette.max_bytes() + 1]) };
        assert!(import.into_content().unwrap_err().contains("limit"));
        assert!(read_bounded(&std::env::temp_dir(), ImportKind::Gis).unwrap_err().contains("regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_rejected_before_opening_or_reading() {
        let path = std::env::temp_dir().join(format!("hookecho-import-fifo-{}", std::process::id()));
        assert!(std::process::Command::new("mkfifo").arg(&path).status().unwrap().success());
        let result = read_bounded(&path, ImportKind::Gis);
        std::fs::remove_file(path).unwrap();
        assert!(result.unwrap_err().contains("regular file"));
    }

    #[test]
    fn one_import_keeps_its_result_until_applied_and_then_allows_retry() {
        finish_import();
        assert!(begin_import());
        deliver_reserved(Import { kind: ImportKind::Gis, tag: String::new(), path: "first.json".into(), bytes: Some(vec![]) });
        assert!(!begin_import());
        assert!(take_error().unwrap().contains("already running"));
        assert_eq!(take_result().unwrap().name(), "first.json");
        assert!(!begin_import(), "taking the source must not unlock preparation");
        finish_import();
        assert!(begin_import());
        finish_import();
        let _ = take_error();
    }
}
