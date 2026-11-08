//! Refcounted per-stop polling registry: one upstream `DataMall` poll per
//! stop per interval, no matter how many connections are subscribed to it.
//! Polling for a stop starts on the first subscriber and stops the instant
//! the last one drops.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sgbr_core::error::CoreError;
use sgbr_core::lta::client::fetch_arrivals_with_agent;
use sgbr_core::lta::model::BusArrivalResponse;
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use ureq::Agent;

/// A fetched arrival snapshot for one stop, cheaply shareable across every
/// subscriber's SSE handler.
pub type Snapshot = Arc<SnapshotData>;

/// One poll's result for a stop, plus when it was fetched.
#[derive(Debug)]
pub struct SnapshotData {
    pub code: String,
    pub response: BusArrivalResponse,
    pub fetched_at_unix: i64,
}

/// Injected so tests never touch the network.
pub trait ArrivalFetcher: Send + Sync + 'static {
    /// Fetch the current arrivals for `stop_code`. Blocking — callers run
    /// this on a dedicated thread (`spawn_blocking`).
    ///
    /// # Errors
    /// Whatever the underlying transport/parse failure was.
    fn fetch(&self, stop_code: &str) -> Result<BusArrivalResponse, CoreError>;
}

/// [`ArrivalFetcher`] that hits the real `DataMall` Bus Arrival endpoint.
pub struct LtaFetcher {
    agent: Agent,
    account_key: String,
}

impl LtaFetcher {
    /// A hung upstream connection would otherwise stall a stop's poll loop
    /// forever (fetches run serially per stop), so the whole request is
    /// capped rather than left on ureq's unlimited defaults.
    const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

    #[must_use]
    pub fn new(account_key: String) -> Self {
        Self {
            agent: Agent::new_with_config(
                Agent::config_builder()
                    .timeout_global(Some(Self::FETCH_TIMEOUT))
                    .build(),
            ),
            account_key,
        }
    }
}

impl std::fmt::Debug for LtaFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LtaFetcher").finish_non_exhaustive()
    }
}

impl ArrivalFetcher for LtaFetcher {
    fn fetch(&self, stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
        fetch_arrivals_with_agent(&self.agent, &self.account_key, stop_code)
    }
}

/// Per-stop bookkeeping: how many live subscriptions reference this stop, a
/// receiver for the latest snapshot, and the handle of the poller task
/// driving it.
struct StopEntry {
    refcount: usize,
    rx: watch::Receiver<Option<Snapshot>>,
    task: JoinHandle<()>,
}

/// Refcounted per-stop poller registry: `subscribe` starts (or joins) a
/// poller for a stop code; the returned [`Subscription`] tears the poller
/// down on `Drop` once nothing else references that stop.
pub struct Registry {
    stops: Mutex<HashMap<String, StopEntry>>,
    fetcher: Arc<dyn ArrivalFetcher>,
    poll_interval: Duration,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("active_stops", &self.active_stops())
            .field("poll_interval", &self.poll_interval)
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// Build a registry that polls every stop at `poll_interval`, fetching
    /// through `fetcher` (the real `LtaFetcher` in production, a mock in
    /// tests).
    #[must_use]
    pub fn new(fetcher: Arc<dyn ArrivalFetcher>, poll_interval: Duration) -> Arc<Self> {
        Arc::new(Self {
            stops: Mutex::new(HashMap::new()),
            fetcher,
            poll_interval,
        })
    }

    /// Number of distinct stop codes currently being polled.
    #[must_use]
    pub fn active_stops(&self) -> usize {
        self.lock_stops().len()
    }

    /// Subscribe to `code`: joins an already-running poller if one exists
    /// (bumping its refcount), or starts a fresh one. The returned
    /// [`Subscription`]'s `rx` immediately carries the latest snapshot (if
    /// any poll has completed yet) and every subsequent one; dropping it
    /// releases this reference.
    #[must_use]
    pub fn subscribe(self: &Arc<Self>, code: &str) -> Subscription {
        let mut stops = self.lock_stops();
        let rx = if let Some(entry) = stops.get_mut(code) {
            entry.refcount += 1;
            entry.rx.clone()
        } else {
            let (tx, rx) = watch::channel(None);
            let task = tokio::spawn(poll_loop(
                code.to_owned(),
                Arc::clone(&self.fetcher),
                self.poll_interval,
                tx,
            ));
            stops.insert(
                code.to_owned(),
                StopEntry {
                    refcount: 1,
                    rx: rx.clone(),
                    task,
                },
            );
            rx
        };
        drop(stops);
        Subscription {
            code: code.to_owned(),
            rx,
            registry: Arc::clone(self),
        }
    }

    /// Called by [`Subscription::drop`]: decrement the refcount for `code`,
    /// aborting the poller and removing the entry once it hits zero.
    fn unsubscribe(&self, code: &str) {
        let mut stops = self.lock_stops();
        let Some(entry) = stops.get_mut(code) else {
            return;
        };
        entry.refcount = entry.refcount.saturating_sub(1);
        if entry.refcount == 0
            && let Some(entry) = stops.remove(code)
        {
            entry.task.abort();
        }
    }

    /// Lock the stop map, recovering from poisoning (a panicked holder would
    /// otherwise wedge every future subscribe/unsubscribe).
    fn lock_stops(&self) -> std::sync::MutexGuard<'_, HashMap<String, StopEntry>> {
        self.stops.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One live reference to a polled stop. Cloning `rx` gives every SSE
/// connection its own cursor into the shared snapshot stream; dropping the
/// `Subscription` releases this connection's claim on the underlying
/// poller.
pub struct Subscription {
    pub code: String,
    pub rx: watch::Receiver<Option<Snapshot>>,
    registry: Arc<Registry>,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("code", &self.code)
            .finish_non_exhaustive()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.registry.unsubscribe(&self.code);
    }
}

/// Poll `code` forever at `interval`: fetch on a blocking thread, publish a
/// fresh [`Snapshot`] on success; on failure log, keep the last value, and
/// still wake every subscriber so it re-renders the old snapshot against the
/// current clock (otherwise countdowns would freeze for the whole outage).
/// Runs until the owning [`tokio::task::JoinHandle`] is aborted (last
/// subscriber dropped).
async fn poll_loop(
    code: String,
    fetcher: Arc<dyn ArrivalFetcher>,
    interval: Duration,
    tx: watch::Sender<Option<Snapshot>>,
) {
    loop {
        let fetch_code = code.clone();
        let fetch_fetcher = Arc::clone(&fetcher);
        let outcome = tokio::task::spawn_blocking(move || fetch_fetcher.fetch(&fetch_code)).await;
        match outcome {
            Ok(Ok(response)) => {
                let snapshot = Arc::new(SnapshotData {
                    code: code.clone(),
                    response,
                    fetched_at_unix: OffsetDateTime::now_utc().unix_timestamp(),
                });
                tx.send_replace(Some(snapshot));
            }
            Ok(Err(e)) => {
                tracing::warn!("arrival fetch failed for stop {code}: {e}");
                tx.send_modify(|_unchanged| {});
            }
            Err(e) => {
                tracing::warn!("arrival fetch task did not complete for stop {code}: {e}");
                tx.send_modify(|_unchanged| {});
            }
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{ArrivalFetcher, Registry};
    use sgbr_core::error::CoreError;
    use sgbr_core::lta::model::BusArrivalResponse;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[derive(Debug)]
    struct MockFetcher {
        calls: AtomicUsize,
        response: BusArrivalResponse,
    }

    impl MockFetcher {
        fn new(response: BusArrivalResponse) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ArrivalFetcher for MockFetcher {
        fn fetch(&self, _stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    fn empty_response() -> BusArrivalResponse {
        serde_json::from_str(r#"{"BusStopCode":"83139","Services":[]}"#)
            .expect("sample response must parse")
    }

    /// Wait for `rx` to observe a value beyond whatever it last saw. Used to
    /// synchronize with the poller's blocking-thread fetch, which runs on a
    /// real OS thread and is unaffected by `tokio::time::pause`.
    async fn wait_for_change(rx: &mut tokio::sync::watch::Receiver<Option<super::Snapshot>>) {
        let _ = rx.changed().await;
    }

    #[tokio::test(start_paused = true)]
    async fn two_subscribers_share_one_poller() {
        let fetcher = Arc::new(MockFetcher::new(empty_response()));
        let registry = Registry::new(
            Arc::clone(&fetcher) as Arc<dyn ArrivalFetcher>,
            Duration::from_secs(15),
        );

        let sub1 = registry.subscribe("83139");
        let sub2 = registry.subscribe("83139");
        let mut rx = sub1.rx.clone();
        wait_for_change(&mut rx).await;
        assert_eq!(fetcher.calls(), 1, "one fetch serves both subscribers");
        assert_eq!(registry.active_stops(), 1);

        tokio::time::advance(Duration::from_secs(15)).await;
        wait_for_change(&mut rx).await;
        assert_eq!(fetcher.calls(), 2, "second interval tick, still one fetch");

        drop(sub1);
        drop(sub2);
    }

    #[tokio::test(start_paused = true)]
    async fn last_drop_tears_down_the_poller() {
        let fetcher = Arc::new(MockFetcher::new(empty_response()));
        let registry = Registry::new(
            Arc::clone(&fetcher) as Arc<dyn ArrivalFetcher>,
            Duration::from_secs(15),
        );

        let sub1 = registry.subscribe("83139");
        let sub2 = registry.subscribe("83139");
        let mut rx = sub1.rx.clone();
        wait_for_change(&mut rx).await;
        assert_eq!(registry.active_stops(), 1);

        drop(sub2);
        assert_eq!(registry.active_stops(), 1, "one subscriber remains");

        drop(sub1);
        assert_eq!(
            registry.active_stops(),
            0,
            "last subscriber torn the poller down"
        );

        let calls_at_teardown = fetcher.calls();
        tokio::time::advance(Duration::from_mins(1)).await;
        // Give any (incorrectly) still-running poller a chance to fire.
        tokio::task::yield_now().await;
        assert_eq!(
            fetcher.calls(),
            calls_at_teardown,
            "no further fetches after the last subscriber drops"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn new_subscriber_sees_cached_snapshot_immediately() {
        let fetcher = Arc::new(MockFetcher::new(empty_response()));
        let registry = Registry::new(
            Arc::clone(&fetcher) as Arc<dyn ArrivalFetcher>,
            Duration::from_secs(15),
        );

        let sub1 = registry.subscribe("83139");
        let mut rx = sub1.rx.clone();
        wait_for_change(&mut rx).await;
        assert!(sub1.rx.borrow().is_some(), "first poll landed");

        // A second subscriber joining after the poll completed gets the
        // cached snapshot on its very first read, without waiting for
        // another interval tick.
        let sub2 = registry.subscribe("83139");
        assert!(
            sub2.rx.borrow().is_some(),
            "new subscriber sees the already-fetched snapshot instantly"
        );

        drop(sub1);
        drop(sub2);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_polls_still_wake_subscribers() {
        #[derive(Debug)]
        struct FailingFetcher;
        impl ArrivalFetcher for FailingFetcher {
            fn fetch(&self, _stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
                Err(CoreError::Http("upstream down".to_owned()))
            }
        }
        let registry = Registry::new(Arc::new(FailingFetcher), Duration::from_secs(15));
        let sub = registry.subscribe("83139");
        let mut rx = sub.rx.clone();
        // Would hang forever if a failed poll published nothing.
        wait_for_change(&mut rx).await;
        assert!(rx.borrow().is_none(), "no snapshot exists, only a wake-up");
        drop(sub);
    }

    /// Confirms the specific `WatchStream` semantics the SSE handler relies
    /// on: wrapping an already-warm `Subscription`'s `rx` yields the cached
    /// snapshot as its very first item, with no interval wait.
    #[tokio::test(start_paused = true)]
    async fn watch_stream_yields_cached_value_as_first_item() {
        use tokio_stream::StreamExt as _;
        use tokio_stream::wrappers::WatchStream;

        let fetcher = Arc::new(MockFetcher::new(empty_response()));
        let registry = Registry::new(
            Arc::clone(&fetcher) as Arc<dyn ArrivalFetcher>,
            Duration::from_secs(15),
        );

        let sub1 = registry.subscribe("83139");
        let mut change_rx = sub1.rx.clone();
        wait_for_change(&mut change_rx).await;

        // A fresh subscriber on the already-warm stop, wrapped exactly like
        // the SSE handler does.
        let sub2 = registry.subscribe("83139");
        let mut stream = WatchStream::new(sub2.rx.clone());
        let first = stream.next().await;
        assert!(
            matches!(first, Some(Some(_))),
            "first WatchStream item must already carry the cached snapshot, got {first:?}"
        );

        drop(sub1);
        drop(sub2);
    }
}
