# Core Data Layer + Slint Desktop Proof — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and test the pure-Rust LTA bus-arrival core, then render live arrivals for one stop in a Slint window on the desktop — proving the core↔UI binding before any phone/Mac work.

**Architecture:** A `sgbr-core` library crate owns all logic: typed DataMall models, an `EstimatedArrival`→minutes calculation (time-injected, so it's deterministic and testable), and a blocking HTTP client. The existing root binary becomes a thin Slint desktop app that converts core data into a view model and displays it. No platform-specific code yet.

**Tech Stack:** Rust 2024 (toolchain 1.96.0), `serde`/`serde_json`, `time` (RFC3339 parsing), `ureq` (blocking HTTP + rustls — no async runtime, mobile-friendly), `thiserror`, `slint` (native GPU UI). Tests use `cargo nextest`; lints are the workspace `[lints]` bar already in the repo.

**Scope boundary / follow-up plans (NOT in this plan):**
- `2026-..-android-bridge-spike.md` — Slint on a real Android device + JNI notification + Glance widget (Linux-buildable, needs NDK + device).
- `2026-..-ios-bridge-spike.md` — Slint on a real iPhone + Obj-C notification + WidgetKit widget (**requires macOS + Xcode**).
- Favourites, search, reminders scheduling, persistence, widget refresh — later feature plans.

**Prerequisite for the live-fetch task only:** a free LTA DataMall AccountKey (https://datamall.lta.gov.sg/). All unit tests run offline against recorded JSON, so the key is needed only for Task 7's manual live check.

---

### Task 1: Scaffold the `sgbr-core` library crate

**Files:**
- Create: `crates/sgbr-core/Cargo.toml`
- Create: `crates/sgbr-core/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members` + `[workspace.dependencies]`)

- [ ] **Step 1: Add the crate to the workspace and shared deps**

In root `Cargo.toml`, change the `[workspace]` table to add members, and extend `[workspace.dependencies]`:

```toml
[workspace]
resolver = "3"
members = ["crates/sgbr-core"]

[workspace.dependencies]
# Shared across crates — extended as the implementation plan lands.
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
tracing = "0.1"
time = { version = "0.3", features = ["parsing", "formatting", "macros"] }
ureq = { version = "3.0", features = ["json"] }

# Internal crates
sgbr-core = { version = "0.1.0", path = "crates/sgbr-core" }
```

- [ ] **Step 2: Create the crate manifest**

`crates/sgbr-core/Cargo.toml`:

```toml
[package]
name = "sgbr-core"
version = "0.1.0"
edition = "2024"

[lints]
workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
time = { workspace = true }
ureq = { workspace = true }

[dev-dependencies]
time = { workspace = true }
```

- [ ] **Step 3: Create the crate root**

`crates/sgbr-core/src/lib.rs`:

```rust
//! Pure-Rust core for SG Bus Ready: LTA DataMall models, arrival-time maths,
//! and the bus-arrival HTTP client. No platform or UI code lives here.

pub mod error;
pub mod lta;
```

- [ ] **Step 4: Verify the workspace still resolves**

Run: `cargo metadata --no-deps --format-version 1 >/dev/null && echo OK`
Expected: prints `OK` (and `cargo` does not error about the missing modules yet — it will until Task 2/3 add them; if so, proceed to Task 2 before re-running).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/sgbr-core/Cargo.toml crates/sgbr-core/src/lib.rs
git commit -m "chore: scaffold sgbr-core crate"
```

---

### Task 2: Error type

**Files:**
- Create: `crates/sgbr-core/src/error.rs`
- Test: inline `#[cfg(test)]` in the same file

- [ ] **Step 1: Write the failing test**

`crates/sgbr-core/src/error.rs`:

```rust
//! Crate-wide error type.

use thiserror::Error;

/// Errors produced by the core layer.
#[derive(Debug, Error)]
pub enum CoreError {
    /// The HTTP request to DataMall failed (network, TLS, status).
    #[error("datamall request failed: {0}")]
    Http(String),

    /// The DataMall response body could not be parsed as the expected JSON.
    #[error("failed to parse datamall response: {0}")]
    Parse(String),

    /// An `EstimatedArrival` field was empty (no bus scheduled).
    #[error("no estimated arrival available")]
    NoArrival,

    /// An `EstimatedArrival` timestamp was not valid RFC3339.
    #[error("invalid arrival timestamp: {0}")]
    InvalidTimestamp(String),
}

#[cfg(test)]
mod tests {
    use super::CoreError;

    #[test]
    fn display_includes_context() {
        let err = CoreError::Http("timeout".to_owned());
        assert_eq!(err.to_string(), "datamall request failed: timeout");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails (then passes)**

Run: `cargo nextest run -p sgbr-core error::`
Expected: compiles and PASSES (this task is the type + its test together; the assertion pins the `Display` output).

- [ ] **Step 3: Run clippy on the crate**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/sgbr-core/src/error.rs
git commit -m "feat(core): add CoreError type"
```

---

### Task 3: DataMall response models

**Files:**
- Create: `crates/sgbr-core/src/lta/mod.rs`
- Create: `crates/sgbr-core/src/lta/model.rs`

- [ ] **Step 1: Create the module file**

`crates/sgbr-core/src/lta/mod.rs`:

```rust
//! LTA DataMall bus-arrival client, models, and arrival-time maths.

pub mod arrival;
pub mod client;
pub mod model;
```

- [ ] **Step 2: Write the failing test for deserialization**

`crates/sgbr-core/src/lta/model.rs`:

```rust
//! Typed mirror of the DataMall Bus Arrival JSON response.
//!
//! Only the fields used by the app are modelled; unknown fields are ignored.

use serde::Deserialize;

/// Top-level Bus Arrival response for one bus stop.
#[derive(Debug, Clone, Deserialize)]
pub struct BusArrivalResponse {
    /// The queried 5-digit bus stop code.
    #[serde(rename = "BusStopCode")]
    pub bus_stop_code: String,
    /// One entry per bus service that calls at this stop.
    #[serde(rename = "Services")]
    pub services: Vec<Service>,
}

/// Arrival info for a single bus service at the stop.
#[derive(Debug, Clone, Deserialize)]
pub struct Service {
    /// The public service number, e.g. `"15"` or `"67"`.
    #[serde(rename = "ServiceNo")]
    pub service_no: String,
    /// The next bus to arrive.
    #[serde(rename = "NextBus")]
    pub next_bus: NextBus,
    /// The bus after that (fields may be empty late at night).
    #[serde(rename = "NextBus2")]
    pub next_bus2: NextBus,
    /// The third bus (fields may be empty late at night).
    #[serde(rename = "NextBus3")]
    pub next_bus3: NextBus,
}

/// One predicted arrival. `estimated_arrival` is empty when no bus is expected.
#[derive(Debug, Clone, Deserialize)]
pub struct NextBus {
    /// RFC3339 timestamp (e.g. `2026-06-21T08:18:00+08:00`), or `""` if none.
    #[serde(rename = "EstimatedArrival")]
    pub estimated_arrival: String,
}

#[cfg(test)]
mod tests {
    use super::BusArrivalResponse;

    /// A trimmed but structurally faithful DataMall response sample.
    const SAMPLE: &str = r#"{
      "BusStopCode": "83139",
      "Services": [
        {
          "ServiceNo": "15",
          "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
          "NextBus2": { "EstimatedArrival": "2026-06-21T08:25:00+08:00" },
          "NextBus3": { "EstimatedArrival": "" }
        }
      ]
    }"#;

    #[test]
    fn parses_sample_response() {
        let parsed: BusArrivalResponse =
            serde_json::from_str(SAMPLE).expect("sample should parse");
        assert_eq!(parsed.bus_stop_code, "83139");
        assert_eq!(parsed.services.len(), 1);
        let svc = parsed.services.first().expect("one service");
        assert_eq!(svc.service_no, "15");
        assert_eq!(svc.next_bus.estimated_arrival, "2026-06-21T08:18:00+08:00");
        assert_eq!(svc.next_bus3.estimated_arrival, "");
    }
}
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo nextest run -p sgbr-core model::`
Expected: `parses_sample_response` PASSES.

- [ ] **Step 4: Run clippy**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/lta/mod.rs crates/sgbr-core/src/lta/model.rs
git commit -m "feat(core): add DataMall bus-arrival models"
```

---

### Task 4: Minutes-until-arrival calculation

**Files:**
- Create: `crates/sgbr-core/src/lta/arrival.rs`

This is the testable heart of the app. `now` is injected (never read from the clock inside the function) so tests are deterministic and no float maths is used.

- [ ] **Step 1: Write the failing tests**

`crates/sgbr-core/src/lta/arrival.rs`:

```rust
//! Convert raw `EstimatedArrival` strings into whole-minute countdowns.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::CoreError;

/// Whole minutes from `now` until `estimated_arrival`.
///
/// Returns [`CoreError::NoArrival`] for an empty string, and
/// [`CoreError::InvalidTimestamp`] for a non-RFC3339 value. A bus already due
/// or just departed yields `0` or a negative number; the caller decides how to
/// present that.
pub fn minutes_until(
    estimated_arrival: &str,
    now: OffsetDateTime,
) -> Result<i64, CoreError> {
    if estimated_arrival.is_empty() {
        return Err(CoreError::NoArrival);
    }
    let eta = OffsetDateTime::parse(estimated_arrival, &Rfc3339)
        .map_err(|e| CoreError::InvalidTimestamp(e.to_string()))?;
    Ok((eta - now).whole_minutes())
}

#[cfg(test)]
mod tests {
    use super::minutes_until;
    use crate::error::CoreError;
    use time::macros::datetime;

    #[test]
    fn computes_future_minutes() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        let mins = minutes_until("2026-06-21T08:18:00+08:00", now)
            .expect("valid future arrival");
        assert_eq!(mins, 8);
    }

    #[test]
    fn truncates_to_whole_minutes() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        // 8m30s away -> 8 whole minutes.
        let mins = minutes_until("2026-06-21T08:18:30+08:00", now)
            .expect("valid arrival");
        assert_eq!(mins, 8);
    }

    #[test]
    fn handles_different_offset_same_instant() {
        // now expressed in +08:00; arrival expressed in UTC for the same wall
        // clock instant 08:18 SGT == 00:18 UTC.
        let now = datetime!(2026-06-21 08:10:00 +8);
        let mins = minutes_until("2026-06-21T00:18:00+00:00", now)
            .expect("valid arrival");
        assert_eq!(mins, 8);
    }

    #[test]
    fn empty_string_is_no_arrival() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        let err = minutes_until("", now).unwrap_err();
        assert!(matches!(err, CoreError::NoArrival));
    }

    #[test]
    fn garbage_is_invalid_timestamp() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        let err = minutes_until("not-a-date", now).unwrap_err();
        assert!(matches!(err, CoreError::InvalidTimestamp(_)));
    }

    #[test]
    fn past_arrival_is_negative() {
        let now = datetime!(2026-06-21 08:20:00 +8);
        let mins = minutes_until("2026-06-21T08:18:00+08:00", now)
            .expect("valid past arrival");
        assert_eq!(mins, -2);
    }
}
```

- [ ] **Step 2: Run the tests to verify they pass**

Run: `cargo nextest run -p sgbr-core arrival::`
Expected: all six tests PASS.

- [ ] **Step 3: Run clippy (note: `float_arithmetic` is denied — confirm none crept in)**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/sgbr-core/src/lta/arrival.rs
git commit -m "feat(core): add minutes_until arrival calculation"
```

---

### Task 5: Arrival view model (core → UI boundary)

**Files:**
- Modify: `crates/sgbr-core/src/lta/arrival.rs` (append)

A small, UI-agnostic struct the Slint layer (and later the widget) will consume, so the UI never touches raw DataMall types.

- [ ] **Step 1: Write the failing test**

Append to `crates/sgbr-core/src/lta/arrival.rs`:

```rust
use crate::lta::model::BusArrivalResponse;

/// One service's next arrivals, reduced to whole-minute countdowns, ready for
/// display. `minutes` holds up to three entries; empty/invalid slots are
/// dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceArrivals {
    /// Public service number, e.g. `"15"`.
    pub service_no: String,
    /// Whole-minute countdowns for the next buses (0–3 entries).
    pub minutes: Vec<i64>,
}

/// Build display-ready arrivals for every service in a response, relative to
/// `now`. Slots with no/invalid timestamps are skipped (not errors).
#[must_use]
pub fn service_arrivals(
    response: &BusArrivalResponse,
    now: OffsetDateTime,
) -> Vec<ServiceArrivals> {
    response
        .services
        .iter()
        .map(|svc| {
            let slots = [&svc.next_bus, &svc.next_bus2, &svc.next_bus3];
            let minutes = slots
                .into_iter()
                .filter_map(|b| minutes_until(&b.estimated_arrival, now).ok())
                .collect();
            ServiceArrivals {
                service_no: svc.service_no.clone(),
                minutes,
            }
        })
        .collect()
}

#[cfg(test)]
mod view_model_tests {
    use super::{service_arrivals, ServiceArrivals};
    use crate::lta::model::BusArrivalResponse;
    use time::macros::datetime;

    const SAMPLE: &str = r#"{
      "BusStopCode": "83139",
      "Services": [
        {
          "ServiceNo": "15",
          "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
          "NextBus2": { "EstimatedArrival": "2026-06-21T08:25:00+08:00" },
          "NextBus3": { "EstimatedArrival": "" }
        }
      ]
    }"#;

    #[test]
    fn drops_empty_slots_and_keeps_order() {
        let resp: BusArrivalResponse =
            serde_json::from_str(SAMPLE).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let out = service_arrivals(&resp, now);
        assert_eq!(
            out,
            vec![ServiceArrivals {
                service_no: "15".to_owned(),
                minutes: vec![8, 15],
            }]
        );
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p sgbr-core arrival::`
Expected: the new `drops_empty_slots_and_keeps_order` plus all earlier arrival tests PASS.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/sgbr-core/src/lta/arrival.rs
git commit -m "feat(core): add ServiceArrivals view model"
```

---

### Task 6: Bus-arrival HTTP client

**Files:**
- Create: `crates/sgbr-core/src/lta/client.rs`

Blocking `ureq` call. The endpoint base and stop code are separate so the URL is testable without a network. The live call is exercised manually in Task 7.

> **Verify before shipping:** confirm the exact path/version against the LTA DataMall API User Guide v6.8. The historically stable path is `ltaodataservice/BusArrivalv2`; the JSON shape modelled in Task 3 is unchanged across versions.

- [ ] **Step 1: Write the failing test for URL construction**

`crates/sgbr-core/src/lta/client.rs`:

```rust
//! Blocking client for the DataMall Bus Arrival endpoint.

use crate::error::CoreError;
use crate::lta::model::BusArrivalResponse;

/// Base URL for the Bus Arrival v2 endpoint (no query string).
pub const BUS_ARRIVAL_URL: &str =
    "https://datamall2.mytransport.sg/ltaodataservice/BusArrivalv2";

/// Build the full request URL for a given stop code.
#[must_use]
pub fn arrival_url(base: &str, bus_stop_code: &str) -> String {
    format!("{base}?BusStopCode={bus_stop_code}")
}

/// Fetch and parse live arrivals for `bus_stop_code` using `account_key`.
///
/// # Errors
/// Returns [`CoreError::Http`] on transport/status failure and
/// [`CoreError::Parse`] when the body is not the expected JSON.
pub fn fetch_arrivals(
    account_key: &str,
    bus_stop_code: &str,
) -> Result<BusArrivalResponse, CoreError> {
    let url = arrival_url(BUS_ARRIVAL_URL, bus_stop_code);
    let body = ureq::get(&url)
        .header("AccountKey", account_key)
        .header("accept", "application/json")
        .call()
        .map_err(|e| CoreError::Http(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| CoreError::Http(e.to_string()))?;
    serde_json::from_str(&body).map_err(|e| CoreError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{arrival_url, BUS_ARRIVAL_URL};

    #[test]
    fn builds_query_url() {
        let url = arrival_url(BUS_ARRIVAL_URL, "83139");
        assert_eq!(
            url,
            "https://datamall2.mytransport.sg/ltaodataservice/BusArrivalv2?BusStopCode=83139"
        );
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo nextest run -p sgbr-core client::`
Expected: `builds_query_url` PASSES (no network used).

- [ ] **Step 3: Run clippy + full crate test suite**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings && cargo nextest run -p sgbr-core`
Expected: clippy clean; all tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sgbr-core/src/lta/client.rs
git commit -m "feat(core): add blocking bus-arrival HTTP client"
```

---

### Task 7: Manual live-fetch check (needs DataMall key)

**Files:**
- Create: `crates/sgbr-core/examples/live_fetch.rs`

A tiny example binary to confirm the real endpoint works end to end. Not part of the unit suite (it needs network + a key).

- [ ] **Step 1: Write the example**

`crates/sgbr-core/examples/live_fetch.rs`:

```rust
//! Manual smoke test: `LTA_ACCOUNT_KEY=... cargo run -p sgbr-core --example live_fetch -- 83139`
//!
//! Prints the next-bus countdowns for one stop using live DataMall data.

use std::env;

use sgbr_core::lta::arrival::service_arrivals;
use sgbr_core::lta::client::fetch_arrivals;
use time::OffsetDateTime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = env::var("LTA_ACCOUNT_KEY")
        .map_err(|_| "set LTA_ACCOUNT_KEY in the environment")?;
    let stop = env::args().nth(1).unwrap_or_else(|| "83139".to_owned());

    let response = fetch_arrivals(&key, &stop)?;
    let now = OffsetDateTime::now_utc();
    for svc in service_arrivals(&response, now) {
        // `print_stdout` is denied in library code, but examples are binaries
        // outside the workspace lint table; this is fine here.
        println!("bus {}: {:?} min", svc.service_no, svc.minutes);
    }
    Ok(())
}
```

> Note: if the workspace lint table still flags `println!` in the example, add `#![allow(clippy::print_stdout)]` at the top of this file with a `// reason = "manual example binary"`.

- [ ] **Step 2: Run it against the live API**

Run: `LTA_ACCOUNT_KEY=<your-key> cargo run -p sgbr-core --example live_fetch -- 83139`
Expected: prints one or more lines like `bus 15: [3, 11] min` with plausible countdowns. If you get an HTTP error, re-check the key and the endpoint path against the v6.8 guide.

- [ ] **Step 3: Commit**

```bash
git add crates/sgbr-core/examples/live_fetch.rs
git commit -m "test(core): add live_fetch example for manual verification"
```

---

### Task 8: Slint desktop render proof

**Files:**
- Create: `crates/sgbr-core/src/lta/arrival.rs` is reused (no change)
- Create: `ui/app.slint`
- Create: `build.rs`
- Modify: `Cargo.toml` (root package deps + build-dependencies)
- Modify: `src/main.rs`

Proves the core renders in a real Slint window on Linux. Uses a fixed sample (no network) so it always runs; swap in `fetch_arrivals` once you have a key.

- [ ] **Step 1: Add Slint to the root package**

In root `Cargo.toml`, under `[dependencies]` and a new `[build-dependencies]`:

```toml
[dependencies]
sgbr-core = { workspace = true }
slint = "1.15"
time = { workspace = true }

[build-dependencies]
slint-build = "1.15"
```

- [ ] **Step 2: Create the Slint markup**

`ui/app.slint`:

```slint
struct ServiceRow {
    service_no: string,
    timing: string,
}

export component AppWindow inherits Window {
    in property <[ServiceRow]> rows;
    title: "SG Bus Ready — spike";
    preferred-width: 320px;
    preferred-height: 400px;

    VerticalLayout {
        padding: 16px;
        spacing: 8px;
        Text {
            text: "Stop 83139";
            font-size: 20px;
            font-weight: 700;
        }
        for row in root.rows: HorizontalLayout {
            Text { text: row.service_no; font-weight: 600; width: 64px; }
            Text { text: row.timing; }
        }
    }
}
```

- [ ] **Step 3: Create the build script**

`build.rs`:

```rust
fn main() {
    slint_build::compile("ui/app.slint").expect("slint compile");
}
```

- [ ] **Step 4: Wire the core into the window**

`src/main.rs`:

```rust
//! SG Bus Ready — Slint desktop spike. Renders core `ServiceArrivals` for one
//! stop. Uses a fixed sample now; swap in `fetch_arrivals` once a key exists.

slint::include_modules!();

use sgbr_core::lta::arrival::{service_arrivals, ServiceArrivals};
use sgbr_core::lta::model::BusArrivalResponse;
use slint::{ModelRc, SharedString, VecModel};
use time::OffsetDateTime;

const SAMPLE: &str = r#"{
  "BusStopCode": "83139",
  "Services": [
    { "ServiceNo": "15",
      "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
      "NextBus2": { "EstimatedArrival": "2026-06-21T08:25:00+08:00" },
      "NextBus3": { "EstimatedArrival": "" } }
  ]
}"#;

fn timing_label(arrivals: &ServiceArrivals) -> String {
    if arrivals.minutes.is_empty() {
        return "no service".to_owned();
    }
    arrivals
        .minutes
        .iter()
        .map(|m| if *m <= 0 { "Arr".to_owned() } else { format!("{m} min") })
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() -> Result<(), slint::PlatformError> {
    // Fixed reference time so the sample always shows positive countdowns.
    let now = time::macros::datetime!(2026-06-21 08:10:00 +8);
    let response: BusArrivalResponse =
        serde_json::from_str(SAMPLE).unwrap_or(BusArrivalResponse {
            bus_stop_code: String::new(),
            services: Vec::new(),
        });

    let rows: Vec<ServiceRow> = service_arrivals(&response, now)
        .iter()
        .map(|a| ServiceRow {
            service_no: SharedString::from(a.service_no.as_str()),
            timing: SharedString::from(timing_label(a).as_str()),
        })
        .collect();

    let window = AppWindow::new()?;
    window.set_rows(ModelRc::new(VecModel::from(rows)));
    window.run()
}
```

> Note: `serde_json` is needed by the root package now — add `serde_json = { workspace = true }` to `[dependencies]` if the compiler reports it missing. The `unwrap_or` avoids a denied `unwrap`.

- [ ] **Step 5: Run clippy on the whole workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. (If `ServiceRow`/`AppWindow` generate lint noise from the macro, that's in generated code and won't be flagged.)

- [ ] **Step 6: Run the app**

Run: `cargo run`
Expected: a 320×400 window opens titled "SG Bus Ready — spike", showing `Stop 83139` and a row `15   8 min, 15 min`. Close the window; the process exits 0.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml build.rs ui/app.slint src/main.rs
git commit -m "feat(ui): render core arrivals in a Slint desktop window"
```

---

### Task 9: Wire workspace verification into the Justfile run and finalize

**Files:**
- Modify: none (verification only)

- [ ] **Step 1: Run the full local CI suite**

Run: `just ci`
Expected: `fmt-check`, `clippy`, `test`, `doc-check`, `deny`, `audit`, `machete` all succeed. (If `cargo-deny`/`cargo-audit`/`cargo-machete` are not installed locally, install via `cargo install` or skip those recipes — CI enforces them regardless.)

- [ ] **Step 2: Confirm the tree is clean and push**

Run: `git status --short && git push`
Expected: empty status; push succeeds.

---

## Self-Review

**Spec coverage (against the design doc):**
- Live arrivals data path — Tasks 3–6 (models, minutes, view model, client). ✅
- "Next 3 buses" — `Service` models three slots; `service_arrivals` keeps up to three. ✅
- Core↔UI boundary that the widget can reuse later — `ServiceArrivals` (Task 5). ✅
- Slint chosen as UI — proven on desktop (Task 8). ✅
- Quality bar — every task runs `clippy -D warnings`; Task 9 runs `just ci`. ✅
- **Deferred (correctly out of scope here):** reminders, favourites, search, persistence, widget, Android/iOS bridges — each is a named follow-up plan. The design doc's reminder/widget/notification requirements are NOT yet covered by a task and are explicitly carried to follow-up plans.

**Placeholder scan:** No TBD/TODO; every code step shows complete code; the one external unknown (exact endpoint version) is called out with a concrete default and a verification instruction. ✅

**Type consistency:** `BusArrivalResponse`/`Service`/`NextBus` (Task 3) are used unchanged in Tasks 5, 6, 8. `minutes_until` (Task 4) → `service_arrivals` (Task 5) → `ServiceArrivals` (Tasks 7, 8). `fetch_arrivals(account_key, bus_stop_code)` signature consistent across Tasks 6 and 7. ✅
