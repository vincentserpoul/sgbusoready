# Multi-stop Commutes — Core (Rust) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalise the pure-Rust commute model from one-line/one-stop to a labelled, scheduled container of multiple stops, each tracking multiple buses, plus the view-model and formatting that feed the UI and the Android notification.

**Architecture:** All changes live in the `sgbr-core` library crate (no platform/UI code). The `Commute` struct gains a `stops: Vec<CommuteStop>` field and loses its flat `line`/`stop` fields; validation moves to per-stop rules. A new `StopArrivals` view model groups same-minute buses per stop and feeds both the in-app timeline and a new per-stop notification line formatter. Scheduling gains a helper that returns the distinct stops (union of buses) to refresh while active. No data migration — the JSON shape changes outright.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `time` crate, `thiserror`. Tests are inline `#[cfg(test)]` modules. Strict clippy + rustfmt (see `clippy.toml`).

This is **Plan 1 of 3**. Plan 2 = Slint UI (timeline component + accordion editor + list cards). Plan 3 = Android (per-stop notification rendering, app-id rename `com.sgbusoready` → `com.sgbuscommute`, label rename, launcher icon).

---

## File Structure

- `crates/sgbr-core/src/commute/model.rs` — **rewrite**: `CommuteStop`, nested `Commute`, new `CommuteError` variants, new `Commute::new` signature, `display_label`.
- `crates/sgbr-core/src/commute/window.rs` — **modify test helper only** (logic untouched; it reads `days`/`start`/`end`).
- `crates/sgbr-core/src/commute/schedule.rs` — **modify test helpers**; **add** `StopPlan` + `active_stop_plans`.
- `crates/sgbr-core/src/commute/store.rs` — **modify test helper**; **add** a multi-stop round-trip test.
- `crates/sgbr-core/src/lta/arrival.rs` — **add** `StopArrivals`, `ArrivalItem`, `stop_arrivals(...)`.
- `crates/sgbr-core/src/commute/display.rs` — **add** `format_stop_line` + `format_active_notification`; keep existing `format_see_you_soon`.

Note: the `Commute::new` signature change cascades to every test helper that builds a commute. Task 1 updates the model **and** all in-crate call sites together so the crate compiles; later tasks are additive.

---

## Task 1: Nested commute model

**Files:**
- Modify: `crates/sgbr-core/src/commute/model.rs`
- Modify (test helpers): `crates/sgbr-core/src/commute/window.rs:77-87`, `crates/sgbr-core/src/commute/schedule.rs:29-57`, `crates/sgbr-core/src/commute/store.rs:62-73`

- [ ] **Step 1: Replace the model's `CommuteError`, struct, constructor, display, and tests**

Replace everything from `pub enum CommuteError` to the end of the `impl Commute` block (lines 55–131 in the current file) with:

```rust
/// Why a [`Commute`] failed validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommuteError {
    #[error("commute must select at least one day")]
    NoDays,
    #[error("commute start or end time is out of range")]
    InvalidTime,
    #[error("commute end time must be after its start time")]
    EndNotAfterStart,
    #[error("commute must have at least one stop")]
    NoStops,
    #[error("commute stop code must not be empty")]
    StopEmptyCode,
    #[error("commute stop must track at least one bus")]
    StopNoBuses,
}

/// One stop within a commute and the buses tracked there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommuteStop {
    /// LTA bus stop code, e.g. `"83139"`.
    pub code: String,
    /// Cached display name, e.g. `"Opp Blk 123"`.
    pub name: String,
    /// Service numbers tracked at this stop, e.g. `["14", "14e"]` (>= 1).
    pub buses: Vec<String>,
}

/// A recurring commute: a set of stops (each with its own tracked buses), on a
/// set of weekdays, within a single-day time window (`start` < `end`, no
/// overnight wrap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commute {
    /// Optional user label; falls back to a stop-derived label.
    pub label: Option<String>,
    /// Days the window is active.
    pub days: Weekdays,
    /// Window open time (inclusive).
    pub start: TimeOfDay,
    /// Window close time (exclusive).
    pub end: TimeOfDay,
    /// Stops tracked by this commute (>= 1).
    pub stops: Vec<CommuteStop>,
}

impl Commute {
    /// Construct a validated commute. See [`CommuteError`] for failure modes.
    pub fn new(
        label: Option<String>,
        days: Weekdays,
        start: TimeOfDay,
        end: TimeOfDay,
        stops: Vec<CommuteStop>,
    ) -> Result<Self, CommuteError> {
        if days.is_empty() {
            return Err(CommuteError::NoDays);
        }
        if start.to_time().is_none() || end.to_time().is_none() {
            return Err(CommuteError::InvalidTime);
        }
        if end <= start {
            return Err(CommuteError::EndNotAfterStart);
        }
        if stops.is_empty() {
            return Err(CommuteError::NoStops);
        }
        for stop in &stops {
            if stop.code.is_empty() {
                return Err(CommuteError::StopEmptyCode);
            }
            if stop.buses.is_empty() {
                return Err(CommuteError::StopNoBuses);
            }
        }
        Ok(Self {
            label,
            days,
            start,
            end,
            stops,
        })
    }

    /// The label to show. Falls back to the first stop's name, suffixed with
    /// `" +N"` when there is more than one stop.
    #[must_use]
    pub fn display_label(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        match self.stops.split_first() {
            Some((first, rest)) if rest.is_empty() => first.name.clone(),
            Some((first, rest)) => format!("{} +{}", first.name, rest.len()),
            None => String::new(),
        }
    }
}
```

- [ ] **Step 2: Replace the model's test module**

Replace the entire `#[cfg(test)] mod tests { ... }` block (lines 133–317) with:

```rust
#[cfg(test)]
mod tests {
    use super::TimeOfDay;
    use super::Weekdays;
    use super::{Commute, CommuteError, CommuteStop};
    use time::Weekday::{Friday, Monday, Saturday, Sunday, Tuesday};

    fn stop(code: &str, name: &str, buses: &[&str]) -> CommuteStop {
        CommuteStop {
            code: code.to_owned(),
            name: name.to_owned(),
            buses: buses.iter().map(|b| (*b).to_owned()).collect(),
        }
    }

    fn weekday_commute() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![stop("83139", "Opp Blk 123", &["14"])],
        )
        .expect("valid commute")
    }

    #[test]
    fn orders_by_hour_then_minute() {
        assert!(TimeOfDay { hour: 8, minute: 0 } < TimeOfDay { hour: 8, minute: 30 });
        assert!(TimeOfDay { hour: 8, minute: 59 } < TimeOfDay { hour: 9, minute: 0 });
        assert_eq!(TimeOfDay { hour: 8, minute: 0 }, TimeOfDay { hour: 8, minute: 0 });
    }

    #[test]
    fn converts_to_time_when_valid() {
        let t = TimeOfDay { hour: 8, minute: 5 }.to_time().expect("valid time");
        assert_eq!(t, time::macros::time!(08:05:00));
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(TimeOfDay { hour: 24, minute: 0 }.to_time().is_none());
        assert!(TimeOfDay { hour: 0, minute: 60 }.to_time().is_none());
    }

    #[test]
    fn contains_only_listed_days() {
        let wd = Weekdays::from_days(&[Monday, Tuesday]);
        assert!(wd.contains(Monday));
        assert!(wd.contains(Tuesday));
        assert!(!wd.contains(Saturday));
        assert!(!wd.contains(Sunday));
    }

    #[test]
    fn empty_contains_nothing_and_reports_empty() {
        let wd = Weekdays::from_days(&[]);
        assert!(wd.is_empty());
        assert!(!wd.contains(Monday));
    }

    #[test]
    fn weekdays_round_trip_as_u8() {
        let wd = Weekdays::from_days(&[Monday, Saturday]);
        let json = serde_json::to_string(&wd).expect("serialize");
        let back: Weekdays = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(wd, back);
    }

    #[test]
    fn label_defaults_to_single_stop_name() {
        let c = weekday_commute();
        assert_eq!(c.display_label(), "Opp Blk 123");
    }

    #[test]
    fn label_defaults_to_first_stop_plus_count() {
        let c = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![
                stop("83139", "Opp Blk 123", &["14"]),
                stop("17009", "Bef Clementi Stn", &["96"]),
            ],
        )
        .expect("valid commute");
        assert_eq!(c.display_label(), "Opp Blk 123 +1");
    }

    #[test]
    fn custom_label_overrides_default() {
        let c = Commute::new(
            Some("Morning work".to_owned()),
            Weekdays::from_days(&[Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![stop("83139", "Opp Blk 123", &["14"])],
        )
        .expect("valid commute");
        assert_eq!(c.display_label(), "Morning work");
    }

    #[test]
    fn rejects_no_days() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::NoDays)
        ));
    }

    #[test]
    fn rejects_end_not_after_start() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 9, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::EndNotAfterStart)
        ));
    }

    #[test]
    fn rejects_out_of_range_time() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 24, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::InvalidTime)
        ));
    }

    #[test]
    fn rejects_no_stops() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![],
            ),
            Err(CommuteError::NoStops)
        ));
    }

    #[test]
    fn rejects_empty_stop_code() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::StopEmptyCode)
        ));
    }

    #[test]
    fn rejects_stop_with_no_buses() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &[])],
            ),
            Err(CommuteError::StopNoBuses)
        ));
    }

    #[test]
    fn commute_serde_round_trip() {
        let c = weekday_commute();
        let json = serde_json::to_string(&c).expect("serialize");
        let back: Commute = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(c, back);
    }
}
```

- [ ] **Step 3: Fix the `window.rs` test helper**

In `crates/sgbr-core/src/commute/window.rs`, replace the `mon_fri_8_to_9` helper (lines 77–87) with:

```rust
    fn mon_fri_8_to_9() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday, Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned()],
            }],
        )
        .expect("valid commute")
    }
```

Then update the test `use` at line 73 to import `CommuteStop`:

```rust
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
```

- [ ] **Step 4: Fix the `schedule.rs` test helpers and `.line` assertion**

In `crates/sgbr-core/src/commute/schedule.rs`, update the test `use` (line 25):

```rust
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
```

Replace `morning()` and `evening()` (lines 29–57) with:

```rust
    fn morning() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned()],
            }],
        )
        .expect("valid commute")
    }

    fn evening() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 18, minute: 0 },
            TimeOfDay { hour: 19, minute: 0 },
            vec![CommuteStop {
                code: "84009".to_owned(),
                name: "Bef Clementi Stn".to_owned(),
                buses: vec!["67".to_owned()],
            }],
        )
        .expect("valid commute")
    }
```

In `active_returns_only_live_commutes`, change the assertion `assert_eq!(active[0].line, "14");` to:

```rust
        assert_eq!(active[0].stops[0].code, "83139");
```

- [ ] **Step 5: Fix the `store.rs` test helper**

In `crates/sgbr-core/src/commute/store.rs`, update the test `use` (line 58):

```rust
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
```

Replace `sample_store` (lines 62–73) with:

```rust
    fn sample_store() -> CommuteStore {
        let c = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned()],
            }],
        )
        .expect("valid commute");
        CommuteStore { commutes: vec![c] }
    }
```

- [ ] **Step 6: Run the whole core test suite**

Run: `cargo test -p sgbr-core`
Expected: PASS, all tests green (the new model tests plus the updated window/schedule/store tests compile and pass).

- [ ] **Step 7: Lint**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt -p sgbr-core -- --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/sgbr-core/src/commute/model.rs \
        crates/sgbr-core/src/commute/window.rs \
        crates/sgbr-core/src/commute/schedule.rs \
        crates/sgbr-core/src/commute/store.rs
git commit -m "feat(core): nest commutes as stops-with-buses"
```

---

## Task 2: Active-stop refresh plan (schedule.rs)

**Files:**
- Modify: `crates/sgbr-core/src/commute/schedule.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `schedule.rs`:

```rust
    fn two_stop_morning() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![
                CommuteStop {
                    code: "83139".to_owned(),
                    name: "Opp Blk 123".to_owned(),
                    buses: vec!["14".to_owned(), "14e".to_owned()],
                },
                CommuteStop {
                    code: "17009".to_owned(),
                    name: "Bef Clementi Stn".to_owned(),
                    buses: vec!["96".to_owned()],
                },
            ],
        )
        .expect("valid commute")
    }

    #[test]
    fn active_stop_plans_lists_distinct_stops_when_active() {
        let list = vec![two_stop_morning()];
        let plans = super::active_stop_plans(&list, datetime!(2026-06-22 08:30:00 +8));
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].code, "83139");
        assert_eq!(plans[0].buses, vec!["14".to_owned(), "14e".to_owned()]);
        assert_eq!(plans[1].code, "17009");
    }

    #[test]
    fn active_stop_plans_empty_when_inactive() {
        let list = vec![two_stop_morning()];
        let plans = super::active_stop_plans(&list, datetime!(2026-06-22 12:00:00 +8));
        assert!(plans.is_empty());
    }

    #[test]
    fn active_stop_plans_unions_buses_across_commutes_for_same_stop() {
        // Two commutes both active Monday 08:30, both tracking stop 83139 with
        // overlapping + distinct buses -> union, deduped, first-seen order.
        let a = two_stop_morning(); // 83139: 14, 14e ; 17009: 96
        let b = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned(), "154".to_owned()],
            }],
        )
        .expect("valid commute");
        let list = vec![a, b];
        let plans = super::active_stop_plans(&list, datetime!(2026-06-22 08:30:00 +8));
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].code, "83139");
        assert_eq!(
            plans[0].buses,
            vec!["14".to_owned(), "14e".to_owned(), "154".to_owned()]
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p sgbr-core active_stop_plans`
Expected: FAIL to compile — `active_stop_plans` / `StopPlan` not found.

- [ ] **Step 3: Implement `StopPlan` + `active_stop_plans`**

Add to `schedule.rs` after the `next_alarm_at` function (before the test module):

```rust
/// A stop to refresh while active, with the union of buses tracked there by
/// every currently-active commute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPlan {
    /// LTA bus stop code.
    pub code: String,
    /// Cached display name (from the first commute that referenced this stop).
    pub name: String,
    /// Union of tracked buses across active commutes, deduped, first-seen order.
    pub buses: Vec<String>,
}

/// The distinct stops across all commutes active at `now`, each carrying the
/// union of buses tracked there. One LTA arrival call per returned stop covers
/// every active commute. Empty when nothing is active.
#[must_use]
pub fn active_stop_plans(commutes: &[Commute], now: OffsetDateTime) -> Vec<StopPlan> {
    let mut plans: Vec<StopPlan> = Vec::new();
    for commute in commutes.iter().filter(|c| c.is_active_at(now)) {
        for stop in &commute.stops {
            if let Some(existing) = plans.iter_mut().find(|p| p.code == stop.code) {
                for bus in &stop.buses {
                    if !existing.buses.contains(bus) {
                        existing.buses.push(bus.clone());
                    }
                }
            } else {
                plans.push(StopPlan {
                    code: stop.code.clone(),
                    name: stop.name.clone(),
                    buses: stop.buses.clone(),
                });
            }
        }
    }
    plans
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p sgbr-core active_stop_plans`
Expected: PASS (3 tests).

- [ ] **Step 5: Lint + commit**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings` (expect clean), then:

```bash
git add crates/sgbr-core/src/commute/schedule.rs
git commit -m "feat(core): active_stop_plans union of stops/buses to refresh"
```

---

## Task 3: Per-stop arrival view model (arrival.rs)

**Files:**
- Modify: `crates/sgbr-core/src/lta/arrival.rs`

- [ ] **Step 1: Write the failing tests**

Add a new test module at the end of `crates/sgbr-core/src/lta/arrival.rs`:

```rust
#[cfg(test)]
mod stop_arrivals_tests {
    use super::{ArrivalItem, StopArrivals, stop_arrivals};
    use crate::lta::model::BusArrivalResponse;
    use time::macros::datetime;

    // Stop 83139 with services 14 (8m, 25m), 14e (8m), and 999 (untracked).
    const SAMPLE: &str = r#"{
      "BusStopCode": "83139",
      "Services": [
        {
          "ServiceNo": "14",
          "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
          "NextBus2": { "EstimatedArrival": "2026-06-21T08:35:00+08:00" },
          "NextBus3": { "EstimatedArrival": "" }
        },
        {
          "ServiceNo": "14e",
          "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
          "NextBus2": { "EstimatedArrival": "" },
          "NextBus3": { "EstimatedArrival": "" }
        },
        {
          "ServiceNo": "999",
          "NextBus":  { "EstimatedArrival": "2026-06-21T08:12:00+08:00" },
          "NextBus2": { "EstimatedArrival": "" },
          "NextBus3": { "EstimatedArrival": "" }
        }
      ]
    }"#;

    #[test]
    fn filters_to_tracked_buses_and_groups_same_minute() {
        let resp: BusArrivalResponse = serde_json::from_str(SAMPLE).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let tracked = vec!["14".to_owned(), "14e".to_owned()];
        let out = stop_arrivals("83139", "Opp Blk 123", &tracked, &resp, now);
        assert_eq!(out.code, "83139");
        assert_eq!(out.name, "Opp Blk 123");
        // 999 excluded (untracked). 14 & 14e share minute 8; 14 also at 25.
        assert_eq!(
            out.items,
            vec![
                ArrivalItem {
                    minutes: 8,
                    buses: vec!["14".to_owned(), "14e".to_owned()],
                },
                ArrivalItem {
                    minutes: 25,
                    buses: vec!["14".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn drops_past_arrivals() {
        const PAST: &str = r#"{
          "BusStopCode": "83139",
          "Services": [
            {
              "ServiceNo": "14",
              "NextBus":  { "EstimatedArrival": "2026-06-21T08:05:00+08:00" },
              "NextBus2": { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
              "NextBus3": { "EstimatedArrival": "" }
            }
          ]
        }"#;
        let resp: BusArrivalResponse = serde_json::from_str(PAST).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let tracked = vec!["14".to_owned()];
        let out = stop_arrivals("83139", "Opp Blk 123", &tracked, &resp, now);
        // 08:05 is in the past (-5m) -> dropped; only 08:18 (8m) kept.
        assert_eq!(
            out.items,
            vec![ArrivalItem { minutes: 8, buses: vec!["14".to_owned()] }]
        );
    }

    #[test]
    fn empty_when_no_tracked_buses_present() {
        let resp: BusArrivalResponse = serde_json::from_str(SAMPLE).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let tracked = vec!["77".to_owned()];
        let out = stop_arrivals("83139", "Opp Blk 123", &tracked, &resp, now);
        assert!(out.items.is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p sgbr-core stop_arrivals`
Expected: FAIL to compile — `StopArrivals` / `ArrivalItem` / `stop_arrivals` not found.

- [ ] **Step 3: Implement the view model and builder**

Add to `crates/sgbr-core/src/lta/arrival.rs` after the `service_arrivals` function (before the first test module). Add `use std::collections::BTreeMap;` to the top-of-file imports.

```rust
/// One bus-stop's upcoming arrivals, soonest-first, with buses arriving at the
/// same whole minute grouped together. Drives both the in-app per-stop timeline
/// and the Live Update notification line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopArrivals {
    /// LTA bus stop code.
    pub code: String,
    /// Cached display name.
    pub name: String,
    /// Arrival groups, ascending by minute.
    pub items: Vec<ArrivalItem>,
}

/// One arrival group: every tracked bus arriving at the same whole minute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrivalItem {
    /// Whole-minute countdown (0 = due). Never negative.
    pub minutes: i64,
    /// Service numbers arriving at this minute, in response order, deduped.
    pub buses: Vec<String>,
}

/// Build a [`StopArrivals`] for `code`/`name` from `response`, keeping only the
/// services in `tracked`, dropping past (negative-minute) arrivals, and grouping
/// same-minute buses. Items are sorted ascending by minute.
#[must_use]
pub fn stop_arrivals(
    code: &str,
    name: &str,
    tracked: &[String],
    response: &BusArrivalResponse,
    now: OffsetDateTime,
) -> StopArrivals {
    let mut by_minute: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for svc in &response.services {
        if !tracked.iter().any(|t| t == &svc.service_no) {
            continue;
        }
        let slots = [&svc.next_bus, &svc.next_bus2, &svc.next_bus3];
        for bus in slots {
            let Ok(minutes) = minutes_until(&bus.estimated_arrival, now) else {
                continue;
            };
            if minutes < 0 {
                continue;
            }
            let entry = by_minute.entry(minutes).or_default();
            if !entry.contains(&svc.service_no) {
                entry.push(svc.service_no.clone());
            }
        }
    }
    let items = by_minute
        .into_iter()
        .map(|(minutes, buses)| ArrivalItem { minutes, buses })
        .collect();
    StopArrivals {
        code: code.to_owned(),
        name: name.to_owned(),
        items,
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p sgbr-core stop_arrivals`
Expected: PASS (3 tests).

- [ ] **Step 5: Lint + commit**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings` (expect clean), then:

```bash
git add crates/sgbr-core/src/lta/arrival.rs
git commit -m "feat(core): StopArrivals view model (filter, group, sort)"
```

---

## Task 4: Per-stop notification line (display.rs)

**Files:**
- Modify: `crates/sgbr-core/src/commute/display.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `display.rs` (and extend its `use super::...` line to include the new names — see Step 3):

```rust
    #[test]
    fn stop_line_is_time_first_with_bracketed_buses() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![
                ArrivalItem { minutes: 2, buses: vec!["14".to_owned()] },
                ArrivalItem {
                    minutes: 4,
                    buses: vec!["14e".to_owned(), "16".to_owned()],
                },
                ArrivalItem { minutes: 11, buses: vec!["154".to_owned()] },
            ],
        };
        assert_eq!(
            format_stop_line(&stop),
            "Opp Blk 123: 2m (14), 4m (14e·16), 11m (154)"
        );
    }

    #[test]
    fn stop_line_shows_due_for_zero_minutes() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![ArrivalItem { minutes: 0, buses: vec!["14".to_owned()] }],
        };
        assert_eq!(format_stop_line(&stop), "Opp Blk 123: due (14)");
    }

    #[test]
    fn stop_line_handles_no_buses() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![],
        };
        assert_eq!(format_stop_line(&stop), "Opp Blk 123: no buses");
    }

    #[test]
    fn active_notification_joins_stop_lines_with_newlines() {
        let a = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![ArrivalItem { minutes: 2, buses: vec!["14".to_owned()] }],
        };
        let b = StopArrivals {
            code: "17009".to_owned(),
            name: "Bef Clementi Stn".to_owned(),
            items: vec![ArrivalItem { minutes: 8, buses: vec!["96".to_owned()] }],
        };
        assert_eq!(
            format_active_notification(&[a, b]),
            "Opp Blk 123: 2m (14)\nBef Clementi Stn: 8m (96)"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p sgbr-core --lib display`
Expected: FAIL to compile — `format_stop_line` / `format_active_notification` / `StopArrivals` not found.

- [ ] **Step 3: Implement the formatters**

In `crates/sgbr-core/src/commute/display.rs`:

Add an import near the top (after the existing `use` lines):

```rust
use crate::lta::arrival::StopArrivals;
```

Add these functions after `format_see_you_soon`:

```rust
/// Build the Live Update line for one stop, time-first with buses bracketed:
/// `"Opp Blk 123: 2m (14), 4m (14e·16), 11m (154)"`. A `minutes` value of `0`
/// or below renders as `"due"`. An empty stop renders `"<name>: no buses"`.
#[must_use]
pub fn format_stop_line(stop: &StopArrivals) -> String {
    if stop.items.is_empty() {
        return format!("{}: no buses", stop.name);
    }
    let parts: Vec<String> = stop
        .items
        .iter()
        .map(|item| {
            let when = if item.minutes <= 0 {
                "due".to_owned()
            } else {
                format!("{}m", item.minutes)
            };
            format!("{when} ({})", item.buses.join("·"))
        })
        .collect();
    format!("{}: {}", stop.name, parts.join(", "))
}

/// Build the full Live Update body for the active commute(s): one
/// [`format_stop_line`] per stop, newline-separated.
#[must_use]
pub fn format_active_notification(stops: &[StopArrivals]) -> String {
    stops
        .iter()
        .map(format_stop_line)
        .collect::<Vec<_>>()
        .join("\n")
}
```

Extend the test module's `use super::...` (line 41) to:

```rust
    use super::{format_active_notification, format_see_you_soon, format_stop_line};
    use crate::lta::arrival::{ArrivalItem, StopArrivals};
```

(Drop `format_live_update` from the `use` if it is no longer referenced by a test; keep it if its existing tests remain — see note below.)

> Note: `format_live_update` is now superseded by `format_stop_line`. Leave the function and its existing tests in place for this plan; the Android plan (Plan 3) removes the last caller and this function together. Do **not** delete it here, or the (cfg-android) `src/android_bridge.rs` reference would dangle.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p sgbr-core --lib display`
Expected: PASS (existing `format_live_update` / `format_see_you_soon` tests plus the four new ones).

- [ ] **Step 5: Lint + commit**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings` (expect clean), then:

```bash
git add crates/sgbr-core/src/commute/display.rs
git commit -m "feat(core): per-stop notification line formatter"
```

---

## Task 5: Multi-stop store round-trip test

**Files:**
- Modify: `crates/sgbr-core/src/commute/store.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `store.rs`:

```rust
    #[test]
    fn multi_stop_commute_round_trips() {
        let c = Commute::new(
            Some("Morning to work".to_owned()),
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![
                CommuteStop {
                    code: "83139".to_owned(),
                    name: "Opp Blk 123".to_owned(),
                    buses: vec!["14".to_owned(), "14e".to_owned()],
                },
                CommuteStop {
                    code: "17009".to_owned(),
                    name: "Bef Clementi Stn".to_owned(),
                    buses: vec!["96".to_owned()],
                },
            ],
        )
        .expect("valid commute");
        let store = CommuteStore { commutes: vec![c] };
        let json = store.to_json().expect("serialize");
        let back = CommuteStore::from_json(&json).expect("deserialize");
        assert_eq!(store, back);
    }
```

- [ ] **Step 2: Run to verify it passes**

Run: `cargo test -p sgbr-core multi_stop_commute_round_trips`
Expected: PASS (serde derives already cover the nested shape; this test documents and locks the format).

- [ ] **Step 3: Commit**

```bash
git add crates/sgbr-core/src/commute/store.rs
git commit -m "test(core): lock multi-stop commute JSON round-trip"
```

---

## Task 6: Full-suite verification

- [ ] **Step 1: Run the entire workspace test suite (host target)**

Run: `cargo test`
Expected: PASS. (The `sgbr-core` crate is fully green; the app crate's host build does not compile `src/android_bridge.rs`, which is `cfg(android)`.)

- [ ] **Step 2: Workspace lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 3: Confirm no accidental host-target breakage from the model change**

Run: `cargo build`
Expected: success.

---

## Self-Review notes (already applied)

- **Spec coverage:** nested model + validation (Task 1), union-of-active-stops refresh (Task 2), `StopArrivals` grouping/sorting (Task 3), time-first bracketed notification line (Task 4), nested persistence (Tasks 1 & 5). UI rendering, the Android notification wiring, app-id/label rename and the launcher icon are deliberately deferred to Plans 2 and 3.
- **Type consistency:** `Commute::new(label, days, start, end, stops)` is used identically in every task and helper; `CommuteStop { code, name, buses }`, `StopPlan { code, name, buses }`, `StopArrivals { code, name, items }`, and `ArrivalItem { minutes, buses }` field names match across tasks.
- **No migration:** confirmed — no old-format reader; `store.rs` only round-trips the new shape.
```
