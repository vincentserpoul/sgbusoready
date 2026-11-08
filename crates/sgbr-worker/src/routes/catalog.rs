//! `GET /v1/bus-stops`, `GET /v1/bus-routes` — bearer-authenticated proxy for
//! the cached LTA catalog. Devices never talk to `DataMall` directly.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sgbr_core::bus_catalog::model::BusCatalog;

use crate::auth::Device;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn bus_stops(
    State(state): State<AppState>,
    _device: Device,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    catalog_view(&state, &headers, "stops", |catalog| catalog.stops.clone()).await
}

/// Flat `(stop_code, service_no)` pairs — the shape
/// `sgbr_core::bus_catalog::parse::build_services_by_stop` reassembles.
pub async fn bus_routes(
    State(state): State<AppState>,
    _device: Device,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    catalog_view(&state, &headers, "routes", |catalog| {
        catalog
            .services_by_stop
            .iter()
            .flat_map(|(stop, services)| {
                services.iter().map(move |svc| (stop.clone(), svc.clone()))
            })
            .collect::<Vec<(String, String)>>()
    })
    .await
}

/// The shared conditional-GET flow behind both catalog endpoints. The `ETag`
/// `"{prefix}-{fetched_at_unix}"` identifies the cached snapshot: a matching
/// `If-None-Match` answers `304` with no body, otherwise `project` renders the
/// JSON body. Either way the response carries the `ETag`.
async fn catalog_view<T: Serialize>(
    state: &AppState,
    headers: &HeaderMap,
    prefix: &str,
    project: impl FnOnce(&BusCatalog) -> T,
) -> Result<Response, ApiError> {
    let catalog = state
        .catalog()
        .snapshot()
        .await
        .ok_or(ApiError::CatalogNotReady)?;
    let etag = format!("\"{prefix}-{}\"", catalog.fetched_at_unix);
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag);
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Json(project(&catalog)).into_response()
    };
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use crate::catalog::CatalogCache;
    use crate::test_support::{TestState, bearer_req, json_body, null_registry, req};
    use axum::http::StatusCode;
    use sgbr_core::bus_catalog::model::{BusCatalog, BusStop};
    use sgbr_core::bus_catalog::parse::build_services_by_stop;
    use std::collections::BTreeMap;

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

    /// A registered device against `catalog` (or an empty cache); returns the
    /// state and its bearer token.
    fn registered(name: &str, catalog: Option<BusCatalog>) -> (TestState, String) {
        let cache = catalog.map_or_else(CatalogCache::default, CatalogCache::with_catalog);
        let t = TestState::with(name, cache, null_registry());
        let token = t.register_device();
        (t, token)
    }

    fn etag_of(response: &axum::response::Response) -> String {
        response
            .headers()
            .get("etag")
            .expect("etag header")
            .to_str()
            .expect("etag str")
            .to_owned()
    }

    #[tokio::test]
    async fn bus_stops_503_when_catalog_empty() {
        let (t, token) = registered("catalog-empty-stops", None);
        let response = t.send(bearer_req("GET", "/v1/bus-stops", &token)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn bus_stops_200_with_etag_then_304_on_if_none_match() {
        let (t, token) = registered("catalog-stops-etag", Some(sample_catalog()));

        let response = t.send(bearer_req("GET", "/v1/bus-stops", &token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let etag = etag_of(&response);
        assert_eq!(etag, "\"stops-1700000000\"");
        let stops: Vec<BusStop> =
            serde_json::from_value(json_body(response).await).expect("parse stops");
        assert_eq!(stops.len(), 2);

        let not_modified = t
            .send(bearer_req("GET", "/v1/bus-stops", &token).header("if-none-match", etag))
            .await;
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn bus_routes_round_trips_through_build_services_by_stop() {
        let catalog = sample_catalog();
        let expected = catalog.services_by_stop.clone();
        let (t, token) = registered("catalog-routes-roundtrip", Some(catalog));

        let response = t.send(bearer_req("GET", "/v1/bus-routes", &token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(etag_of(&response), "\"routes-1700000000\"");
        let pairs: Vec<(String, String)> =
            serde_json::from_value(json_body(response).await).expect("parse pairs");
        assert_eq!(build_services_by_stop(pairs), expected);
    }

    #[tokio::test]
    async fn missing_bearer_token_is_401() {
        let (t, _token) = registered("catalog-no-auth", Some(sample_catalog()));
        let response = t.send(req("GET", "/v1/bus-stops")).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
