//! Opt-in browser measurement. The harness sets `__hookechoMapTraceEnabled` before boot.
//! No data leaves the page. Camera coordinates are retained only in this bounded test buffer,
//! never included in the user-facing diagnostic report.

pub struct Frame {
    pub pane: usize,
    pub frame: u64,
    pub camera: crate::render::mercator::Camera,
    pub viewport: (f32, f32),
    pub scroll: f32,
    pub pinch: f32,
    pub moving: bool,
    pub tile_zoom: u8,
    pub vector_uploads: usize,
    pub raster_uploads: usize,
    pub labels: usize,
    pub prepare_ms: f32,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function mapTraceEnabled() { return globalThis.__hookechoMapTraceEnabled === true; }
export function mapTraceFinish(values) {
    if (!mapTraceEnabled()) return;
    const trace = globalThis.__hookechoMapTrace;
    if (!trace) return;
    const costs = trace.costs ??= [];
    if (costs.length >= 4096) { trace.dropped++; return; }
    costs.push([performance.now(), ...values]);
}
export function mapTraceRecord(values) {
    if (!mapTraceEnabled()) return;
    const trace = globalThis.__hookechoMapTrace ??= {version: 1, frames: [], dropped: 0};
    if (trace.frames.length >= 4096) { trace.dropped++; return; }
    trace.frames.push([performance.now(), ...values]);
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = mapTraceEnabled)]
    fn trace_enabled() -> bool;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = mapTraceRecord)]
    fn trace_record(values: &[f64]);
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = mapTraceFinish)]
    fn trace_finish(values: &[f64]);
}

pub fn enabled() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        trace_enabled()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

pub fn record(frame: Frame) {
    #[cfg(target_arch = "wasm32")]
    trace_record(&[
        frame.pane as f64,
        frame.frame as f64,
        frame.camera.zoom,
        frame.camera.center.0,
        frame.camera.center.1,
        frame.viewport.0 as f64,
        frame.viewport.1 as f64,
        frame.scroll as f64,
        frame.pinch as f64,
        frame.moving as u8 as f64,
        frame.tile_zoom as f64,
        frame.vector_uploads as f64,
        frame.raster_uploads as f64,
        frame.labels as f64,
        frame.prepare_ms as f64,
    ]);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = frame;
}

/// Full pane UI cost includes label layout and painter overlays; it excludes GPU completion.
pub fn finish(pane: usize, frame: u64, labels: usize, total_ms: f32) {
    #[cfg(target_arch = "wasm32")]
    trace_finish(&[pane as f64, frame as f64, labels as f64, total_ms as f64]);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (pane, frame, labels, total_ms);
}
