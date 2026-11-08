//! Blocking client for the worker's HTTP API — the *only* network surface the
//! shipped app has; it never touches `DataMall` directly. Covered end-to-end
//! against the real router in `crates/sgbr-worker/tests/client_stream.rs`.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use time::OffsetDateTime;
use ureq::Agent;

use crate::bus_catalog::model::{BusCatalog, BusStop};
use crate::bus_catalog::parse::assemble_catalog;
use crate::error::CoreError;
use crate::worker::auth::{generate_keypair, sign_registration};
use crate::worker::credentials::WorkerCredentials;
use crate::worker::protocol::{RegisterRequest, RegisterResponse, StopUpdate};
use crate::worker::sse::SseParser;

/// Base URL + bearer token for authenticated worker calls.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub base_url: String,
    pub token: String,
}

/// Global timeout for the one-shot registration call.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(15);

/// Global timeout for the (single, non-paginated) catalog fetch.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

/// Response body size cap for the catalog endpoints — the full stop/route
/// datasets are bigger than ureq's default 10 MiB limit.
const CATALOG_BODY_LIMIT: u64 = 32 * 1024 * 1024;

/// TCP/TLS connect cap for the SSE stream.
const STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on waiting for the stream's response headers.
const STREAM_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);

/// Idle cap on each transport read of the stream. The worker sends a `: hb`
/// comment every 25s, so a healthy connection never trips this while a
/// half-open one is detected within a minute. Deliberately *not*
/// `timeout_recv_body`, which in ureq 3 is a total budget from the headers
/// onward and would sever every stream at a fixed age.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_mins(1);

/// Register a new device against `invite_code`: generate a fresh Ed25519
/// keypair locally, sign the fixed registration message, and exchange it for
/// a bearer token via `POST /v1/devices`.
///
/// # Errors
/// [`CoreError::Auth`] if the worker rejects the invite/signature (401/403),
/// [`CoreError::Crypto`] if local keygen fails, [`CoreError::Http`] /
/// [`CoreError::Parse`] on other transport/body failures.
pub fn register(base_url: &str, invite_code: &str) -> Result<WorkerCredentials, CoreError> {
    let (seed, pubkey) = generate_keypair()?;
    let signature = sign_registration(&seed, invite_code);
    let agent = Agent::new_with_config(
        Agent::config_builder()
            .timeout_global(Some(REGISTER_TIMEOUT))
            .build(),
    );
    let request = RegisterRequest {
        invite_code: invite_code.to_owned(),
        pubkey: pubkey.clone(),
        signature,
    };
    let outcome = agent
        .post(format!("{base_url}/v1/devices"))
        .send_json(&request);
    let mut response = match outcome {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(401 | 403)) => {
            return Err(CoreError::Auth("registration rejected".to_owned()));
        }
        Err(e) => return Err(CoreError::Http(e.to_string())),
    };
    let body: RegisterResponse = response
        .body_mut()
        .read_json()
        .map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(WorkerCredentials {
        token: body.token,
        device_id: body.device_id,
        secret_seed_b64: BASE64.encode(seed),
        pubkey_b64: pubkey,
    })
}

/// Fetch the full bus catalog from the worker (`GET /v1/bus-stops` +
/// `GET /v1/bus-routes`), assembled the same way the direct-`DataMall` fetch
/// assembles it (`bus_catalog::parse::assemble_catalog`). `now` stamps
/// `fetched_at_unix`.
///
/// # Errors
/// [`CoreError::Http`] on transport/status failure, [`CoreError::Parse`] if a
/// response body isn't the expected JSON shape.
pub fn fetch_catalog(cfg: &WorkerConfig, now: OffsetDateTime) -> Result<BusCatalog, CoreError> {
    let agent = Agent::new_with_config(
        Agent::config_builder()
            .timeout_global(Some(CATALOG_TIMEOUT))
            .build(),
    );
    let stops: Vec<BusStop> = get_json(&agent, cfg, "/v1/bus-stops")?;
    let pairs: Vec<(String, String)> = get_json(&agent, cfg, "/v1/bus-routes")?;
    Ok(assemble_catalog(stops, pairs, now))
}

fn get_json<T: serde::de::DeserializeOwned>(
    agent: &Agent,
    cfg: &WorkerConfig,
    path: &str,
) -> Result<T, CoreError> {
    let url = format!("{}{path}", cfg.base_url);
    agent
        .get(&url)
        .header("Authorization", format!("Bearer {}", cfg.token))
        .call()
        .map_err(|e| CoreError::Http(e.to_string()))?
        .body_mut()
        .with_config()
        .limit(CATALOG_BODY_LIMIT)
        .read_json()
        .map_err(|e| CoreError::Parse(e.to_string()))
}

/// Why [`run_stream`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamExit {
    /// The active stop/service plan changed mid-stream (`generation` moved
    /// past `my_gen`); the caller should recompute `query` and reconnect.
    PlanChanged,
    /// The connection dropped (timeout, EOF, or I/O error); the caller should
    /// back off and reconnect with the same `query`.
    Disconnected,
    /// The worker rejected the bearer token (`401`/`403`): the device was
    /// revoked or never registered. Reconnecting with the same credentials
    /// cannot help; the caller should stop and re-register.
    Unauthorized,
}

/// Hold one SSE connection open against `GET /v1/arrivals?stops={query}`,
/// dispatching every `event: arrivals` frame to `on_update`, until the
/// connection drops or `generation` moves past `my_gen` (the caller signals a
/// plan change by bumping the shared counter — checked after every line,
/// including comments/heartbeats).
///
/// Deliberately does **not** loop or retry: reconnect/backoff policy lives in
/// the caller, which can build a fresh `query` between attempts. Every way
/// the connection can end is a [`StreamExit`] (connection and I/O failures
/// are `Disconnected`), so the caller has exactly one match to write.
pub fn run_stream(
    cfg: &WorkerConfig,
    query: &str,
    generation: &AtomicU64,
    my_gen: u64,
    mut on_update: impl FnMut(StopUpdate),
) -> StreamExit {
    let agent = Agent::new_with_config(
        Agent::config_builder()
            .timeout_connect(Some(STREAM_CONNECT_TIMEOUT))
            .timeout_recv_response(Some(STREAM_RESPONSE_TIMEOUT))
            .timeout_per_read(Some(STREAM_IDLE_TIMEOUT))
            .build(),
    );
    let url = format!("{}/v1/arrivals?stops={query}", cfg.base_url);
    let outcome = agent
        .get(&url)
        .header("Authorization", format!("Bearer {}", cfg.token))
        .call();
    let mut response = match outcome {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(401 | 403)) => return StreamExit::Unauthorized,
        Err(_) => return StreamExit::Disconnected,
    };
    let mut reader = BufReader::new(response.body_mut().as_reader());
    let mut parser = SseParser::default();
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(bytes_read) = reader.read_line(&mut line) else {
            return StreamExit::Disconnected;
        };
        if bytes_read == 0 {
            return StreamExit::Disconnected;
        }
        let trimmed = line.strip_suffix('\n').unwrap_or(&line);
        if let Some(event) = parser.push_line(trimmed)
            && event.event == "arrivals"
            && let Ok(update) = serde_json::from_str::<StopUpdate>(&event.data)
        {
            on_update(update);
        }
        if generation.load(Ordering::Relaxed) != my_gen {
            return StreamExit::PlanChanged;
        }
    }
}
