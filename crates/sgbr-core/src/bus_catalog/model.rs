//! Catalog types: the stop directory, the stop→services map, and staleness.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// One bus stop from the LTA `BusStops` dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusStop {
    pub code: String,
    pub name: String,
    pub road: String,
}

/// All stops, a `stop_code → sorted service numbers` map, and the fetch time
/// (unix seconds) used for staleness.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BusCatalog {
    pub stops: Vec<BusStop>,
    pub services_by_stop: BTreeMap<String, Vec<String>>,
    pub fetched_at_unix: i64,
}

/// Cache lifetime (~30 days). Stop names/routes change rarely, so a long TTL
/// avoids needless network use; a background refresh still self-heals.
pub const CATALOG_TTL_SECS: i64 = 30 * 24 * 60 * 60;

impl BusCatalog {
    /// The stop with `code`, if present.
    #[must_use]
    pub fn stop(&self, code: &str) -> Option<&BusStop> {
        self.stops.iter().find(|s| s.code == code)
    }

    /// Service numbers calling at `code` (empty if unknown).
    #[must_use]
    pub fn services(&self, code: &str) -> &[String] {
        self.services_by_stop.get(code).map_or(&[], Vec::as_slice)
    }

    /// True when older than [`CATALOG_TTL_SECS`] relative to `now`.
    #[must_use]
    pub const fn is_stale(&self, now: OffsetDateTime) -> bool {
        now.unix_timestamp().saturating_sub(self.fetched_at_unix) > CATALOG_TTL_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::{BusCatalog, BusStop, CATALOG_TTL_SECS};
    use std::collections::BTreeMap;
    use time::OffsetDateTime;

    fn sample() -> BusCatalog {
        let mut map = BTreeMap::new();
        map.insert("83139".to_owned(), vec!["15".to_owned(), "52".to_owned()]);
        BusCatalog {
            stops: vec![BusStop {
                code: "83139".to_owned(),
                name: "Clementi Ave 2 Blk 333".to_owned(),
                road: "Clementi Ave 2".to_owned(),
            }],
            services_by_stop: map,
            fetched_at_unix: 1_000_000,
        }
    }

    #[test]
    fn stop_and_services_lookup() {
        let c = sample();
        assert_eq!(c.stop("83139").map(|s| s.name.as_str()), Some("Clementi Ave 2 Blk 333"));
        assert!(c.stop("00000").is_none());
        assert_eq!(c.services("83139"), &["15".to_owned(), "52".to_owned()]);
        assert!(c.services("00000").is_empty());
    }

    #[test]
    fn staleness_uses_ttl() {
        let c = sample();
        let fresh = OffsetDateTime::from_unix_timestamp(1_000_000 + CATALOG_TTL_SECS).unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let old = OffsetDateTime::from_unix_timestamp(1_000_000 + CATALOG_TTL_SECS + 1).unwrap_or(OffsetDateTime::UNIX_EPOCH);
        assert!(!c.is_stale(fresh));
        assert!(c.is_stale(old));
    }
}
