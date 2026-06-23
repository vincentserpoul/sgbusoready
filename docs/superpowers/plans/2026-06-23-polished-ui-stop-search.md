# Polished UI + Stop-Search Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (or executing-plans). UI quality is verified by **building + running + screenshotting** (desktop, then the Pixel 6a) — not unit tests; the catalog/search logic is already unit-tested in Plan 1. Steps use checkbox (`- [ ]`).

**Goal:** Redesign the SG Bus Ready UI to the approved "Direction C" visual system (dark-first, gradient accent, custom Slint components, motion) and replace manual line/stop entry with a stop-search flow backed by the Plan-1 `bus_catalog`.

**Architecture:** A Slint design system (`ui/theme.slint` tokens + `ui/components.slint` custom widgets) consumed by three screens (List, Editor, StopSearch) switched by a `Screen` enum on the root with slide transitions. Rust owns a `Rc<RefCell<Option<BusCatalog>>>` loaded from cache on start and refreshed in the background; it answers stop-search and services-at-stop queries for the editor. The stored `Commute` model is unchanged.

**Tech Stack:** Slint 1.15 (globals, custom components, `@linear-gradient`, `drop-shadow`, `animate`/states), the Plan-1 `sgbr_core::bus_catalog` API, existing `CommuteStore`/scheduling.

**Spec:** `docs/superpowers/specs/2026-06-23-polished-ui-stop-search-design.md`
**Depends on:** Plan 1 (merged) — `bus_catalog::{store::load/save, fetch::fetch_catalog, search::search, model::{BusStop,BusCatalog,CATALOG_TTL_SECS}}`.

---

## Conventions for every stage
- **Build/run desktop:** `cargo run --bin sgbusoready` (window opens; `grim /tmp/x.png` then read it — the app window is on niri; `niri msg windows` → `niri msg action move-window-to-floating --id <id>` + `focus-window` to capture it cleanly).
- **Lints:** strict bar applies; the Slint-generated code is already allow-listed in `src/lib.rs`'s `generated` mod. New Rust must avoid `unwrap`/`expect`/`panic`/indexing (tests may use them per `clippy.toml`).
- **Gate each stage:** `cargo clippy --workspace --all-targets -- -D warnings` clean + a screenshot showing the stage's intent + `cargo test --workspace` green.
- **Commit** at the end of each task with a `feat(ui):`/`feat(android):` message + the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.
- **Color tokens (dark-first):** `bg #0A0B10`, `surface #171A22`, `surface-alt #1D2029`, `hairline #262A36`, `text #F0F1F6`, `text-dim #AEB3C0`, `text-faint #6E7280`, accent gradient `135deg #8B7BFF→#FF8AA0`, `accent-solid #8B7BFF`, `on-accent #0A0B10`, hero gradient `135deg #1D1B3A→#2A1F33`, chip-muted `rgba(255,255,255,0.08)`/text `#C8CCD8`, danger `#FF8AA0`.

---

## Stage A — Design system (theme + components)

### Task A1: Palette global
**Files:** Create `ui/theme.slint`.

- [ ] **Step 1:** Create `ui/theme.slint`:

```slint
// Direction-C design tokens. Every component/screen reads from here — no
// hardcoded colors — so a future light theme is a token swap.
export global Palette {
    out property <color> bg: #0A0B10;
    out property <color> surface: #171A22;
    out property <color> surface-alt: #1D2029;
    out property <color> hairline: #262A36;
    out property <color> text: #F0F1F6;
    out property <color> text-dim: #AEB3C0;
    out property <color> text-faint: #6E7280;
    out property <color> accent-solid: #8B7BFF;
    out property <color> on-accent: #0A0B10;
    out property <color> chip-muted-bg: #FFFFFF14;
    out property <color> chip-muted-text: #C8CCD8;
    out property <color> danger: #FF8AA0;
    out property <brush> accent: @linear-gradient(135deg, #8B7BFF, #FF8AA0);
    out property <brush> hero: @linear-gradient(135deg, #1D1B3A, #2A1F33);
}
export global Type {
    out property <length> title: 22px;
    out property <length> body: 14px;
    out property <length> label: 11px;
    out property <length> caption: 12px;
}
```

- [ ] **Step 2:** Verify it imports — temporarily add `import { Palette } from "theme.slint";` at the top of `ui/app.slint`, `cargo build --bin sgbusoready`, expect success. (Leave the import; later tasks use it.)
- [ ] **Step 3:** Commit `feat(ui): Direction-C palette + type tokens`.

### Task A2: Core custom components
**Files:** Create `ui/components.slint`.

Build these components, each token-driven, with a pressed-state `animate` (scale 0.97 + opacity, 120ms ease-out) where interactive. **Write `PrimaryButton`, `Chip`, and `DayToggle` fully (below); follow the same pattern for the rest.**

- [ ] **Step 1:** Create `ui/components.slint` importing the theme and defining:
  - `AppBar { in property <string> title; in property <bool> show-back; in property <string> action-text; callback back(); callback action(); }` — a 56px row: optional `←`, centered/leading title (Type.title-ish, weight 700), optional right action text in `Palette.danger`.
  - `PrimaryButton { in property <string> text; in property <bool> enabled: true; callback clicked(); }` — full-width 48px, `background: Palette.accent`, `Palette.on-accent` text weight 700, radius 14px, pressed animation; dim to 40% opacity when `!enabled`.
  - `Chip { in property <string> text; in property <bool> selected; callback toggled(); }` — pill, `selected` → `Palette.accent`/`on-accent`, else `chip-muted-bg`/`chip-muted-text`; press animation; `toggled()` on click.
  - `DayToggle { in property <string> label; in property <bool> on; callback toggled(); }` — 32px circle, `on` → accent.
  - `TextField { in property <string> placeholder; in-out property <string> text; callback edited(); }` — rounded `surface`/`hairline` input wrapping a Slint `LineEdit` styled minimally (or a `TextInput` over a styled Rectangle).
  - `TimeStepper { in property <string> caption; in-out property <int> hour; in-out property <int> minute; }` — shows `HH:MM`, with `▲ ▼` TouchAreas that inc/dec hour & minute (wrap 0–23 / 0–59); tap-to-edit means stepping (no keyboard).
  - `FloatingAddButton { callback clicked(); }` — 56px accent circle, `+`, drop-shadow.

  Representative full code (PrimaryButton + Chip + DayToggle):

```slint
import { Palette, Type } from "theme.slint";

export component PrimaryButton inherits Rectangle {
    in property <string> text;
    in property <bool> enabled: true;
    callback clicked();
    height: 48px;
    border-radius: 14px;
    background: Palette.accent;
    opacity: root.enabled ? 1.0 : 0.4;
    states [ pressed when ta.pressed : { scale: 0.97; } ]
    animate scale { duration: 120ms; easing: ease-out; }
    ta := TouchArea { clicked => { if (root.enabled) { root.clicked(); } } }
    Text { text: root.text; color: Palette.on-accent; font-weight: 700; font-size: Type.body; }
}

export component Chip inherits Rectangle {
    in property <string> text;
    in property <bool> selected;
    callback toggled();
    height: 32px;
    border-radius: 16px;
    background: root.selected ? Palette.accent-solid : Palette.chip-muted-bg;
    HorizontalLayout { padding-left: 12px; padding-right: 12px;
        Text { vertical-alignment: center; text: root.text; font-weight: 600;
                color: root.selected ? Palette.on-accent : Palette.chip-muted-text; } }
    states [ pressed when ta.pressed : { scale: 0.96; } ]
    animate scale { duration: 100ms; easing: ease-out; }
    ta := TouchArea { clicked => { root.toggled(); } }
}

export component DayToggle inherits Rectangle {
    in property <string> label;
    in property <bool> on;
    callback toggled();
    width: 32px; height: 32px; border-radius: 16px;
    background: root.on ? Palette.accent-solid : Palette.chip-muted-bg;
    Text { text: root.label; font-size: Type.label; font-weight: 600;
            color: root.on ? Palette.on-accent : Palette.chip-muted-text; }
    ta := TouchArea { clicked => { root.toggled(); } }
}
```

- [ ] **Step 2: Build a gallery harness to eyeball them.** Temporarily set `ui/app.slint`'s window to render one of each component (hardcoded), `cargo run`, screenshot, confirm they match Direction C (gradient button, accent/muted chips, circular day toggles, stepper). Iterate on visuals until they look polished. Then restore `app.slint` (Stage B replaces it).
- [ ] **Step 3:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 4:** Commit `feat(ui): Direction-C component library`.

---

## Stage B — Rust catalog state + data contract + List screen

### Task B1: Catalog state + Slint data contract
**Files:** Modify `src/lib.rs`; modify `ui/app.slint` (structs/callbacks).

- [ ] **Step 1: Define the Slint data contract** (structs + properties + callbacks the Rust binds). In `ui/app.slint`:

```slint
struct CommuteRow { label: string, status: string, line: string, stop: string }
struct StopResult { code: string, name: string, road: string }
enum Screen { list, editor, stop-search }

export component AppWindow inherits Window {
    in property <[CommuteRow]> rows;
    in-out property <Screen> screen: Screen.list;
    // editor form state
    in-out property <int> editing-index: -1;
    in-out property <string> form-stop-code;
    in-out property <string> form-stop-name;
    in-out property <string> form-line;
    in property <[string]> stop-services;      // lines at the chosen stop
    in-out property <int> form-days;            // weekday bitmask
    in-out property <int> start-hour; in-out property <int> start-minute;
    in-out property <int> end-hour; in-out property <int> end-minute;
    in-out property <string> error-text;
    // stop search state
    in-out property <string> search-query;
    in property <[StopResult]> search-results;
    in property <bool> catalog-loading;
    callback save(); callback delete(int); callback edit(int); callback new-commute();
    callback search-changed();            // Rust runs bus_catalog::search
    callback stop-picked(string);         // Rust sets form-stop + stop-services
    // ... body added in B2/C ...
}
```

- [ ] **Step 2: Rust catalog state in `src/lib.rs`.** Add an `Rc<RefCell<Option<BusCatalog>>>` (`catalog`) loaded from `bus_catalog::store::load(catalog_path)` at startup (next to `commutes.json`). Wire `on_search_changed`: read `search-query`, if catalog present run `bus_catalog::search(&cat, &q, 30)`, map to `StopResult` rows, `set_search_results`; set `catalog-loading` from whether the catalog is `Some`. Wire `on_stop_picked(code)`: set `form-stop-code/name` from `cat.stop(code)`, set `stop-services` from `cat.services(code)`. Keep the existing commute load/save/delete callbacks, adapting field names (`form-line`/`form-stop-code`).
- [ ] **Step 3:** `cargo build` + `cargo clippy` clean. Commit `feat(ui): catalog state + Slint data contract`.

### Task B2: List screen with new components
**Files:** Modify `ui/app.slint`.

- [ ] **Step 1:** Implement the List screen body: title, a scrollable column of `CommuteCard`s (create this component in `components.slint`: hero/gradient variant when active, accent "N min" chip + muted later chips, "see you soon …" when inactive; tap → `edit(index)`), and a `FloatingAddButton` → `new-commute()` + `screen = editor`. The Rust `rows: [CommuteRow]` already carries label/status (extend `CommuteRow` mapping in `src/lib.rs` to resolve `stop → name` via the catalog and to render active arrivals if desired, else the existing status string).
- [ ] **Step 2: Verify on desktop** — seed `commutes.json`, `cargo run`, screenshot: list matches the mockup (hero card, chips, fab). Iterate.
- [ ] **Step 3:** Commit `feat(ui): redesigned commute list screen`.

---

## Stage C — Editor + StopSearch + navigation

### Task C1: Navigation + screen switching
**Files:** Modify `ui/app.slint`.

- [ ] **Step 1:** In `AppWindow`, render exactly one screen based on `screen` with a horizontal slide `animate` (e.g. each screen is a full-size container; switch via `x` offset + `animate x`). Back actions set `screen` to the previous one. `+`/row-tap set `editor`; the editor's Stop row sets `stop-search`; a search result sets it back to `editor`.
- [ ] **Step 2:** Build + screenshot the transitions. Commit `feat(ui): screen navigation + slide transitions`.

### Task C2: StopSearch screen
**Files:** Modify `ui/app.slint`.

- [ ] **Step 1:** `AppBar`(← / "Choose stop") + `SearchField` two-way bound to `search-query` with `edited => { root.search-changed(); }`; a scrollable list of `StopResultRow` (create in `components.slint`: name bold + `road · code` dim) from `search-results`, each `clicked => { root.stop-picked(r.code); root.screen = Screen.editor; }`. When `catalog-loading`, show an "Updating bus stops…" state with a manual code `TextField` fallback that sets `form-stop-code` directly.
- [ ] **Step 2: Verify on desktop** — needs a catalog cache; generate one via a one-off `fetch_catalog` saved to the desktop data dir (or copy from device), `cargo run`, type "clementi", confirm fuzzy results; type "83139", confirm code-first. Iterate. Commit `feat(ui): stop search screen`.

### Task C3: Editor screen
**Files:** Modify `ui/app.slint`.

- [ ] **Step 1:** `AppBar`(← / "New|Edit commute" / Delete when editing) + Stop row (shows `form-stop-name · form-stop-code` or "Choose a stop" → tap sets `screen = stop-search`) + "Line at this stop" `Chip`s from `stop-services` (tap sets `form-line`) + `DayToggle`×7 (bitmask `form-days`) + two `TimeStepper`s + inline `error-text` + `PrimaryButton` "Save commute" → `save()`. On save, Rust builds `Commute::new(form-line, form-stop-code, days, start, end, None)`, validates (error → `error-text`), persists, re-arms alarms, returns to `list`.
- [ ] **Step 2: Verify on desktop** — add a commute end-to-end (search stop → pick line → days → time → save), confirm it persists and shows in the list. Iterate. Commit `feat(ui): commute editor screen`.

---

## Stage D — Catalog refresh wiring + on-device

### Task D1: Load-on-start + background refresh
**Files:** Modify `src/lib.rs`, `src/main.rs`; add a small Rust refresh helper.

- [ ] **Step 1:** On startup (desktop `main` + `android_main`): load the cached catalog into the shared state. If absent or `is_stale(now)`, spawn a `std::thread` that runs `bus_catalog::fetch_catalog(ACCOUNT_KEY, now)`; on success `store::save` + replace the in-memory catalog and, via `slint::invoke_from_event_loop` + a `Weak<AppWindow>`, re-run the current search so an open StopSearch updates. The `ACCOUNT_KEY` comes from `env!("LTA_API_ACCOUNT_KEY")` (already used by `android_bridge`; expose a shared const or read in both entry points). Never block the UI thread on the fetch.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` green.

### Task D2: On-device verification (Pixel 6a)
- [ ] **Step 1:** `source android/.env.build && cargo ndk -t arm64-v8a -P 35 -o android/app/src/main/jniLibs build && (cd android && ./gradlew assembleDebug) && adb install -r android/app/build/outputs/apk/debug/app-debug.apk`.
- [ ] **Step 2:** Launch; on first run confirm the catalog fetches in the background (logcat `sgbr`), then the StopSearch returns results; add a commute via search → pick line → save; confirm it persists across relaunch and the Live Update still fires at the window boundary (reuse the Plan-D boundary harness). Screenshot each screen on-device; confirm Direction-C visuals.
- [ ] **Step 3:** Commit `feat(android): catalog load + background refresh; on-device verified`.

---

## Stage E — (Optional) Inter font
- [ ] Bundle Inter (OFL) under `ui/fonts/`, register/embed it, set it as the default family. Verify on desktop + device. Only if the system-font baseline needs more brand character. Out of scope for the working feature.

---

## Self-Review Notes
- **Spec coverage:** palette/type/components/motion (Stage A); list hero + chips (B2); editor with service chips from the catalog (C3); stop search fuzzy name+code (C2); navigation/transitions (C1); catalog load + ~30-day background refresh + offline "updating" fallback (D1/C2); on-device verify (D2). `Commute` model unchanged; rows resolve code→name (B1/B2).
- **Verification is visual:** each UI stage gates on a screenshot matching the approved mockups, plus clippy/test green; the catalog/search correctness is already unit-tested (Plan 1).
- **Risk/known-uncertain:** exact Slint syntax for slide transitions and `TextField`/`LineEdit` styling needs on-the-tool iteration (gated by screenshots); Inter embedding deferred to Stage E to de-risk the core redesign.
- **Type consistency:** Slint `CommuteRow{label,status,line,stop}`, `StopResult{code,name,road}`, callbacks `save/delete/edit/new-commute/search-changed/stop-picked` ↔ Rust `on_*`; `bus_catalog::search(&cat,&str,30)->Vec<&BusStop>` and `cat.stop/services` used in B1.
