//! `POST /v1/devices` — device registration against a single-use invite code.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sgbr_core::worker::auth::verify_registration;
use sgbr_core::worker::protocol::{RegisterRequest, RegisterResponse};

use crate::error::ApiError;
use crate::state::AppState;
use crate::store::StoreError;

const PUBKEY_LEN: usize = 32;
const SIGNATURE_LEN: usize = 64;

pub async fn register(
    State(state): State<AppState>,
    body: Result<Json<RegisterRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    // Fold axum's built-in Json rejection (syntactically invalid JSON, wrong
    // field types, missing/invalid content-type) into ApiError so even those
    // failures keep the `{"error": "<msg>"}` envelope.
    let Json(req) = body.map_err(|rejection| ApiError::BadRequest(rejection.body_text()))?;
    validate_shape(&req)?;
    if !verify_registration(&req.pubkey, &req.invite_code, &req.signature) {
        return Err(ApiError::Forbidden("registration rejected".to_owned()));
    }
    let (device_id, token) = state
        .store()
        .register_device(&req.invite_code, &req.pubkey)
        .map_err(|e| match e {
            StoreError::UnknownInvite | StoreError::InviteUsed => {
                ApiError::Forbidden("registration rejected".to_owned())
            }
            io_or_rng @ (StoreError::Io(_) | StoreError::Serde(_) | StoreError::Rng(_)) => {
                ApiError::Internal(io_or_rng.to_string())
            }
        })?;
    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse { token, device_id }),
    ))
}

/// Reject malformed base64/lengths with 400 *before* attempting signature
/// verification (which would otherwise fold every malformed input into a
/// uniform "false" and look like a 403 to the caller).
fn validate_shape(req: &RegisterRequest) -> Result<(), ApiError> {
    if req.invite_code.trim().is_empty() {
        return Err(ApiError::BadRequest("missing invite_code".to_owned()));
    }
    let pubkey = BASE64
        .decode(&req.pubkey)
        .map_err(|_decode_err| ApiError::BadRequest("pubkey is not valid base64".to_owned()))?;
    if pubkey.len() != PUBKEY_LEN {
        return Err(ApiError::BadRequest(format!(
            "pubkey must decode to {PUBKEY_LEN} bytes"
        )));
    }
    let signature = BASE64
        .decode(&req.signature)
        .map_err(|_decode_err| ApiError::BadRequest("signature is not valid base64".to_owned()))?;
    if signature.len() != SIGNATURE_LEN {
        return Err(ApiError::BadRequest(format!(
            "signature must decode to {SIGNATURE_LEN} bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{TestState, bearer_req, json_body, req};
    use axum::body::Body;
    use axum::http::StatusCode;
    use base64::Engine as _;
    use sgbr_core::worker::auth::{generate_keypair, sign_registration};
    use sgbr_core::worker::protocol::{RegisterRequest, RegisterResponse};

    async fn post_devices(t: &TestState, body: &RegisterRequest) -> axum::response::Response {
        t.send_json(req("POST", "/v1/devices"), body).await
    }

    /// A syntactically valid registration for a freshly minted invite.
    fn valid_request(t: &TestState) -> RegisterRequest {
        let invite = t.state.store().mint_invite().expect("mint invite");
        let (seed, pubkey) = generate_keypair().expect("keygen");
        RegisterRequest {
            signature: sign_registration(&seed, &invite.code),
            invite_code: invite.code,
            pubkey,
        }
    }

    #[tokio::test]
    async fn register_happy_path_returns_201_and_working_token() {
        let t = TestState::new("devices-happy");
        let response = post_devices(&t, &valid_request(&t)).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: RegisterResponse =
            serde_json::from_value(json_body(response).await).expect("parse body");
        assert_eq!(body.device_id.len(), 16);
        assert_eq!(body.token.len(), 64);

        // The returned token authenticates against a Bearer-guarded route. No
        // catalog is loaded in this state, so auth passing shows as a 503
        // (catalog not ready), not a 401.
        let auth_response = t
            .send(bearer_req("GET", "/v1/bus-stops", &body.token))
            .await;
        assert_eq!(auth_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn bad_signature_is_403() {
        let t = TestState::new("devices-bad-sig");
        let mut req = valid_request(&t);
        // Corrupt the signature but keep it valid base64 of the right length.
        let flipped = if req.signature.starts_with('A') {
            "B"
        } else {
            "A"
        };
        req.signature.replace_range(0..1, flipped);
        let response = post_devices(&t, &req).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unknown_invite_is_403() {
        let t = TestState::new("devices-unknown-invite");
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let req = RegisterRequest {
            invite_code: "NOPE-CODE".to_owned(),
            pubkey,
            signature: sign_registration(&seed, "NOPE-CODE"),
        };
        let response = post_devices(&t, &req).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn reused_invite_is_403() {
        let t = TestState::new("devices-reused-invite");
        let first_req = valid_request(&t);
        let first = post_devices(&t, &first_req).await;
        assert_eq!(first.status(), StatusCode::CREATED);

        let (seed2, pubkey2) = generate_keypair().expect("keygen 2");
        let second = post_devices(
            &t,
            &RegisterRequest {
                signature: sign_registration(&seed2, &first_req.invite_code),
                invite_code: first_req.invite_code,
                pubkey: pubkey2,
            },
        )
        .await;
        assert_eq!(second.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn malformed_base64_pubkey_is_400() {
        let t = TestState::new("devices-malformed-pubkey");
        let invite = t.state.store().mint_invite().expect("mint invite");
        let req = RegisterRequest {
            invite_code: invite.code,
            pubkey: "not valid base64!!".to_owned(),
            signature: "AAAA".to_owned(),
        };
        let response = post_devices(&t, &req).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Assert the body is the `{"error": "<msg>"}` envelope every failure
    /// must use; returns the `error` message.
    async fn assert_error_envelope(response: axum::response::Response) -> String {
        json_body(response)
            .await
            .get("error")
            .and_then(|v| v.as_str())
            .expect("error body must have a string `error` field")
            .to_owned()
    }

    async fn post_raw_json(t: &TestState, body: &'static str) -> axum::response::Response {
        t.send_body(
            req("POST", "/v1/devices").header("content-type", "application/json"),
            Body::from(body),
        )
        .await
    }

    #[tokio::test]
    async fn malformed_json_body_is_400_with_error_envelope() {
        let t = TestState::new("devices-malformed-json");
        let response = post_raw_json(&t, "{not json").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_ne!(assert_error_envelope(response).await, "");
    }

    #[tokio::test]
    async fn wrong_typed_field_is_400_with_error_envelope() {
        let t = TestState::new("devices-wrong-typed-field");
        // Syntactically valid JSON, but `pubkey` is a number, not a string.
        let response = post_raw_json(
            &t,
            r#"{"invite_code":"ABCD2345","pubkey":42,"signature":"AAAA"}"#,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_ne!(assert_error_envelope(response).await, "");
    }

    #[tokio::test]
    async fn wrong_length_pubkey_is_400() {
        let t = TestState::new("devices-wrong-length-pubkey");
        let invite = t.state.store().mint_invite().expect("mint invite");
        let req = RegisterRequest {
            invite_code: invite.code,
            pubkey: base64::engine::general_purpose::STANDARD.encode(b"too-short"),
            signature: base64::engine::general_purpose::STANDARD.encode([0_u8; 64]),
        };
        let response = post_devices(&t, &req).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
