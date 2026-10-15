//! Process-lifetime counters for the numbers the perf work is judged on.
//!
//! Every later optimization has to show a before/after, and "how many requests did that poll
//! actually make" is not a question a profiler answers — it needs a count at the fetch site. The
//! counters are read by the app's Perf window, by `--serve`'s `/metrics`, and by a once-a-minute
//! log line in headless runs.
//!
//! Browser counters use thread-local cells; native counters use relaxed atomics.
//! Decode-path timings include worker scheduling and serialization, not just decoder CPU time.

/// One counter each, in [`Snapshot`] order.
#[cfg(not(target_arch = "wasm32"))]
static COUNTERS: [std::sync::atomic::AtomicU64; Counter::COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; Counter::COUNT];

/// What can be counted. Adding one means adding its label to [`Counter::LABELS`].
#[derive(Copy, Clone, Debug)]
#[repr(usize)]
pub enum Counter {
    /// HTTP requests issued by an instrumented feed path.
    NetRequests,
    /// Response bytes received by an instrumented feed path.
    NetBytes,
    /// Requests an upstream answered `304 Not Modified`.
    NetNotModified,
    /// Volume fetches skipped because the feed had not published a new one.
    FetchSkipped,
    /// Decoded volumes served from the app's scan cache.
    ScanCacheHits,
    ScanCacheMisses,
    /// Bucket listings served from the day-list cache.
    DayListHits,
    DayListMisses,
    /// Frames the app drew.
    FramesDrawn,
    /// Sweeps run through `bin_scan_opts`.
    SweepsBinned,
    /// Radar GPU textures built from scratch.
    RadarTexturesBuilt,
    /// Tile-quad vertex lists rebuilt and re-uploaded.
    TileQuadsBuilt,
    /// Radar gate, flag and palette bytes submitted to the GPU.
    RadarUploadBytes,
    DecodePathMicros,
    DecodePaths,
    CustomCompiles,
    CustomEvaluations,
    CustomCacheHits,
    /// Vector tile geometry accepted for GPU upload.
    VectorTileUploads,
    /// Completed vector tiles skipped because they no longer overlap the view.
    VectorTileOffscreenSkips,
}

impl Counter {
    const COUNT: usize = Self::LABELS.len();

    /// Prometheus/log names, in declaration order.
    pub const LABELS: [&'static str; 20] = [
        "net_requests",
        "net_bytes",
        "net_not_modified",
        "fetch_skipped",
        "scan_cache_hits",
        "scan_cache_misses",
        "day_list_hits",
        "day_list_misses",
        "frames_drawn",
        "sweeps_binned",
        "radar_textures_built",
        "tile_quads_built",
        "radar_upload_bytes",
        "decode_path_micros",
        "decode_paths",
        "custom_compiles",
        "custom_evaluations",
        "custom_cache_hits",
        "vector_tile_uploads",
        "vector_tile_offscreen_skips",
    ];
}

/// Add `n` to `c`.
#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub fn add(c: Counter, n: u64) {
    COUNTERS[c as usize].fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_arch = "wasm32")]
#[inline]
pub fn add(c: Counter, n: u64) {
    BROWSER_COUNTERS.with(|counters| {
        let mut values = counters.get();
        values[c as usize] = values[c as usize].saturating_add(n);
        counters.set(values);
    });
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static BROWSER_COUNTERS: std::cell::Cell<[u64; Counter::COUNT]> = const { std::cell::Cell::new([0; Counter::COUNT]) };
}

/// A wall-time measurement of an asynchronous decode path, including failures.
pub struct DecodeTimer(crate::clock::Instant);
impl Default for DecodeTimer {
    fn default() -> Self { Self(crate::clock::Instant::now()) }
}
impl Drop for DecodeTimer {
    fn drop(&mut self) {
        bump(Counter::DecodePaths);
        add(Counter::DecodePathMicros, self.0.elapsed().as_micros().min(u64::MAX as u128) as u64);
    }
}

/// Add one to `c`.
#[inline]
pub fn bump(c: Counter) {
    add(c, 1);
}

/// Count one instrumented HTTP response of `bytes`.
#[inline]
pub fn net(bytes: usize) {
    bump(Counter::NetRequests);
    add(Counter::NetBytes, bytes as u64);
}

/// Every counter's current value, paired with its label, in [`Counter::LABELS`] order.
#[cfg(not(target_arch = "wasm32"))]
pub fn snapshot() -> Vec<(&'static str, u64)> {
    Counter::LABELS
        .iter()
        .zip(&COUNTERS)
        .map(|(l, c)| (*l, c.load(std::sync::atomic::Ordering::Relaxed)))
        .collect()
}

#[cfg(target_arch = "wasm32")]
pub fn snapshot() -> Vec<(&'static str, u64)> {
    BROWSER_COUNTERS.with(|counters| Counter::LABELS.iter().zip(counters.get()).map(|(label, value)| (*label, value)).collect())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A label per counter, and a count that lands on the right one — an off-by-one here would
    /// silently report every wave's before/after against the wrong number.
    #[test]
    fn labels_line_up_with_counters() {
        assert_eq!(Counter::LABELS.len(), Counter::COUNT);
        let before = snapshot();
        net(1234);
        let after = snapshot();
        let delta = |name: &str| {
            let f = |v: &Vec<(&'static str, u64)>| v.iter().find(|(l, _)| *l == name).unwrap().1;
            f(&after) - f(&before)
        };
        assert_eq!(delta("net_requests"), 1);
        assert_eq!(delta("net_bytes"), 1234);
        assert_eq!(delta("frames_drawn"), 0);
    }
}
