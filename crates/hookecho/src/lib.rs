//! Hook Echo-WX library crate: the app shell, render pipelines, and platform glue shared by the
//! desktop binary (`main.rs`) and the Android `cdylib` (`android_main`, below).
//!
//! The same `HookEchoApp` drives both; only the launch path differs — desktop builds a windowed
//! `eframe::NativeOptions`, Android hands eframe the `AndroidApp` from the activity glue and points
//! [`paths`] at the app-private data dir.

pub mod app;
pub mod audio;
pub mod basemap_style;
pub mod colormap;
pub mod dialog;
pub mod digest;
pub mod events;
pub mod geo;
pub mod gps;
pub mod headless;
pub mod hotkeys;
pub mod icon;
pub mod loopexport;
pub mod overlay_build;
pub mod paths;
pub mod render;
pub mod render3d;
pub mod settings;
pub mod theme;
pub mod tiles;
pub mod timeline;
pub mod tray;
pub mod ui;
pub mod vector_tiles;
pub mod view;

pub use app::HookEchoApp;

/// Launch the windowed desktop app (Windows/Linux/macOS). Called from `main.rs` after it has
/// dispatched any `--headless-*` verifier.
#[cfg(not(target_os = "android"))]
pub fn run_desktop() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Hook Echo-WX")
            .with_icon(icon::icon_data()),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Hook Echo-WX",
        native_options,
        Box::new(|cc| Ok(Box::new(HookEchoApp::new(cc)))),
    )
}

/// Android entry point. The `android-native-activity` glue (via winit) calls this with the
/// `AndroidApp` handle; we route storage to the app-private dir, wire logs to logcat, and hand the
/// handle to eframe. `#[no_mangle]` so the generated NativeActivity glue can find it by symbol.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    // Rust panics otherwise die silently on Android (stderr goes nowhere) — route them to logcat.
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));

    // Settings, caches, and exports live under the activity's private internal data dir.
    if let Some(path) = app.internal_data_path() {
        paths::set_base(path);
    }

    let native_options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        "Hook Echo-WX",
        native_options,
        Box::new(|cc| Ok(Box::new(HookEchoApp::new(cc)))),
    ) {
        log::error!("eframe exited: {e}");
    }
}
