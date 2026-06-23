# Bus Catalog Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a pure-Rust bus catalog to `sgbr-core` — fetch + cache all SG bus stops and the stop→lines map from LTA DataMall, and search stops fuzzily by name or code — so the UI redesign (separate plan) can offer stop search.

**Architecture:** A new `bus_catalog` module in `sgbr-core`, split by responsibility: `model` (types + staleness), `parse` (OData page → rows + the stop→services inversion), `fetch` (paginated DataMall calls), `store` (atomic JSON cache), `search` (`nucleo-matcher` fuzzy + code-prefix boost). Everything except the thin HTTP glue is unit-tested. The `Commute` model is untouched.

**Tech Stack:** Rust, `serde`/`serde_json`, `ureq` 3.0 (mirroring `lta/client.rs`), `time`, and `nucleo-matcher` 0.3 (Helix fuzzy matcher). Strict workspace lints apply (no `unwrap`/`expect`/`panic`/indexing; use `try_from`).

**This is plan 1 of 2.** Plan 2 (polished UI + stop-search flow) consumes the API defined here: `bus_catalog::store::{load,save}`, `fetch::fetch_catalog`, `search::search`, and `model::{BusStop, BusCatalog, CATALOG_TTL_SECS}`.

**Spec:** `docs/superpowers/specs/2026-06-23-polished-ui-stop-search-design.md`

---

## File Structure

- Create `crates/sgbr-core/src/bus_catalog/mod.rs` — module wiring + docs.
- Create `crates/sgbr-core/src/bus_catalog/model.rs` — `BusStop`, `BusCatalog`, `CATALOG_TTL_SECS`.
- Create `crates/sgbr-core/src/bus_catalog/parse.rs` — page parsing + `build_services_by_stop` + `service_sort_key`.
- Create `crates/sgbr-core/src/bus_catalog/store.rs` — `load`/`save`.
- Create `crates/sgbr-core/src/bus_catalog/search.rs` — `search`.
- Create `crates/sgbr-core/src/bus_catalog/fetch.rs` — `page_url`, `fetch_catalog`.
- Modify `crates/sgbr-core/src/lib.rs` — add `pub mod bus_catalog;`.
- Modify `crates/sgbr-core/Cargo.toml` — add `nucleo-matcher = "0.3"`.

Tests are inline `#[cfg(test)] mod tests` per file (matching the existing `sgbr-core` style, e.g. `lta/client.rs`).

> **Note on fetch concurrency:** the spec mentions concurrent page fetches. The total page count isn't known up front (OData has no count here), so this plan paginates **sequentially** until a short page — simple and correct. The once-a-month background refresh needn't be parallel; the *search* is the hot path and is the optimized part. Revisit only if the refresh proves too slow on-device.

---

## Task 1: Module scaffold + dependency

**Files:**
- Modify: `crates/sgbr-core/Cargo.toml`
- Create: `crates/sgbr-core/src/bus_catalog/mod.rs`
- Modify: `crates/sgbr-core/src/lib.rs`

- [ ] **Step 1: Add the dependency.** In `crates/sgbr-core/Cargo.toml`, under `[dependencies]`, add:

```toml
nucleo-matcher = "0.3"
```

- [ ] **Step 2: Create the module file** `crates/sgbr-core/src/bus_catalog/mod.rs`:

```rust
//! Static LTA bus catalog: a stop directory + the stop→services map, with a
//! cached fetch and fuzzy search. Pure logic; the app owns refresh scheduling.

pub mod fetch;
pub mod model;
pub(crate) mod parse;
pub mod search;
pub mod store;
```

- [ ] **Step 3: Register the module.** In `crates/sgbr-core/src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod bus_catalog;
```

- [ ] **Step 4: Add empty stubs so it compiles.** Create the four remaining files with a single doc line each so `mod.rs` resolves (they're filled in later tasks):
  - `model.rs`: `//! Catalog types.`
  - `parse.rs`: `//! OData page parsing + stop→services inversion.`
  - `store.rs`: `//! Atomic JSON cache for the catalog.`
  - `search.rs`: `//! Fuzzy stop search.`
  - `fetch.rs`: `//! Paginated DataMall fetch.`

- [ ] **Step 5: Verify it builds.**

Run: `cargo build -p sgbr-core`
Expected: compiles (empty modules), `nucleo-matcher` resolves.

- [ ] **Step 6: Commit.**

```bash
git add crates/sgbr-core/Cargo.toml crates/sgbr-core/src/bus_catalog crates/sgbr-core/src/lib.rs Cargo.lock
git commit -m "feat(core): scaffold bus_catalog module + nucleo-matcher dep"
```

---

## Task 2: Model — `BusStop`, `BusCatalog`, staleness

**Files:**
- Modify: `crates/sgbr-core/src/bus_catalog/model.rs`

- [ ] **Step 1: Write the failing test.** Replace `model.rs` contents with the types + tests:

```rust
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
    pub fn is_stale(&self, now: OffsetDateTime) -> bool {
        now.unix_timestamp() - self.fetched_at_unix > CATALOG_TTL_SECS
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
```

> The test's `from_unix_timestamp(...).unwrap_or(...)` keeps it within `unwrap_used` (it's `unwrap_or`, not `unwrap`).

- [ ] **Step 2: Run the tests.**

Run: `cargo test -p sgbr-core bus_catalog::model`
Expected: PASS (2 tests).

- [ ] **Step 3: Lint clean.**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit.**

```bash
git add crates/sgbr-core/src/bus_catalog/model.rs
git commit -m "feat(core): bus_catalog model + staleness"
```

---

## Task 3: Parse — pages + stop→services inversion

**Files:**
- Modify: `crates/sgbr-core/src/bus_catalog/parse.rs`

- [ ] **Step 1: Write parse + inversion + tests.** Replace `parse.rs`:

```rust
//! Parse LTA OData pages (`{"value":[...]}`) and invert routes into a map.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::bus_catalog::model::BusStop;
use crate::error::CoreError;

#[derive(Deserialize)]
struct StopsPage {
    value: Vec<RawStop>,
}

#[derive(Deserialize)]
struct RawStop {
    #[serde(rename = "BusStopCode")]
    code: String,
    #[serde(rename = "Description")]
    name: String,
    #[serde(rename = "RoadName")]
    road: String,
}

/// Parse one `BusStops` page into [`BusStop`]s.
pub(crate) fn parse_stops_page(json: &str) -> Result<Vec<BusStop>, CoreError> {
    let page: StopsPage = serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(page
        .value
        .into_iter()
        .map(|r| BusStop { code: r.code, name: r.name, road: r.road })
        .collect())
}

#[derive(Deserialize)]
struct RoutesPage {
    value: Vec<RawRoute>,
}

#[derive(Deserialize)]
struct RawRoute {
    #[serde(rename = "ServiceNo")]
    service: String,
    #[serde(rename = "BusStopCode")]
    stop: String,
}

/// Parse one `BusRoutes` page into `(stop_code, service_no)` pairs.
pub(crate) fn parse_routes_page(json: &str) -> Result<Vec<(String, String)>, CoreError> {
    let page: RoutesPage = serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(page.value.into_iter().map(|r| (r.stop, r.service)).collect())
}

/// Sort key for a service number: leading digits as a number, then the whole
/// string — so "2" < "15" < "151" < "151A", and non-numeric (e.g. "NR7") sort last.
pub(crate) fn service_sort_key(service: &str) -> (u32, String) {
    let digits: String = service.chars().take_while(char::is_ascii_digit).collect();
    let num = digits.parse::<u32>().unwrap_or(u32::MAX);
    (num, service.to_owned())
}

/// Invert `(stop, service)` pairs into `stop → sorted, deduped services`.
pub(crate) fn build_services_by_stop(pairs: Vec<(String, String)>) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (stop, service) in pairs {
        map.entry(stop).or_default().push(service);
    }
    for services in map.values_mut() {
        services.sort_by(|a, b| service_sort_key(a).cmp(&service_sort_key(b)));
        services.dedup();
    }
    map
}

#[cfg(test)]
mod tests {
    use super::{build_services_by_stop, parse_routes_page, parse_stops_page, service_sort_key};

    #[test]
    fn parses_stops_page() {
        let json = r#"{"value":[
            {"BusStopCode":"01012","RoadName":"Victoria St","Description":"Hotel Grand Pacific","Latitude":1.0,"Longitude":2.0}
        ]}"#;
        let stops = parse_stops_page(json).unwrap_or_default();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops.first().map(|s| s.name.as_str()), Some("Hotel Grand Pacific"));
        assert_eq!(stops.first().map(|s| s.road.as_str()), Some("Victoria St"));
    }

    #[test]
    fn parses_routes_page() {
        let json = r#"{"value":[
            {"ServiceNo":"15","BusStopCode":"83139","Direction":1,"StopSequence":5},
            {"ServiceNo":"52","BusStopCode":"83139","Direction":1,"StopSequence":9}
        ]}"#;
        let pairs = parse_routes_page(json).unwrap_or_default();
        assert_eq!(pairs, vec![("83139".to_owned(), "15".to_owned()), ("83139".to_owned(), "52".to_owned())]);
    }

    #[test]
    fn service_sort_is_numeric_aware() {
        let mut v = vec!["151".to_owned(), "2".to_owned(), "15".to_owned(), "151A".to_owned()];
        v.sort_by(|a, b| service_sort_key(a).cmp(&service_sort_key(b)));
        assert_eq!(v, vec!["2", "15", "151", "151A"]);
    }

    #[test]
    fn inverts_and_dedups() {
        let pairs = vec![
            ("83139".to_owned(), "52".to_owned()),
            ("83139".to_owned(), "15".to_owned()),
            ("83139".to_owned(), "15".to_owned()),
        ];
        let map = build_services_by_stop(pairs);
        assert_eq!(map.get("83139"), Some(&vec!["15".to_owned(), "52".to_owned()]));
    }
}
```

> The tests use `.unwrap()`/`.unwrap_or_default()`; `unwrap` is allowed in `#[cfg(test)]`? **No** — the strict lints apply to tests too. The examples above use `unwrap_or_default()` and `.map(...)` to stay clean. Keep that style; do not introduce bare `.unwrap()`.

- [ ] **Step 2: Run the tests.**

Run: `cargo test -p sgbr-core bus_catalog::parse`
Expected: PASS (4 tests).

- [ ] **Step 3: Lint clean.**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit.**

```bash
git add crates/sgbr-core/src/bus_catalog/parse.rs
git commit -m "feat(core): bus_catalog page parsing + stop→services inversion"
```

---

## Task 4: Store — atomic JSON cache

**Files:**
- Modify: `crates/sgbr-core/src/bus_catalog/store.rs`

- [ ] **Step 1: Write load/save + round-trip test.** Replace `store.rs`:

```rust
//! Atomic JSON cache for the catalog (mirrors `commute::store`).

use std::fs;
use std::path::Path;

use crate::bus_catalog::model::BusCatalog;
use crate::error::CoreError;

/// Load a catalog from `path`. A missing or invalid file is an error; the caller
/// may then fetch a fresh one.
pub fn load(path: &Path) -> Result<BusCatalog, CoreError> {
    let bytes = fs::read(path).map_err(|e| CoreError::Io(e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|e| CoreError::Parse(e.to_string()))
}

/// Save `catalog` to `path` atomically (temp file + rename), creating parents.
pub fn save(catalog: &BusCatalog, path: &Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::Io(e.to_string()))?;
    }
    let bytes = serde_json::to_vec(catalog).map_err(|e| CoreError::Parse(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|e| CoreError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| CoreError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{load, save};
    use crate::bus_catalog::model::{BusCatalog, BusStop};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sgbr_catalog_{}_{name}.json", std::process::id()))
    }

    #[test]
    fn round_trips() {
        let mut map = BTreeMap::new();
        map.insert("83139".to_owned(), vec!["15".to_owned()]);
        let catalog = BusCatalog {
            stops: vec![BusStop { code: "83139".to_owned(), name: "Clementi".to_owned(), road: "Ave 2".to_owned() }],
            services_by_stop: map,
            fetched_at_unix: 42,
        };
        let path = tmp_path("round");
        assert!(save(&catalog, &path).is_ok());
        let loaded = load(&path).unwrap_or_default();
        assert_eq!(loaded, catalog);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_is_err() {
        assert!(load(&tmp_path("nope")).is_err());
    }
}
```

- [ ] **Step 2: Run the tests.**

Run: `cargo test -p sgbr-core bus_catalog::store`
Expected: PASS (2 tests).

- [ ] **Step 3: Lint clean.**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit.**

```bash
git add crates/sgbr-core/src/bus_catalog/store.rs
git commit -m "feat(core): bus_catalog atomic JSON cache"
```

---

## Task 5: Search — fuzzy name + code-prefix boost

**Files:**
- Modify: `crates/sgbr-core/src/bus_catalog/search.rs`

- [ ] **Step 1: Write search + tests.** Replace `search.rs`:

```rust
//! Fuzzy stop search over names, with a boost for stop-code prefix matches.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::bus_catalog::model::{BusCatalog, BusStop};

/// Added when the stop code prefix-matches the query, so code hits outrank fuzzy
/// name hits ("83139" jumps that stop to the top).
const CODE_BOOST: u32 = 1_000_000;

/// Up to `limit` stops best matching `query` (fuzzy on name, prefix on code),
/// best first. An empty/whitespace query yields no results.
#[must_use]
pub fn search<'a>(catalog: &'a BusCatalog, query: &str, limit: usize) -> Vec<&'a BusStop> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(trimmed, CaseMatching::Ignore, Normalization::Smart);

    let mut buf: Vec<char> = Vec::new();
    let mut scored: Vec<(u32, &BusStop)> = Vec::new();
    for stop in &catalog.stops {
        let name_score = pattern.score(Utf32Str::new(&stop.name, &mut buf), &mut matcher);
        let code_hit = stop.code.starts_with(trimmed);
        match (name_score, code_hit) {
            (Some(s), true) => scored.push((s.saturating_add(CODE_BOOST), stop)),
            (Some(s), false) => scored.push((s, stop)),
            (None, true) => scored.push((CODE_BOOST, stop)),
            (None, false) => {}
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, stop)| stop).collect()
}

#[cfg(test)]
mod tests {
    use super::search;
    use crate::bus_catalog::model::{BusCatalog, BusStop};

    fn stop(code: &str, name: &str) -> BusStop {
        BusStop { code: code.to_owned(), name: name.to_owned(), road: String::new() }
    }

    fn catalog() -> BusCatalog {
        BusCatalog {
            stops: vec![
                stop("17009", "Clementi Int"),
                stop("83139", "Clementi Ave 2 Blk 333"),
                stop("01012", "Hotel Grand Pacific"),
                stop("16009", "NUS Clementi Rd"),
            ],
            ..BusCatalog::default()
        }
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(search(&catalog(), "  ", 10).is_empty());
    }

    #[test]
    fn fuzzy_name_match() {
        let results = search(&catalog(), "clementi", 10);
        assert!(results.iter().all(|s| s.name.contains("Clementi")));
        assert!(results.len() >= 3);
        assert!(!results.iter().any(|s| s.code == "01012"));
    }

    #[test]
    fn code_prefix_ranks_first() {
        let results = search(&catalog(), "83139", 10);
        assert_eq!(results.first().map(|s| s.code.as_str()), Some("83139"));
    }
}
```

- [ ] **Step 2: Run the tests.**

Run: `cargo test -p sgbr-core bus_catalog::search`
Expected: PASS (3 tests). If `Pattern::score`/`Utf32Str::new` signatures differ in the resolved 0.3.x, adjust to the compiler's guidance (the shape is: parse a `Pattern`, score each name's `Utf32Str` against it, `Option<u32>`).

- [ ] **Step 3: Lint clean.**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit.**

```bash
git add crates/sgbr-core/src/bus_catalog/search.rs
git commit -m "feat(core): bus_catalog fuzzy stop search"
```

---

## Task 6: Fetch — paginated DataMall assembly

**Files:**
- Modify: `crates/sgbr-core/src/bus_catalog/fetch.rs`

- [ ] **Step 1: Write fetch glue + the `page_url` test.** Replace `fetch.rs`:

```rust
//! Fetch the full catalog from LTA DataMall (paginated OData), then assemble it.
//! Pagination is sequential (page count is unknown up front); this runs on a
//! background thread in the app, so wall-time isn't on any UI path.

use std::time::Duration;

use time::OffsetDateTime;
use ureq::Agent;

use crate::bus_catalog::model::BusCatalog;
use crate::bus_catalog::parse::{build_services_by_stop, parse_routes_page, parse_stops_page};
use crate::error::CoreError;

const BUS_STOPS_URL: &str = "https://datamall2.mytransport.sg/ltaodataservice/BusStops";
const BUS_ROUTES_URL: &str = "https://datamall2.mytransport.sg/ltaodataservice/BusRoutes";
const PAGE_SIZE: usize = 500;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// OData page URL: `{base}?$skip={skip}`.
#[must_use]
pub fn page_url(base: &str, skip: usize) -> String {
    format!("{base}?$skip={skip}")
}

fn fetch_page(account_key: &str, url: &str) -> Result<String, CoreError> {
    let config = Agent::config_builder().timeout_global(Some(REQUEST_TIMEOUT)).build();
    let agent = Agent::new_with_config(config);
    agent
        .get(url)
        .header("AccountKey", account_key)
        .header("accept", "application/json")
        .call()
        .map_err(|e| CoreError::Http(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| CoreError::Http(e.to_string()))
}

fn fetch_all<T>(
    account_key: &str,
    base: &str,
    parse: impl Fn(&str) -> Result<Vec<T>, CoreError>,
) -> Result<Vec<T>, CoreError> {
    let mut all: Vec<T> = Vec::new();
    let mut skip = 0;
    loop {
        let json = fetch_page(account_key, &page_url(base, skip))?;
        let page = parse(&json)?;
        let count = page.len();
        all.extend(page);
        if count < PAGE_SIZE {
            break;
        }
        skip += PAGE_SIZE;
    }
    Ok(all)
}

/// Fetch and assemble the whole catalog; `now` stamps `fetched_at_unix`.
///
/// # Errors
/// Returns [`CoreError::Http`]/[`CoreError::Parse`] on any page failure (partial
/// data is discarded — the caller keeps its existing cache).
pub fn fetch_catalog(account_key: &str, now: OffsetDateTime) -> Result<BusCatalog, CoreError> {
    let stops = fetch_all(account_key, BUS_STOPS_URL, parse_stops_page)?;
    let pairs = fetch_all(account_key, BUS_ROUTES_URL, parse_routes_page)?;
    Ok(BusCatalog {
        stops,
        services_by_stop: build_services_by_stop(pairs),
        fetched_at_unix: now.unix_timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::{page_url, BUS_STOPS_URL};

    #[test]
    fn builds_skip_url() {
        assert_eq!(
            page_url(BUS_STOPS_URL, 500),
            "https://datamall2.mytransport.sg/ltaodataservice/BusStops?$skip=500"
        );
    }
}
```

- [ ] **Step 2: Run the test.**

Run: `cargo test -p sgbr-core bus_catalog::fetch`
Expected: PASS (1 test). `fetch_catalog`/`fetch_all`/`fetch_page` are networked glue — not unit-tested here; they're exercised live in Plan 2's on-device verification.

- [ ] **Step 3: Lint clean.**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit.**

```bash
git add crates/sgbr-core/src/bus_catalog/fetch.rs
git commit -m "feat(core): bus_catalog paginated DataMall fetch"
```

---

## Task 7: Live smoke check + finalize

**Files:** none (verification only).

- [ ] **Step 1: Full test + lint sweep.**

Run: `cargo test --workspace`
Expected: all prior 54 tests + the new bus_catalog tests pass.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: One-off live fetch smoke test (manual, optional but recommended).** Add a temporary throwaway test (do NOT commit it) to confirm the real endpoints assemble against your key:

```rust
// TEMP — in fetch.rs tests, run with: cargo test -p sgbr-core live_fetch -- --ignored --nocapture
#[test]
#[ignore]
fn live_fetch() {
    let key = std::env::var("LTA_API_ACCOUNT_KEY").unwrap_or_default();
    let now = time::OffsetDateTime::now_utc();
    let catalog = super::fetch_catalog(&key, now).unwrap_or_default();
    println!("stops={} stops_with_services={}", catalog.stops.len(), catalog.services_by_stop.len());
    assert!(catalog.stops.len() > 4000);
    assert!(!catalog.services("83139").is_empty());
}
```

Run: `set -a; . ./.env; set +a; cargo test -p sgbr-core live_fetch -- --ignored --nocapture`
Expected: prints ~5000 stops and a non-empty service list for 83139. **Then delete this temp test** (network tests don't belong in the committed suite).

- [ ] **Step 3: Confirm the public API surface** the UI (Plan 2) will call exists and is `pub`:
  - `sgbr_core::bus_catalog::model::{BusStop, BusCatalog, CATALOG_TTL_SECS}`
  - `sgbr_core::bus_catalog::store::{load, save}`
  - `sgbr_core::bus_catalog::fetch::fetch_catalog`
  - `sgbr_core::bus_catalog::search::search`

Run: `cargo doc -p sgbr-core --no-deps`
Expected: builds; the items above appear.

- [ ] **Step 4: Commit (if Step 1 produced any formatting/lock changes).**

```bash
git add -A
git commit -m "chore(core): bus_catalog test/lint sweep" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Spec coverage:** catalog model + `services_by_stop` (Task 2/3); cache with ~30-day TTL + staleness (Task 2/4); paginated fetch of BusStops + BusRoutes (Task 6); `nucleo-matcher` fuzzy search over name + code-prefix boost (Task 5). The refresh *scheduling* and all UI live in Plan 2 — intentionally out of this plan.
- **Deviation (honest):** fetch is sequential, not concurrent (page count unknown up front; the monthly background refresh isn't on a UI path). Search — the hot path — is the optimized part. Flagged in the header note.
- **Lints:** every snippet avoids `unwrap`/`expect`/`panic`/indexing and uses `try_from`-free safe ops; tests use `unwrap_or_default`/`map`, never bare `unwrap`. `nucleo-matcher` is the only new dep.
- **Type consistency:** `BusStop{code,name,road}`, `BusCatalog{stops, services_by_stop: BTreeMap<String,Vec<String>>, fetched_at_unix}`, `search(&BusCatalog, &str, usize) -> Vec<&BusStop>`, `fetch_catalog(&str, OffsetDateTime) -> Result<BusCatalog, CoreError>`, `store::{load,save}(&Path,...)` — used identically across tasks and match what Plan 2 will call.
