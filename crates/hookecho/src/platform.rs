//! Platform glue that varies at runtime. Desktop: no-ops. Android: `android_main` stashes the
//! `AndroidApp` handle here so every frame can feed the system-bar insets into egui's safe-area
//! input — egui-winit 0.35 only wires that up for iOS, and the NativeActivity surface extends
//! under the status bar and gesture bar. With the insets fed, egui's root Ui, panels, and
//! windows avoid the system chrome natively.

/// Foreground gating for background work.
///
/// eframe stops calling `update()` once Android tears the surface down, so anything on its own
/// thread or on the tokio pool — the volume poller, the live chunk stream, the GPS loop — keeps
/// burning battery and mobile data behind a screen nobody is looking at. Rather than plumbing
/// winit lifecycle events into every one of them, the UI stamps each frame here and the workers
/// ask whether a frame happened recently and the window still has focus.
///
/// Off-Android this is always "active": desktop background windows are cheap and users expect a
/// tiled radar pane to keep updating.
pub mod activity {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::OnceLock;
    use std::time::Instant;

    /// A frame older than this means the surface is gone, whatever the last focus event said.
    const STALE_MS: u64 = 3_000;

    static START: OnceLock<Instant> = OnceLock::new();
    static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);
    static FOCUSED: AtomicBool = AtomicBool::new(true);

    fn now_ms() -> u64 {
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    /// Called once per frame from the UI. Returns true when this frame follows a gap — the
    /// caller uses that to kick one refresh so a resumed app isn't showing stale radar.
    pub fn mark_frame(focused: bool) -> bool {
        let last = LAST_FRAME_MS.swap(now_ms(), Ordering::Relaxed);
        let was_active = FOCUSED.swap(focused, Ordering::Relaxed);
        focused && (!was_active || now_ms().saturating_sub(last) > STALE_MS)
    }

    /// Whether background workers should do anything right now.
    pub fn is_active() -> bool {
        if !cfg!(target_os = "android") {
            return true;
        }
        FOCUSED.load(Ordering::Relaxed)
            && now_ms().saturating_sub(LAST_FRAME_MS.load(Ordering::Relaxed)) <= STALE_MS
    }
}

/// Feed system-bar insets into egui's safe-area input (no-op off-Android).
#[cfg(not(target_os = "android"))]
pub fn apply_safe_area(_ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}

/// Show/hide the soft keyboard (no-op off-Android — hardware keyboards just work).
#[cfg(not(target_os = "android"))]
pub fn show_soft_input(_show: bool) {}

/// Read the system clipboard (Android JNI; desktop text fields already paste natively).
#[cfg(not(target_os = "android"))]
pub fn clipboard_text() -> Option<String> {
    None
}

/// Start streaming the device's location (Android only; desktop uses gpsd — see `gps.rs`).
/// Returns `None` off-Android so the caller falls back to the gpsd path.
#[cfg(not(target_os = "android"))]
pub fn start_location() -> Option<std::sync::mpsc::Receiver<(f64, f64)>> {
    None
}

/// Speak `text` through the platform voice (Android only; desktop shells out — see `speech.rs`).
#[cfg(not(target_os = "android"))]
pub fn speak(_text: &str) -> Result<(), String> {
    Err("not android".into())
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::OnceLock;
    use winit::platform::android::activity::AndroidApp;

    static APP: OnceLock<AndroidApp> = OnceLock::new();

    /// Stash the activity handle (called once from `android_main` before the event loop).
    pub fn set_app(app: AndroidApp) {
        let _ = APP.set(app);
    }

    /// The stashed activity handle (for the sibling IME/clipboard module).
    pub(super) fn app() -> Option<&'static AndroidApp> {
        APP.get()
    }

    /// Convert the activity's content rect (pixels, relative to the full window) into egui
    /// safe-area insets (points). On gesture-nav phones the system bars are transparent
    /// overlays, so the content rect legitimately reports full-screen — floor the top/bottom at
    /// the standard status-bar / gesture-bar heights so the UI clears them anyway.
    /// `// ponytail: real per-device insets need a JNI WindowInsets query — floors cover v1.`
    pub fn apply_safe_area(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let Some(app) = APP.get() else { return };
        let rect = app.content_rect();
        let ppp = ctx.pixels_per_point();
        let (mut left, mut right, mut top, mut bottom) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        if rect.bottom > rect.top {
            if let Some(win) = app.native_window() {
                let (w, h) = (win.width() as f32, win.height() as f32);
                left = (rect.left as f32 / ppp).max(0.0);
                right = ((w - rect.right as f32) / ppp).max(0.0);
                top = (rect.top as f32 / ppp).max(0.0);
                bottom = ((h - rect.bottom as f32) / ppp).max(0.0);
            }
        }
        raw_input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
            left,
            right,
            top: top.max(28.0),
            bottom: bottom.max(20.0),
        }));
    }
}

#[cfg(target_os = "android")]
mod android_ime {
    use jni::objects::{JObject, JString};
    use jni::JNIEnv;

    /// Ask Android for the soft keyboard. `android-activity` does the JNI for this one.
    pub fn show_soft_input(show: bool) {
        let Some(app) = super::android::app() else {
            return;
        };
        if show {
            app.show_soft_input(true);
        } else {
            app.hide_soft_input(false);
        }
    }

    /// Read the system clipboard as text: `ClipboardManager.getPrimaryClip()` →
    /// `getItemAt(0).coerceToText(activity)`. Any JNI failure (thrown exception, empty clip)
    /// clears the exception and returns `None`.
    pub fn clipboard_text() -> Option<String> {
        let app = super::android::app()?;
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        match read_clip(&mut env, &activity) {
            Ok(text) => text,
            Err(e) => {
                log::warn!("clipboard read failed: {e:?}");
                let _ = env.exception_clear();
                None
            }
        }
    }

    fn read_clip(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<Option<String>> {
        let service = env.new_string("clipboard")?;
        let cm = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[(&service).into()],
            )?
            .l()?;
        if cm.is_null() {
            return Ok(None);
        }
        let clip = env
            .call_method(&cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?
            .l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?;
        if count == 0 {
            return Ok(None);
        }
        let item = env
            .call_method(
                &clip,
                "getItemAt",
                "(I)Landroid/content/ClipData$Item;",
                &[0i32.into()],
            )?
            .l()?;
        let text = env
            .call_method(
                &item,
                "coerceToText",
                "(Landroid/content/Context;)Ljava/lang/CharSequence;",
                &[activity.into()],
            )?
            .l()?;
        if text.is_null() {
            return Ok(None);
        }
        let s = env
            .call_method(&text, "toString", "()Ljava/lang/String;", &[])?
            .l()?;
        let out: String = env.get_string(&JString::from(s))?.into();
        Ok((!out.is_empty()).then_some(out))
    }
}

#[cfg(target_os = "android")]
mod android_location {
    use jni::objects::JObject;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::Duration;

    /// Request the fine-location permission if it isn't already granted, then poll the system's
    /// last known fix and stream changes to the caller.
    ///
    /// Polling `getLastKnownLocation` on a plain thread instead of registering a
    /// `LocationListener` is deliberate: a listener needs a `Looper`-backed thread and a Java
    /// callback object, and a NativeActivity has neither to spare. The cost is that a fix can be
    /// a little stale — irrelevant at chase cadence, where the camera moves every couple of minutes.
    ///
    /// A NativeActivity also can't observe the permission dialog's result, so the poll re-checks
    /// the grant each pass and starts reporting once the user says yes. Until then (and if they
    /// say no) the tap-to-set-position path keeps working.
    pub fn start_location() -> Option<Receiver<(f64, f64)>> {
        super::android::app()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || poll_loop(tx));
        Some(rx)
    }

    fn poll_loop(tx: Sender<(f64, f64)>) {
        let mut last: Option<(f64, f64)> = None;
        let mut asked = false;
        loop {
            if !super::activity::is_active() {
                // Backgrounded: skip both JNI round trips, keep the thread parked.
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
            match read_fix(&mut asked) {
                Ok(Some(pos)) => {
                    // Only send real movement (~1 m) so the chase handoff isn't re-run every poll.
                    let moved = last.is_none_or(|(lo, la): (f64, f64)| {
                        (lo - pos.0).abs() > 1e-5 || (la - pos.1).abs() > 1e-5
                    });
                    if moved {
                        last = Some(pos);
                        if tx.send(pos).is_err() {
                            return; // app dropped the receiver
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => log::warn!("location poll failed: {e:?}"),
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    /// One permission check + `getLastKnownLocation("gps")` round trip.
    fn read_fix(asked: &mut bool) -> jni::errors::Result<Option<(f64, f64)>> {
        let Some(app) = super::android::app() else {
            return Ok(None);
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };

        let perm = env.new_string("android.permission.ACCESS_FINE_LOCATION")?;
        let granted = env
            .call_method(
                &activity,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[(&perm).into()],
            )?
            .i()?;
        if granted != 0 {
            // PackageManager.PERMISSION_GRANTED == 0; anything else means we must ask (once).
            if !*asked {
                *asked = true;
                let arr = env.new_object_array(1, "java/lang/String", &perm)?;
                env.call_method(
                    &activity,
                    "requestPermissions",
                    "([Ljava/lang/String;I)V",
                    &[(&arr).into(), 1i32.into()],
                )?;
            }
            let _ = env.exception_clear();
            return Ok(None);
        }

        let service = env.new_string("location")?;
        let lm = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[(&service).into()],
            )?
            .l()?;
        if lm.is_null() {
            return Ok(None);
        }
        let provider = env.new_string("gps")?;
        let loc = env
            .call_method(
                &lm,
                "getLastKnownLocation",
                "(Ljava/lang/String;)Landroid/location/Location;",
                &[(&provider).into()],
            )?
            .l()?;
        if loc.is_null() {
            return Ok(None); // no fix yet (cold start / indoors)
        }
        let lat = env.call_method(&loc, "getLatitude", "()D", &[])?.d()?;
        let lon = env.call_method(&loc, "getLongitude", "()D", &[])?.d()?;
        Ok(Some((lon, lat)))
    }
}

/// Android speech synthesis (`TextToSpeech`), for spoken warnings.
#[cfg(target_os = "android")]
mod android_tts {
    use jni::objects::{JObject, JValue};
    use std::sync::OnceLock;
    use std::time::Duration;

    /// The `TextToSpeech` instance, kept alive for the process. Building one per utterance would
    /// re-run engine init (~1 s) every time and leak service connections.
    static TTS: OnceLock<jni::objects::GlobalRef> = OnceLock::new();

    /// Android TTS through raw JNI rather than a Kotlin helper: the APK is a pure NativeActivity
    /// with `hasCode="false"`, and adding a Java/Kotlin source set to carry one class would mean
    /// the Kotlin gradle plugin, a stdlib dependency, and flipping `hasCode` — a lot of build
    /// surface for one method call.
    ///
    /// The cost of skipping Kotlin is the `OnInitListener`: implementing a Java interface from JNI
    /// needs a runtime proxy, so we pass `null` (AOSP null-checks it before dispatch) and instead
    /// poll `speak` until the engine stops returning ERROR. Init takes well under a second in
    /// practice; the retry window is generous because a dropped tornado warning is the bad
    /// outcome, not a slow one.
    pub fn speak(text: &str) -> Result<(), String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        loop {
            match try_speak(text) {
                Ok(true) => return Ok(()),
                Ok(false) => {
                    if std::time::Instant::now() >= deadline {
                        return Err("TTS engine never became ready".into());
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(e) => return Err(format!("{e:?}")),
            }
        }
    }

    /// One `speak()` attempt. `Ok(false)` means the engine isn't ready yet (retry).
    fn try_speak(text: &str) -> jni::errors::Result<bool> {
        let Some(app) = super::android::app() else {
            return Ok(false);
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };

        if TTS.get().is_none() {
            let class = env.find_class("android/speech/tts/TextToSpeech")?;
            let obj = env.new_object(
                &class,
                "(Landroid/content/Context;Landroid/speech/tts/TextToSpeech$OnInitListener;)V",
                &[JValue::Object(&activity), JValue::Object(&JObject::null())],
            )?;
            let global = env.new_global_ref(&obj)?;
            let _ = TTS.set(global);
        }
        let tts = TTS.get().expect("just set");

        let msg = env.new_string(text)?;
        let id = env.new_string("hookecho")?;
        // QUEUE_ADD = 1: warnings stack rather than cutting each other off.
        let res = env.call_method(
            tts.as_obj(),
            "speak",
            "(Ljava/lang/CharSequence;ILandroid/os/Bundle;Ljava/lang/String;)I",
            &[
                JValue::Object(&msg),
                JValue::Int(1),
                JValue::Object(&JObject::null()),
                JValue::Object(&id),
            ],
        );
        match res {
            // SUCCESS = 0, ERROR = -1 (engine not bound yet).
            Ok(v) => Ok(v.i()? == 0),
            Err(e) => {
                let _ = env.exception_clear();
                Err(e)
            }
        }
    }
}

#[cfg(target_os = "android")]
pub use android::{apply_safe_area, set_app};
#[cfg(target_os = "android")]
pub use android_ime::{clipboard_text, show_soft_input};
#[cfg(target_os = "android")]
pub use android_location::start_location;
#[cfg(target_os = "android")]
pub use android_tts::speak;
