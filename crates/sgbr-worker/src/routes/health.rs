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
    use crate::test_support::{TestState, json_body, req};
    use axum::http::StatusCode;

    async fn healthz_body(t: &TestState) -> serde_json::Value {
        let response = t.send(req("GET", "/v1/healthz")).await;
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    }

    #[tokio::test]
    async fn healthz_is_200_without_auth() {
        let t = TestState::new("healthz-plain");
        let body = healthz_body(&t).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["active_stops"], 0);
        assert!(body["catalog_age_secs"].is_null());
    }

    #[tokio::test]
    async fn healthz_reports_live_active_stops() {
        let t = TestState::new("healthz-active-stops");
        let sub = t.state.registry().subscribe("83139");
        assert_eq!(healthz_body(&t).await["active_stops"], 1);

        drop(sub);
        assert_eq!(healthz_body(&t).await["active_stops"], 0);
    }
}
