use super::{RequestLane, SourceHealth};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use wxdata::clock::Instant;

/// Retry one idempotent read once. The caller owns the overall timeout, so both attempts remain
/// bounded as one request generation and cancellation still stops the whole operation.
pub(super) async fn retry_once<T, E, F, Fut>(delay: std::time::Duration, mut operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    match operation().await {
        Ok(value) => Ok(value),
        Err(_) => {
            wxdata::task::sleep(delay).await;
            operation().await
        }
    }
}

struct RequestStatus {
    fetching: bool,
    /// Hashed source arguments. The hash deduplicates without retaining API keys or locations.
    identity: u64,
    last_attempt: Instant,
    last_success: Option<Instant>,
    data_time: Option<DateTime<Utc>>,
    last_failure: Option<(Instant, String)>,
    cadence: std::time::Duration,
    abort: Option<futures_util::future::AbortHandle>,
    successes: u64,
    failures: u64,
}

/// Latest generation and fetch health in each result lane.
#[derive(Default)]
pub(super) struct RequestBook {
    next: u64,
    latest: HashMap<RequestLane, u64>,
    status: HashMap<RequestLane, RequestStatus>,
}

impl RequestBook {
    /// Start distinct work, or join the identical request already running in this lane.
    pub(super) fn start(&mut self, lane: RequestLane, identity: u64) -> Option<u64> {
        if self
            .status
            .get(&lane)
            .is_some_and(|status| status.fetching && status.identity == identity)
        {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        if let Some(handle) = self.status.get_mut(&lane).and_then(|status| status.abort.take()) {
            handle.abort();
        }
        self.latest.insert(lane.clone(), self.next);
        let now = Instant::now();
        let cadence = lane.cadence();
        self.status
            .entry(lane)
            .and_modify(|s| {
                s.fetching = true;
                s.identity = identity;
                s.last_attempt = now;
                s.cadence = cadence;
            })
            .or_insert(RequestStatus {
                fetching: true,
                identity,
                last_attempt: now,
                last_success: None,
                data_time: None,
                last_failure: None,
                cadence,
                abort: None,
                successes: 0,
                failures: 0,
            });
        Some(self.next)
    }

    pub(super) fn attach_abort(
        &mut self,
        lane: &RequestLane,
        generation: u64,
        handle: futures_util::future::AbortHandle,
    ) {
        if self.is_current(lane, generation) {
            if let Some(status) = self.status.get_mut(lane) {
                status.abort = Some(handle);
                return;
            }
        }
        handle.abort();
    }

    pub(super) fn cancel(&mut self, lane: &RequestLane) -> bool {
        let Some(status) = self.status.get_mut(lane) else {
            return false;
        };
        if !status.fetching {
            return false;
        }
        if let Some(handle) = status.abort.take() {
            handle.abort();
        }
        status.fetching = false;
        self.latest.remove(lane);
        true
    }

    pub(super) fn is_current(&self, lane: &RequestLane, generation: u64) -> bool {
        self.latest.get(lane) == Some(&generation)
    }

    /// Finish only the newest generation. An old failure cannot poison a newer success.
    pub(super) fn finish(
        &mut self,
        lane: &RequestLane,
        generation: u64,
        error: Option<&str>,
        data_time: Option<DateTime<Utc>>,
    ) -> bool {
        if !self.is_current(lane, generation) {
            return false;
        }
        if let Some(s) = self.status.get_mut(lane) {
            s.fetching = false;
            s.abort = None;
            match error {
                Some(e) => {
                    s.failures += 1;
                    s.last_failure = Some((Instant::now(), e.to_string()));
                }
                None => {
                    s.successes += 1;
                    s.last_success = Some(Instant::now());
                    if data_time.is_some() {
                        s.data_time = data_time;
                    }
                }
            }
        }
        true
    }

    pub(super) fn health(&self, lane: &RequestLane) -> SourceHealth {
        let now = Instant::now();
        let Some(s) = self.status.get(lane) else {
            return SourceHealth {
                source: lane.label(),
                fetching: false,
                last_attempt: None,
                last_success: None,
                data_age: None,
                last_failure: None,
                error: None,
                cadence: lane.cadence(),
                successes: 0,
                failures: 0,
            };
        };
        SourceHealth {
            source: lane.label(),
            fetching: s.fetching,
            last_attempt: Some(now.saturating_duration_since(s.last_attempt)),
            last_success: s.last_success.map(|t| now.saturating_duration_since(t)),
            data_age: s
                .data_time
                .map(|time| (Utc::now() - time).to_std().unwrap_or_default()),
            last_failure: s
                .last_failure
                .as_ref()
                .map(|(t, _)| now.saturating_duration_since(*t)),
            error: s.last_failure.as_ref().map(|(_, e)| e.clone()),
            cadence: s.cadence,
            successes: s.successes,
            failures: s.failures,
        }
    }

    pub(super) fn diagnostics(&self) -> Vec<serde_json::Value> {
        self.status
            .iter()
            .map(|(lane, status)| {
                let health = self.health(lane);
                serde_json::json!({
                    "source": lane.label(),
                    "state": format!("{:?}", health.state()),
                    "fetching": status.fetching,
                    "successes": status.successes,
                    "failures": status.failures,
                    "data_time": status.data_time,
                    "cadence_seconds": status.cadence.as_secs(),
                    "last_success_seconds_ago": health.last_success.map(|age| age.as_secs()),
                    "last_failure_seconds_ago": health.last_failure.map(|age| age.as_secs()),
                    "retry_in_seconds": health.next_retry().map(|age| age.as_secs()),
                })
            })
            .collect()
    }
}
