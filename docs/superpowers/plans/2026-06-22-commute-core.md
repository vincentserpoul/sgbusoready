# Commute Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the pure-Rust commute model, window logic, display formatting, and list serialization to `sgbr-core`, so the Android layer (a later plan) can schedule windows and render Live Updates from a fully-tested core.

**Architecture:** All logic lives in `sgbr-core` with no platform or UI dependencies. A `Commute` is described with serde-native primitives (`String` line/stop, a `Weekdays` bitmask, two `TimeOfDay` values). Window decisions (`is_active_at`, `next_window_start`) are time-injected — they take a `time::OffsetDateTime` `now`, mirroring the existing `minutes_until`/`service_arrivals` style — so every branch is unit-testable. Display helpers turn arrivals and the next window into the exact strings the notification and the in-app list show. A `CommuteStore` round-trips the list to/from JSON; actual file IO is left to the app layer (the next plan).

**Tech Stack:** Rust (edition 2024), `serde` (derive), `time` 0.3 (`parsing`/`formatting`/`macros`), `thiserror`. No new dependencies.

**Scope note:** This is plan 1 of 2. Plan 2 (Android: Gradle/`cargo-ndk` restructure, AlarmManager scheduling, foreground service, Live Update notification) builds on this core **and** on the Spike #2 groundwork (`docs/superpowers/plans/2026-06-21-android-spike-2-gradle-notification-widget.md`). It is **out of scope here.**

**Spec:** `docs/superpowers/specs/2026-06-22-commutes-live-update-design.md`

---

## File Structure

New module tree under the existing crate:

```
crates/sgbr-core/src/
  lib.rs            (MODIFY: add `pub mod commute;`)
  commute/
    mod.rs          (NEW: declares submodules)
    model.rs        (NEW: TimeOfDay, Weekdays, Commute, CommuteError, validation, serde)
    window.rs       (NEW: is_active_at, next_window_start)
    display.rs      (NEW: format_live_update, format_see_you_soon)
    store.rs        (NEW: CommuteStore + to_json/from_json)
```

Each file has one responsibility: `model` defines data + validation, `window` answers "is this commute live now / when next", `display` produces user-facing strings, `store` serializes the list. Tests live inline in each file in a `#[cfg(test)] mod tests` block, matching `arrival.rs`.

No `Cargo.toml` changes: `serde` (derive), `serde_json`, `time`, and `thiserror` are already dependencies of `sgbr-core`.

---

## Task 1: `TimeOfDay` value type

A minute-resolution time of day, ordered and serde-native.

**Files:**
- Create: `crates/sgbr-core/src/commute/mod.rs`
- Create: `crates/sgbr-core/src/commute/model.rs`
- Modify: `crates/sgbr-core/src/lib.rs`
- Test: inline in `crates/sgbr-core/src/commute/model.rs`

- [ ] **Step 1: Wire the module tree**

Create `crates/sgbr-core/src/commute/mod.rs` with exactly:

```rust
//! Recurring commutes: the data model, window logic, display formatting, and
//! list persistence. Pure logic — no platform or UI code.

pub mod display;
pub mod model;
pub mod store;
pub mod window;
```

Add to `crates/sgbr-core/src/lib.rs` after the existing `pub mod lta;` line:

```rust
pub mod commute;
```

(The crate will not compile until Tasks 1–6 create the four submodules; that is expected mid-plan. Build/clippy gating happens at the end of each task once the relevant file exists. If you need an interim compile, temporarily comment the not-yet-created `pub mod` lines in `mod.rs` and restore them as each file lands. Do not commit with modules commented out.)

- [ ] **Step 2: Write the failing test**

Create `crates/sgbr-core/src/commute/model.rs` with the test module first:

```rust
//! Commute data model with validation and serde-native representation.

#[cfg(test)]
mod tests {
    use super::TimeOfDay;

    #[test]
    fn orders_by_hour_then_minute() {
        assert!(TimeOfDay { hour: 8, minute: 0 } < TimeOfDay { hour: 8, minute: 30 });
        assert!(TimeOfDay { hour: 8, minute: 59 } < TimeOfDay { hour: 9, minute: 0 });
        assert_eq!(
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 8, minute: 0 }
        );
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
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::model`
Expected: FAIL — `cannot find type TimeOfDay` (does not compile yet).

- [ ] **Step 4: Write minimal implementation**

Insert above the `#[cfg(test)]` block in `model.rs`:

```rust
use serde::{Deserialize, Serialize};
use time::Time;

/// A minute-resolution time of day (`hour` 0–23, `minute` 0–59).
///
/// Ordering is lexicographic by `hour` then `minute` (derived), which is the
/// natural chronological order within a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl TimeOfDay {
    /// Convert to a [`time::Time`], or `None` if the fields are out of range.
    #[must_use]
    pub fn to_time(self) -> Option<Time> {
        Time::from_hms(self.hour, self.minute, 0).ok()
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::model`
Expected: PASS (3 tests in `model::tests`).

- [ ] **Step 6: Commit**

```bash
git add crates/sgbr-core/src/lib.rs crates/sgbr-core/src/commute/mod.rs crates/sgbr-core/src/commute/model.rs
git commit -m "feat(core): add TimeOfDay value type for commutes"
```

---

## Task 2: `Weekdays` bitmask

A compact set of weekdays, built from `time::Weekday`, serialized as a single `u8`.

**Files:**
- Modify: `crates/sgbr-core/src/commute/model.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Add these tests inside the existing `mod tests` block in `model.rs`:

```rust
    use super::Weekdays;
    use time::Weekday::{Monday, Saturday, Sunday, Tuesday};

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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::model`
Expected: FAIL — `cannot find type Weekdays`.

- [ ] **Step 3: Write minimal implementation**

Add to `model.rs` (below `TimeOfDay`, above the test module). Note `use time::Weekday;` joins the existing `use time::Time;` — make it `use time::{Time, Weekday};`:

```rust
/// A set of weekdays stored as a 7-bit mask: bit `n` (0 = Monday … 6 = Sunday)
/// corresponds to `Weekday::number_days_from_monday()`. Serializes as its `u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Weekdays(pub u8);

impl Weekdays {
    /// Build a set from a slice of weekdays. Duplicates collapse.
    #[must_use]
    pub fn from_days(days: &[Weekday]) -> Self {
        let mut mask = 0u8;
        for day in days {
            mask |= 1u8 << day.number_days_from_monday();
        }
        Self(mask)
    }

    /// Is `day` in the set?
    #[must_use]
    pub const fn contains(self, day: Weekday) -> bool {
        self.0 & (1u8 << day.number_days_from_monday()) != 0
    }

    /// True when no days are selected.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::model`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/model.rs
git commit -m "feat(core): add Weekdays bitmask for commute days"
```

---

## Task 3: `Commute` struct + validation + serde round-trip

The validated commute record, constructed through a checked `new`.

**Files:**
- Modify: `crates/sgbr-core/src/commute/model.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Add inside `mod tests`:

```rust
    use super::{Commute, CommuteError};
    use time::Weekday::Friday;

    fn weekday_commute() -> Commute {
        Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            None,
        )
        .expect("valid commute")
    }

    #[test]
    fn label_defaults_to_line_at_stop() {
        let c = weekday_commute();
        assert_eq!(c.display_label(), "14 @ 83139");
    }

    #[test]
    fn custom_label_overrides_default() {
        let c = Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            Some("Morning work".to_owned()),
        )
        .expect("valid commute");
        assert_eq!(c.display_label(), "Morning work");
    }

    #[test]
    fn rejects_empty_line_and_stop() {
        let days = Weekdays::from_days(&[Monday]);
        let start = TimeOfDay { hour: 8, minute: 0 };
        let end = TimeOfDay { hour: 9, minute: 0 };
        assert!(matches!(
            Commute::new("", "83139", days, start, end, None),
            Err(CommuteError::EmptyLine)
        ));
        assert!(matches!(
            Commute::new("14", "", days, start, end, None),
            Err(CommuteError::EmptyStop)
        ));
    }

    #[test]
    fn rejects_no_days() {
        assert!(matches!(
            Commute::new(
                "14",
                "83139",
                Weekdays::from_days(&[]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                None,
            ),
            Err(CommuteError::NoDays)
        ));
    }

    #[test]
    fn rejects_end_not_after_start() {
        let days = Weekdays::from_days(&[Monday]);
        assert!(matches!(
            Commute::new(
                "14",
                "83139",
                days,
                TimeOfDay { hour: 9, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                None,
            ),
            Err(CommuteError::EndNotAfterStart)
        ));
    }

    #[test]
    fn rejects_out_of_range_time() {
        let days = Weekdays::from_days(&[Monday]);
        assert!(matches!(
            Commute::new(
                "14",
                "83139",
                days,
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 24, minute: 0 },
                None,
            ),
            Err(CommuteError::InvalidTime)
        ));
    }

    #[test]
    fn commute_serde_round_trip() {
        let c = weekday_commute();
        let json = serde_json::to_string(&c).expect("serialize");
        let back: Commute = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(c, back);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::model`
Expected: FAIL — `cannot find type Commute` / `CommuteError`.

- [ ] **Step 3: Write minimal implementation**

Add to `model.rs`. Add `use thiserror::Error;` to the imports at the top:

```rust
/// Why a [`Commute`] failed validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommuteError {
    #[error("commute line must not be empty")]
    EmptyLine,
    #[error("commute stop must not be empty")]
    EmptyStop,
    #[error("commute must select at least one day")]
    NoDays,
    #[error("commute end time must be after its start time")]
    EndNotAfterStart,
    #[error("commute start or end time is out of range")]
    InvalidTime,
}

/// A recurring commute: one bus line at one stop, on a set of weekdays, within
/// a single-day time window (`start` < `end`, no overnight wrap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commute {
    /// Bus service number, e.g. `"14"`.
    pub line: String,
    /// LTA bus stop code, e.g. `"83139"`.
    pub stop: String,
    /// Days the window is active.
    pub days: Weekdays,
    /// Window open time (inclusive).
    pub start: TimeOfDay,
    /// Window close time (exclusive).
    pub end: TimeOfDay,
    /// Optional user label; falls back to `"<line> @ <stop>"`.
    pub label: Option<String>,
}

impl Commute {
    /// Construct a validated commute. See [`CommuteError`] for failure modes.
    pub fn new(
        line: &str,
        stop: &str,
        days: Weekdays,
        start: TimeOfDay,
        end: TimeOfDay,
        label: Option<String>,
    ) -> Result<Self, CommuteError> {
        if line.is_empty() {
            return Err(CommuteError::EmptyLine);
        }
        if stop.is_empty() {
            return Err(CommuteError::EmptyStop);
        }
        if days.is_empty() {
            return Err(CommuteError::NoDays);
        }
        if start.to_time().is_none() || end.to_time().is_none() {
            return Err(CommuteError::InvalidTime);
        }
        if end <= start {
            return Err(CommuteError::EndNotAfterStart);
        }
        Ok(Self {
            line: line.to_owned(),
            stop: stop.to_owned(),
            days,
            start,
            end,
            label,
        })
    }

    /// The label to show, falling back to `"<line> @ <stop>"`.
    #[must_use]
    pub fn display_label(&self) -> String {
        match &self.label {
            Some(l) => l.clone(),
            None => format!("{} @ {}", self.line, self.stop),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::model`
Expected: PASS (all model tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/model.rs
git commit -m "feat(core): add validated Commute record"
```

---

## Task 4: `is_active_at` window check

**Files:**
- Create: `crates/sgbr-core/src/commute/window.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Create `crates/sgbr-core/src/commute/window.rs` with the test module first. (`2026-06-22` is a Monday, `06-27` a Saturday.)

```rust
//! Window logic: is a commute live right now, and when does it next open?
//! All functions take an injected `now` so every branch is unit-testable.

#[cfg(test)]
mod tests {
    use crate::commute::model::{Commute, TimeOfDay, Weekdays};
    use time::Weekday::{Friday, Monday, Tuesday};
    use time::macros::datetime;

    fn mon_fri_8_to_9() -> Commute {
        Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Monday, Tuesday, Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            None,
        )
        .expect("valid commute")
    }

    #[test]
    fn active_inside_window_on_selected_day() {
        // Monday 08:30 +8
        assert!(mon_fri_8_to_9().is_active_at(datetime!(2026-06-22 08:30:00 +8)));
    }

    #[test]
    fn inactive_before_start() {
        assert!(!mon_fri_8_to_9().is_active_at(datetime!(2026-06-22 07:59:00 +8)));
    }

    #[test]
    fn inactive_at_end_exclusive() {
        // 09:00 is the exclusive end -> not active.
        assert!(!mon_fri_8_to_9().is_active_at(datetime!(2026-06-22 09:00:00 +8)));
    }

    #[test]
    fn inactive_on_unselected_day() {
        // Saturday 2026-06-27 08:30 -> day not selected.
        assert!(!mon_fri_8_to_9().is_active_at(datetime!(2026-06-27 08:30:00 +8)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::window`
Expected: FAIL — `no method named is_active_at`.

- [ ] **Step 3: Write minimal implementation**

Add above the test module in `window.rs`:

```rust
use time::OffsetDateTime;

use crate::commute::model::{Commute, TimeOfDay};

impl Commute {
    /// True when `now` falls on a selected day and within `[start, end)`.
    #[must_use]
    pub fn is_active_at(&self, now: OffsetDateTime) -> bool {
        if !self.days.contains(now.weekday()) {
            return false;
        }
        let current = TimeOfDay {
            hour: now.hour(),
            minute: now.minute(),
        };
        self.start <= current && current < self.end
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::window`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/window.rs
git commit -m "feat(core): add Commute::is_active_at window check"
```

---

## Task 5: `next_window_start`

The next datetime (strictly after `now`) at which the window opens, scanning up to 7 days.

**Files:**
- Modify: `crates/sgbr-core/src/commute/window.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Add inside the existing `mod tests` in `window.rs`:

```rust
    #[test]
    fn next_start_is_today_when_before_window() {
        // Monday 07:00 -> today 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-22 07:00:00 +8)),
            Some(datetime!(2026-06-22 08:00:00 +8))
        );
    }

    #[test]
    fn next_start_skips_to_next_selected_day_after_window() {
        // Monday 09:30 (after today's window) -> Tuesday 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-22 09:30:00 +8)),
            Some(datetime!(2026-06-23 08:00:00 +8))
        );
    }

    #[test]
    fn next_start_skips_unselected_days() {
        // Friday 10:00 -> skip Sat/Sun (unselected) -> Monday 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-26 10:00:00 +8)),
            Some(datetime!(2026-06-29 08:00:00 +8))
        );
    }

    #[test]
    fn next_start_at_exact_start_returns_next_occurrence() {
        // Exactly 08:00 Monday: window is open now, so "next start" is Tuesday.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-22 08:00:00 +8)),
            Some(datetime!(2026-06-23 08:00:00 +8))
        );
    }
```

(`2026-06-26` is a Friday; `06-29` the following Monday.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::window`
Expected: FAIL — `no method named next_window_start`.

- [ ] **Step 3: Write minimal implementation**

Add to the `impl Commute` block in `window.rs` (extend imports to `use time::{OffsetDateTime, PrimitiveDateTime};`):

```rust
    /// The next moment strictly after `now` at which this commute's window
    /// opens, scanning today plus the next 7 days. Returns `None` only if no
    /// days are selected or the start time is out of range (neither happens for
    /// a `Commute` built via [`Commute::new`]).
    #[must_use]
    pub fn next_window_start(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        let start_time = self.start.to_time()?;
        let mut date = now.date();
        // Today plus the next 7 calendar days covers every weekly recurrence.
        for _ in 0..8 {
            if self.days.contains(date.weekday()) {
                let candidate =
                    PrimitiveDateTime::new(date, start_time).assume_offset(now.offset());
                if candidate > now {
                    return Some(candidate);
                }
            }
            date = date.next_day()?;
        }
        None
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::window`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/window.rs
git commit -m "feat(core): add Commute::next_window_start"
```

---

## Task 6: Display formatting

Two pure string builders: the live arrivals line for the notification, and the "see you soon" line for the in-app list.

**Files:**
- Create: `crates/sgbr-core/src/commute/display.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Create `crates/sgbr-core/src/commute/display.rs` with the test module first:

```rust
//! User-facing string builders for the Live Update and the in-app list row.

#[cfg(test)]
mod tests {
    use super::{format_live_update, format_see_you_soon};
    use time::macros::datetime;

    #[test]
    fn live_update_lists_up_to_three_countdowns() {
        assert_eq!(
            format_live_update("14", &[3, 11, 19]),
            "Bus 14 · 3 min · 11 min · 19 min"
        );
    }

    #[test]
    fn live_update_shows_due_for_zero_or_negative() {
        assert_eq!(format_live_update("14", &[0, 5]), "Bus 14 · due · 5 min");
        assert_eq!(format_live_update("14", &[-2, 7]), "Bus 14 · due · 7 min");
    }

    #[test]
    fn live_update_handles_no_buses() {
        assert_eq!(format_live_update("14", &[]), "Bus 14 · no buses");
    }

    #[test]
    fn see_you_soon_formats_short_weekday_and_time() {
        // Tuesday 2026-06-23 08:00.
        assert_eq!(
            format_see_you_soon(datetime!(2026-06-23 08:00:00 +8)),
            "see you soon · next Tue 08:00"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::display`
Expected: FAIL — `cannot find function format_live_update`.

- [ ] **Step 3: Write minimal implementation**

Add above the test module in `display.rs`:

```rust
use time::OffsetDateTime;
use time::macros::format_description;

/// Build the Live Update line for one commute, e.g.
/// `"Bus 14 · 3 min · 11 min · 19 min"`. A `minutes` entry of `0` or below
/// renders as `"due"`. An empty slice renders `"Bus <line> · no buses"`.
#[must_use]
pub fn format_live_update(line: &str, minutes: &[i64]) -> String {
    if minutes.is_empty() {
        return format!("Bus {line} · no buses");
    }
    let parts: Vec<String> = minutes
        .iter()
        .map(|&m| if m <= 0 { "due".to_owned() } else { format!("{m} min") })
        .collect();
    format!("Bus {line} · {}", parts.join(" · "))
}

/// Build the in-app "see you soon" row for a commute that is not active now,
/// e.g. `"see you soon · next Tue 08:00"`. `next_start` is the value returned by
/// [`crate::commute::model::Commute::next_window_start`].
#[must_use]
pub fn format_see_you_soon(next_start: OffsetDateTime) -> String {
    let fmt = format_description!("[weekday repr:short] [hour]:[minute]");
    let when = next_start.format(&fmt).unwrap_or_default();
    format!("see you soon · next {when}")
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::display`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/display.rs
git commit -m "feat(core): add commute display formatting"
```

---

## Task 7: `CommuteStore` list serialization

The persisted list of commutes, round-tripped to/from JSON. File IO is deferred to the app layer.

**Files:**
- Create: `crates/sgbr-core/src/commute/store.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Create `crates/sgbr-core/src/commute/store.rs` with the test module first:

```rust
//! The persisted commute list. Serializes to/from JSON; callers own file IO.

#[cfg(test)]
mod tests {
    use super::CommuteStore;
    use crate::commute::model::{Commute, TimeOfDay, Weekdays};
    use time::Weekday::Monday;

    fn sample_store() -> CommuteStore {
        let c = Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            None,
        )
        .expect("valid commute");
        CommuteStore { commutes: vec![c] }
    }

    #[test]
    fn json_round_trip_preserves_commutes() {
        let store = sample_store();
        let json = store.to_json().expect("serialize");
        let back = CommuteStore::from_json(&json).expect("deserialize");
        assert_eq!(store, back);
    }

    #[test]
    fn empty_store_round_trips() {
        let store = CommuteStore { commutes: vec![] };
        let json = store.to_json().expect("serialize");
        let back = CommuteStore::from_json(&json).expect("deserialize");
        assert_eq!(store, back);
    }

    #[test]
    fn garbage_json_is_parse_error() {
        let err = CommuteStore::from_json("not json").unwrap_err();
        assert!(matches!(err, crate::error::CoreError::Parse(_)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::store`
Expected: FAIL — `cannot find type CommuteStore`.

- [ ] **Step 3: Write minimal implementation**

Add above the test module in `store.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::commute::model::Commute;
use crate::error::CoreError;

/// The full, ordered list of configured commutes. Order is the user's display
/// order; the UI reorders by mutating `commutes`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommuteStore {
    pub commutes: Vec<Commute>,
}

impl CommuteStore {
    /// Serialize the list to pretty JSON.
    pub fn to_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(self).map_err(|e| CoreError::Parse(e.to_string()))
    }

    /// Parse a list from JSON produced by [`Self::to_json`].
    pub fn from_json(json: &str) -> Result<Self, CoreError> {
        serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::store`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/store.rs
git commit -m "feat(core): add CommuteStore JSON serialization"
```

---

## Task 8: Full quality gate

Run the project's full lint and test bar (the `youtun4`-derived strict config) over everything.

**Files:** none (verification only).

- [ ] **Step 1: Run the whole core test suite**

Run: `cargo test -p sgbr-core`
Expected: PASS — all existing `lta`/`error` tests plus the new `commute::{model,window,display,store}` tests.

- [ ] **Step 2: Run clippy with the workspace lints**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean (no warnings). The strict bar denies `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `float_arithmetic`, `cast_possible_truncation`, etc. Note the new code avoids all of these: no indexing (iterators only), no floats, only widening `u8`→shift operations, and no `unwrap`/`expect` outside `#[cfg(test)]` (tests are exempt from `unwrap_used`/`expect_used` via the standard clippy test allowance — if clippy still flags a test, that is unexpected; re-read the offending line rather than adding an `#[allow]`).

- [ ] **Step 3: Check formatting**

Run: `cargo fmt --check`
Expected: clean. If it reports diffs, run `cargo fmt` and amend the relevant commit.

- [ ] **Step 4: Final confirmation**

Confirm `git status` is clean and `git log --oneline -8` shows the seven feature commits plus this plan. No further commit needed unless `cargo fmt` produced changes — in that case:

```bash
git add -A
git commit -m "style(core): rustfmt commute module"
```

---

## Self-Review Notes

- **Spec coverage:** commute model (Task 3) ✓; window logic `is_active_at`/`next_window_start` (Tasks 4–5) ✓; settings persistence (Task 7) ✓; display formatting for Live Update + see-you-soon (Task 6) ✓; "single-day window, no overnight" — enforced by `end <= start` rejection in Task 3 ✓; Rust unit tests for window boundaries, day rollover, settings round-trip, formatting ✓. Android surfaces, AlarmManager, foreground service, file IO, Slint UI — deferred to plan 2 by design.
- **Type consistency:** `TimeOfDay { hour, minute }`, `Weekdays::from_days`/`contains`/`is_empty`, `Commute::new`/`display_label`/`is_active_at`/`next_window_start`, `format_live_update`/`format_see_you_soon`, `CommuteStore { commutes }`/`to_json`/`from_json` are used identically across all tasks. `CommuteError` variants (`EmptyLine`, `EmptyStop`, `NoDays`, `EndNotAfterStart`, `InvalidTime`) match between definition and tests.
- **Placeholders:** none — every step has full code or an exact command with expected output.
- **Calendar facts used:** 2026-06-22 = Monday, 06-23 = Tuesday, 06-26 = Friday, 06-27 = Saturday, 06-29 = Monday (verify with `date -d` if in doubt; tests will fail loudly if wrong).
