//! The main thread's half of the Web Worker decode bridge.
//!
//! A browser tab runs the UI, the network and the radar decode on one thread, and a Level 2
//! volume is tens of MB of bzip2 — long enough that the map freezes for seconds while it lands.
//! `web/index.html` compiles the wasm module once, hands the same `WebAssembly.Module` to
//! `web/decode-worker.js`, and publishes
//! `globalThis.__decodeVolume(bytes, op) -> Promise<Uint8Array>`.
//! This calls it. The worker instantiates a second, throwaway heap, so the ~150 MB of decode
//! scratch never touches the heap the map is drawing from.
//!
//! Everything here is best-effort: no worker (old browser, `file://`, a trap that poisoned the
//! worker heap) means [`Error::Unavailable`], and the caller decodes inline as it always did. A
//! trap retires that worker instance, but the page builds a fresh one, so "inline" is normally
//! one job and not the rest of the session.

use wasm_bindgen::JsCast;

/// Why the worker could not be used. Never a decode failure — those come back as `Err(other)`.
#[derive(Debug)]
pub enum Error {
    /// No bridge on the page, or the worker died and index.html retired it.
    Unavailable,
    /// The worker ran and rejected: a genuinely bad volume.
    Decode(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unavailable => write!(f, "decode worker unavailable"),
            Error::Decode(e) => write!(f, "decode worker: {e}"),
        }
    }
}

/// Decode `bytes` in the worker, returning the postcard-encoded `Scan`.
pub async fn decode_volume(bytes: Vec<u8>) -> Result<Vec<u8>, Error> {
    call("decode", bytes).await
}

/// Assemble a framed live chunk window in the worker, returning the postcard-encoded partial
/// `Scan`. Same bridge, same fallback: [`Error::Unavailable`] means "assemble it inline".
pub async fn assemble_chunks(framed: Vec<u8>) -> Result<Vec<u8>, Error> {
    call("assemble", framed).await
}

/// Run one job on the worker. `op` names the export it should call.
async fn call(op: &str, bytes: Vec<u8>) -> Result<Vec<u8>, Error> {
    let global = js_sys::global();
    let f = js_sys::Reflect::get(&global, &"__decodeVolume".into())
        .ok()
        .and_then(|v| v.dyn_into::<js_sys::Function>().ok())
        .ok_or(Error::Unavailable)?;

    // A JS-heap copy, deliberately: the worker takes ownership of the buffer (it is transferred),
    // and transferring a view of wasm memory detaches the entire wasm heap out from under the app.
    let arg = js_sys::Uint8Array::from(bytes.as_slice());
    let promise: js_sys::Promise = f
        .call2(&global, &arg, &op.into())
        .map_err(|_| Error::Unavailable)?
        .dyn_into()
        .map_err(|_| Error::Unavailable)?;

    match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(v) => Ok(js_sys::Uint8Array::new(&v).to_vec()),
        Err(e) => {
            let msg = e
                .as_string()
                .or_else(|| {
                    js_sys::Reflect::get(&e, &"message".into())
                        .ok()?
                        .as_string()
                })
                .unwrap_or_else(|| "worker rejected".into());
            // The bridge rejects with this exact string once it has terminated a dead worker, so
            // the caller falls back inline instead of reporting a broken volume.
            if msg.contains("worker unavailable") {
                Err(Error::Unavailable)
            } else {
                Err(Error::Decode(msg))
            }
        }
    }
}
