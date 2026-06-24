# Multi-stop Commutes + rename to "SG Bus Commute" — design

**Date:** 2026-06-24
**Status:** Approved (brainstorming)
**Supersedes the data model in:** `2026-06-22-commutes-live-update-design.md` (flat one-line/one-stop commute)

## Summary

Generalise a **commute** from a single bus line at a single stop into a labelled,
scheduled container of **multiple stops**, each tracking **multiple buses**. The
currently-active commute(s) drive a single refresh cycle and one Live Update
notification covering every tracked bus. The active commute is shown in the list
as a per-stop arrival **timeline**. The app is renamed **SG Bus Commute** with a
new launcher icon.

No data migration: the on-disk format changes to the nested shape and any existing
`commutes.json` (seed/test data) is simply replaced.

## Concepts

- **Commute** = `label` + active `days` + a single-day time `window` (`start` < `end`,
  no overnight) + an ordered list of **stops**.
- **Stop** = an LTA bus stop (`code` + cached `name`) + an ordered list of tracked
  **buses** (service numbers).
- **Window** is the daily active interval (kept named "window", not "interval").
- **Active** = now falls on one of `days` and within `[start, end)`.
- **Overlap** = when two commutes are active at once, behaviour is the **union** of
  all their stops/buses (no "winner", no overlap prevention).

## Data model (`crates/sgbr-core/src/commute/model.rs`)

```rust
struct Commute {
    label: Option<String>,   // display fallback: see display_label()
    days: Weekdays,          // unchanged 7-bit mask
    start: TimeOfDay,        // window open  (inclusive)
    end: TimeOfDay,          // window close (exclusive)
    stops: Vec<CommuteStop>, // >= 1
}

struct CommuteStop {
    code: String,            // LTA BusStopCode
    name: String,            // cached stop name for display
    buses: Vec<String>,      // >= 1 service numbers
}
```

`TimeOfDay` and `Weekdays` are unchanged.

### Validation (`CommuteError`)

`Commute::new` (or a builder) validates, in order:

1. `NoDays` — `days.is_empty()`.
2. `InvalidTime` — `start`/`end` out of range.
3. `EndNotAfterStart` — `end <= start`.
4. `NoStops` — `stops.is_empty()`.
5. `StopEmptyCode` — any stop has an empty `code`.
6. `StopNoBuses` — any stop has zero buses.

Empty stops are **rejected at save time** (an explicit error), not silently pruned,
so the user always knows what was saved.

The old `EmptyLine` / `EmptyStop` variants are replaced by the above.

### Display label

`display_label()`:
- `Some(label)` → that label.
- `None` with one stop → `"<name>"` (the stop's cached name).
- `None` with multiple stops → `"<first stop name> +N"` (e.g. `"Opp Blk 123 +1"`).

### Serialization

`serde`-native nested JSON. `CommuteStore` round-trips the new shape. No
backward-compatible reader for the old flat format (per decision: no migration).

## Scheduling, fetch & active behaviour

`crates/sgbr-core/src/commute/schedule.rs` and the LTA client:

- `active_commutes_at(now)` returns **all** active commutes.
- The set of stops to refresh = **distinct** stop codes across all active commutes.
- For each distinct stop, **one** LTA `v3/BusArrival` call (returns all services at
  the stop); filter the response to the union of buses tracked at that stop by any
  active commute. This reduces calls versus per-(stop,line) fetching.
- `next_boundary` / alarm arming: unchanged in spirit — the soonest window boundary
  across all commutes arms the next exact alarm; window end stops the service when
  no commute remains active.
- Refresh cadence stays the existing ~15s while active.

### Arrival view model

A per-stop, time-sorted projection for the UI/notification:

```
StopArrivals { code, name, items: Vec<ArrivalItem> }   // sorted by minutes asc
ArrivalItem  { minutes: u32, buses: Vec<String> }       // same-minute buses grouped
```

`buses` groups services arriving at the **same stop at the same minute** (drives the
shared "14e·16" tag and the bracketed notification text).

## UI — Slint (`ui/app.slint`, `ui/components.slint`)

### Commutes list (list screen)

Per commute card:

- **Active commute:** red outline + live dot, and a **per-stop timeline**:
  - One lane per stop, labelled `name · code`.
  - All lanes share one horizontal scale `0 → max upcoming arrival`, **floored at 15
    min** (never shrinks below 15).
  - Each arrival: a **bus-number pill above the line** (same-minute buses share one
    pill, e.g. `14e·16`), a thin leader to the exact point on the line, and the
    **arrival minute in red below** the line.
  - Grey reference labels at `now / 5 / 10 / 15…` every 5 min.
  - Footer: `days · window`.
- **Off-window commute:** `See you soon · <day> <start>` + summary line
  `N stops · M buses · days · window`.
- Empty state unchanged ("No commutes yet — tap + to add one.").
- Tapping a card opens the editor.

A new Slint component (e.g. `CommuteTimeline` / `StopLane`) renders the lanes.
Slint draws the line, pills, leaders and minute labels from the `StopArrivals` model.

### Editor (inline accordion — approach A)

Single scrolling form:

1. **Label** — text field (optional).
2. **Active days** — 7 day toggles (unchanged component).
3. **Window** — Start / End native time pickers (unchanged).
4. **Stops & buses** — for each stop, a **stop card**:
   - Header: stop `name` + `code`, and an **× remove** control.
   - Body: **all services at that stop** rendered as toggle-chips; selected chips
     (red) are the tracked buses; tap to toggle.
   - **+ Add stop** button → opens the existing **stop-search screen**; on pick,
     append a stop card with its services as chips (all unselected).
5. **Save** (validates, surfaces `CommuteError` inline) and **Delete** (app bar,
   edit mode only).

Reuses the current stop-search screen unchanged. The per-stop service list comes
from the bus catalog (services-at-stop), as the single-line editor already does.

State plumbing: the flat editor properties (`form-line`, single day bools, etc.)
are replaced by a repeating stops/chips model. `save()` assembles the nested
`Commute` on the Rust side.

## Notification / Live Update (Android)

Ongoing notification (existing channel/foreground-service mechanism), content =
**one line per stop**, time-first with buses in brackets, across all active
commutes' stops:

```
Opp Blk 123: 2m (14), 4m (14e·16), 11m (154)
Bef Clementi Stn: 8m (96·156), 13m (96)
```

Built from the `StopArrivals` view model (NotificationCompat inbox/big-text style).
No custom RemoteViews timeline.

## Rename & icon

- **Name → "SG Bus Commute"**:
  - Android manifest `android:label` / `app_name`, Slint `Window.title`, README/docs.
- **Application id → `com.sgbuscommute`** (renamed from `com.sgbusoready`):
  - `applicationId` / `namespace` in `android/app/build.gradle.kts` and manifest.
  - Rename the Kotlin source package directory `com/sgbusoready` → `com/sgbuscommute`
    and all `package` / `import` declarations.
  - Update the JNI export symbols accordingly:
    `Java_com_sgbusoready_CommuteNative_*` → `Java_com_sgbuscommute_CommuteNative_*`
    (in `src/android_bridge.rs`), and any `load_app_class` binary names
    (`com.sgbuscommute.*`).
  - Reinstall is a fresh package (old `com.sgbusoready` install must be uninstalled;
    its seeded `commutes.json` does not carry over — acceptable, no migration).
- **Launcher icon** (the locked design):
  - Singapore-flag split — red upper zone (~⅔), white lower strip.
  - A wide white **front-bus** glyph on the red zone: rounded body, curved-bottom
    windshield, two side mirrors, two headlights, two legs (all red cut-outs/details).
  - A red **route line with multiple dots** on the white strip.
  - Regenerate `mipmap-*/ic_launcher.png` + `ic_launcher_round.png` at all densities,
    and update the adaptive `ic_launcher_foreground.xml` / `ic_launcher_background.xml`
    vector drawables to match.

## Out of scope (this slice)

- iOS / Live Activities.
- Reordering stops or commutes; per-bus mute/snooze.
- Changes to the API-36 Live Update chip beyond what already exists.
- Any data migration from the old flat format.

## Affected files (non-exhaustive)

- `crates/sgbr-core/src/commute/model.rs` — nested model + validation + display.
- `crates/sgbr-core/src/commute/store.rs` — nested JSON round-trip.
- `crates/sgbr-core/src/commute/schedule.rs` — union of active commutes; distinct-stop refresh.
- `crates/sgbr-core/src/commute/display.rs` — `StopArrivals` view model + notification line format.
- `crates/sgbr-core/src/lta/…` — per-stop fetch + filter to tracked buses.
- `ui/app.slint`, `ui/components.slint` — timeline component, accordion editor, list cards.
- `src/lib.rs` / `src/main.rs` — editor state plumbing for nested stops/buses.
- `src/android_bridge.rs` + Kotlin (`CommuteService`, `NotificationHelper`, `CommuteNative`) — render per-stop notification lines.
- `android/app/src/main/AndroidManifest.xml` + `res/` — label rename + icon assets.
- `android/app/build.gradle.kts` — `applicationId` / `namespace` → `com.sgbuscommute`.
- `android/app/src/main/kotlin/com/sgbuscommute/*` — package dir rename + `package`/`import` updates.
- `src/android_bridge.rs` — JNI export symbol rename (`Java_com_sgbuscommute_*`) + `load_app_class` names.
```
