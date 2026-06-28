# Design — Rolling timeline, clock labels & home redesign

Date: 2026-06-28
Status: Approved (pending spec review)

## Summary

Four UX improvements to SG Bus Ready plus a whole-codebase Rust quality pass:

1. **Derived rolling timeline** — drop the per-commute timeline-range setting; the
   axis length is the commute window duration (`end − start`), and the axis slides
   with real time as `[now, now + duration]`.
2. **Clock labels at both ends of the timeline** — bold `now` (left) and
   `now + duration` (right) times on each lane.
3. **Refresh labels on every poll** — recompute the end labels and the "now"
   marker on the existing 15 s arrivals tick.
4. **Home redesign** — a status hero header, a friendlier empty state, and richer
   inactive commute cards.
5. **Whole-codebase Rust refactor** — bring the entire workspace up to the strict
   quality bar (`.claude/skills/rust.md` + the youtun4 lint config), done last so
   the final code shape is what gets cleaned.

Architecture principle throughout: pure logic and string/format computation live
in `sgbr-core`; wiring lives in `src/lib.rs`; `.slint` files stay presentational.

---

## 1. Derived rolling timeline

### Current state
`Commute` stores a fixed `scale_minutes: u16` (default 15, clamped 10–120,
serde-defaulted for legacy records). The editor offers 15/30/45/60 m chips bound
to a `form-scale` property; `TimelineLane` takes a constant `scale-max`. Bus pills
are positioned by minutes-from-now via `cx-of(minutes)`.

### Target
- **Remove** the stored `scale_minutes` field, its `default_scale_minutes`
  helper, the `MIN/MAX/DEFAULT_SCALE_MINUTES` constants used for the chip range,
  the `with_scale_minutes` builder, and the related model tests.
- **Add** a method `Commute::scale_minutes(&self) -> u16` returning the window
  duration in minutes: `(end.hour*60 + end.minute) − (start.hour*60 + start.minute)`.
  `Commute::new` already guarantees `end > start`, so the result is always ≥ 1.
- **Serde back-compat:** legacy persisted commutes carry a `scale_minutes` field.
  The store must continue to deserialize them (serde ignores unknown fields unless
  `deny_unknown_fields` is set — verify `store.rs` and the struct do not set it).
  No migration write is required; the field is simply dropped on next save.
- **No cap** on the derived duration (per decision). A very long window produces a
  long axis; graduation ticks already stop at 60 m, beyond which only the baseline
  shows. Acceptable for realistic commute windows; documented here, not enforced.

### Rolling behaviour
The axis length (`scale-max`) is constant per render (= duration). The axis always
represents `[now, now + duration]`. Because pills are positioned by minutes-from-now,
they scroll left automatically as time advances — **no change to pill positioning**.
Worked example, window 09:30–10:00 (duration 30 m):

| Real time | Axis shown      |
|-----------|-----------------|
| 09:30     | 09:30 → 10:00   |
| 09:35     | 09:35 → 10:05   |
| 09:59     | 09:59 → 10:29   |

### Editor change (`ui/app.slint`)
Remove the timeline-range chip row (the 15m/30m/45m/60m group) and the
`form-scale` property. Remove its wiring in `src/lib.rs` (form population + save).

---

## 2. Clock labels at both ends of the timeline

### TimelineLane (`ui/components.slint`)
Add two string inputs to the component:
- `start-label` — rendered bold at the far left of the axis (e.g. `09:35`).
- `end-label` — rendered bold at the far right of the axis (e.g. `10:05`).

These sit at the axis extremities, visually aligned with the baseline ends. The
existing grey 5-minute graduation ticks and their relative-minute numbers stay
unchanged underneath. The "now" marker keeps its position at the left baseline.

### Computation
The labels are clock strings derived from the current local time and the commute
duration. A pure helper in `sgbr-core` formats them:

```
fn timeline_labels(now: Time, duration_minutes: u16) -> (String, String)
// returns ("HH:MM", "HH:MM") for (now, now + duration), 24h zero-padded,
// wrapping past midnight via modulo 24h.
```

`src/lib.rs` calls this when it builds each `StopLane` and sets `start-label` /
`end-label` alongside the arrival tags. `scale-max` is set from
`commute.scale_minutes()`.

---

## 3. Refresh labels on every poll

The existing repeated 15 s timer (`arrivals_timer` in `src/lib.rs`, active only
while the list is visible and a commute is active) already rebuilds lane models via
`spawn_arrivals` → `lanes_model`. Because `timeline_labels` is computed there from
"now", the end labels and the "now" marker refresh on the same tick automatically —
no new timer. Labels may lag wall-clock by up to 15 s, which is acceptable at
minute resolution.

---

## 4. Home redesign (`ui/app.slint` list screen + `src/lib.rs` + `sgbr-core`)

### 4a. Status hero header
A header band at the top of the list screen showing:
- **Large current time** + **weekday** (e.g. `09:35` / `Sat`).
- A **status line** computed in Rust:
  - `● Live now · <label>` when a commute is currently active;
  - else `Next · <label> in <countdown>` (e.g. `Next · Work in 2h 25m`);
  - else `No upcoming commutes`.

Exposed as Slint properties (e.g. `hero-time`, `hero-day`, `hero-status`). Updated
on the same 15 s tick (and on screen entry). Countdown/next-commute selection is a
pure function in `sgbr-core` over the commute set + current time.

### 4b. Friendlier empty state
Replace the single `"No commutes yet — tap + to add one."` line with a centered
block: a drawn bus glyph (Slint `Path`/shapes, no asset dependency), a
`No commutes yet` heading, and a `Tap + to track your first bus` hint.

### 4c. Richer inactive cards
Replace the single `inactive_summary()` line with:
- **Day pills** — M T W T F S S, with the commute's days filled and *today*
  emphasized.
- **Next window** — `next <Day> HH:MM`.
- **Counts** — `N stops · M buses`.

The data backing these (which day indices are on, today's index, next-window
label, counts) is computed in `sgbr-core` / `src/lib.rs` and exposed as structured
Slint properties; the `.slint` card renders pills + lines from them. The day-pill
labels reuse existing weekday short-name formatting where possible.

---

## 5. Whole-codebase Rust refactor (final phase)

Run **after** features 1–4 land, so the refactor cleans the final shape rather than
soon-to-be-rewritten code. Apply `.claude/skills/rust.md` and the youtun4 strict
bar across `crates/sgbr-core`, `src/`, and the platform bridges:

- Clean clippy: `all = deny`; `pedantic/cargo/...` warnings resolved; **no**
  `unwrap_used`, `expect_used`, `panic`, `unimplemented`, `unreachable`,
  `indexing_slicing`, `float_arithmetic`, `print_stdout`, `print_stderr`,
  `cast_*` in non-bridge code.
- `unsafe_code = "deny"` workspace-wide; bridge crates (Android JNI / iOS) override
  locally and carry `// SAFETY:` comments (`undocumented_unsafe_blocks` on).
- Code health: split oversized files into focused modules (notably the ~839-line
  `src/lib.rs`), prefer borrowing over cloning, iterators over manual loops, named
  constants over magic numbers, early returns over deep nesting.
- Gate: `cargo fmt --check` clean **and** `cargo clippy` clean **and** tests pass.

Behaviour-preserving only — no functional changes in this phase.

---

## Testing

- **`sgbr-core` unit tests** (pure, deterministic — pass a fixed `now`):
  - `Commute::scale_minutes()` for several windows incl. 1-minute and multi-hour.
  - `timeline_labels()` incl. a midnight-wrapping case and zero-padding.
  - Next-commute / countdown selection: active case, upcoming case, none case.
  - Inactive-card data: day-on set, today index, next-window label, counts.
  - Legacy-commute deserialization still succeeds (ignores `scale_minutes`).
- **Manual / visual:** run the app (desktop and on the connected Android phone),
  confirm the rolling axis labels advance across polls, the hero status reflects
  active vs upcoming vs none, the empty state and inactive cards render correctly.
- **Quality gates** green after phase 5 (`fmt --check`, `clippy`, `test`).

## Out of scope

- Capping or adaptive graduation for very long windows.
- Sub-15 s ("every second") marker animation.
- Any change to LTA fetching, notification content, or persistence format beyond
  dropping the `scale_minutes` field.

## Open risks

- `serde` `deny_unknown_fields` on the commute/store types would break legacy
  loads — must be verified absent before removing the field.
- Drawing a bus glyph purely with Slint shapes may need iteration to look clean;
  fallback is a simple geometric mark if a recognizable bus proves fiddly.
