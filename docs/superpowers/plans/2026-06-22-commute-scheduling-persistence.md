# Commute Scheduling + Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the pure-Rust support layer the Android foreground service will call: persist the commute list to a file, and compute window boundaries / which commutes are active now — so the on-device plan (next) only has to wire Kotlin to these tested functions.

**Architecture:** Extends the merged `sgbr-core::commute` module. File IO lands in `commute/store.rs` (the design deferred it to "the app layer"; we put it in core so it's tested once). Per-commute boundary helpers extend `commute/window.rs`. List-level selection/scheduling goes in a new `commute/schedule.rs` (single commute logic stays in `window.rs`; "across the whole list" logic is its own unit). A new `CoreError::Io` variant carries filesystem failures.

**Tech Stack:** Rust (edition 2024), `serde`/`serde_json`, `time` 0.3, `thiserror`, `std::fs`. No new dependencies (file-IO tests use a unique path under `std::env::temp_dir()`, so no `tempfile` crate and no supply-chain change).

**Scope note:** This is the autonomous pure-Rust slice. The on-device Android work (Gradle/`cargo-ndk`, JNI, foreground service, AlarmManager, the Live Update notification) is a **separate later plan** that consumes these functions. Out of scope here.

**Spec:** `docs/superpowers/specs/2026-06-22-commutes-live-update-design.md`
**Builds on:** the merged `commute` module (`TimeOfDay`, `Weekdays`, `Commute`, `is_active_at`, `next_window_start`, `CommuteStore::to_json`/`from_json`).

---

## File Structure

```
crates/sgbr-core/src/
  error.rs              (MODIFY: add CoreError::Io variant)
  commute/
    mod.rs              (MODIFY: add `pub mod schedule;`)
    store.rs            (MODIFY: add CommuteStore::load / save file IO)
    window.rs           (MODIFY: add Commute::current_window_end / next_boundary)
    schedule.rs         (NEW: active_commutes_at / next_alarm_at over a slice)
```

Each unit keeps one responsibility: `store` = serialize + persist, `window` = one commute's timing, `schedule` = decisions across the whole list. Tests stay inline per file, matching the existing module.

---

## Task 1: `CoreError::Io` + file persistence

Give `CommuteStore` the ability to load from and save to a path, tolerating a missing file on first run.

**Files:**
- Modify: `crates/sgbr-core/src/error.rs`
- Modify: `crates/sgbr-core/src/commute/store.rs`
- Test: inline in `store.rs`

- [ ] **Step 1: Add the `Io` error variant**

In `crates/sgbr-core/src/error.rs`, add this variant to the `CoreError` enum (after `Parse`):

```rust
    /// A filesystem operation on the persisted store failed.
    #[error("commute store io failed: {0}")]
    Io(String),
```

- [ ] **Step 2: Write the failing test**

Add to the existing `mod tests` block in `crates/sgbr-core/src/commute/store.rs`:

```rust
    use std::path::PathBuf;

    /// A unique, process-isolated temp file path (no `tempfile` dependency).
    fn temp_store_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("sgbr-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.push("commutes.json");
        dir
    }

    #[test]
    fn load_missing_file_returns_empty_store() {
        let path = temp_store_path("missing");
        // Ensure it does not exist.
        let _ = std::fs::remove_file(&path);
        let store = CommuteStore::load(&path).expect("load missing");
        assert_eq!(store, CommuteStore::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_store_path("roundtrip");
        let store = sample_store();
        store.save(&path).expect("save");
        let back = CommuteStore::load(&path).expect("load");
        assert_eq!(store, back);
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let mut path = temp_store_path("nested");
        path.pop(); // drop commutes.json
        path.push("deeper");
        path.push("commutes.json");
        let store = sample_store();
        store.save(&path).expect("save into new dir");
        assert!(path.exists());
    }

    #[test]
    fn load_corrupt_file_is_parse_error() {
        let path = temp_store_path("corrupt");
        std::fs::write(&path, "not json").expect("write corrupt");
        let err = CommuteStore::load(&path).unwrap_err();
        assert!(matches!(err, crate::error::CoreError::Parse(_)));
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::store`
Expected: FAIL — `no function named load` / `no method named save`.

- [ ] **Step 4: Write minimal implementation**

In `crates/sgbr-core/src/commute/store.rs`, extend the imports at the top to:

```rust
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commute::model::Commute;
use crate::error::CoreError;
```

Add these methods to the existing `impl CommuteStore` block (below `from_json`):

```rust
    /// Load the store from `path`. A missing file yields an empty store (the
    /// natural first-run state); a present-but-unparseable file is an error.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        match fs::read_to_string(path) {
            Ok(json) => Self::from_json(&json),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CoreError::Io(e.to_string())),
        }
    }

    /// Persist the store to `path`, creating parent directories as needed.
    /// Writes to a sibling temp file then renames, so a crash mid-write cannot
    /// leave a half-written settings file.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoreError::Io(e.to_string()))?;
        }
        let json = self.to_json()?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| CoreError::Io(e.to_string()))?;
        fs::rename(&tmp, path).map_err(|e| CoreError::Io(e.to_string()))?;
        Ok(())
    }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::store`
Expected: PASS (the original 3 serialization tests plus the 4 new file-IO tests).

- [ ] **Step 6: Commit**

```bash
git add crates/sgbr-core/src/error.rs crates/sgbr-core/src/commute/store.rs
git commit -m "feat(core): persist CommuteStore to a file with atomic save"
```

---

## Task 2: Per-commute window boundaries

Two helpers a scheduler needs: when does the *current* window end, and what is the *next* boundary (end if active now, else next start).

**Files:**
- Modify: `crates/sgbr-core/src/commute/window.rs`
- Test: inline in the same file

- [ ] **Step 1: Write the failing test**

Add inside the existing `mod tests` block in `window.rs`:

```rust
    #[test]
    fn current_window_end_some_when_active() {
        // Monday 08:30 -> active, window ends 09:00 the same day.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.current_window_end(datetime!(2026-06-22 08:30:00 +8)),
            Some(datetime!(2026-06-22 09:00:00 +8))
        );
    }

    #[test]
    fn current_window_end_none_when_inactive() {
        // Monday 07:00 -> not active -> no current window.
        let c = mon_fri_8_to_9();
        assert_eq!(c.current_window_end(datetime!(2026-06-22 07:00:00 +8)), None);
    }

    #[test]
    fn next_boundary_is_end_when_active() {
        // Active Monday 08:30 -> next boundary is this window's end 09:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_boundary(datetime!(2026-06-22 08:30:00 +8)),
            Some(datetime!(2026-06-22 09:00:00 +8))
        );
    }

    #[test]
    fn next_boundary_is_next_start_when_inactive() {
        // Inactive Monday 07:00 -> next boundary is today's start 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_boundary(datetime!(2026-06-22 07:00:00 +8)),
            Some(datetime!(2026-06-22 08:00:00 +8))
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::window`
Expected: FAIL — `no method named current_window_end`.

- [ ] **Step 3: Write minimal implementation**

Add to the `impl Commute` block in `window.rs` (below `next_window_start`):

```rust
    /// If the commute is active at `now`, the [`OffsetDateTime`] of today's
    /// window close (`end`); otherwise `None`.
    #[must_use]
    pub fn current_window_end(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        if !self.is_active_at(now) {
            return None;
        }
        let end_time = self.end.to_time()?;
        Some(PrimitiveDateTime::new(now.date(), end_time).assume_offset(now.offset()))
    }

    /// The next moment this commute changes state: its window close if active
    /// now, otherwise its next window open. `None` only when `next_window_start`
    /// is `None` (no days / invalid time — neither happens via `Commute::new`).
    #[must_use]
    pub fn next_boundary(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        if self.is_active_at(now) {
            self.current_window_end(now)
        } else {
            self.next_window_start(now)
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::window`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sgbr-core/src/commute/window.rs
git commit -m "feat(core): add Commute::current_window_end and next_boundary"
```

---

## Task 3: List-level selection and scheduling

What the service asks each time it wakes: which commutes are live now, and when to set the next alarm.

**Files:**
- Create: `crates/sgbr-core/src/commute/schedule.rs`
- Modify: `crates/sgbr-core/src/commute/mod.rs`
- Test: inline in `schedule.rs`

- [ ] **Step 1: Declare the module**

Add to `crates/sgbr-core/src/commute/mod.rs`, keeping the list alphabetical:

```rust
pub mod schedule;
```

(Final `mod.rs` body: `display`, `model`, `schedule`, `store`, `window`.)

- [ ] **Step 2: Write the failing test**

Create `crates/sgbr-core/src/commute/schedule.rs` with the test module first:

```rust
//! Decisions across the whole commute list: which are active now, and when the
//! scheduler should next wake. Single-commute timing lives in `window.rs`.

#[cfg(test)]
mod tests {
    use super::{active_commutes_at, next_alarm_at};
    use crate::commute::model::{Commute, TimeOfDay, Weekdays};
    use time::Weekday::{Monday, Tuesday};
    use time::macros::datetime;

    fn morning() -> Commute {
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

    fn evening() -> Commute {
        Commute::new(
            "67",
            "84009",
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 18, minute: 0 },
            TimeOfDay { hour: 19, minute: 0 },
            None,
        )
        .expect("valid commute")
    }

    #[test]
    fn active_returns_only_live_commutes() {
        // Monday 08:30 -> morning active, evening not.
        let list = vec![morning(), evening()];
        let active = active_commutes_at(&list, datetime!(2026-06-22 08:30:00 +8));
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].line, "14");
    }

    #[test]
    fn active_empty_when_none_live() {
        let list = vec![morning(), evening()];
        let active = active_commutes_at(&list, datetime!(2026-06-22 12:00:00 +8));
        assert!(active.is_empty());
    }

    #[test]
    fn next_alarm_is_earliest_boundary() {
        // Monday 08:30: morning is active (boundary 09:00), evening inactive
        // (next start 18:00). Earliest boundary is 09:00.
        let list = vec![morning(), evening()];
        assert_eq!(
            next_alarm_at(&list, datetime!(2026-06-22 08:30:00 +8)),
            Some(datetime!(2026-06-22 09:00:00 +8))
        );
    }

    #[test]
    fn next_alarm_when_none_active_is_earliest_start() {
        // Monday 12:00: both inactive. morning next start = Tuesday 08:00,
        // evening next start = Monday 18:00. Earliest is Monday 18:00.
        let list = vec![morning(), evening()];
        assert_eq!(
            next_alarm_at(&list, datetime!(2026-06-22 12:00:00 +8)),
            Some(datetime!(2026-06-22 18:00:00 +8))
        );
    }

    #[test]
    fn next_alarm_empty_list_is_none() {
        let list: Vec<Commute> = vec![];
        assert_eq!(next_alarm_at(&list, datetime!(2026-06-22 08:30:00 +8)), None);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p sgbr-core --lib commute::schedule`
Expected: FAIL — `cannot find function active_commutes_at`.

- [ ] **Step 4: Write minimal implementation**

Add above the test module in `schedule.rs`:

```rust
use time::OffsetDateTime;

use crate::commute::model::Commute;

/// The commutes whose window is open at `now`, in list order.
#[must_use]
pub fn active_commutes_at(commutes: &[Commute], now: OffsetDateTime) -> Vec<&Commute> {
    commutes.iter().filter(|c| c.is_active_at(now)).collect()
}

/// The earliest moment any commute next changes state — the time the scheduler
/// should set its next alarm for. `None` when the list is empty (or no commute
/// has a valid boundary).
#[must_use]
pub fn next_alarm_at(commutes: &[Commute], now: OffsetDateTime) -> Option<OffsetDateTime> {
    commutes.iter().filter_map(|c| c.next_boundary(now)).min()
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p sgbr-core --lib commute::schedule`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/sgbr-core/src/commute/mod.rs crates/sgbr-core/src/commute/schedule.rs
git commit -m "feat(core): add active_commutes_at and next_alarm_at scheduling"
```

---

## Task 4: Full quality gate

**Files:** none (verification only).

- [ ] **Step 1: Run the whole core test suite**

Run: `cargo test -p sgbr-core`
Expected: PASS — all prior `commute`/`lta`/`error` tests plus the new persistence (4), boundary (4), and schedule (5) tests.

- [ ] **Step 2: Clippy with the workspace lints**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. The new code avoids the denied lints: no indexing (iterators only; tests use `[0]` on a `Vec`, which is allowed in `#[cfg(test)]`), no floats, no lossy casts, and no `unwrap`/`expect` outside `#[cfg(test)]` (the `unwrap_err`/`expect` calls are all in tests).

- [ ] **Step 3: Formatting**

Run: `cargo fmt --check`
Expected: clean. If it reports diffs, run `cargo fmt` and amend the relevant commit, or add a `style(core): rustfmt` commit.

- [ ] **Step 4: Final confirmation**

Confirm `git status` is clean and `git log --oneline -5` shows the three feature commits.

---

## Self-Review Notes

- **Spec coverage:** "settings persistence (load/save the commute list)" — Task 1 ✓ (the design said file IO could live in the app layer; we put it in core, tested). The scheduling/selection helpers (Tasks 2–3) are new support for the spec's "AlarmManager wakes only at window boundaries" and "one Live Update per currently-active commute" — `next_alarm_at` gives the boundary to arm; `active_commutes_at` gives the set to render. No UI/Android/JNI here (deferred to the on-device plan), matching the scope note.
- **Type consistency:** `CoreError::Io(String)` matches the existing `Http`/`Parse(String)` shape and is produced by `save`/`load`. `current_window_end`/`next_boundary` (window.rs) are consumed by `next_alarm_at` (schedule.rs). `active_commutes_at`/`next_alarm_at` take `&[Commute]` + `OffsetDateTime` and return `Vec<&Commute>` / `Option<OffsetDateTime>`, used identically in their tests.
- **Placeholders:** none — every step has full code or an exact command with expected output.
- **No-new-dependency check:** file-IO tests use `std::env::temp_dir()` + `std::process::id()` for isolation, so `Cargo.toml`, `Cargo.lock`, and the cargo-vet supply chain are untouched.
- **Calendar facts:** 2026-06-22 = Monday, 06-23 = Tuesday (verified previously; tests fail loudly if wrong).
