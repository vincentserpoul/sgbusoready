//! `GET /v1/arrivals?stops=83139:15.14e,83141:16` — the live SSE arrivals
//! stream. One entry per bus stop code, its tracked service numbers joined
//! by `.`, entries joined by `,`. Every connection subscribes to the
//! refcounted [`crate::registry::Registry`] poller for each requested stop
//! and receives an `event: arrivals` frame only when that connection's own
//! filtered, minute-rendered payload for a stop changes.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::Deserialize;
use sgbr_core::lta::arrival::{ArrivalItem, arrival_items};
use sgbr_core::worker::protocol::StopUpdate;
use time::OffsetDateTime;
use time::macros::offset;
use tokio_stream::wrappers::WatchStream;
use tokio_stream::{Stream, StreamExt as _, StreamMap};

use crate::auth::Device;
use crate::error::ApiError;
use crate::registry::Subscription;
use crate::state::AppState;

/// SSE keep-alive cadence — matches what
/// `sgbr_core::worker::client::run_stream` expects (`: hb` roughly every
/// 25s).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);

const MIN_STOPS: usize = 1;
const MAX_STOPS: usize = 16;
const MAX_SERVICES: usize = 20;
const CODE_LEN: usize = 5;
const MAX_SVC_LEN: usize = 6;

/// One parsed `CODE:SVC.SVC` entry from the `stops` query parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopSpec {
    pub code: String,
    pub services: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct StopsQuery {
    stops: Option<String>,
}

/// Parse the `stops` query value: `stop *("," stop)`, `stop = CODE ":" SVC
/// *("." SVC)`. `CODE` is exactly 5 ASCII digits; `SVC` is 1-6 alphanumeric
/// chars. 1-16 stops, 1-20 services per stop. Any violation is a
/// `ApiError::BadRequest`.
pub fn parse_stops(q: &str) -> Result<Vec<StopSpec>, ApiError> {
    if q.trim().is_empty() {
        return Err(ApiError::BadRequest("stops must not be empty".to_owned()));
    }
    let parts: Vec<&str> = q.split(',').collect();
    if parts.len() < MIN_STOPS || parts.len() > MAX_STOPS {
        return Err(ApiError::BadRequest(format!(
            "stops must list between {MIN_STOPS} and {MAX_STOPS} stop entries"
        )));
    }
    parts.into_iter().map(parse_one_stop).collect()
}

fn parse_one_stop(part: &str) -> Result<StopSpec, ApiError> {
    let (code, rest) = part
        .split_once(':')
        .ok_or_else(|| ApiError::BadRequest(format!("malformed stop entry: {part}")))?;
    validate_code(code)?;
    let services: Vec<String> = rest.split('.').map(str::to_owned).collect();
    if services.len() > MAX_SERVICES {
        return Err(ApiError::BadRequest(format!(
            "stop {code} lists more than {MAX_SERVICES} services"
        )));
    }
    for svc in &services {
        validate_service(svc)?;
    }
    Ok(StopSpec {
        code: code.to_owned(),
        services,
    })
}

fn validate_code(code: &str) -> Result<(), ApiError> {
    if code.len() == CODE_LEN && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "stop code must be exactly {CODE_LEN} digits: {code}"
        )))
    }
}

fn validate_service(svc: &str) -> Result<(), ApiError> {
    if !svc.is_empty() && svc.len() <= MAX_SVC_LEN && svc.chars().all(|c| c.is_ascii_alphanumeric())
    {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "invalid service number: {svc:?}"
        )))
    }
}

/// `now`, rendered in Singapore time — matches the offset every other
/// `arrival_items` caller in the codebase uses (see `unix_to_sgt` in the
/// desktop/Android bridge and `sgbr::lib::now_sgt`), even though the minutes
/// computed are instant-based and offset-independent; this keeps the idiom
/// consistent across the worker and the app.
fn now_sgt() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(offset!(+8))
}

pub async fn arrivals(
    State(state): State<AppState>,
    _device: Device,
    Query(query): Query<StopsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let raw = query
        .stops
        .ok_or_else(|| ApiError::BadRequest("missing stops parameter".to_owned()))?;
    let specs = parse_stops(&raw)?;

    let registry = Arc::clone(state.registry());
    let subs: Vec<Subscription> = specs
        .iter()
        .map(|spec| registry.subscribe(&spec.code))
        .collect();

    let mut map: StreamMap<usize, WatchStream<_>> = StreamMap::new();
    for (i, sub) in subs.iter().enumerate() {
        map.insert(i, WatchStream::new(sub.rx.clone()));
    }

    let stream = async_stream::stream! {
        // Keep every subscription (and thus its registry refcount) alive for
        // exactly as long as this connection's stream is polled.
        let _subs = subs;
        let mut map = map;
        let mut last_sent: Vec<Option<Vec<ArrivalItem>>> = vec![None; specs.len()];
        while let Some((i, snap_opt)) = map.next().await {
            let Some(snapshot) = snap_opt else { continue };
            let Some(spec) = specs.get(i) else { continue };
            tracing::trace!(
                code = %snapshot.code,
                fetched_at_unix = snapshot.fetched_at_unix,
                "polled arrival snapshot"
            );
            let items = arrival_items(&spec.services, &snapshot.response, now_sgt());
            let Some(slot) = last_sent.get_mut(i) else { continue };
            if slot.as_ref() == Some(&items) {
                continue;
            }
            *slot = Some(items.clone());
            let update = StopUpdate { code: spec.code.clone(), items };
            match Event::default().event("arrivals").json_data(&update) {
                Ok(event) => yield Ok(event),
                Err(e) => tracing::warn!("failed to serialize arrivals event: {e}"),
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(HEARTBEAT_INTERVAL).text("hb")))
}

#[cfg(test)]
mod parse_tests {
    use super::{ApiError, StopSpec, parse_stops};

    fn is_bad_request(result: &Result<Vec<StopSpec>, ApiError>) -> bool {
        matches!(result, Err(ApiError::BadRequest(_)))
    }

    #[test]
    fn parses_single_stop_single_service() {
        let specs = parse_stops("83139:15").expect("valid");
        assert_eq!(
            specs,
            vec![StopSpec {
                code: "83139".to_owned(),
                services: vec!["15".to_owned()],
            }]
        );
    }

    #[test]
    fn parses_multi_stop_multi_service() {
        let specs = parse_stops("83139:15.14e,83141:16").expect("valid");
        assert_eq!(
            specs,
            vec![
                StopSpec {
                    code: "83139".to_owned(),
                    services: vec!["15".to_owned(), "14e".to_owned()],
                },
                StopSpec {
                    code: "83141".to_owned(),
                    services: vec!["16".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn rejects_empty_query() {
        assert!(is_bad_request(&parse_stops("")));
        assert!(is_bad_request(&parse_stops("   ")));
    }

    #[test]
    fn rejects_short_code() {
        assert!(is_bad_request(&parse_stops("8313:15")));
    }

    #[test]
    fn rejects_long_code() {
        assert!(is_bad_request(&parse_stops("831399:15")));
    }

    #[test]
    fn rejects_non_digit_code() {
        assert!(is_bad_request(&parse_stops("8313a:15")));
    }

    #[test]
    fn rejects_zero_services() {
        assert!(is_bad_request(&parse_stops("83139:")));
    }

    #[test]
    fn rejects_more_than_16_stops() {
        let q = (0..17).map(|_| "83139:15").collect::<Vec<_>>().join(",");
        assert!(is_bad_request(&parse_stops(&q)));
    }

    #[test]
    fn accepts_exactly_16_stops() {
        let q = (0..16).map(|_| "83139:15").collect::<Vec<_>>().join(",");
        parse_stops(&q).expect("16 stops should be accepted");
    }

    #[test]
    fn rejects_more_than_20_services() {
        let services = (0..21).map(|i| i.to_string()).collect::<Vec<_>>().join(".");
        let q = format!("83139:{services}");
        assert!(is_bad_request(&parse_stops(&q)));
    }

    #[test]
    fn accepts_exactly_20_services() {
        let services = (0..20).map(|i| i.to_string()).collect::<Vec<_>>().join(".");
        let q = format!("83139:{services}");
        parse_stops(&q).expect("20 services should be accepted");
    }

    #[test]
    fn rejects_empty_service_in_list() {
        assert!(is_bad_request(&parse_stops("83139:15..14e")));
    }

    #[test]
    fn rejects_service_longer_than_6_chars() {
        assert!(is_bad_request(&parse_stops("83139:1234567")));
    }

    #[test]
    fn accepts_service_exactly_6_chars() {
        parse_stops("83139:123456").expect("6-char service should be accepted");
    }

    #[test]
    fn rejects_junk_separators() {
        assert!(is_bad_request(&parse_stops("83139;15")));
        assert!(is_bad_request(&parse_stops("83139:15,")));
        assert!(is_bad_request(&parse_stops(",83139:15")));
    }
}

#[cfg(test)]
mod handler_tests {
    use super::super::router;
    use crate::catalog::CatalogCache;
    use crate::config::Config;
    use crate::registry::{ArrivalFetcher, Registry};
    use crate::state::AppState;
    use crate::store::DeviceStore;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use sgbr_core::error::CoreError;
    use sgbr_core::lta::model::BusArrivalResponse;
    use sgbr_core::worker::auth::generate_keypair;
    use sgbr_core::worker::protocol::StopUpdate;
    use sgbr_core::worker::sse::SseParser;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tower::ServiceExt as _;

    /// Fetcher that replays a scripted sequence of responses (looping the
    /// last one once exhausted), and counts calls per stop.
    #[derive(Debug)]
    struct ScriptedFetcher {
        script: Mutex<Vec<BusArrivalResponse>>,
        calls: AtomicUsize,
    }

    impl ScriptedFetcher {
        fn new(script: Vec<BusArrivalResponse>) -> Self {
            Self {
                script: Mutex::new(script),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl ArrivalFetcher for ScriptedFetcher {
        fn fetch(&self, _stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
                    .unwrap_or_else(|| response_with_arrival("15", 5)))
            }
        }
    }

    fn response_with_arrival(service_no: &str, minutes: i64) -> BusArrivalResponse {
        let now = time::OffsetDateTime::now_utc();
        // +30s buffer so the whole-minute truncation in `minutes_until`
        // stays stable across the (small, but real) delay between building
        // this fixture and the handler evaluating it against a later `now`.
        let eta = now + time::Duration::minutes(minutes) + time::Duration::seconds(30);
        let eta_str = eta
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let body = format!(
            r#"{{"BusStopCode":"83139","Services":[{{"ServiceNo":"{service_no}","NextBus":{{"EstimatedArrival":"{eta_str}"}},"NextBus2":{{"EstimatedArrival":""}},"NextBus3":{{"EstimatedArrival":""}}}}]}}"#
        );
        serde_json::from_str(&body).expect("sample response must parse")
    }

    fn tmp_state_with_fetcher(
        name: &str,
        fetcher: Arc<dyn ArrivalFetcher>,
    ) -> (AppState, String, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_arrivals_test_{}_{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        let store = DeviceStore::load_or_default(dir.join("store.json")).expect("load store");
        let invite = store.mint_invite().expect("mint invite");
        let (_seed, pubkey) = generate_keypair().expect("keygen");
        let (_device_id, token) = store
            .register_device(&invite.code, &pubkey)
            .expect("register device");
        let config = Config {
            lta_account_key: "unused".to_owned(),
            bind: "127.0.0.1:0".to_owned(),
            data_dir: dir.clone(),
            admin_token: "test-admin-token".to_owned(),
            poll_secs: 15,
        };
        // Fast poll interval: these tests advance real time a little (via
        // short sleeps) rather than `tokio::time::pause`, since they drive
        // the handler through the real router/body-stream machinery.
        let registry = Registry::new(fetcher, Duration::from_millis(20));
        let state = AppState::new(
            Arc::new(config),
            Arc::new(store),
            Arc::new(CatalogCache::empty()),
            registry,
        );
        (state, token, dir)
    }

    #[tokio::test]
    async fn missing_bearer_is_401() {
        let (state, _token, dir) = tmp_state_with_fetcher(
            "no-auth",
            Arc::new(ScriptedFetcher::new(vec![response_with_arrival("15", 5)])),
        );
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/arrivals?stops=83139:15")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_stops_param_is_400_with_envelope() {
        let (state, token, dir) = tmp_state_with_fetcher(
            "no-stops",
            Arc::new(ScriptedFetcher::new(vec![response_with_arrival("15", 5)])),
        );
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/arrivals")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("parse body");
        assert!(body.get("error").and_then(|v| v.as_str()).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn malformed_stops_param_is_400() {
        let (state, token, dir) = tmp_state_with_fetcher(
            "bad-stops",
            Arc::new(ScriptedFetcher::new(vec![response_with_arrival("15", 5)])),
        );
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/arrivals?stops=nope")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read SSE frames off `body` until `parser` dispatches `want` many
    /// `arrivals` events, or `max_reads` chunks are exhausted (bounds the
    /// test instead of hanging forever on a plumbing regression).
    async fn read_arrivals_events(
        body: &mut axum::body::Body,
        want: usize,
        max_reads: usize,
    ) -> Vec<StopUpdate> {
        let mut parser = SseParser::default();
        let mut out = Vec::new();
        let mut reads = 0;
        while out.len() < want && reads < max_reads {
            reads += 1;
            let Some(frame) = body.frame().await else {
                break;
            };
            let Ok(frame) = frame else { break };
            let Ok(data) = frame.into_data() else {
                continue;
            };
            let text = String::from_utf8_lossy(&data);
            for line in text.split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                if let Some(event) = parser.push_line(line)
                    && event.event == "arrivals"
                    && let Ok(update) = serde_json::from_str::<StopUpdate>(&event.data)
                {
                    out.push(update);
                }
            }
        }
        out
    }

    #[tokio::test]
    async fn identical_responses_yield_one_event_then_a_change_yields_a_second() {
        let fetcher = Arc::new(ScriptedFetcher::new(vec![
            response_with_arrival("15", 5),
            response_with_arrival("15", 5),
            response_with_arrival("15", 9),
            response_with_arrival("15", 9),
            response_with_arrival("15", 9),
        ]));
        let (state, token, dir) = tmp_state_with_fetcher("change-gate", fetcher);

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/arrivals?stops=83139:15")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();

        // Two identical polls collapse into exactly one `arrivals` event,
        // then a changed poll produces a second.
        let events = read_arrivals_events(&mut body, 2, 200).await;
        assert_eq!(
            events.len(),
            2,
            "expected exactly 2 change events, got {events:?}"
        );
        assert_eq!(events[0].items.first().map(|i| i.minutes), Some(5));
        assert_eq!(events[1].items.first().map(|i| i.minutes), Some(9));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
