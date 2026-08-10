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

/// Yield for `d`, but give up early once `active` goes false.
///
/// Returns whether the wait ran to completion. Sleeping the whole interval in one go left a
/// cancelled live stream alive for up to its poll interval; slicing it costs one predicate call a
/// second and is the same on both targets, since neither has a cancellable timer worth the
/// plumbing.
pub async fn sleep_while(d: std::time::Duration, active: impl Fn() -> bool) -> bool {
    let slice = std::time::Duration::from_secs(1);
    let mut left = d;
    while !left.is_zero() {
        if !active() {
            return false;
        }
        let step = left.min(slice);
        sleep(step).await;
        left -= step;
    }
    active()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn sleep_while_gives_up_within_a_slice() {
        let calls = AtomicUsize::new(0);
        // False from the second check on: a long wait must end after roughly one slice, not
        // after the whole interval.
        let done = sleep_while(Duration::from_secs(15), || {
            calls.fetch_add(1, Ordering::Relaxed) == 0
        })
        .await;
        assert!(!done, "an inactive wait reports that it did not finish");
        assert_eq!(calls.load(Ordering::Relaxed), 2, "one slice, then give up");
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_while_runs_the_whole_wait_when_active() {
        assert!(sleep_while(Duration::from_secs(3), || true).await);
    }
}
