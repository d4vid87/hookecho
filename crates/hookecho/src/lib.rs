//! Hook Echo-WX library crate: the app shell, render pipelines, and platform glue shared by the
//! desktop binary (`main.rs`) and the Android `cdylib` (`android_main`, below).
//!
//! The same `HookEchoApp` drives both; only the launch path differs — desktop builds a windowed
//! `eframe::NativeOptions`, Android hands eframe the `AndroidApp` from the activity glue and points
//! [`paths`] at the app-private data dir.

pub mod alert_snapshot;
pub mod app;
pub mod astro;
pub mod audio;
pub mod basemap_style;
/// Live camera video (desktop only — Android cannot spawn an ffmpeg child).
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub mod cam;
pub mod cloud;
pub mod colormap;
pub mod dialog;
pub mod digest;
/// Terrain heights (DEM) and the beam-vs-terrain blockage raster.
pub mod elevation;
pub mod events;
pub mod fronts_draw;
pub mod geo;
pub mod gps;
/// Off-screen rendering for the CLI verifiers and the server snapshot.
#[cfg(not(target_arch = "wasm32"))]
pub mod headless;
pub mod hotkeys;
pub mod icon;
pub mod loopexport;
pub mod notify;
#[cfg(not(target_arch = "wasm32"))]
pub mod nwr;
pub mod overlay_build;
pub mod paths;
pub mod platform;
/// External-process placefile plugins — a browser cannot spawn a process.
#[cfg(not(target_arch = "wasm32"))]
pub mod plugins;
pub mod products;
pub mod profiling;
pub mod rain_arrival;
pub mod render;
pub mod render3d;
/// Where background work goes: a tokio runtime natively, the page's event loop on the web.
pub mod rt;
/// The `--serve` HTTP endpoint (desktop only — Android has no headless mode to render from).
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub mod serve;
pub mod settings;
pub mod share;
pub mod speech;
/// Live station markers and their telemetry cards.
pub mod stationlayer;
/// The `--status` report; native only — it builds its own runtime.
#[cfg(not(target_arch = "wasm32"))]
pub mod status;
pub mod theme;
pub mod tiles;
pub mod timefmt;
pub mod timeline;
pub mod tray;
pub mod ui;
pub mod vector_tiles;
pub mod view;
pub mod workspace;
pub mod wind_draw;

pub use app::HookEchoApp;

/// Launch the windowed desktop app (Windows/Linux/macOS). Called from `main.rs` after it has
/// dispatched any `--headless-*` verifier.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub fn run_desktop() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            // The floating chrome has fixed-width cards; below this they stack on top of the map
            // and each other.
            .with_min_inner_size([800.0, 500.0])
            .with_title("Hook Echo-WX")
            // Matches the .desktop file, so Wayland taskbars find the icon.
            .with_app_id("hookecho")
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

/// Web entry point, called from `web/index.html` with the id of a `<canvas>`.
///
/// Same `HookEchoApp` as every other platform — eframe's `WebRunner` takes the identical creation
/// closure `run_native` does. What differs is what isn't there: no filesystem (so settings are
/// defaults and caches live in memory), no live chunk stream, and any feed whose host refuses
/// cross-origin requests simply doesn't load. The radar buckets and the NWS API do allow it,
/// which is what makes this worth shipping.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn start(canvas_id: String) -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast as _;
    console_error_panic_hook::set_once();
    // Browser console logging: the app logs through `log`, same as everywhere else.
    let _ = console_log::init_with_level(log::Level::Info);

    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(&canvas_id))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("no such canvas"))?;

    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(HookEchoApp::new(cc)))),
        )
        .await
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
    // The UI queries this handle for system-bar insets each frame.
    platform::set_app(app.clone());

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
