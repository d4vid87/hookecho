//! Process-lifetime counters for the numbers the perf work is judged on.
//!
//! Every later optimization has to show a before/after, and "how many requests did that poll
//! actually make" is not a question a profiler answers — it needs a count at the fetch site. The
//! counters are read by the app's Perf window, by `--serve`'s `/metrics`, and by a once-a-minute
//! log line in headless runs.
//!
//! On wasm every function here is an empty inline body, so the browser build carries no counters
//! and no bytes: the shared code paths are the same ones, and a native measurement of a shared
//! path stands in for the wasm one.
//!
// ponytail: flat atomics, no labels, no histograms — the ceiling is "which wave moved which
// number". A per-host or per-feed breakdown wants a real metrics crate, not more statics here.

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
}

#[cfg(not(target_arch = "wasm32"))]
impl Counter {
    const COUNT: usize = Self::LABELS.len();

    /// Prometheus/log names, in declaration order.
    pub const LABELS: [&'static str; 12] = [
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
pub fn add(_c: Counter, _n: u64) {}

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
