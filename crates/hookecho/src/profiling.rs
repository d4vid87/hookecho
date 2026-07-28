//! Frame profiling, compiled out unless the `profiling` cargo feature is on.
//!
//! Build with `cargo run --features profiling` (or `android/build.sh` with `FEATURES=profiling`),
//! then attach `puffin_viewer --url 127.0.0.1:8585`. On Android, `adb forward tcp:8585 tcp:8585`
//! first. A TCP server instead of an in-app flamegraph window keeps the profiler off the device's
//! own frame budget — the thing being measured — and avoids pinning an extra egui-versioned dep.

/// Wraps a block of work in a puffin scope. No-op (and argument-free) without the feature.
#[macro_export]
macro_rules! prof_scope {
    ($name:expr) => {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!($name);
    };
}

/// Marks the end of a frame and, on the first call, starts the puffin TCP server.
pub fn new_frame() {
    #[cfg(feature = "profiling")]
    {
        use std::sync::OnceLock;
        static SERVER: OnceLock<Option<puffin_http::Server>> = OnceLock::new();
        SERVER.get_or_init(|| {
            puffin::set_scopes_on(true);
            match puffin_http::Server::new("0.0.0.0:8585") {
                Ok(s) => {
                    log::info!("puffin server listening on 0.0.0.0:8585");
                    Some(s)
                }
                Err(e) => {
                    log::error!("puffin server failed to start: {e}");
                    None
                }
            }
        });
        puffin::GlobalProfiler::lock().new_frame();
    }
}
