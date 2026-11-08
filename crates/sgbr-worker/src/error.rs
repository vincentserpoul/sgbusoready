//! Worker-wide API error type — maps every failure to `{"error": "<msg>"}`
//! JSON plus the right HTTP status code.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Errors any route handler can return; `IntoResponse` picks the status code.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The request body/fields failed validation (bad base64, wrong length, ...).
    #[error("{0}")]
    BadRequest(String),
    /// No/invalid bearer token or admin token.
    #[error("unauthorized")]
    Unauthorized,
    /// A well-formed request was rejected on its merits (bad signature, invite
    /// unknown/used).
    #[error("{0}")]
    Forbidden(String),
    /// The referenced resource does not exist.
    #[error("not found")]
    NotFound,
    /// The bus catalog hasn't been fetched yet.
    #[error("catalog not ready")]
    CatalogNotReady,
    /// Something failed on our side (storage IO, RNG, ...).
    #[error("{0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::CatalogNotReady => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorBody {
            error: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;
    use axum::response::IntoResponse;

    #[test]
    fn catalog_not_ready_is_503() {
        let response = ApiError::CatalogNotReady.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn bad_request_is_400() {
        let response = ApiError::BadRequest("nope".to_owned()).into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn unauthorized_is_401() {
        let response = ApiError::Unauthorized.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn forbidden_is_403() {
        let response = ApiError::Forbidden("nope".to_owned()).into_response();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn not_found_is_404() {
        let response = ApiError::NotFound.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn internal_is_500() {
        let response = ApiError::Internal("boom".to_owned()).into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
