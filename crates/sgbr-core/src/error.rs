//! Crate-wide error type.

use thiserror::Error;

/// Errors produced by the core layer.
#[derive(Debug, Error)]
pub enum CoreError {
    /// The HTTP request to `DataMall` failed (network, TLS, status).
    #[error("datamall request failed: {0}")]
    Http(String),

    /// The `DataMall` response body could not be parsed as the expected JSON.
    #[error("failed to parse datamall response: {0}")]
    Parse(String),

    /// An `EstimatedArrival` field was empty (no bus scheduled).
    #[error("no estimated arrival available")]
    NoArrival,

    /// An `EstimatedArrival` timestamp was not valid RFC3339.
    #[error("invalid arrival timestamp: {0}")]
    InvalidTimestamp(String),

    /// A filesystem operation on the persisted store failed.
    #[error("commute store io failed: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::CoreError;

    #[test]
    fn display_includes_context() {
        let err = CoreError::Http("timeout".to_owned());
        assert_eq!(err.to_string(), "datamall request failed: timeout");
    }
}
