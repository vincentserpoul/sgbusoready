//! Shared fixtures for the worker's unit tests: a self-cleaning temp dir, an
//! `AppState` wired to it, and request/response helpers for driving the real
//! router in-process. Test-only (`#[cfg(test)]` in `lib.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::request::Builder;
use axum::http::{Request, Response as HttpResponse};
use axum::response::Response;
use http_body_util::BodyExt as _;
use serde::Serialize;
use sgbr_core::error::CoreError;
use sgbr_core::lta::model::BusArrivalResponse;
use sgbr_core::worker::auth::generate_keypair;
use tower::ServiceExt as _;

use crate::catalog::CatalogCache;
use crate::config::Config;
use crate::registry::{ArrivalFetcher, Registry};
use crate::routes::router;
use crate::state::AppState;
use crate::store::DeviceStore;

/// The admin token every test `Config` carries.
pub const ADMIN_TOKEN: &str = "test-admin-token";

/// A fresh, empty directory under the OS temp dir, removed on drop (also on
/// panic — so a failing test does not leave its store behind).
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_test_{}_{n}_{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A registry whose fetcher always fails — for tests of routes that never
/// exercise the arrivals stream but still need an `AppState`.
pub fn null_registry() -> Arc<Registry> {
    struct NullFetcher;
    impl ArrivalFetcher for NullFetcher {
        fn fetch(&self, _stop_code: &str) -> Result<BusArrivalResponse, CoreError> {
            Err(CoreError::Http("test fetcher: no network".to_owned()))
        }
    }
    Registry::new(Arc::new(NullFetcher), Duration::from_secs(15))
}

/// An `AppState` backed by a tempdir store, kept alive (and cleaned up) with
/// the directory it writes to.
pub struct TestState {
    pub state: AppState,
    _dir: TempDir,
}

impl TestState {
    /// Empty catalog, network-less registry.
    pub fn new(name: &str) -> Self {
        Self::with(name, CatalogCache::default(), null_registry())
    }

    pub fn with(name: &str, catalog: CatalogCache, registry: Arc<Registry>) -> Self {
        let dir = TempDir::new(name);
        let store =
            DeviceStore::load_or_default(dir.path().join("store.json")).expect("load store");
        let config = Config {
            lta_account_key: "unused".to_owned(),
            bind: "127.0.0.1:0".to_owned(),
            data_dir: dir.path().to_path_buf(),
            admin_token: ADMIN_TOKEN.to_owned(),
            poll_secs: 15,
        };
        let state = AppState::new(
            Arc::new(config),
            Arc::new(store),
            Arc::new(catalog),
            registry,
        );
        Self { state, _dir: dir }
    }

    /// Mint an invite and register a device straight against the store;
    /// returns the bearer token.
    pub fn register_device(&self) -> String {
        let invite = self.state.store().mint_invite().expect("mint invite");
        let (_seed, pubkey) = generate_keypair().expect("keygen");
        let (_device_id, token) = self
            .state
            .store()
            .register_device(&invite.code, &pubkey)
            .expect("register device");
        token
    }

    /// Send `request` with an empty body through the real router.
    pub async fn send(&self, request: Builder) -> Response {
        self.send_body(request, Body::empty()).await
    }

    /// Send `request` with `body` serialized as JSON.
    pub async fn send_json<T: Serialize>(&self, request: Builder, body: &T) -> Response {
        self.send_body(
            request.header("content-type", "application/json"),
            Body::from(serde_json::to_vec(body).expect("serialize body")),
        )
        .await
    }

    pub async fn send_body(&self, request: Builder, body: Body) -> Response {
        let request: Request<Body> = request.body(body).expect("build request");
        router(self.state.clone())
            .oneshot(request)
            .await
            .expect("router response")
    }
}

/// A bare request builder.
pub fn req(method: &str, uri: &str) -> Builder {
    Request::builder().method(method).uri(uri)
}

/// A request carrying the test admin token.
pub fn admin_req(method: &str, uri: &str) -> Builder {
    req(method, uri).header("X-Admin-Token", ADMIN_TOKEN)
}

/// A request carrying `Authorization: Bearer {token}`.
pub fn bearer_req(method: &str, uri: &str, token: &str) -> Builder {
    req(method, uri).header("authorization", format!("Bearer {token}"))
}

/// Read the whole body as JSON.
pub async fn json_body<B>(response: HttpResponse<B>) -> serde_json::Value
where
    B: axum::body::HttpBody,
    B::Error: std::fmt::Debug,
{
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body must be JSON")
}
