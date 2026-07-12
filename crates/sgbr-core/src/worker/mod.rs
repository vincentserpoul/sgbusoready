//! Shared protocol + blocking client for the (future) worker backend.
//!
//! Ungated: the app and the eventual `sgbr-worker` crate both depend on
//! `sgbr-core`, and both need these types — unlike `lta::client` /
//! `bus_catalog::fetch`, nothing here talks to `DataMall` or needs the
//! `AccountKey`. See `docs/superpowers/specs/2026-07-12-backend-worker-design.md`.

pub mod auth;
pub mod client;
pub mod credentials;
pub mod protocol;
pub mod sse;
