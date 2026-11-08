//! Bus-stop catalog: shared state, stop search, and background refresh.
//!
//! The catalog is shared with a background refresh thread, so it lives behind an
//! `Arc<Mutex<…>>`. Search runs synchronously off the cached catalog; a stale or
//! missing catalog is refreshed on a background thread and swapped in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use time::OffsetDateTime;

use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::bus_catalog::search::search as catalog_search;
use sgbr_core::bus_catalog::store as catalog_store;
use sgbr_core::worker::client::{WorkerConfig, fetch_catalog};

use crate::{AppWindow, StopResult, WORKER_URL, credentials_path, load_credentials, now_sgt};

/// Catalog is shared with a background refresh thread, so it's `Arc<Mutex>`.
pub type Catalog = Arc<Mutex<Option<BusCatalog>>>;

/// Set while a background fetch is running, so the periodic retry (the 15s
/// timer in `run_app`) never stacks a second fetch on top of a slow one.
static REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Run `f` with a borrow of the catalog, tolerating lock poisoning.
pub fn with_catalog<R>(catalog: &Catalog, f: impl FnOnce(Option<&BusCatalog>) -> R) -> R {
    f(catalog
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref())
}

pub fn catalog_path(store_path: &Path) -> PathBuf {
    store_path.with_file_name("bus_catalog.json")
}

/// Re-run the current stop search and refresh the loading flag (used both on
/// keystroke and after a background catalog refresh lands).
pub fn refresh_search(window: &AppWindow, catalog: &Catalog) {
    let query = window.get_search_query().to_string();
    let results: Vec<StopResult> = with_catalog(catalog, |cat| {
        cat.map(|k| {
            catalog_search(k, &query, 30)
                .into_iter()
                .map(|s| StopResult {
                    code: SharedString::from(s.code.as_str()),
                    name: SharedString::from(s.name.as_str()),
                    road: SharedString::from(s.road.as_str()),
                })
                .collect()
        })
        .unwrap_or_default()
    });
    window.set_search_results(ModelRc::new(VecModel::from(results)));
    window.set_catalog_loading(with_catalog(catalog, |c| c.is_none()));
}

/// If the catalog is missing or stale (and worker credentials are available),
/// fetch a fresh one from the worker on a background thread, persist it, swap
/// it in, and refresh the UI. Cheap to call repeatedly: it returns at once
/// when nothing is needed or a fetch is already in flight, so the 15s timer
/// retries a failed fetch (worker still warming up, no signal) for free.
pub fn spawn_refresh_if_stale(catalog: &Catalog, window: &AppWindow, store_path: &Path) {
    let Some(creds) = load_credentials(&credentials_path(store_path)) else {
        return;
    };
    let now = now_sgt();
    let needs = with_catalog(catalog, |c| c.is_none_or(|k| k.is_stale(now)));
    if !needs || REFRESH_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return;
    }
    let catalog = Arc::clone(catalog);
    let weak = window.as_weak();
    let cat_path = catalog_path(store_path);
    let cfg = WorkerConfig {
        base_url: WORKER_URL.to_owned(),
        token: creds.token,
    };
    std::thread::spawn(move || {
        let fetched = fetch_catalog(&cfg, OffsetDateTime::now_utc());
        REFRESH_IN_FLIGHT.store(false, Ordering::Release);
        let Ok(fresh) = fetched else {
            return;
        };
        let _ = catalog_store::save(&fresh, &cat_path);
        if let Ok(mut guard) = catalog.lock() {
            *guard = Some(fresh);
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                // Rows label from cached stop names, so they need no relabel; just
                // refresh the stop-search now the catalog is available.
                refresh_search(&w, &catalog);
            }
        });
    });
}
