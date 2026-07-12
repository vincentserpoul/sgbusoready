//! Admin endpoints: invite mint/list, device list/revoke. Every handler is
//! gated by the [`crate::auth::Admin`] extractor (`X-Admin-Token`).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;

use crate::auth::Admin;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct InviteView {
    pub code: String,
    pub created_at_unix: i64,
    pub used_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceView {
    pub device_id: String,
    pub created_at_unix: i64,
    pub invite_code: String,
}

#[derive(Debug, Serialize)]
pub struct MintedInvite {
    pub code: String,
}

pub async fn mint_invite(
    _admin: Admin,
    State(state): State<AppState>,
) -> Result<Json<MintedInvite>, ApiError> {
    let invite = state
        .store()
        .mint_invite()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(MintedInvite { code: invite.code }))
}

pub async fn list_invites(_admin: Admin, State(state): State<AppState>) -> Json<Vec<InviteView>> {
    let invites = state
        .store()
        .list_invites()
        .into_iter()
        .map(|i| InviteView {
            code: i.code,
            created_at_unix: i.created_at_unix,
            used_by: i.used_by,
        })
        .collect();
    Json(invites)
}

pub async fn list_devices(_admin: Admin, State(state): State<AppState>) -> Json<Vec<DeviceView>> {
    let devices = state
        .store()
        .list_devices()
        .into_iter()
        .map(|d| DeviceView {
            device_id: d.device_id,
            created_at_unix: d.created_at_unix,
            invite_code: d.invite_code,
        })
        .collect();
    Json(devices)
}

pub async fn revoke_device(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let revoked = state
        .store()
        .revoke_device(&id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
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
    use std::sync::Arc;
    use tower::ServiceExt as _;

    /// Matches `^[A-HJ-NP-Z2-9]{8}$` without pulling in a regex crate: 8
    /// chars from the invite alphabet (uppercase letters minus I/O, digits
    /// minus 0/1).
    fn matches_invite_code_shape(code: &str) -> bool {
        code.len() == 8
            && code.bytes().all(|b| {
                (b.is_ascii_uppercase() && b != b'I' && b != b'O')
                    || (b.is_ascii_digit() && b != b'0' && b != b'1')
            })
    }

    fn state(name: &str) -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_admin_test_{}_{name}",
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
        );
        (state, dir)
    }

    #[tokio::test]
    async fn missing_admin_token_is_401() {
        let (state, dir) = state("missing-token");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wrong_admin_token_is_401() {
        let (state, dir) = state("wrong-token");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("X-Admin-Token", "not-the-token")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn mint_invite_returns_code_matching_expected_alphabet() {
        let (state, dir) = state("mint");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("X-Admin-Token", "test-admin-token")
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
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("parse body");
        let code = body["code"].as_str().expect("code field");
        assert!(
            matches_invite_code_shape(code),
            "code {code} has unexpected shape"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn revoke_then_bearer_auth_fails_and_revoke_unknown_is_404() {
        let (state, dir) = state("revoke");
        let invite = state.store().mint_invite().expect("mint invite");
        let (_seed, pubkey) = sgbr_core::worker::auth::generate_keypair().expect("keygen");
        let (device_id, token) = state
            .store()
            .register_device(&invite.code, &pubkey)
            .expect("register");

        // Bearer works before revoke (503 because catalog is empty, not 401).
        let before = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(before.status(), StatusCode::SERVICE_UNAVAILABLE);

        let revoke_response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/admin/devices/{device_id}"))
                    .header("X-Admin-Token", "test-admin-token")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(revoke_response.status(), StatusCode::NO_CONTENT);

        let after = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

        let revoke_again = router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/admin/devices/{device_id}"))
                    .header("X-Admin-Token", "test-admin-token")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(revoke_again.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_devices_reports_registered_device() {
        let (state, dir) = state("list-devices");
        let invite = state.store().mint_invite().expect("mint invite");
        let (_seed, pubkey) = sgbr_core::worker::auth::generate_keypair().expect("keygen");
        let (device_id, _token) = state
            .store()
            .register_device(&invite.code, &pubkey)
            .expect("register");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/admin/devices")
                    .header("X-Admin-Token", "test-admin-token")
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
        let devices: Vec<serde_json::Value> =
            serde_json::from_slice(&bytes).expect("parse devices");
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices.first().and_then(|d| d["device_id"].as_str()),
            Some(device_id.as_str())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
