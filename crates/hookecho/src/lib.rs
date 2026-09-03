//! HookEcho library crate: the app shell, render pipelines, and platform glue shared by the
//! desktop binary (`main.rs`) and the Android `cdylib` (`android_main`, below).
//!
//! The same `HookEchoApp` drives both; only the launch path differs — desktop builds a windowed
//! `eframe::NativeOptions`, Android hands eframe the `AndroidApp` from the activity glue and points
//! [`paths`] at the app-private data dir.

pub mod alert_rollup;
pub mod alert_snapshot;
pub mod app;
pub mod astro;
pub mod audio;
pub mod backtest;
pub mod basemap_style;
/// Live camera video (desktop only — Android cannot spawn an ffmpeg child).
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub mod cam;
/// Dead reckoning ahead of the car, for the next radar handoff.
pub mod chase;
pub mod chaselog;
/// Caption, color bar and city labels blitted onto an off-screen render.
#[cfg(not(target_arch = "wasm32"))]
pub mod chrome;
pub mod cloud;
pub mod colormap;
#[cfg(not(target_arch = "wasm32"))]
pub mod crash;
pub mod dialog;
pub mod digest;
/// Terrain heights (DEM) and the beam-vs-terrain blockage raster.
pub mod elevation;
pub mod events;
pub mod fielddiff;
pub mod fonts;
pub mod fronts_draw;
pub mod geo;
pub mod gps;
/// Off-screen rendering for the CLI verifiers and the server snapshot.
#[cfg(not(target_arch = "wasm32"))]
pub mod headless;
pub mod hotkeys;
pub mod icon;
pub mod labelplace;
pub mod loopexport;
/// MQTT publishing for home automation; native only (no TCP socket in a browser).
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub mod mqtt;
pub mod notify;
#[cfg(not(target_arch = "wasm32"))]
pub mod nwr;
pub mod outage_draw;
pub mod overlay_build;
pub mod paths;
/// The perf counters' readout — native only, see the module docs.
#[cfg(not(target_arch = "wasm32"))]
pub mod perf;
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
/// User alert rules: which detections, where, are worth telling the user about.
pub mod rules;
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
/// Cache sizes and the buttons that clear them; needs a filesystem, so not on the web.
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;
pub mod textview;
pub mod theme;
pub mod tiles;
pub mod timefmt;
pub mod timeline;
pub mod tray;
pub mod tropical_draw;
pub mod ui;
pub mod vector_tiles;
pub mod view;
pub mod volume;
pub mod webcache;
pub mod wind_draw;
pub mod wind_gpu;
pub mod workspace;

pub use app::HookEchoApp;

/// Ask the device for no more texture than the adapter admits to having.
///
/// egui-wgpu's default device descriptor requests `max_texture_dimension_2d: 8192` unconditionally
/// so a depth buffer can cover a 4k display. On an adapter that reports less — Firefox on Linux
/// hands out a WebGL2 context capped at 2048 — `request_device` refuses the whole descriptor and
/// the app never starts (issue #17). Nothing here needs 8192: the one place the cap matters
/// (`app.rs`, field-grid decimation) reads it back off the created device.
///
/// Applies everywhere rather than on web only, because a desktop GLES fallback fails the same way.
fn cap_texture_limit_to_adapter(options: &mut egui_wgpu::WgpuConfiguration) {
    let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup else {
        return;
    };
    let base = std::sync::Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = std::sync::Arc::new(move |adapter| {
        let mut desc = base(adapter);
        let cap = adapter.limits().max_texture_dimension_2d;
        desc.required_limits.max_texture_dimension_2d =
            desc.required_limits.max_texture_dimension_2d.min(cap);
        desc
    });
}

/// Is the OS drawing the window frame?
///
/// Normally it isn't: the app is borderless and draws its own controls into the floating chrome
/// (`app::chrome::window_frame`). `--decorated` puts the OS frame back — a safety valve for a
/// window manager where the compositor-side drag or resize misbehaves, not a layout toggle. There
/// is no borderless anything on Android or the web, so there the answer is always yes.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub fn os_decorated() -> bool {
    static DECORATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DECORATED.get_or_init(|| std::env::args().any(|a| a == "--decorated"))
}

#[cfg(any(target_os = "android", target_arch = "wasm32"))]
pub fn os_decorated() -> bool {
    true
}

/// Launch the windowed desktop app (Windows/Linux/macOS). Called from `main.rs` after it has
/// dispatched any `--headless-*` verifier.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
pub fn run_desktop() -> eframe::Result<()> {
    let mut native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            // The floating chrome has fixed-width cards; below this they stack on top of the map
            // and each other.
            .with_min_inner_size([800.0, 500.0])
            .with_title("HookEcho")
            .with_decorations(os_decorated())
            // Matches the .desktop file, so Wayland taskbars find the icon.
            .with_app_id("hookecho")
            .with_icon(icon::icon_data()),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    cap_texture_limit_to_adapter(&mut native_options.wgpu_options);
    eframe::run_native(
        "HookEcho",
        native_options,
        Box::new(|cc| Ok(Box::new(HookEchoApp::new(cc)))),
    )
}

/// Decode one Level 2 volume, returning the `Scan` as postcard bytes.
///
/// This is the *worker's* entry point, not the page's: `web/decode-worker.js` instantiates a
/// second copy of this same module (the page hands it the already-compiled `WebAssembly.Module`)
/// purely to call this. `start` is never called there, so nothing else in the app wakes up —
/// the worker is a decode function with a private 150 MB heap.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn decode_archive2(bytes: Vec<u8>) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    wxdata::level2::decode_and_encode(bytes).map_err(|e| e.to_string().into())
}

/// Assemble a live chunk window, returning the partial `Scan` as postcard bytes.
///
/// The live stream's version of [`decode_archive2`], and the reason it exists: assembling the
/// accumulated chunks is the same bzip2-and-decode work, it happens at every sweep boundary
/// rather than once a volume, and on the page's thread that is a hitch every twenty seconds for
/// as long as the tab is open. `framed` is `wxdata::live::frame_chunks`' output.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn assemble_live_chunks(framed: Vec<u8>) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    wxdata::live::assemble_and_encode(&framed).map_err(|e| e.to_string().into())
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

    // nexrad-data builds its S3 URLs itself and fetches them directly, which is the one feed path
    // that never saw `net::fetch_url`. Point it at the same same-origin proxy as everything else:
    // archive volumes then get edge-cached, so a site every visitor opens is fetched from S3 once
    // an hour rather than once per visitor. (Live chunks stay direct — see `CORS_OK`.)
    wxdata::net::install_s3_proxy_rewriter();

    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(&canvas_id))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("no such canvas"))?;

    let mut web_options = eframe::WebOptions::default();
    cap_texture_limit_to_adapter(&mut web_options.wgpu_options);

    eframe::WebRunner::new()
        .start(
            canvas,
            web_options,
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
    // Settings, caches, and exports live under the activity's private internal data dir. This
    // comes before the panic hook, which writes its report in there.
    if let Some(path) = app.internal_data_path() {
        paths::set_base(path);
    }
    // Rust panics otherwise die silently on Android (stderr goes nowhere) — route them to logcat,
    // and leave a report the next start can show.
    crash::install_hook();
    // The UI queries this handle for system-bar insets each frame.
    platform::set_app(app.clone());

    let mut native_options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    cap_texture_limit_to_adapter(&mut native_options.wgpu_options);
    if let Err(e) = eframe::run_native(
        "HookEcho",
        native_options,
        Box::new(|cc| Ok(Box::new(HookEchoApp::new(cc)))),
    ) {
        log::error!("eframe exited: {e}");
    }
}
