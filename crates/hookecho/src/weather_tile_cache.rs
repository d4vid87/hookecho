//! Small shared cache and request budget for weather imagery across all map panes.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use wxdata::clock::Instant;

const CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_INFLIGHT: usize = 4;
const RAINVIEWER_PER_MINUTE: usize = 80;
const SLOT_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);
const SLOT_RECHECK: std::time::Duration = std::time::Duration::from_millis(25);

#[derive(Default)]
struct Cache {
    entries: VecDeque<(String, Arc<Vec<u8>>)>,
    bytes: usize,
    inflight: usize,
    rainviewer: VecDeque<Instant>,
}

impl Cache {
    fn get(&mut self, url: &str) -> Option<Arc<Vec<u8>>> {
        let index = self.entries.iter().position(|(key, _)| key == url)?;
        let entry = self.entries.remove(index)?;
        let bytes = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(bytes)
    }

    fn remove(&mut self, url: &str) {
        if let Some(index) = self.entries.iter().position(|(key, _)| key == url) {
            if let Some((_, bytes)) = self.entries.remove(index) { self.bytes -= bytes.len(); }
        }
    }

    fn put(&mut self, url: String, bytes: Arc<Vec<u8>>) {
        self.remove(&url);
        if bytes.is_empty() || bytes.len() > MAX_TILE_BYTES { return; }
        while self.bytes + bytes.len() > CACHE_BYTES || self.entries.len() >= 128 {
            if let Some((_, old)) = self.entries.pop_front() { self.bytes -= old.len(); }
            else { break; }
        }
        self.bytes += bytes.len();
        self.entries.push_back((url, bytes));
    }

    fn start(&mut self, rainviewer: bool, now: Instant) -> anyhow::Result<bool> {
        self.rainviewer.retain(|time| now.duration_since(*time).as_secs() < 60);
        anyhow::ensure!(!rainviewer || self.rainviewer.len() < RAINVIEWER_PER_MINUTE,
            "RainViewer request budget reached; retry shortly");
        if self.inflight >= MAX_INFLIGHT { return Ok(false); }
        self.inflight += 1;
        if rainviewer { self.rainviewer.push_back(now); }
        Ok(true)
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub(crate) fn is_weather_tile(url: &str) -> bool {
    url.starts_with("https://tilecache.rainviewer.com/")
        || url.starts_with("https://www.jma.go.jp/bosai/jmatile/data/nowc/")
        || url.starts_with("https://geo.weather.gc.ca/geomet?")
        || url.starts_with("https://maps.dwd.de/geoserver/dwd/wms?")
}

pub(crate) fn get(url: &str) -> Option<Arc<Vec<u8>>> {
    cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(url)
}

pub(crate) fn put(url: &str, bytes: Arc<Vec<u8>>) {
    cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).put(url.to_owned(), bytes);
}

pub(crate) fn invalidate(url: &str) {
    cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(url);
}

pub(crate) struct Permit<'a> { cache: &'a Mutex<Cache> }
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut cache = self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.inflight = cache.inflight.saturating_sub(1);
    }
}

async fn acquire(cache: &Mutex<Cache>, rainviewer: bool) -> anyhow::Result<Permit<'_>> {
    wxdata::task::timeout(SLOT_WAIT_LIMIT, async {
        loop {
            let admitted = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                .start(rainviewer, Instant::now())?;
            if admitted { return Ok(Permit { cache }); }
            // Waiting owns no slot or rate-budget entry, so aborting needs no queue cleanup.
            wxdata::task::sleep(SLOT_RECHECK).await;
        }
    }).await?
}

/// Count actual upstream reads, after cache lookup and shared-read coalescing. Normal
/// cross-pane concurrency waits for a slot instead of becoming a failed tile/backoff.
pub(crate) async fn start(url: &str) -> anyhow::Result<Permit<'static>> {
    acquire(cache(), url.starts_with("https://tilecache.rainviewer.com/")).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_bytes_are_shared_and_eviction_is_bounded() {
        let mut cache = Cache::default();
        let bytes = Arc::new(vec![7; MAX_TILE_BYTES]);
        cache.put("a".into(), bytes.clone());
        assert!(Arc::ptr_eq(&bytes, &cache.get("a").unwrap()));
        cache.put("b".into(), bytes.clone());
        cache.put("c".into(), bytes);
        assert_eq!(cache.bytes, CACHE_BYTES);
        assert!(cache.get("a").is_none());
        cache.remove("b");
        assert_eq!(cache.bytes, MAX_TILE_BYTES);
    }
    #[test]
    fn request_budget_spans_all_panes_and_recovers_after_window() {
        let mut cache = Cache::default();
        let now = Instant::now();
        for _ in 0..RAINVIEWER_PER_MINUTE {
            assert!(cache.start(true, now).unwrap());
            cache.inflight -= 1;
        }
        assert!(cache.start(true, now).is_err());
        let later = now + std::time::Duration::from_secs(60);
        assert!(cache.start(true, later).unwrap());
        for _ in 1..MAX_INFLIGHT { assert!(cache.start(false, later).unwrap()); }
        assert!(!cache.start(false, later).unwrap());
    }
    #[tokio::test]
    async fn saturated_slots_wait_without_spending_rate_budget_and_cancellation_releases_slots() {
        let cache = Mutex::new(Cache::default());
        let mut held = Vec::new();
        for _ in 0..MAX_INFLIGHT { held.push(acquire(&cache, false).await.unwrap()); }
        let mut canceled = Box::pin(acquire(&cache, true));
        assert!(futures_util::poll!(canceled.as_mut()).is_pending());
        drop(canceled);
        assert_eq!(cache.lock().unwrap().inflight, MAX_INFLIGHT);
        assert!(cache.lock().unwrap().rainviewer.is_empty());

        let mut waiting = Box::pin(acquire(&cache, true));
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        drop(held.pop());
        let admitted = waiting.await.expect("a completed read frees the waiting tile");
        assert_eq!(cache.lock().unwrap().inflight, MAX_INFLIGHT);
        assert_eq!(cache.lock().unwrap().rainviewer.len(), 1);
        drop(admitted);
        drop(held);
        assert_eq!(cache.lock().unwrap().inflight, 0);
        assert_eq!(cache.lock().unwrap().rainviewer.len(), 1, "completed requests still count for this minute");
    }

    #[test]
    fn only_known_weather_hosts_are_budgeted() {
        assert!(is_weather_tile("https://tilecache.rainviewer.com/v2/radar/1/256/0/0/0/2/1_1.png"));
        assert!(!is_weather_tile("https://tilecache.rainviewer.com.evil.test/tile"));
        assert!(!is_weather_tile("https://api.mapbox.com/tile?access_token=private"));
    }
}
