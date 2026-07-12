//! Shared catalog assembly: invert routes into a stop→services map and stamp
//! a [`BusCatalog`]. Ungated — used by both the direct-`DataMall` fetch
//! (`bus_catalog::fetch`, which additionally parses the LTA `OData` page
//! envelope) and the worker-backed fetch (`worker::client::fetch_catalog`,
//! which gets plain JSON arrays and has no envelope to parse).

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::bus_catalog::model::{BusCatalog, BusStop};

/// Sort key for a service number: leading digits as a number, then the whole
/// string — so "2" < "15" < "151" < "151A", and non-numeric (e.g. "NR7") sort last.
pub fn service_sort_key(service: &str) -> (u32, String) {
    let digits: String = service.chars().take_while(char::is_ascii_digit).collect();
    let num = digits.parse::<u32>().unwrap_or(u32::MAX);
    (num, service.to_owned())
}

/// Assemble a full [`BusCatalog`] from parsed stops and raw `(stop, service)`
/// route pairs, stamped with `now`. Shared by the direct-`DataMall` fetch
/// (`bus_catalog::fetch`) and the worker-backed fetch
/// (`worker::client::fetch_catalog`) so both build the catalog identically.
#[must_use]
pub fn assemble_catalog(
    stops: Vec<BusStop>,
    pairs: Vec<(String, String)>,
    now: OffsetDateTime,
) -> BusCatalog {
    BusCatalog {
        stops,
        services_by_stop: build_services_by_stop(pairs),
        fetched_at_unix: now.unix_timestamp(),
    }
}

/// Invert `(stop, service)` pairs into `stop → sorted, deduped services`.
pub fn build_services_by_stop(pairs: Vec<(String, String)>) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (stop, service) in pairs {
        map.entry(stop).or_default().push(service);
    }
    for services in map.values_mut() {
        services.sort_by_key(|s| service_sort_key(s));
        services.dedup();
    }
    map
}

#[cfg(test)]
mod tests {
    use super::{build_services_by_stop, service_sort_key};

    #[test]
    fn service_sort_is_numeric_aware() {
        let mut v = vec![
            "151".to_owned(),
            "2".to_owned(),
            "15".to_owned(),
            "151A".to_owned(),
        ];
        v.sort_by_key(|s| service_sort_key(s));
        assert_eq!(v, vec!["2", "15", "151", "151A"]);
    }

    #[test]
    fn service_sort_non_numeric_sorts_last() {
        let mut v = vec!["NR7".to_owned(), "2".to_owned(), "15".to_owned()];
        v.sort_by_key(|s| service_sort_key(s));
        assert_eq!(v, vec!["2", "15", "NR7"]);
    }

    #[test]
    fn inverts_and_dedups() {
        let pairs = vec![
            ("83139".to_owned(), "52".to_owned()),
            ("83139".to_owned(), "15".to_owned()),
            ("83139".to_owned(), "15".to_owned()),
        ];
        let map = build_services_by_stop(pairs);
        assert_eq!(
            map.get("83139"),
            Some(&vec!["15".to_owned(), "52".to_owned()])
        );
    }
}
