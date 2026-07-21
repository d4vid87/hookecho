//! Platform glue that varies at runtime. Desktop: no-ops. Android: `android_main` stashes the
//! `AndroidApp` handle here so every frame can feed the system-bar insets into egui's safe-area
//! input — egui-winit 0.35 only wires that up for iOS, and the NativeActivity surface extends
//! under the status bar and gesture bar. With the insets fed, egui's root Ui, panels, and
//! windows avoid the system chrome natively.

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
        let Some(app) = super::android::app() else { return };
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
        let vm =
            unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity =
            unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        match read_clip(&mut env, &activity) {
            Ok(text) => text,
            Err(_) => {
                let _ = env.exception_clear();
                None
            }
        }
    }

    fn read_clip(
        env: &mut JNIEnv,
        activity: &JObject,
    ) -> jni::errors::Result<Option<String>> {
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
pub use android::{apply_safe_area, set_app};
#[cfg(target_os = "android")]
pub use android_ime::{clipboard_text, show_soft_input};
