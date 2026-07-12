//! Shared application state handed to every axum handler.

use std::sync::Arc;

use crate::catalog::CatalogCache;
use crate::config::Config;
use crate::store::DeviceStore;

/// Cheaply cloneable handle to the worker's shared state (each field is an
/// `Arc`). The SSE registry joins this in Phase 2.
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<Config>,
    store: Arc<DeviceStore>,
    catalog: Arc<CatalogCache>,
}

impl AppState {
    #[must_use]
    pub const fn new(
        config: Arc<Config>,
        store: Arc<DeviceStore>,
        catalog: Arc<CatalogCache>,
    ) -> Self {
        Self {
            config,
            store,
            catalog,
        }
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    #[must_use]
    pub fn store(&self) -> &DeviceStore {
        &self.store
    }

    #[must_use]
    pub fn catalog(&self) -> &CatalogCache {
        &self.catalog
    }
}
