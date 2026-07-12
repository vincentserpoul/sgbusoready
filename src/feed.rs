//! Process-global arrivals feed: one long-lived SSE connection to the worker,
//! fanned out to every stop the active commutes care about. `rows.rs` reads
//! the latest per-stop snapshot synchronously (no network on the UI thread);
//! this module owns the background stream thread that keeps it warm.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use sgbr_core::commute::schedule::StopPlan;
use sgbr_core::lta::arrival::ArrivalItem;
use sgbr_core::worker::client::{StreamExit, WorkerConfig, run_stream};
use sgbr_core::worker::credentials::WorkerCredentials;
use sgbr_core::worker::protocol::{StopUpdate, stream_query};

/// Backoff cap for reconnect attempts (seconds).
const MAX_BACKOFF_SECS: u64 = 60;

/// Process-global arrivals cache, fed by one background SSE stream thread.
#[allow(
    missing_debug_implementations,
    reason = "listeners are boxed closures, which cannot derive Debug"
)]
pub struct ArrivalsFeed {
    /// Latest arrival items per stop code (union of every tracked bus there).
    snapshots: Mutex<HashMap<String, Vec<ArrivalItem>>>,
    /// Bumped on every `set_plans` call whose normalized plan actually
    /// changes (or whose thread needs restarting); the stream thread checks
    /// this to know when to reconnect with a fresh query or exit.
    generation: AtomicU64,
    /// The desired subscription set, normalized for stable diffing.
    plans: Mutex<Vec<StopPlan>>,
    /// Whether the background stream thread is currently alive.
    running: AtomicBool,
    /// Whether any snapshot has ever arrived (for a cold-start wait).
    first: Mutex<bool>,
    first_cv: Condvar,
    /// Change notifications, fired (on the stream thread) after every
    /// applied update.
    listeners: Mutex<Vec<Box<dyn Fn() + Send>>>,
}

/// The single process-wide feed instance.
pub fn feed() -> &'static ArrivalsFeed {
    static FEED: OnceLock<ArrivalsFeed> = OnceLock::new();
    FEED.get_or_init(ArrivalsFeed::new)
}

/// Sort stops by code and dedup+sort each stop's buses, so two plan lists
/// describing the same desired subscriptions in a different order compare
/// equal.
fn normalize(mut plans: Vec<StopPlan>) -> Vec<StopPlan> {
    for plan in &mut plans {
        plan.buses.sort();
        plan.buses.dedup();
    }
    plans.sort_by(|a, b| a.code.cmp(&b.code));
    plans
}

impl ArrivalsFeed {
    fn new() -> Self {
        Self {
            snapshots: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
            plans: Mutex::new(Vec::new()),
            running: AtomicBool::new(false),
            first: Mutex::new(false),
            first_cv: Condvar::new(),
            listeners: Mutex::new(Vec::new()),
        }
    }

    fn lock_snapshots(&self) -> MutexGuard<'_, HashMap<String, Vec<ArrivalItem>>> {
        self.snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_plans(&self) -> MutexGuard<'_, Vec<StopPlan>> {
        self.plans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_first(&self) -> MutexGuard<'_, bool> {
        self.first.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_listeners(&self) -> MutexGuard<'_, Vec<Box<dyn Fn() + Send>>> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Update the desired subscription set. A no-op if the normalized plan
    /// is unchanged and the stream thread's state is already consistent with
    /// it (running when there's something to track, absent when there's
    /// nothing or no credentials yet). Otherwise stores the new plan, bumps
    /// `generation` so any live stream thread reconnects with a fresh query,
    /// and — if there's something to track and credentials are available —
    /// spawns the stream thread when one isn't already running.
    pub fn set_plans(
        &'static self,
        plans: Vec<StopPlan>,
        creds: Option<&WorkerCredentials>,
        base_url: &str,
    ) {
        let normalized = normalize(plans);
        let unchanged = *self.lock_plans() == normalized;
        let already_running = self.running.load(Ordering::Acquire);
        let thread_state_consistent = normalized.is_empty() || already_running || creds.is_none();
        if unchanged && thread_state_consistent {
            return;
        }
        let is_empty = normalized.is_empty();
        *self.lock_plans() = normalized;
        self.generation.fetch_add(1, Ordering::SeqCst);
        if is_empty {
            return;
        }
        let Some(creds) = creds else {
            return;
        };
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let cfg = WorkerConfig {
            base_url: base_url.to_owned(),
            token: creds.token.clone(),
        };
        std::thread::spawn(move || self.run_loop(&cfg));
    }

    /// The background stream loop: reconnects on plan changes, backs off on
    /// disconnects, exits once the plan goes empty.
    fn run_loop(&'static self, cfg: &WorkerConfig) {
        let mut backoff_secs: u64 = 1;
        loop {
            let my_gen = self.generation.load(Ordering::SeqCst);
            let plans = self.lock_plans().clone();
            if plans.is_empty() {
                break;
            }
            let query = stream_query(&plans);
            let mut got_update = false;
            let exit = run_stream(cfg, &query, &self.generation, my_gen, |update| {
                got_update = true;
                self.apply_update(update);
            });
            match exit {
                Ok(StreamExit::PlanChanged) => {
                    backoff_secs = 1;
                }
                // `Stopped` is reserved for a future orchestrator loop and
                // never produced by `run_stream` today; treat it as a clean
                // exit rather than folding it into the reconnect path below.
                Ok(StreamExit::Stopped) => break,
                Ok(StreamExit::Disconnected) | Err(_) => {
                    if got_update {
                        backoff_secs = 1;
                    }
                    self.sleep_backoff(backoff_secs, my_gen);
                    backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                }
            }
        }
        self.running.store(false, Ordering::Release);
    }

    /// Sleep up to `secs`, in <=1s slices, abandoning early the moment
    /// `generation` moves past `baseline_gen` (a plan change or empty-out).
    fn sleep_backoff(&self, secs: u64, baseline_gen: u64) {
        let mut remaining = secs;
        while remaining > 0 {
            if self.generation.load(Ordering::SeqCst) != baseline_gen {
                return;
            }
            let slice = remaining.min(1);
            std::thread::sleep(Duration::from_secs(slice));
            remaining -= slice;
        }
    }

    /// Apply one stop's update: store the snapshot, mark the first-arrival
    /// gate open, and notify listeners.
    fn apply_update(&self, update: StopUpdate) {
        self.lock_snapshots().insert(update.code, update.items);
        *self.lock_first() = true;
        self.first_cv.notify_all();
        self.notify_listeners();
    }

    fn notify_listeners(&self) {
        for listener in self.lock_listeners().iter() {
            listener();
        }
    }

    /// The latest snapshot for `code` (empty if never seen).
    pub fn items_for(&self, code: &str) -> Vec<ArrivalItem> {
        self.lock_snapshots().get(code).cloned().unwrap_or_default()
    }

    /// Block up to `timeout` for the first-ever snapshot to arrive; `true` if
    /// one has (or arrives in time), `false` on timeout. Not called anywhere
    /// yet in this phase — the Android cold-start path (Phase 5) is its
    /// first caller; exercised directly by a unit test below in the
    /// meantime.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the Phase-5 Android bridge cold start; exercised directly by a unit test until then"
        )
    )]
    pub fn wait_first(&self, timeout: Duration) -> bool {
        let first = self.lock_first();
        if *first {
            return true;
        }
        let result = self
            .first_cv
            .wait_timeout_while(first, timeout, |done| !*done);
        let (guard, _) = result.unwrap_or_else(PoisonError::into_inner);
        *guard
    }

    /// Register a change listener, called (on the stream thread) after every
    /// applied update.
    pub fn add_listener(&self, listener: impl Fn() + Send + 'static) {
        self.lock_listeners().push(Box::new(listener));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use sgbr_core::commute::schedule::StopPlan;
    use sgbr_core::worker::protocol::StopUpdate;

    use super::ArrivalsFeed;

    fn plan(code: &str, buses: &[&str]) -> StopPlan {
        StopPlan {
            code: code.to_owned(),
            name: String::new(),
            buses: buses.iter().map(|b| (*b).to_owned()).collect(),
        }
    }

    /// A fresh, independent instance for each test (not the process-global
    /// `feed()` singleton, which would leak state across parallel tests) —
    /// leaked to get the `'static` lifetime `set_plans` requires for its
    /// spawned-thread path, even though these tests never exercise it
    /// (creds is always `None`).
    fn fresh() -> &'static ArrivalsFeed {
        Box::leak(Box::new(ArrivalsFeed::new()))
    }

    #[test]
    fn identical_plans_do_not_bump_generation() {
        let feed = fresh();
        feed.set_plans(vec![plan("83139", &["15"])], None, "http://example.invalid");
        let gen_after_first = feed.generation.load(Ordering::SeqCst);
        feed.set_plans(vec![plan("83139", &["15"])], None, "http://example.invalid");
        assert_eq!(feed.generation.load(Ordering::SeqCst), gen_after_first);
    }

    #[test]
    fn reordered_equivalent_plans_do_not_bump_generation() {
        let feed = fresh();
        feed.set_plans(
            vec![plan("83139", &["15", "14"]), plan("83141", &["16"])],
            None,
            "http://example.invalid",
        );
        let gen_after_first = feed.generation.load(Ordering::SeqCst);
        feed.set_plans(
            vec![plan("83141", &["16"]), plan("83139", &["14", "15"])],
            None,
            "http://example.invalid",
        );
        assert_eq!(feed.generation.load(Ordering::SeqCst), gen_after_first);
    }

    #[test]
    fn changed_plans_bump_generation() {
        let feed = fresh();
        feed.set_plans(vec![plan("83139", &["15"])], None, "http://example.invalid");
        let gen_after_first = feed.generation.load(Ordering::SeqCst);
        feed.set_plans(
            vec![plan("83139", &["15", "16"])],
            None,
            "http://example.invalid",
        );
        assert!(feed.generation.load(Ordering::SeqCst) > gen_after_first);
    }

    #[test]
    fn emptying_the_plan_still_bumps_generation() {
        let feed = fresh();
        feed.set_plans(vec![plan("83139", &["15"])], None, "http://example.invalid");
        let gen_after_first = feed.generation.load(Ordering::SeqCst);
        feed.set_plans(vec![], None, "http://example.invalid");
        assert!(feed.generation.load(Ordering::SeqCst) > gen_after_first);
    }

    #[test]
    fn items_for_unknown_stop_is_empty() {
        let feed = fresh();
        assert!(feed.items_for("nope").is_empty());
    }

    #[test]
    fn wait_first_times_out_without_a_snapshot() {
        let feed = fresh();
        assert!(!feed.wait_first(Duration::from_millis(50)));
    }

    #[test]
    fn wait_first_returns_true_once_signaled_from_another_thread() {
        let feed = fresh();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            feed.apply_update(StopUpdate {
                code: "83139".to_owned(),
                items: Vec::new(),
            });
        });
        assert!(feed.wait_first(Duration::from_secs(2)));
    }

    #[test]
    fn listener_is_called_on_snapshot_update() {
        let feed = fresh();
        let called = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&called);
        feed.add_listener(move || flag.store(true, Ordering::SeqCst));
        feed.apply_update(StopUpdate {
            code: "83139".to_owned(),
            items: Vec::new(),
        });
        assert!(called.load(Ordering::SeqCst));
    }
}
