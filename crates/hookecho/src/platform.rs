//! Platform glue that varies at runtime. Desktop: no-ops. Android: `android_main` stashes the
//! `AndroidApp` handle here so every frame can feed the system-bar insets into egui's safe-area
//! input — egui-winit 0.35 only wires that up for iOS, and the NativeActivity surface extends
//! under the status bar and gesture bar. With the insets fed, egui's root Ui, panels, and
//! windows avoid the system chrome natively.

/// Feed system-bar insets into egui's safe-area input (no-op off-Android).
#[cfg(not(target_os = "android"))]
pub fn apply_safe_area(_ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}

#[cfg(target_os = "android")]
mod android {
    use std::sync::OnceLock;
    use winit::platform::android::activity::AndroidApp;

    static APP: OnceLock<AndroidApp> = OnceLock::new();

    /// Stash the activity handle (called once from `android_main` before the event loop).
    pub fn set_app(app: AndroidApp) {
        let _ = APP.set(app);
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
pub use android::{apply_safe_area, set_app};
