//! `GET /v1/bus-stops`, `GET /v1/bus-routes` — bearer-authenticated proxy for
//! the cached LTA catalog. Devices never talk to `DataMall` directly.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sgbr_core::bus_catalog::model::BusStop;

use crate::auth::Device;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn bus_stops(
    State(state): State<AppState>,
    _device: Device,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let catalog = state
        .catalog()
        .snapshot()
        .await
        .ok_or(ApiError::CatalogNotReady)?;
    let etag = format!("\"stops-{}\"", catalog.fetched_at_unix);
    if if_none_match_hits(&headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let body: Vec<BusStop> = catalog.stops.clone();
    Ok(with_etag(Json(body).into_response(), &etag))
}

pub async fn bus_routes(
    State(state): State<AppState>,
    _device: Device,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let catalog = state
        .catalog()
        .snapshot()
        .await
        .ok_or(ApiError::CatalogNotReady)?;
    let etag = format!("\"routes-{}\"", catalog.fetched_at_unix);
    if if_none_match_hits(&headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let pairs: Vec<(String, String)> = catalog
        .services_by_stop
        .iter()
        .flat_map(|(stop, services)| services.iter().map(move |svc| (stop.clone(), svc.clone())))
        .collect();
    Ok(with_etag(Json(pairs).into_response(), &etag))
}

fn if_none_match_hits(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag)
}

fn not_modified(etag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

fn with_etag(mut response: Response, etag: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use crate::catalog::CatalogCache;
    use crate::config::Config;
    use crate::state::AppState;
    use crate::store::DeviceStore;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use sgbr_core::bus_catalog::model::{BusCatalog, BusStop};
    use sgbr_core::bus_catalog::parse::build_services_by_stop;
    use sgbr_core::worker::auth::generate_keypair;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt as _;

    fn sample_catalog() -> BusCatalog {
        let mut services_by_stop = BTreeMap::new();
        services_by_stop.insert("83139".to_owned(), vec!["14e".to_owned(), "15".to_owned()]);
        services_by_stop.insert("83141".to_owned(), vec!["16".to_owned()]);
        BusCatalog {
            stops: vec![
                BusStop {
                    code: "83139".to_owned(),
                    name: "Opp Blk 123".to_owned(),
                    road: "Clementi Ave 2".to_owned(),
                },
                BusStop {
                    code: "83141".to_owned(),
                    name: "Aft Blk 456".to_owned(),
                    road: "Clementi Ave 3".to_owned(),
                },
            ],
            services_by_stop,
            fetched_at_unix: 1_700_000_000,
        }
    }

    fn registered_state_with_catalog(
        name: &str,
        catalog: Option<BusCatalog>,
    ) -> (AppState, String, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_catalog_route_test_{}_{name}",
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
        let cache = catalog.map_or_else(CatalogCache::empty, CatalogCache::with_catalog);
        let state = AppState::new(Arc::new(config), Arc::new(store), Arc::new(cache));
        (state, token, dir)
    }

    #[tokio::test]
    async fn bus_stops_503_when_catalog_empty() {
        let (state, token, dir) = registered_state_with_catalog("empty-stops", None);
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bus_stops_200_with_etag_then_304_on_if_none_match() {
        let (state, token, dir) =
            registered_state_with_catalog("stops-etag", Some(sample_catalog()));

        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_owned();
        assert_eq!(etag, "\"stops-1700000000\"");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        let stops: Vec<BusStop> = serde_json::from_slice(&bytes).expect("parse stops");
        assert_eq!(stops.len(), 2);

        let not_modified = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .header("authorization", format!("Bearer {token}"))
                    .header("if-none-match", etag)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bus_routes_round_trips_through_build_services_by_stop() {
        let catalog = sample_catalog();
        let expected = catalog.services_by_stop.clone();
        let (state, token, dir) = registered_state_with_catalog("routes-roundtrip", Some(catalog));

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-routes")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("etag")
                .expect("etag header")
                .to_str()
                .expect("etag str"),
            "\"routes-1700000000\""
        );
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        let pairs: Vec<(String, String)> = serde_json::from_slice(&bytes).expect("parse pairs");
        let rebuilt = build_services_by_stop(pairs);
        assert_eq!(rebuilt, expected);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_bearer_token_is_401() {
        let (state, _token, dir) = registered_state_with_catalog("no-auth", Some(sample_catalog()));
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
