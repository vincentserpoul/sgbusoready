//! In-memory bus catalog cache + background refresh loop. The worker is the
//! only component that fetches from LTA `DataMall`; devices get the catalog
//! via `GET /v1/bus-stops` / `GET /v1/bus-routes` (see `routes::catalog`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sgbr_core::bus_catalog::fetch::fetch_catalog;
use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::bus_catalog::store as catalog_store;
use time::OffsetDateTime;
use tokio::sync::RwLock;

/// How often the refresh loop wakes up to check staleness.
const REFRESH_CHECK_INTERVAL: Duration = Duration::from_hours(1);

/// Catalog older than this triggers a refetch on the next check.
const REFRESH_AGE_THRESHOLD_SECS: i64 = 24 * 60 * 60;

const CATALOG_FILE_NAME: &str = "catalog.json";

/// The last successfully fetched (or disk-loaded) catalog, if any.
#[derive(Debug, Default)]
pub struct CatalogCache {
    inner: RwLock<Option<Arc<BusCatalog>>>,
}

impl CatalogCache {
    /// An empty cache — before the first successful fetch/load.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// Seed the cache with an already-known catalog (used by the refresh
    /// loop after a successful fetch, and directly by tests).
    #[must_use]
    pub fn with_catalog(catalog: BusCatalog) -> Self {
        Self {
            inner: RwLock::new(Some(Arc::new(catalog))),
        }
    }

    /// Try to load `{data_dir}/catalog.json`; fall back to [`Self::empty`] on
    /// any failure (missing file, corrupt JSON — the refresh loop self-heals).
    #[must_use]
    pub fn load_or_empty(data_dir: &Path) -> Self {
        catalog_store::load(&data_dir.join(CATALOG_FILE_NAME))
            .ok()
            .map_or_else(Self::empty, Self::with_catalog)
    }

    /// The current catalog, if the cache has ever been populated.
    pub async fn snapshot(&self) -> Option<Arc<BusCatalog>> {
        self.inner.read().await.clone()
    }

    async fn set(&self, catalog: BusCatalog) {
        *self.inner.write().await = Some(Arc::new(catalog));
    }
}

const fn is_stale(catalog: &BusCatalog, now: OffsetDateTime) -> bool {
    now.unix_timestamp().saturating_sub(catalog.fetched_at_unix) > REFRESH_AGE_THRESHOLD_SECS
}

/// Runs forever: on an empty or >24h-stale cache, fetch a fresh catalog from
/// LTA on a blocking thread, persist it to disk, and swap it into `cache`.
/// Sleeps [`REFRESH_CHECK_INTERVAL`] between checks; the first check runs
/// immediately. Fetch failures are logged and the previous value is kept.
pub async fn run_refresh_loop(cache: Arc<CatalogCache>, account_key: String, data_dir: PathBuf) {
    let catalog_path = data_dir.join(CATALOG_FILE_NAME);
    loop {
        let needs_refresh = match cache.snapshot().await {
            None => true,
            Some(existing) => is_stale(&existing, OffsetDateTime::now_utc()),
        };
        if needs_refresh {
            refresh_once(&cache, &account_key, &catalog_path).await;
        }
        tokio::time::sleep(REFRESH_CHECK_INTERVAL).await;
    }
}

async fn refresh_once(cache: &Arc<CatalogCache>, account_key: &str, catalog_path: &Path) {
    match fetch_blocking(account_key.to_owned()).await {
        Ok(catalog) => apply_fetched_catalog(cache, catalog, catalog_path).await,
        Err(e) => tracing::warn!("catalog refresh failed: {e}"),
    }
}

/// Run the blocking upstream fetch on a dedicated thread; flattens both the
/// join failure and the fetch failure into one `String` for logging.
async fn fetch_blocking(account_key: String) -> Result<BusCatalog, String> {
    let now = OffsetDateTime::now_utc();
    let joined = tokio::task::spawn_blocking(move || fetch_catalog(&account_key, now))
        .await
        .map_err(|e| format!("refresh task did not complete: {e}"))?;
    joined.map_err(|e| e.to_string())
}

async fn apply_fetched_catalog(
    cache: &Arc<CatalogCache>,
    catalog: BusCatalog,
    catalog_path: &Path,
) {
    if let Err(e) = catalog_store::save(&catalog, catalog_path) {
        tracing::warn!("failed to persist catalog to disk: {e}");
    }
    cache.set(catalog).await;
}

#[cfg(test)]
mod tests {
    use super::{CatalogCache, is_stale};
    use sgbr_core::bus_catalog::model::{BusCatalog, BusStop};
    use std::collections::BTreeMap;
    use time::OffsetDateTime;

    fn sample_catalog(fetched_at_unix: i64) -> BusCatalog {
        let mut services_by_stop = BTreeMap::new();
        services_by_stop.insert("83139".to_owned(), vec!["15".to_owned()]);
        BusCatalog {
            stops: vec![BusStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                road: "Clementi Ave 2".to_owned(),
            }],
            services_by_stop,
            fetched_at_unix,
        }
    }

    #[tokio::test]
    async fn empty_cache_has_no_snapshot() {
        let cache = CatalogCache::empty();
        assert!(cache.snapshot().await.is_none());
    }

    #[tokio::test]
    async fn with_catalog_seeds_a_snapshot() {
        let cache = CatalogCache::with_catalog(sample_catalog(1000));
        let snapshot = cache.snapshot().await.expect("snapshot");
        assert_eq!(snapshot.fetched_at_unix, 1000);
    }

    #[test]
    fn staleness_uses_24h_threshold() {
        let catalog = sample_catalog(1_000_000);
        let fresh = OffsetDateTime::from_unix_timestamp(1_000_000 + 24 * 60 * 60)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let stale = OffsetDateTime::from_unix_timestamp(1_000_000 + 24 * 60 * 60 + 1)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        assert!(!is_stale(&catalog, fresh));
        assert!(is_stale(&catalog, stale));
    }

    #[test]
    fn load_or_empty_falls_back_to_empty_when_file_missing() {
        let dir = std::env::temp_dir().join(format!(
            "sgbr_worker_catalog_missing_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = CatalogCache::load_or_empty(&dir);
        // Can't await here (sync test); just assert construction didn't panic
        // by checking the debug repr is the empty-cache shape.
        assert!(format!("{cache:?}").contains("None"));
    }
}
