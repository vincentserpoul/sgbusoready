//! Library surface for `sgbr-worker`.
//!
//! `main.rs` is a thin CLI wrapper around this crate. It exists (rather than
//! everything living directly in `main.rs`, bin-only) so integration tests
//! under `tests/` can build a real `AppState` (tempdir store, mock
//! `ArrivalFetcher`, in-memory catalog) and drive the real `axum` router
//! in-process against the real `sgbr_core::worker::client` — see
//! `tests/client_stream.rs`. No behavior lives here beyond re-exporting the
//! module tree `main.rs` used to own directly.

mod auth;
pub mod catalog;
pub mod config;
mod error;
pub mod registry;
pub mod routes;
pub mod state;
pub mod store;
