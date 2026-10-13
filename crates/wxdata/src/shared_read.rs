//! Coalesce simultaneous idempotent reads without retaining completed response bodies.
use futures_util::FutureExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

type Result<T> = std::result::Result<Arc<T>, Arc<str>>;
#[cfg(not(target_arch = "wasm32"))]
type ReadFuture<T> = futures_util::future::BoxFuture<'static, Result<T>>;
#[cfg(target_arch = "wasm32")]
type ReadFuture<T> = futures_util::future::LocalBoxFuture<'static, Result<T>>;
type Flight<T> = futures_util::future::Shared<ReadFuture<T>>;

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// Weak entries ensure the final consumer releases the response and pending future. Dropping one
/// consumer leaves the others running. Keys must include all arguments that affect the response.
pub struct Pool<T> {
    flights: Mutex<HashMap<String, Weak<Flight<T>>>>,
    starts: std::sync::atomic::AtomicU64,
    joins: std::sync::atomic::AtomicU64,
}
impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self { flights: Default::default(), starts: Default::default(), joins: Default::default() }
    }
}
impl<T: 'static + MaybeSend + Sync> Pool<T> {
    pub async fn read<F>(&self, key: String, operation: F) -> anyhow::Result<Arc<T>>
    where F: std::future::Future<Output = anyhow::Result<T>> + MaybeSend + 'static {
        let flight = {
            let mut entries = self.flights.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.retain(|_, flight| flight.strong_count() > 0);
            if let Some(flight) = entries.get(&key).and_then(Weak::upgrade) {
                self.joins.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                flight
            } else {
                self.starts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let operation = async move {
                    operation.await.map(Arc::new).map_err(|error| Arc::<str>::from(error.to_string()))
                };
                #[cfg(not(target_arch = "wasm32"))]
                let future = operation.boxed();
                #[cfg(target_arch = "wasm32")]
                let future = operation.boxed_local();
                let flight = Arc::new(future.shared());
                // Bound bookkeeping during unusual bursts. Excess reads still run independently.
                if entries.len() < 256 { entries.insert(key, Arc::downgrade(&flight)); }
                flight
            }
        };
        let result = flight.as_ref().clone().await;
        drop(flight);
        result.map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// Only aggregate counts are exported; keys may include private provider credentials.
    pub fn stats(&self) -> (u64, u64, usize) {
        let mut entries = self.flights.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, flight| flight.strong_count() > 0);
        (self.starts.load(std::sync::atomic::Ordering::Relaxed), self.joins.load(std::sync::atomic::Ordering::Relaxed), entries.len())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn bytes_pool() -> &'static Pool<Vec<u8>> {
    static POOL: std::sync::OnceLock<Pool<Vec<u8>>> = std::sync::OnceLock::new();
    POOL.get_or_init(Default::default)
}

#[cfg(target_arch = "wasm32")]
thread_local! { static BYTE_POOL: Arc<Pool<Vec<u8>>> = Arc::new(Pool::default()); }
#[cfg(target_arch = "wasm32")]
fn bytes_pool() -> Arc<Pool<Vec<u8>>> { BYTE_POOL.with(Arc::clone) }

pub fn byte_stats() -> (u64, u64, usize) { bytes_pool().stats() }

pub async fn bytes<F>(key: String, operation: F) -> anyhow::Result<Arc<Vec<u8>>>
where F: std::future::Future<Output = anyhow::Result<Vec<u8>>> + MaybeSend + 'static {
    #[cfg(not(target_arch = "wasm32"))]
    { bytes_pool().read(key, operation).await }
    #[cfg(target_arch = "wasm32")]
    {
        // Keep the pool alive outside the thread-local borrow across await.
        let pool = bytes_pool();
        pool.read(key, operation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn simultaneous_reads_share_body_and_do_not_cache_completed_results() {
        let pool = Pool::<Vec<u8>>::default();
        let (first, second) = futures_util::join!(
            pool.read("same".into(), async { tokio::task::yield_now().await; Ok(vec![1,2,3]) }),
            pool.read("same".into(), async { panic!("duplicate operation ran") }),
        );
        let first = first.unwrap();
        assert!(Arc::ptr_eq(&first, &second.unwrap()));
        assert_eq!(pool.stats(), (1,1,0));
        assert_eq!(&*pool.read("same".into(), async { Ok(vec![4]) }).await.unwrap(), &[4]);
    }
    #[tokio::test]
    async fn cancellation_does_not_cancel_a_remaining_consumer() {
        let pool = Pool::<Vec<u8>>::default();
        let (send, receive) = tokio::sync::oneshot::channel();
        let mut first = Box::pin(pool.read("same".into(), async move { receive.await.unwrap(); Ok(vec![7]) }));
        assert!(futures_util::poll!(first.as_mut()).is_pending());
        let mut second = Box::pin(pool.read("same".into(), async { panic!("duplicate") }));
        assert!(futures_util::poll!(second.as_mut()).is_pending());
        drop(first);
        send.send(()).unwrap();
        assert_eq!(&*second.await.unwrap(), &[7]);
        assert_eq!(pool.stats(), (1,1,0));
    }
}
