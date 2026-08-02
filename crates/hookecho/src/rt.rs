//! Where background work goes, on whichever platform this is.
//!
//! Natively that's a tokio runtime; in the browser there is no runtime to own — the page's event
//! loop is it, and a future is handed to `spawn_local`. Everything that fetches in this app
//! already returns its result over an `std::sync::mpsc` channel and asks egui to repaint, which
//! works identically either way, so this shim is the whole of the difference.

/// A handle to spawn work on, cheap to clone and hand to a subsystem.
#[derive(Clone)]
pub struct Spawner {
    #[cfg(not(target_arch = "wasm32"))]
    handle: tokio::runtime::Handle,
}

#[cfg(not(target_arch = "wasm32"))]
impl Spawner {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self { handle }
    }

    /// Run a future in the background. The result is dropped — callers report back over a channel.
    pub fn spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.handle.spawn(fut);
    }

    /// Run blocking work (file I/O, a long decode) off the UI thread.
    pub fn spawn_blocking<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.handle.spawn_blocking(f);
    }

    /// The underlying tokio handle, for the native-only subsystems that hold one themselves.
    pub fn handle(&self) -> &tokio::runtime::Handle {
        &self.handle
    }
}

#[cfg(target_arch = "wasm32")]
impl Spawner {
    pub fn new() -> Self {
        Self {}
    }

    pub fn spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + 'static,
    {
        wasm_bindgen_futures::spawn_local(fut);
    }

    /// There is no thread pool to move work to, so "blocking" work runs on the main thread.
    ///
    // ponytail: wasm spawn_blocking is spawn_local on the main thread; the ceiling is jank on big
    // CPU work, and the upgrade path is a web worker if it ever hurts.
    pub fn spawn_blocking<F>(&self, f: F)
    where
        F: FnOnce() + 'static,
    {
        wasm_bindgen_futures::spawn_local(async move { f() });
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for Spawner {
    fn default() -> Self {
        Self::new()
    }
}
