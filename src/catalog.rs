//! Bus-stop catalog: shared state, stop search, and background refresh.
//!
//! The catalog is shared with a background refresh thread, so it lives behind an
//! `Arc<Mutex<…>>`. Search runs synchronously off the cached catalog; a stale or
//! missing catalog is refreshed on a background thread and swapped in.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use time::OffsetDateTime;

use sgbr_core::bus_catalog::fetch::fetch_catalog;
use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::bus_catalog::search::search as catalog_search;
use sgbr_core::bus_catalog::store as catalog_store;

use crate::{ACCOUNT_KEY, AppWindow, StopResult, now_sgt};

/// Catalog is shared with a background refresh thread, so it's `Arc<Mutex>`.
pub type Catalog = Arc<Mutex<Option<BusCatalog>>>;

/// Run `f` with a borrow of the catalog, tolerating lock poisoning.
pub fn with_catalog<R>(catalog: &Catalog, f: impl FnOnce(Option<&BusCatalog>) -> R) -> R {
    match catalog.lock() {
        Ok(guard) => f(guard.as_ref()),
        Err(poisoned) => f(poisoned.into_inner().as_ref()),
    }
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

/// If the catalog is missing or stale (and a key is compiled in), fetch a fresh
/// one on a background thread, persist it, swap it in, and refresh the UI.
pub fn spawn_refresh_if_stale(catalog: &Catalog, window: &AppWindow, store_path: &Path) {
    if ACCOUNT_KEY.is_empty() {
        return;
    }
    let now = now_sgt();
    let needs = with_catalog(catalog, |c| c.is_none_or(|k| k.is_stale(now)));
    if !needs {
        return;
    }
    let catalog = Arc::clone(catalog);
    let weak = window.as_weak();
    let cat_path = catalog_path(store_path);
    std::thread::spawn(move || {
        let Ok(fresh) = fetch_catalog(ACCOUNT_KEY, OffsetDateTime::now_utc()) else {
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
