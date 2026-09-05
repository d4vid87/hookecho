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
/// Desktop is always "active": background windows are cheap and users expect a tiled radar pane
/// to keep updating. The browser is gated like Android — a hidden tab stops being animated, so
/// the same "has there been a frame lately" test catches it — because there the timers driving
/// the pollers keep running behind a page nobody can see.
pub mod activity {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::OnceLock;
    use wxdata::clock::Instant;

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
        let after_gap = now_ms().saturating_sub(last) > STALE_MS;
        // In a browser, focus is the wrong question: a visible tab behind another window is not
        // focused and should keep updating. The gap is the whole signal there.
        if cfg!(target_arch = "wasm32") {
            return after_gap;
        }
        focused && (!was_active || after_gap)
    }

    /// Whether background workers should do anything right now.
    pub fn is_active() -> bool {
        let fresh_frame =
            now_ms().saturating_sub(LAST_FRAME_MS.load(Ordering::Relaxed)) <= STALE_MS;
        // A hidden tab stops getting animation frames, so the same staleness rule Android uses
        // for a torn-down surface answers this for free — no `visibilitychange` listener, and
        // nothing that can disagree with what the renderer is actually doing. It matters because
        // the timers do *not* stop: a backgrounded tab kept polling volumes, streaming live
        // chunks and refreshing feeds behind a page nobody was looking at, and every one of those
        // holds memory a 32-bit heap never gives back.
        if cfg!(target_arch = "wasm32") {
            return fresh_frame;
        }
        // Desktop background windows are cheap, and a tiled radar pane is expected to keep
        // updating whether or not it has focus.
        if !cfg!(target_os = "android") {
            return true;
        }
        FOCUSED.load(Ordering::Relaxed) && fresh_frame
    }
}

/// Feed system-bar insets into egui's safe-area input (no-op off-Android).
#[cfg(not(target_os = "android"))]
pub fn apply_safe_area(_ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}

/// Show/hide the soft keyboard (no-op off-Android — hardware keyboards just work).
#[cfg(not(target_os = "android"))]
pub fn show_soft_input(_show: bool) {}

/// Translate the Android IME's editor state into egui input events (no-op off-Android).
#[cfg(not(target_os = "android"))]
pub fn pump_ime(_raw_input: &mut egui::RawInput) {}

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

/// Start or stop the Android background alert service (`AlertService.kt`). No-op elsewhere —
/// desktop already keeps watching because the window is still open.
pub fn set_background_alerts(_enabled: bool) {
    #[cfg(target_os = "android")]
    android_alerts::set_enabled(_enabled);
}

/// Tell the Android alert stack to poll on the relaxed schedule (or back on the normal one).
/// A no-op everywhere else: a desktop's background alert path is the open window.
pub fn set_battery_saver(_on: bool) {
    #[cfg(target_os = "android")]
    android_alerts::set_saver(_on);
}

/// Delivery health from the Android alert stack, parsed from `AlertAlarm.health`. `None`
/// everywhere else — on desktop the window being open *is* the delivery mechanism.
pub fn alert_health() -> Option<AlertHealth> {
    #[cfg(target_os = "android")]
    {
        android_alerts::health().and_then(|s| AlertHealth::parse(&s))
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// What the Android alert stack reports about itself.
#[derive(Debug, PartialEq)]
pub struct AlertHealth {
    /// The user's background-alerts switch.
    pub enabled: bool,
    /// Whether the OS will honour an exact alarm. False means polls arrive on Doze's schedule.
    pub exact: bool,
    /// Whether we are exempt from battery optimisation.
    pub exempt: bool,
    /// Since the last poll actually ran. `None` if it never has.
    pub since_poll: Option<std::time::Duration>,
    /// Until the next scheduled alarm. `None` if none is armed.
    pub until_next: Option<std::time::Duration>,
}

impl AlertHealth {
    /// `enabled \t exact \t exempt \t sincePollMs \t untilNextMs`, -1 for never/unknown.
    pub fn parse(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 5 {
            return None;
        }
        let ms = |s: &str| {
            s.parse::<i64>()
                .ok()
                .filter(|v| *v >= 0)
                .map(|v| std::time::Duration::from_millis(v as u64))
        };
        Some(Self {
            enabled: f[0] == "true",
            exact: f[1] == "true",
            exempt: f[2] == "true",
            since_poll: ms(f[3]),
            until_next: ms(f[4]),
        })
    }
}

/// Open the OS screen that grants exact alarms (Android 12+). No-op elsewhere.
pub fn request_exact_alarms() {
    #[cfg(target_os = "android")]
    android_alerts::call_void("requestExact");
}

/// Prompt for the battery-optimisation exemption. No-op elsewhere.
pub fn request_battery_exemption() {
    #[cfg(target_os = "android")]
    android_alerts::call_void("requestExemption");
}

/// Open `url` in the system browser. Used by the Google sign-in, which has to hand the user off
/// to a real browser and get them back. Desktop shells out to the platform opener; Android goes
/// through an `ACTION_VIEW` intent.
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        android_browser::open(url).map_err(|e| format!("could not open a browser: {e:?}"))
    }
    #[cfg(not(target_os = "android"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        let mut cmd = std::process::Command::new(opener);
        no_window(&mut cmd);
        cmd.arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open {opener}: {e}"))
    }
}

/// Hand `link` to the platform's own share sheet, if it has one. Returns whether it took it —
/// `false` means the caller should fall back to the clipboard.
///
/// Only the browser has one today, and only some of them: `navigator.share` is missing on desktop
/// Firefox and outside a secure context, so the presence check is the whole design here. It also
/// requires a user gesture, which is satisfied because this is only ever reached from a click.
pub fn share_link(title: &str, link: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{JsCast, JsValue};
        let Some(nav) = web_sys::window().map(|w| w.navigator()) else {
            return false;
        };
        let key = JsValue::from_str("share");
        let Ok(f) = js_sys::Reflect::get(&nav, &key) else {
            return false;
        };
        let Ok(f) = f.dyn_into::<js_sys::Function>() else {
            return false;
        };
        let data = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &data,
            &JsValue::from_str("title"),
            &JsValue::from_str(title),
        );
        let _ = js_sys::Reflect::set(&data, &JsValue::from_str("url"), &JsValue::from_str(link));
        match f.call1(&nav, &data) {
            // The promise rejects when the user dismisses the sheet, which is not our problem and
            // certainly not a reason to paste something into their clipboard after the fact.
            Ok(p) => {
                if let Ok(p) = p.dyn_into::<js_sys::Promise>() {
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = wasm_bindgen_futures::JsFuture::from(p).await;
                    });
                }
                true
            }
            Err(e) => {
                log::warn!("share sheet refused: {e:?}");
                false
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (title, link);
        false
    }
}

/// Keep a child process from flashing its own console window on Windows.
///
/// Every helper we shell out to (the PowerShell speech synthesizer, ffmpeg, the URL opener) is a
/// console program, so Windows hands each one a fresh black window for the half-second it lives.
/// `CREATE_NO_WINDOW` suppresses it; a no-op everywhere else.
pub fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Hold a Wi-Fi multicast lock for the process, so Android stops dropping the broadcast packets
/// position sharing listens for (it filters them out to save power unless a lock is held). No-op
/// elsewhere, and acquired once — the lock is released when the process dies.
pub fn hold_multicast_lock() {
    #[cfg(target_os = "android")]
    {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            if let Err(e) = android_net::multicast_lock() {
                log::warn!("multicast lock failed, LAN position sharing may not receive: {e:?}");
            }
        });
    }
}

/// Whether the active network bills by the byte (Android only; a desktop link is never metered
/// for our purposes). Cached for a minute — the answer changes when the user walks out of Wi-Fi
/// range, not between frames.
pub fn is_metered() -> bool {
    #[cfg(not(target_os = "android"))]
    {
        false
    }
    #[cfg(target_os = "android")]
    {
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
        let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, v)) = *c {
            if at.elapsed() < Duration::from_secs(60) {
                return v;
            }
        }
        let v = android_net::metered().unwrap_or(false);
        if c.map(|(_, prev)| prev) != Some(v) {
            log::info!("network metered: {v}");
        }
        *c = Some((Instant::now(), v));
        v
    }
}

#[cfg(target_os = "android")]
pub(crate) mod android {
    use std::sync::OnceLock;
    use winit::platform::android::activity::AndroidApp;

    static APP: OnceLock<AndroidApp> = OnceLock::new();

    /// Stash the activity handle (called once from `android_main` before the event loop).
    pub fn set_app(app: AndroidApp) {
        let _ = APP.set(app);
    }

    /// The stashed activity handle — for the sibling IME/clipboard module, and for `dialog`,
    /// which calls into the activity to open the system file picker.
    pub(crate) fn app() -> Option<&'static AndroidApp> {
        APP.get()
    }

    /// Feed egui the real window insets, in points.
    ///
    /// The source of truth is `decorView.getRootWindowInsets()` masked to system bars, the
    /// display cutout and the IME — that last one is what makes a focused text field rise above
    /// the keyboard instead of hiding under it, and it is why the phone UI no longer has to pin
    /// dialogs to the top of the screen.
    ///
    /// The content rect is the fallback, floored at the standard status-bar / gesture-bar heights
    /// (on gesture-nav phones the bars are transparent overlays, so the content rect honestly
    /// reports full-screen and the floors are all that keeps chrome off them).
    pub fn apply_safe_area(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let Some(app) = APP.get() else { return };
        let ppp = ctx.pixels_per_point();
        if let Some(i) = super::android_insets::window_insets() {
            raw_input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
                left: i[0] as f32 / ppp,
                right: i[2] as f32 / ppp,
                top: i[1] as f32 / ppp,
                bottom: i[3] as f32 / ppp,
            }));
            return;
        }
        let rect = app.content_rect();
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
    ///
    /// Showing also resets the GameTextInput buffer and our mirror of it, so the first keystroke
    /// into a newly focused field doesn't replay whatever the last field contained.
    pub fn show_soft_input(show: bool) {
        let Some(app) = super::android::app() else {
            return;
        };
        if show {
            reset_text_input(app);
            app.show_soft_input(true);
        } else {
            app.hide_soft_input(false);
            reset_text_input(app);
        }
    }

    fn reset_text_input(app: &winit::platform::android::activity::AndroidApp) {
        app.set_text_input_state(TextInputState {
            text: String::new(),
            selection: TextSpan { start: 0, end: 0 },
            compose_region: None,
        });
        *MIRROR.lock().unwrap() = String::new();
    }

    use std::sync::Mutex;
    use winit::platform::android::activity::input::{TextInputState, TextSpan};

    /// Our copy of what GameTextInput last reported.
    static MIRROR: Mutex<String> = Mutex::new(String::new());

    /// Turn GameTextInput's buffer into egui input events.
    ///
    /// GameActivity replaces NativeActivity's raw ASCII key events with a real IME — autocorrect,
    /// suggestions, composing regions, every language the phone has — but it reports the result as
    /// *editor state*, not keystrokes, and neither winit nor egui reads that state. So the bridge
    /// lives here: each frame, diff the IME's buffer against our mirror and synthesise the
    /// backspaces and text insertions that get egui's focused field to the same string.
    ///
    /// ponytail: the diff assumes edits land at the end of the buffer (common prefix, then delete
    /// the tail and retype it), which is exactly what typing, autocorrect and suggestion taps do
    /// in the single-line fields this app has. A caret moved into the middle of a long string
    /// retypes more than it strictly needs to — invisible unless the field is huge. Track the
    /// reported selection instead if a multi-line field ever shows up.
    pub fn pump_ime(raw_input: &mut egui::RawInput) {
        let Some(app) = super::android::app() else {
            return;
        };
        let text = app.text_input_state().text;
        let mut mirror = MIRROR.lock().unwrap();
        if text == *mirror {
            return;
        }
        let old: Vec<char> = mirror.chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old
            .iter()
            .zip(new.iter())
            .take_while(|(a, b)| a == b)
            .count();
        for _ in prefix..old.len() {
            raw_input.events.push(egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            raw_input.events.push(egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let inserted: String = new[prefix..].iter().collect();
        if !inserted.is_empty() {
            raw_input.events.push(egui::Event::Text(inserted));
        }
        *mirror = text;
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

/// The one JNI call into our own Kotlin: `AlertService.setEnabled(activity, enabled)`.
///
/// The class is loaded through the activity's own ClassLoader rather than `find_class`, because
/// this runs on a Rust thread attached to the VM, where the default loader is the system one and
/// knows nothing about the APK's classes.
#[cfg(target_os = "android")]
mod android_alerts {
    use jni::objects::{JClass, JObject, JString, JValue};

    pub(super) fn set_enabled(enabled: bool) {
        if let Err(e) = try_set(enabled) {
            log::warn!("background alerts toggle failed: {e:?}");
        }
    }

    /// Call a `(Context)V` static on `AlertAlarm` — the two permission prompts share this shape.
    pub(super) fn call_void(method: &str) {
        if let Err(e) = try_call_alarm(method) {
            log::warn!("AlertAlarm.{method} failed: {e:?}");
        }
    }

    fn try_call_alarm(method: &str) -> jni::errors::Result<()> {
        with_class("io.hookecho.HookEcho.AlertAlarm", |env, class, activity| {
            let res = env.call_static_method(
                class,
                method,
                "(Landroid/content/Context;)V",
                &[JValue::Object(activity)],
            );
            if res.is_err() {
                let _ = env.exception_clear();
            }
            res.map(|_| ())
        })
    }

    /// `AlertAlarm.health(Context)` as its raw tab-separated line.
    pub(super) fn health() -> Option<String> {
        let out = with_class("io.hookecho.HookEcho.AlertAlarm", |env, class, activity| {
            let s = env
                .call_static_method(
                    class,
                    "health",
                    "(Landroid/content/Context;)Ljava/lang/String;",
                    &[JValue::Object(activity)],
                )?
                .l()?;
            Ok(env.get_string(&JString::from(s))?.into())
        });
        match out {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("AlertAlarm.health failed: {e:?}");
                None
            }
        }
    }

    /// Load a class through the activity's loader and hand it to `f` with the activity.
    ///
    /// The app's own classes are not on the JNI thread's default loader — `FindClass` from a
    /// thread the JVM did not create only sees the system loader — so every call into our Kotlin
    /// goes through `getClassLoader().loadClass()`.
    fn with_class<T>(
        name: &str,
        f: impl FnOnce(&mut jni::JNIEnv, &JClass, &JObject) -> jni::errors::Result<T>,
    ) -> jni::errors::Result<T> {
        let Some(app) = super::android::app() else {
            return Err(jni::errors::Error::NullPtr("no android app"));
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let loader = env
            .call_method(
                &activity,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?;
        let jname = env.new_string(name)?;
        let class = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[(&jname).into()],
            )?
            .l()?;
        f(&mut env, &JClass::from(class), &activity)
    }

    pub(super) fn set_saver(on: bool) {
        if let Err(e) = try_set_saver(on) {
            log::warn!("battery saver toggle failed: {e:?}");
        }
    }

    fn try_set_saver(on: bool) -> jni::errors::Result<()> {
        with_class(
            "io.hookecho.HookEcho.AlertService",
            |env, class, activity| {
                let res = env.call_static_method(
                    class,
                    "setBatterySaver",
                    "(Landroid/content/Context;Z)V",
                    &[JValue::Object(activity), JValue::Bool(on as u8)],
                );
                if res.is_err() {
                    let _ = env.exception_clear();
                }
                res.map(|_| ())
            },
        )
    }

    fn try_set(enabled: bool) -> jni::errors::Result<()> {
        with_class(
            "io.hookecho.HookEcho.AlertService",
            |env, class, activity| {
                let res = env.call_static_method(
                    class,
                    "setEnabled",
                    "(Landroid/content/Context;Z)V",
                    &[JValue::Object(activity), JValue::Bool(enabled as u8)],
                );
                if res.is_err() {
                    let _ = env.exception_clear();
                }
                res.map(|_| ())
            },
        )
    }
}

/// Predictive back (Android 13+ gesture, mandatory from 16).
///
/// Two directions cross the JNI boundary here — the first Kotlin→Rust call in the app:
/// Rust pushes "I have something to dismiss" so the OS knows whether to animate its home-screen
/// preview, and Kotlin calls back into `nativeOnBack` when the user actually completes the
/// gesture while the app is consuming it.
#[cfg(target_os = "android")]
mod android_back {
    use jni::objects::{JObject, JValue};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Set by `nativeOnBack`, drained by the UI thread on its next frame. The frame that reads it
    /// runs the same `mobile_back` chain the old `BrowserBack` key event did.
    static PENDING: AtomicBool = AtomicBool::new(false);
    /// Last value pushed to Kotlin, so a steady state costs no JNI at all.
    static PUSHED: AtomicBool = AtomicBool::new(false);

    /// # Safety
    /// Called by the JVM with a valid env/object pair; touches only an atomic.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_zip_batman_hookecho_MainActivity_nativeOnBack(
        _env: jni::JNIEnv,
        _this: JObject,
    ) {
        PENDING.store(true, Ordering::Relaxed);
    }

    pub fn take_back_pressed() -> bool {
        PENDING.swap(false, Ordering::Relaxed)
    }

    /// Tell the activity whether the app will consume the next back gesture.
    pub fn set_back_consumed(consumed: bool) {
        if PUSHED.swap(consumed, Ordering::Relaxed) == consumed {
            return;
        }
        match push(consumed) {
            Ok(()) => log::debug!("predictive back: consumed={consumed}"),
            Err(e) => log::warn!("predictive back state push failed: {e:?}"),
        }
    }

    fn push(consumed: bool) -> jni::errors::Result<()> {
        let Some(app) = super::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let res = env.call_method(
            &activity,
            "setBackConsumed",
            "(Z)V",
            &[JValue::Bool(consumed as u8)],
        );
        if res.is_err() {
            let _ = env.exception_clear();
        }
        res.map(|_| ())
    }
}

/// The home-screen radar widget's one call into Java: "there is a new picture on disk".
#[cfg(target_os = "android")]
mod android_widget {
    use jni::objects::JObject;

    /// Tell the home-screen radar widget a new picture is on disk.
    pub fn refresh_radar_widget() {
        if let Err(e) = refresh() {
            log::warn!("radar widget refresh failed: {e:?}");
        }
    }

    fn refresh() -> jni::errors::Result<()> {
        use jni::objects::{JClass, JValue};
        let Some(app) = super::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        // The app's own ClassLoader, not `find_class`: a thread attached from Rust gets the
        // *system* loader, which has no app dex on its path, so `find_class` leaves a pending
        // ClassNotFoundException and the next JNI call aborts the runtime under -Xcheck:jni.
        // (Same reason `android_alerts::try_set` does this.)
        let loader = env
            .call_method(
                &activity,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?;
        let name = env.new_string("io.hookecho.HookEcho.RadarWidget")?;
        let class = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[(&name).into()],
            )?
            .l()?;
        let class = JClass::from(class);
        let res = env.call_static_method(
            &class,
            "refresh",
            "(Landroid/content/Context;)V",
            &[JValue::Object(&activity)],
        );
        if res.is_err() {
            let _ = env.exception_clear();
        }
        res.map(|_| ())
    }
}

/// Nudge the Android home-screen radar widget after a new snapshot (no-op elsewhere).
#[cfg(not(target_os = "android"))]
pub fn refresh_radar_widget() {}

/// Whether a back press/gesture is waiting to be handled (Android only).
#[cfg(not(target_os = "android"))]
pub fn take_back_pressed() -> bool {
    false
}

/// Tell Android whether the app will consume the next back gesture (no-op elsewhere).
#[cfg(not(target_os = "android"))]
pub fn set_back_consumed(_consumed: bool) {}

/// `WindowInsets` glue for [`android::apply_safe_area`].
///
/// Answers in *pixels* as `[left, top, right, bottom]`, or `None` when the query fails (API 29,
/// no attached window yet, a thrown exception) so the caller falls back to the content rect.
#[cfg(target_os = "android")]
mod android_insets {
    use jni::objects::{JObject, JValue};
    use jni::JNIEnv;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    /// Frames between refreshes.
    ///
    /// ponytail: polled, not pushed. Insets change on rotation, on a bar hiding, and when the
    /// keyboard animates — the first two are rare and the third is a ~250ms animation, so 1-in-6
    /// frames (~10Hz at 60fps) is imperceptible and costs a JNI round trip instead of an
    /// OnApplyWindowInsetsListener plumbed back through Kotlin. Add the listener if the keyboard
    /// animation ever visibly stutters the layout.
    const REFRESH_FRAMES: u64 = 6;

    static TICK: AtomicU64 = AtomicU64::new(0);
    static CACHE: Mutex<Option<[i32; 4]>> = Mutex::new(None);

    pub(super) fn window_insets() -> Option<[i32; 4]> {
        let n = TICK.fetch_add(1, Ordering::Relaxed);
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if n % REFRESH_FRAMES == 0 || cache.is_none() {
            if let Some(v) = read() {
                *cache = Some(v);
            }
        }
        *cache
    }

    fn read() -> Option<[i32; 4]> {
        let app = super::android::app()?;
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        match query(&mut env, &activity) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("window insets query failed, using content rect: {e:?}");
                let _ = env.exception_clear();
                None
            }
        }
    }

    /// `decorView.rootWindowInsets.getInsets(systemBars | displayCutout | ime)`.
    ///
    /// `getInsets` is API 30; on 29 the call throws `NoSuchMethodError`, which `read` turns into
    /// the content-rect fallback.
    fn query(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<Option<[i32; 4]>> {
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        let insets = env
            .call_method(
                &decor,
                "getRootWindowInsets",
                "()Landroid/view/WindowInsets;",
                &[],
            )?
            .l()?;
        if insets.is_null() {
            return Ok(None);
        }
        let types = env.find_class("android/view/WindowInsets$Type")?;
        let mut mask = 0i32;
        for m in ["systemBars", "displayCutout", "ime"] {
            mask |= env.call_static_method(&types, m, "()I", &[])?.i()?;
        }
        let got = env
            .call_method(
                &insets,
                "getInsets",
                "(I)Landroid/graphics/Insets;",
                &[JValue::Int(mask)],
            )?
            .l()?;
        if got.is_null() {
            return Ok(None);
        }
        let f = |env: &mut JNIEnv, name: &str| env.get_field(&got, name, "I").and_then(|v| v.i());
        Ok(Some([
            f(env, "left")?,
            f(env, "top")?,
            f(env, "right")?,
            f(env, "bottom")?,
        ]))
    }
}

/// ConnectivityManager glue for [`is_metered`].
#[cfg(target_os = "android")]
mod android_net {
    use jni::objects::JObject;
    use jni::JNIEnv;

    pub(super) fn metered() -> Option<bool> {
        let app = super::android::app()?;
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        match read_metered(&mut env, &activity) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("metered check failed: {e:?}");
                let _ = env.exception_clear();
                None
            }
        }
    }

    /// `ConnectivityManager.isActiveNetworkMetered()` — one call, and it already accounts for the
    /// user's "treat this Wi-Fi as metered" override, which capability flags alone do not.
    fn read_metered(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<Option<bool>> {
        let service = env.new_string("connectivity")?;
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
        Ok(Some(
            env.call_method(&cm, "isActiveNetworkMetered", "()Z", &[])?
                .z()?,
        ))
    }

    /// `WifiManager.createMulticastLock("hookecho").acquire()`. The lock object is deliberately
    /// leaked into a global ref: releasing it would put the packet filter back, and it must
    /// outlive this call for the whole process.
    pub(super) fn multicast_lock() -> jni::errors::Result<()> {
        let Some(app) = super::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let service = env.new_string("wifi")?;
        let wm = env
            .call_method(
                &activity,
                "getApplicationContext",
                "()Landroid/content/Context;",
                &[],
            )?
            .l()?;
        let wm = env
            .call_method(
                &wm,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[(&service).into()],
            )?
            .l()?;
        if wm.is_null() {
            return Ok(());
        }
        let tag = env.new_string("hookecho")?;
        let lock = env
            .call_method(
                &wm,
                "createMulticastLock",
                "(Ljava/lang/String;)Landroid/net/wifi/WifiManager$MulticastLock;",
                &[(&tag).into()],
            )?
            .l()?;
        env.call_method(&lock, "setReferenceCounted", "(Z)V", &[false.into()])?;
        env.call_method(&lock, "acquire", "()V", &[])?;
        std::mem::forget(env.new_global_ref(&lock)?);
        Ok(())
    }
}

/// `startActivity(new Intent(ACTION_VIEW, Uri.parse(url)))` — the Android half of [`open_url`].
#[cfg(target_os = "android")]
mod android_browser {
    use jni::objects::JObject;

    pub(super) fn open(url: &str) -> jni::errors::Result<()> {
        let Some(app) = super::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let url = env.new_string(url)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[(&url).into()],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[(&action).into(), (&uri).into()],
        )?;
        let res = env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[(&intent).into()],
        );
        if res.is_err() {
            let _ = env.exception_clear();
        }
        res.map(|_| ())
    }
}

/// A short, physical acknowledgement. What the phone does when the screen says nothing.
#[derive(Clone, Copy)]
pub enum Haptic {
    /// A detent: one scrubber frame, one pane. The lightest thing the OS offers.
    Tick,
    /// A press was recognised as a long press, before anything appears.
    Press,
    /// A warning fired. The one buzz that should be felt through a pocket.
    Alert,
}

/// Play `h` on the device. No-op off Android.
///
/// ponytail: no app-level "enable haptics" setting — `performHapticFeedback` already obeys the
/// system's own touch-feedback switch, and a second toggle that can disagree with it is a support
/// question, not a feature.
pub fn haptic(h: Haptic) {
    #[cfg(target_os = "android")]
    {
        // HapticFeedbackConstants: CLOCK_TICK, LONG_PRESS, CONFIRM.
        let effect = match h {
            Haptic::Tick => 4,
            Haptic::Press => 0,
            Haptic::Alert => 16,
        };
        let _ = android_haptics::play(effect);
    }
    #[cfg(not(target_os = "android"))]
    let _ = h;
}

/// `decorView.performHapticFeedback(effect)` — the Android half of [`haptic`].
#[cfg(target_os = "android")]
mod android_haptics {
    use jni::objects::{GlobalRef, JObject};
    use std::sync::OnceLock;

    /// The decor view, held for the life of the process. Resolving it costs four JNI calls, and a
    /// scrubber drag asks for a tick every frame.
    static VIEW: OnceLock<Option<GlobalRef>> = OnceLock::new();

    fn view() -> Option<&'static GlobalRef> {
        VIEW.get_or_init(|| resolve().ok().flatten()).as_ref()
    }

    fn resolve() -> jni::errors::Result<Option<GlobalRef>> {
        let Some(app) = super::android::app() else {
            return Ok(None);
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let window = env
            .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        Ok(Some(env.new_global_ref(&decor)?))
    }

    pub(super) fn play(effect: i32) -> jni::errors::Result<()> {
        let Some(view) = view() else {
            return Ok(());
        };
        let Some(app) = super::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let res = env.call_method(
            view.as_obj(),
            "performHapticFeedback",
            "(I)Z",
            &[effect.into()],
        );
        if res.is_err() {
            let _ = env.exception_clear();
        }
        res.map(|_| ())
    }
}

/// Hand a typeset text product to the Kotlin `TextViewActivity`, which puts it in a WebView.
///
/// The document travels as a string extra rather than a file: it is generated fresh each time, it
/// is inert (see [`crate::textview`]), and a `file://` extra would need a FileProvider and a grant
/// to say the same thing.
#[cfg(target_os = "android")]
pub fn open_textview(title: &str, html: &str) -> Result<(), String> {
    android_textview::open(title, html).map_err(|e| format!("could not open the reader: {e:?}"))
}

#[cfg(target_os = "android")]
mod android_textview {
    use jni::objects::JObject;

    pub(super) fn open(title: &str, html: &str) -> jni::errors::Result<()> {
        let Some(app) = super::android::app() else {
            return Ok(());
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let intent = env.new_object("android/content/Intent", "()V", &[])?;
        let class = env.new_string("io.hookecho.HookEcho.TextViewActivity")?;
        env.call_method(
            &intent,
            "setClassName",
            "(Landroid/content/Context;Ljava/lang/String;)Landroid/content/Intent;",
            &[(&activity).into(), (&class).into()],
        )?;
        for (key, value) in [("title", title), ("html", html)] {
            let key = env.new_string(key)?;
            let value = env.new_string(value)?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                &[(&key).into(), (&value).into()],
            )?;
        }
        let res = env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[(&intent).into()],
        );
        if res.is_err() {
            let _ = env.exception_clear();
        }
        res.map(|_| ())
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
        // "gps" alone is null indoors and for the first minutes of a cold start. "fused" is the
        // one the platform actually keeps warm, and "network" is the last resort — take whichever
        // answers first.
        let mut loc = JObject::null();
        for name in ["fused", "gps", "network"] {
            let provider = env.new_string(name)?;
            let l = env
                .call_method(
                    &lm,
                    "getLastKnownLocation",
                    "(Ljava/lang/String;)Landroid/location/Location;",
                    &[(&provider).into()],
                )
                .map(|v| v.l());
            let _ = env.exception_clear(); // an unknown provider throws; try the next one
            if let Ok(Ok(l)) = l {
                if !l.is_null() {
                    loc = l;
                    break;
                }
            }
        }
        if loc.is_null() {
            return Ok(None); // no fix yet from any provider
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

    /// Android TTS through raw JNI rather than a Kotlin helper: this predates the alert service's
    /// Kotlin source set, and it works, so it stays raw. (A helper class would now be cheap — if
    /// this ever needs touching, that is the direction.)
    ///
    /// The cost of skipping Kotlin is the `OnInitListener`: implementing a Java interface from JNI
    /// needs a runtime proxy, so we pass `null` (AOSP null-checks it before dispatch) and instead
    /// poll `speak` until the engine stops returning ERROR. Init takes well under a second in
    /// practice; the retry window is generous because a dropped tornado warning is the bad
    /// outcome, not a slow one.
    pub fn speak(text: &str) -> Result<(), String> {
        let deadline = wxdata::clock::Instant::now() + Duration::from_secs(6);
        loop {
            match try_speak(text) {
                Ok(true) => {
                    // `speak` only queues. The cross-platform speech worker needs this call to
                    // finish at the utterance boundary so an emergency can run next.
                    std::thread::sleep(Duration::from_millis(20));
                    let speech_deadline = wxdata::clock::Instant::now() + Duration::from_secs(120);
                    while is_speaking().map_err(|e| format!("{e:?}"))? {
                        if wxdata::clock::Instant::now() >= speech_deadline {
                            return Err("TTS utterance did not finish".into());
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    return Ok(());
                }
                Ok(false) => {
                    if wxdata::clock::Instant::now() >= deadline {
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

    fn is_speaking() -> jni::errors::Result<bool> {
        let Some(app) = super::android::app() else {
            return Ok(false);
        };
        let Some(tts) = TTS.get() else {
            return Ok(false);
        };
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }?;
        let mut env = vm.attach_current_thread()?;
        env.call_method(tts.as_obj(), "isSpeaking", "()Z", &[])?.z()
    }
}

#[cfg(target_os = "android")]
pub use android::{apply_safe_area, set_app};
#[cfg(target_os = "android")]
pub use android_back::{set_back_consumed, take_back_pressed};
#[cfg(target_os = "android")]
pub use android_ime::{clipboard_text, pump_ime, show_soft_input};
#[cfg(target_os = "android")]
pub use android_location::start_location;
#[cfg(target_os = "android")]
pub use android_tts::speak;
#[cfg(target_os = "android")]
pub use android_widget::refresh_radar_widget;

/// Timeouts for an HTTP client, where the platform has them.
///
/// A hung request with no timeout holds its slot forever: whatever it was loading stays loading,
/// and a tile that never answers leaves that square of map permanently blank. reqwest's web
/// backend exposes neither knob — fetch owns the request there — so on wasm this is a no-op.
pub fn http_timeouts(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(not(target_arch = "wasm32"))]
    let b = b
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10));
    b
}

#[cfg(test)]
mod activity_tests {
    use super::activity;

    /// The gate answers three different questions on three platforms, and the browser's answer is
    /// new. This pins the desktop one, which must not change: a window that has not drawn for a
    /// while is still expected to keep polling, because on a desktop it is probably just behind
    /// another window.
    #[test]
    fn a_desktop_window_is_always_active() {
        assert!(activity::is_active());
        // Even long after the last frame — there is no frame stamp in a test at all.
        assert!(activity::is_active());
    }

    /// A resume is what re-arms the pollers, so it has to be reported exactly once per gap.
    #[test]
    fn the_first_frame_is_not_a_resume_but_a_later_gap_is() {
        // Two frames back to back: no gap between them, so nothing to catch up on.
        activity::mark_frame(true);
        assert!(!activity::mark_frame(true), "consecutive frames are not a resume");
    }
}

#[cfg(test)]
mod health_tests {
    use super::AlertHealth;

    #[test]
    fn health_line_parses_and_never_ran_is_none() {
        let h = AlertHealth::parse("true\tfalse\ttrue\t-1\t42000").unwrap();
        assert!(h.enabled && !h.exact && h.exempt);
        assert_eq!(h.since_poll, None);
        assert_eq!(h.until_next, Some(std::time::Duration::from_secs(42)));
        // A short line is a Kotlin-side change we haven't followed; report nothing, not garbage.
        assert_eq!(AlertHealth::parse("true\tfalse"), None);
    }
}
