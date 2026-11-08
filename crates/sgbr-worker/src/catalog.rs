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

/// While the cache is still empty every catalog route answers 503, so a
/// failed fetch is retried on this backoff (doubling up to the cap) rather
/// than after a full `REFRESH_CHECK_INTERVAL`.
const EMPTY_RETRY_INITIAL: Duration = Duration::from_secs(30);
const EMPTY_RETRY_MAX: Duration = Duration::from_mins(10);

const CATALOG_FILE_NAME: &str = "catalog.json";

/// The last successfully fetched (or disk-loaded) catalog, if any. `Default`
/// is the empty cache (before the first successful fetch/load).
#[derive(Debug, Default)]
pub struct CatalogCache {
    inner: RwLock<Option<Arc<BusCatalog>>>,
}

impl CatalogCache {
    /// Seed the cache with an already-known catalog (used by the refresh
    /// loop after a successful fetch, and directly by tests).
    #[must_use]
    pub fn with_catalog(catalog: BusCatalog) -> Self {
        Self {
            inner: RwLock::new(Some(Arc::new(catalog))),
        }
    }

    /// Try to load `{data_dir}/catalog.json`; fall back to the empty cache on
    /// any failure (missing file, corrupt JSON — the refresh loop self-heals).
    #[must_use]
    pub fn load_or_empty(data_dir: &Path) -> Self {
        catalog_store::load(&data_dir.join(CATALOG_FILE_NAME))
            .ok()
            .map_or_else(Self::default, Self::with_catalog)
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
/// The first check runs immediately; afterwards it sleeps
/// `REFRESH_CHECK_INTERVAL` while a catalog is held, or the short
/// `EMPTY_RETRY_*` backoff while the cache is still empty. Fetch failures are
/// logged and the previous value is kept.
pub async fn run_refresh_loop(cache: Arc<CatalogCache>, account_key: String, data_dir: PathBuf) {
    let catalog_path = data_dir.join(CATALOG_FILE_NAME);
    let mut empty_retry = EMPTY_RETRY_INITIAL;
    loop {
        let needs_refresh = match cache.snapshot().await {
            None => true,
            Some(existing) => is_stale(&existing, OffsetDateTime::now_utc()),
        };
        if needs_refresh {
            refresh_once(&cache, account_key.clone(), &catalog_path).await;
        }
        let cache_empty = cache.snapshot().await.is_none();
        tokio::time::sleep(next_sleep(cache_empty, &mut empty_retry)).await;
    }
}

/// How long to sleep before the next check: the regular interval once a
/// catalog is held (resetting the backoff), else the current backoff step,
/// which then doubles up to `EMPTY_RETRY_MAX`.
fn next_sleep(cache_empty: bool, empty_retry: &mut Duration) -> Duration {
    if cache_empty {
        let now = *empty_retry;
        *empty_retry = (now * 2).min(EMPTY_RETRY_MAX);
        now
    } else {
        *empty_retry = EMPTY_RETRY_INITIAL;
        REFRESH_CHECK_INTERVAL
    }
}

/// One refresh attempt: fetch upstream on a blocking thread, persist to disk,
/// swap into `cache`. Any failure is logged and the previous value is kept.
async fn refresh_once(cache: &CatalogCache, account_key: String, catalog_path: &Path) {
    let catalog = match fetch_blocking(account_key).await {
        Ok(catalog) => catalog,
        Err(e) => {
            tracing::warn!("catalog refresh failed: {e}");
            return;
        }
    };
    if let Err(e) = catalog_store::save(&catalog, catalog_path) {
        tracing::warn!("failed to persist catalog to disk: {e}");
    }
    cache.set(catalog).await;
}

/// Run the blocking upstream fetch on a dedicated thread; flattens the join
/// failure and the fetch failure into one message for logging.
async fn fetch_blocking(account_key: String) -> Result<BusCatalog, String> {
    let now = OffsetDateTime::now_utc();
    tokio::task::spawn_blocking(move || fetch_catalog(&account_key, now))
        .await
        .map_err(|e| format!("task did not complete: {e}"))?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogCache, EMPTY_RETRY_INITIAL, EMPTY_RETRY_MAX, REFRESH_CHECK_INTERVAL, is_stale,
        next_sleep,
    };
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
        let cache = CatalogCache::default();
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
    fn empty_cache_backs_off_then_resets_once_populated() {
        let mut retry = EMPTY_RETRY_INITIAL;
        assert_eq!(next_sleep(true, &mut retry), EMPTY_RETRY_INITIAL);
        assert_eq!(next_sleep(true, &mut retry), EMPTY_RETRY_INITIAL * 2);
        for _ in 0..10 {
            next_sleep(true, &mut retry);
        }
        assert_eq!(next_sleep(true, &mut retry), EMPTY_RETRY_MAX, "capped");
        assert_eq!(next_sleep(false, &mut retry), REFRESH_CHECK_INTERVAL);
        assert_eq!(retry, EMPTY_RETRY_INITIAL, "reset once a catalog is held");
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
