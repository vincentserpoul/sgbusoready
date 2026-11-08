//! Admin endpoints: invite mint/list, device list/revoke. Every handler is
//! gated by the `crate::auth::Admin` extractor (`X-Admin-Token`).

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
    use crate::test_support::{TestState, admin_req, bearer_req, json_body, req};
    use axum::http::StatusCode;

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

    #[tokio::test]
    async fn missing_admin_token_is_401() {
        let t = TestState::new("admin-missing-token");
        let response = t.send(req("POST", "/v1/admin/invites")).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_admin_token_is_401() {
        let t = TestState::new("admin-wrong-token");
        let response = t
            .send(req("POST", "/v1/admin/invites").header("X-Admin-Token", "not-the-token"))
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn mint_invite_returns_code_matching_expected_alphabet() {
        let t = TestState::new("admin-mint");
        let response = t.send(admin_req("POST", "/v1/admin/invites")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let code = body["code"].as_str().expect("code field");
        assert!(
            matches_invite_code_shape(code),
            "code {code} has unexpected shape"
        );
    }

    #[tokio::test]
    async fn revoke_then_bearer_auth_fails_and_revoke_unknown_is_404() {
        let t = TestState::new("admin-revoke");
        let token = t.register_device();
        let device_id = t
            .state
            .store()
            .list_devices()
            .pop()
            .expect("one device")
            .device_id;

        // Bearer works before revoke (503 because catalog is empty, not 401).
        let before = t.send(bearer_req("GET", "/v1/bus-stops", &token)).await;
        assert_eq!(before.status(), StatusCode::SERVICE_UNAVAILABLE);

        let revoke_uri = format!("/v1/admin/devices/{device_id}");
        let revoked = t.send(admin_req("DELETE", &revoke_uri)).await;
        assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

        let after = t.send(bearer_req("GET", "/v1/bus-stops", &token)).await;
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

        let revoke_again = t.send(admin_req("DELETE", &revoke_uri)).await;
        assert_eq!(revoke_again.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_devices_reports_registered_device() {
        let t = TestState::new("admin-list-devices");
        let _token = t.register_device();
        let expected_id = t
            .state
            .store()
            .list_devices()
            .pop()
            .expect("one device")
            .device_id;

        let response = t.send(admin_req("GET", "/v1/admin/devices")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let devices = json_body(response).await;
        let devices = devices.as_array().expect("array of devices");
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices.first().and_then(|d| d["device_id"].as_str()),
            Some(expected_id.as_str())
        );
    }
}
