//! Bearer-token device auth and admin-token guard.

use std::fmt::Write as _;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::state::AppState;

/// Lowercase-hex-encode `bytes`. Shared by [`token_hash`] and
/// `store::random_hex`.
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            // `String`'s `Write` impl never fails; nothing to propagate.
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// Hex-encoded sha256 of `token`. This — never the raw token — is what the
/// store persists and what device lookups compare against.
#[must_use]
pub fn token_hash(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

/// A device authenticated via `Authorization: Bearer <token>`.
#[derive(Debug, Clone)]
pub struct Device {
    pub device_id: String,
}

impl FromRequestParts<AppState> for Device {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        let hash = token_hash(token);
        let device = state
            .store()
            .device_by_token_hash(&hash)
            .ok_or(ApiError::Unauthorized)?;
        Ok(Self {
            device_id: device.device_id,
        })
    }
}

/// Proof the caller supplied the correct `X-Admin-Token`.
#[derive(Debug, Clone, Copy)]
pub struct Admin;

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let provided = parts
            .headers
            .get("X-Admin-Token")
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;
        if constant_time_eq(provided, &state.config().admin_token) {
            Ok(Self)
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}

/// Compare `a` and `b` without leaking timing based on where they first
/// differ: hash both sides first (fixed-length output), then fold every byte
/// pair with XOR instead of short-circuiting `==`.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let ha = token_hash(a);
    let hb = token_hash(b);
    let mut diff: u8 = 0;
    for (x, y) in ha.bytes().zip(hb.bytes()) {
        diff |= x ^ y;
    }
    diff == 0 && ha.len() == hb.len()
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, token_hash};

    #[test]
    fn token_hash_is_deterministic_hex_sha256() {
        let hash = token_hash("hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(hash, token_hash("hello"));
        assert_ne!(hash, token_hash("hellp"));
    }

    #[test]
    fn constant_time_eq_matches_equal_strings() {
        assert!(constant_time_eq("dev-admin", "dev-admin"));
    }

    #[test]
    fn constant_time_eq_rejects_different_strings() {
        assert!(!constant_time_eq("dev-admin", "wrong-token"));
        assert!(!constant_time_eq("dev-admin", "dev-adminX"));
        assert!(!constant_time_eq("", "dev-admin"));
    }
}
