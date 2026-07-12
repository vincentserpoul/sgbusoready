//! Shared application state handed to every axum handler.

use std::sync::Arc;

use crate::catalog::CatalogCache;
use crate::config::Config;
use crate::registry::Registry;
use crate::store::DeviceStore;

/// Cheaply cloneable handle to the worker's shared state (each field is an
/// `Arc`).
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<Config>,
    store: Arc<DeviceStore>,
    catalog: Arc<CatalogCache>,
    registry: Arc<Registry>,
}

impl AppState {
    #[must_use]
    pub const fn new(
        config: Arc<Config>,
        store: Arc<DeviceStore>,
        catalog: Arc<CatalogCache>,
        registry: Arc<Registry>,
    ) -> Self {
        Self {
            config,
            store,
            catalog,
            registry,
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

    /// The refcounted arrivals-polling registry. Returned as `&Arc<Registry>`
    /// (not `&Registry`) because `Registry::subscribe` needs an `Arc` handle
    /// to hand back to each `Subscription` for its `Drop` teardown.
    #[must_use]
    pub const fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }
}
