//! `POST /v1/devices` — device registration against a single-use invite code.

use axum::Json;
use axum::extract::State;
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
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
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
    use super::super::router;
    use crate::catalog::CatalogCache;
    use crate::config::Config;
    use crate::state::AppState;
    use crate::store::DeviceStore;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use base64::Engine as _;
    use http_body_util::BodyExt as _;
    use sgbr_core::worker::auth::{generate_keypair, sign_registration};
    use sgbr_core::worker::protocol::{RegisterRequest, RegisterResponse};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt as _;

    fn tmp_state(name: &str) -> (AppState, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_devices_test_{}_{name}",
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

    async fn post_devices(state: AppState, body: &RegisterRequest) -> axum::response::Response {
        router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/devices")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(body).expect("serialize")))
                    .expect("build request"),
            )
            .await
            .expect("router response")
    }

    #[tokio::test]
    async fn register_happy_path_returns_201_and_working_token() {
        let (state, dir) = tmp_state("happy");
        let invite = state.store().mint_invite().expect("mint invite");
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let signature = sign_registration(&seed, &invite.code);
        let req = RegisterRequest {
            invite_code: invite.code.clone(),
            pubkey,
            signature,
        };

        let response = post_devices(state.clone(), &req).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        let body: RegisterResponse = serde_json::from_slice(&bytes).expect("parse body");
        assert_eq!(body.device_id.len(), 16);
        assert_eq!(body.token.len(), 64);

        // The returned token authenticates against a Bearer-guarded route.
        let auth_response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/bus-stops")
                    .header("authorization", format!("Bearer {}", body.token))
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        // No catalog loaded yet in this state, but auth itself must pass:
        // that's a 503 (catalog not ready), not a 401.
        assert_eq!(auth_response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bad_signature_is_403() {
        let (state, dir) = tmp_state("bad-sig");
        let invite = state.store().mint_invite().expect("mint invite");
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let mut signature = sign_registration(&seed, &invite.code);
        // Corrupt the signature but keep it valid base64 of the right length.
        signature.replace_range(0..1, if signature.starts_with('A') { "B" } else { "A" });
        let req = RegisterRequest {
            invite_code: invite.code,
            pubkey,
            signature,
        };

        let response = post_devices(state, &req).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unknown_invite_is_403() {
        let (state, dir) = tmp_state("unknown-invite");
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let signature = sign_registration(&seed, "NOPE-CODE");
        let req = RegisterRequest {
            invite_code: "NOPE-CODE".to_owned(),
            pubkey,
            signature,
        };

        let response = post_devices(state, &req).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn reused_invite_is_403() {
        let (state, dir) = tmp_state("reused-invite");
        let invite = state.store().mint_invite().expect("mint invite");
        let (seed1, pubkey1) = generate_keypair().expect("keygen 1");
        let signature1 = sign_registration(&seed1, &invite.code);
        let first = post_devices(
            state.clone(),
            &RegisterRequest {
                invite_code: invite.code.clone(),
                pubkey: pubkey1,
                signature: signature1,
            },
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);

        let (seed2, pubkey2) = generate_keypair().expect("keygen 2");
        let signature2 = sign_registration(&seed2, &invite.code);
        let second = post_devices(
            state,
            &RegisterRequest {
                invite_code: invite.code,
                pubkey: pubkey2,
                signature: signature2,
            },
        )
        .await;
        assert_eq!(second.status(), StatusCode::FORBIDDEN);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn malformed_base64_pubkey_is_400() {
        let (state, dir) = tmp_state("malformed-pubkey");
        let invite = state.store().mint_invite().expect("mint invite");
        let req = RegisterRequest {
            invite_code: invite.code,
            pubkey: "not valid base64!!".to_owned(),
            signature: "AAAA".to_owned(),
        };
        let response = post_devices(state, &req).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn malformed_json_body_is_400() {
        let (state, dir) = tmp_state("malformed-json");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/devices")
                    .header("content-type", "application/json")
                    .body(Body::from("{not json"))
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn wrong_length_pubkey_is_400() {
        let (state, dir) = tmp_state("wrong-length-pubkey");
        let invite = state.store().mint_invite().expect("mint invite");
        let req = RegisterRequest {
            invite_code: invite.code,
            pubkey: base64::engine::general_purpose::STANDARD.encode(b"too-short"),
            signature: base64::engine::general_purpose::STANDARD.encode([0_u8; 64]),
        };
        let response = post_devices(state, &req).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
