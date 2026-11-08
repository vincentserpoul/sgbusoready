//! Integration test: the real blocking device client
//! (`sgbr_core::worker::client`) driven against the real `sgbr-worker` axum
//! router over a loopback TCP socket — `axum::serve` on an ephemeral port,
//! `run_stream`/`register` on a real `std::thread`, nothing shortcuts the
//! HTTP boundary. Arrival data is supplied by a mock `ArrivalFetcher`, so no
//! test here ever touches the network beyond loopback.
//!
//! Every blocking wait (client-thread join, update receive) is capped by a
//! timeout so a regression hangs the *test*, not the CI run.

// This whole binary is already only ever compiled under `--test` (integration
// tests are inherently `cfg(test)`), but clippy's `*-in-tests` allowances
// (see clippy.toml) key off an explicit `#[cfg(test)]` ancestor, not the
// crate's actual compilation mode — without this, `expect`/`panic!` in plain
// helper functions below (not themselves `#[tokio::test]`) get denied same as
// production code. Redundant at compile time; required for clippy.
#![cfg(test)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use sgbr_core::error::CoreError;
use sgbr_core::lta::model::BusArrivalResponse;
use sgbr_core::worker::auth::generate_keypair;
use sgbr_core::worker::client::{self, StreamExit, WorkerConfig};
use sgbr_core::worker::protocol::StopUpdate;
use sgbr_worker::catalog::CatalogCache;
use sgbr_worker::config::Config;
use sgbr_worker::registry::{ArrivalFetcher, Registry};
use sgbr_worker::routes::router;
use sgbr_worker::state::AppState;
use sgbr_worker::store::DeviceStore;
use tokio::net::TcpListener;

/// Cap on joining the client's `std::thread` (register or `run_stream`) —
/// a hang here fails the test instead of wedging the CI run.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap on waiting for one `StopUpdate` off the `on_update` channel.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll interval used by every test registry: fast enough to keep total
/// suite runtime modest while still resembling real change-gated polling.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------
// Fixture fetchers
// ---------------------------------------------------------------------

/// Fetcher that replays a scripted sequence of responses (looping the last
/// one once exhausted), counting calls. A local copy of the pattern
/// `sgbr-worker`'s own `routes::arrivals` handler tests use — `registry.rs`'s
/// `MockFetcher` is `#[cfg(test)]`-private to that module's unit tests and
/// invisible to this external integration-test crate; duplicating ~20 lines
/// here beats widening the crate's public API for test-only code.
#[derive(Debug)]
struct ScriptedFetcher {
    script: Mutex<Vec<BusArrivalResponse>>,
}

impl ScriptedFetcher {
    fn new(script: Vec<BusArrivalResponse>) -> Self {
        Self {
            script: Mutex::new(script),
        }
    }
}

impl ArrivalFetcher for ScriptedFetcher {
    fn fetch(&self, _stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
        let mut script = self
            .script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if script.len() > 1 {
            Ok(script.remove(0))
        } else {
            Ok(script
                .first()
                .cloned()
                .unwrap_or_else(|| response_with_arrival("83139", "15", 5)))
        }
    }
}

/// Fetcher whose response depends on which stop code the registry asks for —
/// needed for the multi-stop test to prove each stream entry carries the
/// right per-stop items, not a shared/mixed-up snapshot.
#[derive(Debug)]
struct PerStopFetcher;

impl ArrivalFetcher for PerStopFetcher {
    fn fetch(&self, stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
        let minutes = if stop_code == "83139" { 5 } else { 9 };
        Ok(response_with_arrival(stop_code, "15", minutes))
    }
}

/// Build a single-service `BusArrivalResponse` whose `NextBus` arrives in
/// `minutes`. The +30s buffer keeps the whole-minute truncation in
/// `minutes_until` stable across the (small, real) delay between building
/// this fixture and the handler evaluating it against a later `now`.
fn response_with_arrival(stop_code: &str, service_no: &str, minutes: i64) -> BusArrivalResponse {
    let now = time::OffsetDateTime::now_utc();
    let eta = now + time::Duration::minutes(minutes) + time::Duration::seconds(30);
    let eta_str = eta
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let body = format!(
        r#"{{"BusStopCode":"{stop_code}","Services":[{{"ServiceNo":"{service_no}","NextBus":{{"EstimatedArrival":"{eta_str}"}},"NextBus2":{{"EstimatedArrival":""}},"NextBus3":{{"EstimatedArrival":""}}}}]}}"#
    );
    serde_json::from_str(&body).expect("sample response must parse")
}

// ---------------------------------------------------------------------
// Server / state setup
// ---------------------------------------------------------------------

/// Build an `AppState` backed by a tempdir store, an empty catalog, and
/// `fetcher` behind a fresh `Registry`. Mirrors the setup every worker route
/// test in `crates/sgbr-worker/src/routes/arrivals.rs` already uses.
fn build_state(name: &str, fetcher: Arc<dyn ArrivalFetcher>) -> (AppState, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "sgbr_worker_client_stream_test_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tmp dir");
    let store = DeviceStore::load_or_default(dir.join("store.json")).expect("load store");
    let config = Config {
        lta_account_key: "unused".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.clone(),
        admin_token: "test-admin-token".to_owned(),
        poll_secs: 15,
    };
    let registry = Registry::new(fetcher, POLL_INTERVAL);
    let state = AppState::new(
        Arc::new(config),
        Arc::new(store),
        Arc::new(CatalogCache::default()),
        registry,
    );
    (state, dir)
}

/// Mint an invite and register a device directly against the store (bypasses
/// the wire — scenario 1 below is what proves `register()` itself works
/// end-to-end; every other scenario just needs *a* valid token to test the
/// arrivals stream).
fn register_device_directly(state: &AppState) -> String {
    let invite = state.store().mint_invite().expect("mint invite");
    let (_seed, pubkey) = generate_keypair().expect("keygen");
    let (_device_id, token) = state
        .store()
        .register_device(&invite.code, &pubkey)
        .expect("register device");
    token
}

/// Bind an ephemeral loopback port and serve `state`'s router on the calling
/// test's own tokio runtime. Returns the base URL and the accept-loop's
/// `JoinHandle` (`.abort()` it at teardown — note this only stops *new*
/// connections; see `KillableServer` below for genuinely severing an
/// already-open one).
async fn spawn_server(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });
    (format!("http://{addr}"), handle)
}

/// A server run on its own dedicated multi-thread tokio runtime, so it can be
/// killed outright. `axum::serve` spawns one *detached* tokio task per
/// accepted connection internally (see `axum::serve::handle_connection` in
/// axum 0.8's `src/serve/mod.rs`) — aborting only the accept-loop's own
/// `JoinHandle` (as `spawn_server` returns) stops new connections but does
/// **not** close an already-open SSE stream, since that connection's task was
/// never a child of the aborted one. Tearing down the whole runtime does:
/// every task scheduled on it, detached or not, gets dropped, closing every
/// socket — the closest in-process analogue to the worker process dying
/// mid-stream.
struct KillableServer {
    runtime: Option<tokio::runtime::Runtime>,
}

impl KillableServer {
    fn start(state: AppState) -> (String, Self) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build dedicated server runtime");
        let (addr_tx, addr_rx) = mpsc::channel();
        runtime.spawn(async move {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind ephemeral port");
            let addr = listener.local_addr().expect("local addr");
            let _ = addr_tx.send(addr);
            let _ = axum::serve(listener, router(state)).await;
        });
        let addr = addr_rx
            .recv_timeout(JOIN_TIMEOUT)
            .expect("server did not report its bound address in time");
        (
            format!("http://{addr}"),
            Self {
                runtime: Some(runtime),
            },
        )
    }

    /// Forcibly tear down the whole runtime, closing every open connection.
    fn kill(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(JOIN_TIMEOUT);
        }
    }
}

impl Drop for KillableServer {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Kill `server` from async test code without tripping tokio's "cannot drop
/// a runtime from within an asynchronous context" guard: `Runtime::
/// shutdown_timeout` is a blocking call, so it has to happen on tokio's
/// blocking pool, not directly on this test's own runtime worker thread.
async fn kill_server(mut server: KillableServer) {
    tokio::task::spawn_blocking(move || server.kill())
        .await
        .expect("kill task itself panicked");
}

// ---------------------------------------------------------------------
// Client-thread + channel plumbing
// ---------------------------------------------------------------------

/// Run `run_stream` on a real OS thread — the actual blocking `ureq` client
/// under test — forwarding every `on_update` call onto `updates_tx`.
fn spawn_client_thread(
    cfg: WorkerConfig,
    query: String,
    generation: Arc<AtomicU64>,
    my_gen: u64,
    updates_tx: mpsc::Sender<StopUpdate>,
) -> std::thread::JoinHandle<StreamExit> {
    std::thread::spawn(move || {
        client::run_stream(&cfg, &query, &generation, my_gen, |update| {
            let _ = updates_tx.send(update);
        })
    })
}

/// Join a `std::thread` from async test code without blocking a tokio worker
/// thread: `JoinHandle::join()` itself runs on tokio's blocking pool, and the
/// whole thing is capped by `JOIN_TIMEOUT` so a client hang fails the test.
async fn join_with_timeout<T: Send + 'static>(handle: std::thread::JoinHandle<T>) -> T {
    tokio::time::timeout(
        JOIN_TIMEOUT,
        tokio::task::spawn_blocking(move || handle.join()),
    )
    .await
    .expect("client thread did not finish within the timeout")
    .expect("spawn_blocking task itself panicked")
    .expect("client thread panicked")
}

/// Wait for the next `StopUpdate` without ever blocking the tokio runtime
/// thread synchronously: a non-blocking `try_recv` poll loop yielding via
/// `tokio::time::sleep` between attempts, capped by `timeout`.
async fn recv_update(rx: &mpsc::Receiver<StopUpdate>, timeout: Duration) -> StopUpdate {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            Ok(update) => return update,
            Err(mpsc::TryRecvError::Empty) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "timed out waiting for an arrivals update"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                panic!("update channel disconnected before an expected update arrived");
            }
        }
    }
}

/// Assert no further update arrives within `window` — used to prove the
/// change gate collapsed identical polls rather than merely being slow.
async fn assert_no_update_within(rx: &mpsc::Receiver<StopUpdate>, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    while tokio::time::Instant::now() < deadline {
        if let Ok(update) = rx.try_recv() {
            panic!("expected no further update, got {update:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------
// 1. Register end-to-end
// ---------------------------------------------------------------------

#[tokio::test]
async fn register_end_to_end_then_token_authenticates() {
    let fetcher: Arc<dyn ArrivalFetcher> = Arc::new(ScriptedFetcher::new(vec![]));
    let (state, dir) = build_state("register-happy", fetcher);
    let invite = state.store().mint_invite().expect("mint invite");
    let (base_url, server) = spawn_server(state).await;

    let register_handle = {
        let base_url = base_url.clone();
        let code = invite.code.clone();
        std::thread::spawn(move || client::register(&base_url, &code))
    };
    let creds = join_with_timeout(register_handle)
        .await
        .expect("registration against a freshly minted invite must succeed");
    assert_eq!(creds.token.len(), 64);
    assert_eq!(creds.device_id.len(), 16);

    // The returned token authenticates a follow-up request: an empty
    // catalog is a 503 (catalog not ready), not a 401 — proving auth itself
    // passed, over the real wire.
    let auth_check_handle = {
        let base_url = base_url.clone();
        let token = creds.token.clone();
        std::thread::spawn(move || -> Result<u16, String> {
            let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
            match agent
                .get(format!("{base_url}/v1/bus-stops"))
                .header("Authorization", format!("Bearer {token}"))
                .call()
            {
                Ok(response) => Ok(response.status().as_u16()),
                Err(ureq::Error::StatusCode(code)) => Ok(code),
                Err(e) => Err(e.to_string()),
            }
        })
    };
    let status = join_with_timeout(auth_check_handle)
        .await
        .expect("follow-up request should complete");
    assert_eq!(
        status, 503,
        "empty catalog => 503 SERVICE_UNAVAILABLE, not 401 — proves the token authenticated"
    );

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn register_with_unknown_invite_is_auth_error() {
    let fetcher: Arc<dyn ArrivalFetcher> = Arc::new(ScriptedFetcher::new(vec![]));
    let (state, dir) = build_state("register-bad-invite", fetcher);
    let (base_url, server) = spawn_server(state).await;

    let handle = std::thread::spawn(move || client::register(&base_url, "NOPE-CODE"));
    let result = join_with_timeout(handle).await;
    match result {
        Err(CoreError::Auth(_)) => {}
        other => panic!("expected Err(CoreError::Auth(_)), got {other:?}"),
    }

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// 2. Stream happy path
// ---------------------------------------------------------------------

#[tokio::test]
async fn stream_happy_path_delivers_cached_snapshot_immediately() {
    let fetcher: Arc<dyn ArrivalFetcher> =
        Arc::new(ScriptedFetcher::new(vec![response_with_arrival(
            "83139", "15", 5,
        )]));
    let (state, dir) = build_state("stream-happy", fetcher);
    let token = register_device_directly(&state);
    // `KillableServer` (not `spawn_server`): this fetcher settles on one
    // fixture forever, so once we're done asserting there is no further SSE
    // traffic to trip a generation check on — the clean way to end the
    // stream here is a hard server kill (covered on its own merits by
    // scenario 6 below), not a generation bump that would otherwise sit
    // blocked until the 25s heartbeat.
    let (base_url, server) = KillableServer::start(state);

    let generation = Arc::new(AtomicU64::new(0));
    let cfg = WorkerConfig { base_url, token };
    let (updates_tx, updates_rx) = mpsc::channel();
    let handle = spawn_client_thread(
        cfg,
        "83139:15".to_owned(),
        Arc::clone(&generation),
        0,
        updates_tx,
    );

    let update = recv_update(&updates_rx, RECV_TIMEOUT).await;
    assert_eq!(update.code, "83139");
    assert_eq!(update.items.len(), 1);
    assert_eq!(update.items[0].minutes, 5);
    assert_eq!(update.items[0].buses, vec!["15".to_owned()]);

    kill_server(server).await;
    let exit = join_with_timeout(handle).await;
    assert_eq!(exit, StreamExit::Disconnected);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// 3. Change gate over the wire
// ---------------------------------------------------------------------

#[tokio::test]
async fn change_gate_over_the_wire_collapses_identical_polls() {
    let fetcher: Arc<dyn ArrivalFetcher> = Arc::new(ScriptedFetcher::new(vec![
        response_with_arrival("83139", "15", 5),
        response_with_arrival("83139", "15", 5),
        response_with_arrival("83139", "15", 9),
        response_with_arrival("83139", "15", 9),
        response_with_arrival("83139", "15", 9),
    ]));
    let (state, dir) = build_state("change-gate", fetcher);
    let token = register_device_directly(&state);
    // `KillableServer`, not `spawn_server`: see the comment in
    // `stream_happy_path_delivers_cached_snapshot_immediately` — this
    // fetcher settles on a steady value, so a hard kill (not a generation
    // bump, which needs live SSE traffic to be checked promptly) is the
    // clean way to end the stream once we're done asserting.
    let (base_url, server) = KillableServer::start(state);

    let generation = Arc::new(AtomicU64::new(0));
    let cfg = WorkerConfig { base_url, token };
    let (updates_tx, updates_rx) = mpsc::channel();
    let handle = spawn_client_thread(
        cfg,
        "83139:15".to_owned(),
        Arc::clone(&generation),
        0,
        updates_tx,
    );

    let first = recv_update(&updates_rx, RECV_TIMEOUT).await;
    assert_eq!(first.items.first().map(|i| i.minutes), Some(5));
    let second = recv_update(&updates_rx, RECV_TIMEOUT).await;
    assert_eq!(second.items.first().map(|i| i.minutes), Some(9));

    // Two identical (repeat) polls collapsed into the single `first` event
    // above; the scripted fetcher then holds steady on the `9`-minute
    // response, so no third change event should ever arrive.
    assert_no_update_within(&updates_rx, Duration::from_millis(300)).await;

    kill_server(server).await;
    let exit = join_with_timeout(handle).await;
    assert_eq!(exit, StreamExit::Disconnected);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// 4. Auth failure
// ---------------------------------------------------------------------

#[tokio::test]
async fn stream_with_invalid_token_does_not_hang_or_panic() {
    let fetcher: Arc<dyn ArrivalFetcher> =
        Arc::new(ScriptedFetcher::new(vec![response_with_arrival(
            "83139", "15", 5,
        )]));
    let (state, dir) = build_state("auth-fail", fetcher);
    let (base_url, server) = spawn_server(state).await;

    let generation = Arc::new(AtomicU64::new(0));
    let cfg = WorkerConfig {
        base_url,
        token: "not-a-real-token".to_owned(),
    };
    let (updates_tx, _updates_rx) = mpsc::channel();
    let handle = spawn_client_thread(
        cfg,
        "83139:15".to_owned(),
        Arc::clone(&generation),
        0,
        updates_tx,
    );

    // A 401 must be distinguishable from a dropped connection: the feed stops
    // retrying on `Unauthorized` (a revoked token can never recover by
    // reconnecting) but backs off and retries on `Disconnected`.
    assert_eq!(join_with_timeout(handle).await, StreamExit::Unauthorized);

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// 5. Generation abort
// ---------------------------------------------------------------------

#[tokio::test]
async fn generation_bump_aborts_stream_promptly() {
    // A fetcher whose ETA advances on every poll, so every ~50ms tick
    // produces a genuine change (and thus a fresh SSE frame) — this drives
    // promptness off real data events rather than the server's 25s
    // heartbeat, per the brief.
    let script: Vec<BusArrivalResponse> = (0..100)
        .map(|i| response_with_arrival("83139", "15", 5 + i))
        .collect();
    let fetcher: Arc<dyn ArrivalFetcher> = Arc::new(ScriptedFetcher::new(script));
    let (state, dir) = build_state("gen-abort", fetcher);
    let token = register_device_directly(&state);
    let (base_url, server) = spawn_server(state).await;

    let generation = Arc::new(AtomicU64::new(0));
    let cfg = WorkerConfig { base_url, token };
    let (updates_tx, updates_rx) = mpsc::channel();
    let handle = spawn_client_thread(
        cfg,
        "83139:15".to_owned(),
        Arc::clone(&generation),
        0,
        updates_tx,
    );

    // Prove the stream is actually live before asking it to stop.
    let _first = recv_update(&updates_rx, RECV_TIMEOUT).await;

    let started = tokio::time::Instant::now();
    generation.fetch_add(1, Ordering::SeqCst);
    let exit = join_with_timeout(handle).await;
    assert_eq!(exit, StreamExit::PlanChanged);
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "generation abort took {elapsed:?} — expected a couple of ~{POLL_INTERVAL:?} poll intervals"
    );

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// 6. Server-drop reconnect signal
// ---------------------------------------------------------------------

#[tokio::test]
async fn server_drop_mid_stream_yields_disconnected_not_a_hang() {
    let fetcher: Arc<dyn ArrivalFetcher> =
        Arc::new(ScriptedFetcher::new(vec![response_with_arrival(
            "83139", "15", 5,
        )]));
    let (state, dir) = build_state("server-drop", fetcher);
    let token = register_device_directly(&state);
    let (base_url, server) = KillableServer::start(state);

    let generation = Arc::new(AtomicU64::new(0));
    let cfg = WorkerConfig { base_url, token };
    let (updates_tx, updates_rx) = mpsc::channel();
    let handle = spawn_client_thread(
        cfg,
        "83139:15".to_owned(),
        Arc::clone(&generation),
        0,
        updates_tx,
    );

    // Prove the connection is actually live before killing the server.
    let _first = recv_update(&updates_rx, RECV_TIMEOUT).await;

    kill_server(server).await;

    assert_eq!(join_with_timeout(handle).await, StreamExit::Disconnected);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// 7. Multi-stop
// ---------------------------------------------------------------------

#[tokio::test]
async fn multi_stop_query_delivers_updates_for_both_stops() {
    let fetcher: Arc<dyn ArrivalFetcher> = Arc::new(PerStopFetcher);
    let (state, dir) = build_state("multi-stop", fetcher);
    let token = register_device_directly(&state);
    // `KillableServer`, not `spawn_server`: `PerStopFetcher` returns a fixed
    // value per stop forever, so — as in the happy-path and change-gate
    // tests above — a hard kill (not a generation bump) is the clean way to
    // end the stream once we're done asserting.
    let (base_url, server) = KillableServer::start(state);

    let generation = Arc::new(AtomicU64::new(0));
    let cfg = WorkerConfig { base_url, token };
    let (updates_tx, updates_rx) = mpsc::channel();
    let handle = spawn_client_thread(
        cfg,
        "83139:15,83141:15".to_owned(),
        Arc::clone(&generation),
        0,
        updates_tx,
    );

    let first = recv_update(&updates_rx, RECV_TIMEOUT).await;
    let second = recv_update(&updates_rx, RECV_TIMEOUT).await;
    let mut by_code: HashMap<String, StopUpdate> = HashMap::new();
    by_code.insert(first.code.clone(), first);
    by_code.insert(second.code.clone(), second);

    let a = by_code.get("83139").expect("an update for stop 83139");
    assert_eq!(a.items.first().map(|i| i.minutes), Some(5));
    let b = by_code.get("83141").expect("an update for stop 83141");
    assert_eq!(b.items.first().map(|i| i.minutes), Some(9));

    kill_server(server).await;
    let exit = join_with_timeout(handle).await;
    assert_eq!(exit, StreamExit::Disconnected);

    let _ = std::fs::remove_dir_all(&dir);
}
