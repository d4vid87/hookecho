//! Where CPU work runs, per target.
//!
//! Decoding a volume or assembling live chunks is tens of MB of pure CPU. On native that has to
//! come off the async worker or it stalls every other fetch sharing that thread. On wasm there is
//! no tokio and no thread to move to, so it runs where it stands — the web build is single
//! threaded either way.

/// Run `f` off the async runtime and await its result.
#[cfg(not(target_arch = "wasm32"))]
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> anyhow::Result<T> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// wasm: no threads, so this is a plain call. Never fails, but keeps the native signature.
#[cfg(target_arch = "wasm32")]
pub async fn blocking<T>(f: impl FnOnce() -> T) -> anyhow::Result<T> {
    Ok(f())
}

/// Run `f` on the current thread, telling the runtime it is about to block.
///
/// Requires the multi-threaded runtime, which is what the app and the headless harness both build.
#[cfg(not(target_arch = "wasm32"))]
pub fn in_place<T>(f: impl FnOnce() -> T) -> T {
    tokio::task::block_in_place(f)
}

/// wasm: nothing to tell.
#[cfg(target_arch = "wasm32")]
pub fn in_place<T>(f: impl FnOnce() -> T) -> T {
    f()
}

/// Yield for `d`.
#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(d: std::time::Duration) {
    tokio::time::sleep(d).await;
}

/// wasm: the page's `setTimeout`, which is the only timer there is.
#[cfg(target_arch = "wasm32")]
pub async fn sleep(d: std::time::Duration) {
    gloo_timers::future::TimeoutFuture::new(d.as_millis() as u32).await;
}
