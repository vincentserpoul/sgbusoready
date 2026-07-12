//! `GET /v1/healthz` — public liveness/readiness probe.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use time::OffsetDateTime;

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HealthBody {
    pub status: &'static str,
    pub catalog_age_secs: Option<i64>,
    pub active_stops: u32,
    pub version: &'static str,
}

pub async fn healthz(State(state): State<AppState>) -> Json<HealthBody> {
    let catalog_age_secs = match state.catalog().snapshot().await {
        Some(catalog) => {
            let now = OffsetDateTime::now_utc().unix_timestamp();
            Some(now.saturating_sub(catalog.fetched_at_unix))
        }
        None => None,
    };
    let active_stops = u32::try_from(state.registry().active_stops()).unwrap_or(u32::MAX);
    Json(HealthBody {
        status: "ok",
        catalog_age_secs,
        active_stops,
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use crate::catalog::CatalogCache;
    use crate::config::Config;
    use crate::registry::test_registry;
    use crate::state::AppState;
    use crate::store::DeviceStore;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use std::sync::Arc;
    use tower::ServiceExt as _;

    fn tmp_state(name: &str) -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_healthz_test_{}_{name}",
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
        let state = AppState::new(
            Arc::new(config),
            Arc::new(store),
            Arc::new(CatalogCache::empty()),
            test_registry(),
        );
        (state, dir)
    }

    async fn healthz_body(state: AppState) -> serde_json::Value {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/healthz")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("parse body")
    }

    #[tokio::test]
    async fn healthz_is_200_without_auth() {
        let (state, dir) = tmp_state("plain");
        let body = healthz_body(state).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["active_stops"], 0);
        assert!(body["catalog_age_secs"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn healthz_reports_live_active_stops() {
        let (state, dir) = tmp_state("active-stops");
        let sub = state.registry().subscribe("83139");
        let body = healthz_body(state.clone()).await;
        assert_eq!(body["active_stops"], 1);

        drop(sub);
        let body_after = healthz_body(state).await;
        assert_eq!(body_after["active_stops"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
