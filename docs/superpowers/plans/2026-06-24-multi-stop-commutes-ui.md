# Multi-stop Commutes — UI (Slint + app glue) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or superpowers:subagent-driven-development. Pure helpers are TDD; Slint/glue tasks are verified by `cargo build` + clippy (and a desktop run where a display is available). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Render the active commute as a per-stop arrival **timeline** in the list, render off-window commutes with a summary, and replace the single-line editor with the **inline accordion** (multiple stops, each with toggle-chip buses). Restores a building workspace (the core rewrite left `src/lib.rs` on the old flat API).

**Architecture:** A small pure helper in `sgbr-core` computes the shared timeline scale. New Slint structs carry per-stop lanes and editor stops to the UI; new Slint components render a timeline lane and a stop-editor card. `src/lib.rs` is rewritten to the nested `Commute` API: it owns the editor's stop list as Rust state (rebuilding the Slint model on every mutation, mirroring how the commute store already drives `rows`), builds list rows with timeline lanes from `StopArrivals`, and assembles a nested `Commute` on save.

**Tech Stack:** Slint (declarative UI, dark theme in `ui/theme.slint`), Rust app crate bridging Slint generated types to `sgbr-core`. This is **Plan 2 of 3** (Plan 1 = core, done; Plan 3 = Android + branding/icon).

**Prereq:** Plan 1 merged on branch `feat/multi-stop-commutes` (`sgbr-core` exposes `CommuteStop`, `Commute{label,days,start,end,stops}`, `Commute::new(label,days,start,end,stops)`, `display_label`, `schedule::active_stop_plans`, `lta::arrival::{StopArrivals,ArrivalItem,stop_arrivals}`, `commute::display::{format_stop_line,format_active_notification,format_see_you_soon}`).

---

## File Structure

- `crates/sgbr-core/src/lta/arrival.rs` — **add** `timeline_scale_max(&[StopArrivals]) -> i64` (pure, TDD).
- `ui/app.slint` — **rewrite** list cards (timeline for active) and editor (accordion); new structs `StopLane`, `ArrivalTag`, `EditStop`; extend `CommuteRow`.
- `ui/components.slint` — **add** `TimelineLane` and `StopEditorCard` components; keep existing widgets.
- `src/lib.rs` — **rewrite** the commute↔Slint glue to the nested API: list rows + lanes, editor stop-list state, save/delete/edit, stop-picked appends a stop.

---

## Data contracts (Slint structs, defined in `ui/app.slint`)

```slint
// One bus arrival group on a lane: pre-joined bus label (e.g. "14e·16") and its
// whole-minute countdown. `minutes` positions it on the shared scale.
struct ArrivalTag { buses: string, minutes: int }

// One stop's timeline lane.
struct StopLane { name: string, code: string, tags: [ArrivalTag] }

// A commute list row. When `active`, `lanes` + `scale-max` drive the timeline;
// otherwise `status` carries the "see you soon …" summary.
struct CommuteRow {
    label: string,
    status: string,       // off-window summary / fallback text
    active: bool,
    index: int,
    lanes: [StopLane],    // empty when inactive
    scale-max: int,       // shared axis max in minutes (>= 15)
}

// An editor stop card: all services at the stop, and which are selected.
struct EditStop { code: string, name: string, services: [string], selected: [bool] }
```

`scale-max` and each `ArrivalTag.minutes` let Slint compute a tag's x-position as `minutes / scale-max` of the lane width.

---

## Task 1: Timeline scale helper (core, TDD)

**Files:**
- Modify: `crates/sgbr-core/src/lta/arrival.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `stop_arrivals_tests` module in `arrival.rs` (extend its `use super::...` to include `timeline_scale_max`):

```rust
    #[test]
    fn scale_max_floors_at_15() {
        let stops = vec![StopArrivals {
            code: "1".to_owned(),
            name: "A".to_owned(),
            items: vec![ArrivalItem { minutes: 3, buses: vec!["14".to_owned()] }],
        }];
        assert_eq!(timeline_scale_max(&stops), 15);
    }

    #[test]
    fn scale_max_uses_largest_minute_when_above_15() {
        let stops = vec![
            StopArrivals {
                code: "1".to_owned(),
                name: "A".to_owned(),
                items: vec![ArrivalItem { minutes: 3, buses: vec!["14".to_owned()] }],
            },
            StopArrivals {
                code: "2".to_owned(),
                name: "B".to_owned(),
                items: vec![ArrivalItem { minutes: 18, buses: vec!["96".to_owned()] }],
            },
        ];
        assert_eq!(timeline_scale_max(&stops), 18);
    }

    #[test]
    fn scale_max_is_15_for_empty() {
        assert_eq!(timeline_scale_max(&[]), 15);
    }
```

The test module's `use super::...` must read: `use super::{ArrivalItem, stop_arrivals, timeline_scale_max};`

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p sgbr-core scale_max`
Expected: FAIL to compile — `timeline_scale_max` not found.

- [ ] **Step 3: Implement**

Add to `arrival.rs` after `stop_arrivals`:

```rust
/// The shared timeline axis maximum, in minutes: the largest upcoming arrival
/// across all `stops`, floored at 15 so the scale never shrinks below a quarter
/// hour. Always at least 15.
#[must_use]
pub fn timeline_scale_max(stops: &[StopArrivals]) -> i64 {
    let largest = stops
        .iter()
        .flat_map(|s| s.items.iter())
        .map(|i| i.minutes)
        .max()
        .unwrap_or(0);
    largest.max(15)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p sgbr-core scale_max`
Expected: PASS (3 tests).

- [ ] **Step 5: Lint + commit**

Run: `cargo clippy -p sgbr-core --all-targets -- -D warnings` (clean), then:

```bash
git add crates/sgbr-core/src/lta/arrival.rs
git commit -m "feat(core): timeline_scale_max for shared arrival axis"
```

---

## Task 2: Slint structs + components

**Files:**
- Modify: `ui/app.slint` (struct definitions at top)
- Modify: `ui/components.slint` (new components)

- [ ] **Step 1: Replace the struct block + `CommuteRow` in `ui/app.slint`**

At the top of `ui/app.slint`, replace the existing `struct CommuteRow { ... }` and `struct StopResult { ... }` block with the four structs from the **Data contracts** section above (`ArrivalTag`, `StopLane`, `CommuteRow`, `EditStop`) plus the unchanged `StopResult`:

```slint
struct ArrivalTag { buses: string, minutes: int }
struct StopLane { name: string, code: string, tags: [ArrivalTag] }
struct CommuteRow {
    label: string,
    status: string,
    active: bool,
    index: int,
    lanes: [StopLane],
    scale-max: int,
}
struct EditStop { code: string, name: string, services: [string], selected: [bool] }
struct StopResult { code: string, name: string, road: string }
```

- [ ] **Step 2: Add `TimelineLane` to `ui/components.slint`**

Append to `ui/components.slint`. The lane draws a baseline, a "now" dot, and each tag as a bus pill above the line with a thin leader down to the line and the red minute below. Tag x-position = `tag.minutes / scale-max` of the usable width.

```slint
export component TimelineLane inherits Rectangle {
    in property <string> name;
    in property <[ArrivalTag]> tags;
    in property <int> scale-max: 15;
    height: 70px;
    // local padding so pills near the ends stay on-card
    property <length> pad-l: 6px;
    property <length> usable: self.width - root.pad-l - 10px;
    VerticalLayout {
        spacing: 0px;
        Text {
            text: root.name;
            font-size: Type.label;
            color: Palette.text-dim;
            overflow: elide;
        }
        Rectangle {
            vertical-stretch: 1;
            // baseline
            Rectangle {
                y: parent.height - 22px;
                x: root.pad-l;
                width: root.usable;
                height: 2px;
                background: Palette.hairline;
            }
            // now dot
            Rectangle {
                x: root.pad-l - 3px;
                y: parent.height - 26px;
                width: 8px; height: 8px; border-radius: 4px;
                background: Palette.accent-solid;
            }
            for tag in root.tags: Rectangle {
                property <length> cx: root.pad-l + root.usable * (tag.minutes / max(root.scale-max, 1));
                // leader
                Rectangle {
                    x: parent.cx - 1px;
                    y: 22px;
                    width: 2px;
                    height: parent.height - 22px - 22px;
                    background: Palette.hairline;
                }
                // bus pill (above the line)
                Rectangle {
                    x: parent.cx - self.width / 2;
                    y: 2px;
                    height: 20px;
                    border-radius: 7px;
                    background: Palette.accent-solid;
                    HorizontalLayout {
                        padding-left: 7px; padding-right: 7px;
                        Text {
                            vertical-alignment: center;
                            text: tag.buses;
                            font-size: Type.label;
                            font-weight: 700;
                            color: Palette.on-accent;
                        }
                    }
                }
                // red minute (below the line)
                Text {
                    x: parent.cx - self.width / 2;
                    y: parent.height - 18px;
                    text: tag.minutes <= 0 ? "due" : "\{tag.minutes}";
                    font-size: Type.label;
                    font-weight: 700;
                    color: Palette.accent-solid;
                }
            }
        }
    }
}
```

- [ ] **Step 3: Add `StopEditorCard` to `ui/components.slint`**

```slint
export component StopEditorCard inherits Rectangle {
    in property <string> name;
    in property <string> code;
    in property <[string]> services;
    in property <[bool]> selected;
    callback remove();
    callback toggle(int);          // service index
    border-radius: 12px;
    background: Palette.surface;
    border-width: 1px;
    border-color: Palette.hairline;
    VerticalLayout {
        padding: 12px;
        spacing: 8px;
        HorizontalLayout {
            VerticalLayout {
                horizontal-stretch: 1;
                spacing: 1px;
                Text { text: root.name == "" ? root.code : root.name; font-weight: 700; font-size: Type.body; color: Palette.text; overflow: elide; }
                Text { text: root.code; font-size: Type.label; color: Palette.text-dim; }
            }
            Rectangle {
                width: 30px;
                Text { text: "✕"; font-size: 18px; color: Palette.text-dim; }
                TouchArea { clicked => { root.remove(); } }
            }
        }
        Flickable {
            height: 46px;
            viewport-width: row.preferred-width;
            row := HorizontalLayout {
                spacing: 8px;
                for svc[i] in root.services: Chip {
                    text: svc;
                    selected: root.selected[i];
                    toggled => { root.toggle(i); }
                }
                if root.services.length == 0: Text {
                    text: "No services found";
                    color: Palette.text-faint;
                    vertical-alignment: center;
                    font-size: Type.body;
                }
            }
        }
    }
}
```

- [ ] **Step 4: Verify the components compile (full app build happens in later tasks)**

Run: `cargo build`
Expected: this will still FAIL because `src/lib.rs` references the old `CommuteRow` fields and old API — that is fixed in Tasks 3–4. Confirm the only errors are in `src/lib.rs`/`ui/app.slint` usages (no Slint *parse* errors in `components.slint`). If `slint` reports a syntax error inside `TimelineLane`/`StopEditorCard`, fix it before proceeding (Slint specifics: `for x[i] in model`, computed `property <length>` inside elements, `max(...)` builtin, string interpolation `"\{expr}"`).

- [ ] **Step 5: Commit (WIP — workspace not yet building)**

```bash
git add ui/app.slint ui/components.slint
git commit -m "feat(ui): timeline + stop-editor Slint components and structs"
```

---

## Task 3: List rendering with timeline (app glue)

**Files:**
- Modify: `src/lib.rs`

Goal: build `CommuteRow`s with timeline lanes for active commutes and a summary for inactive ones, using the nested API.

- [ ] **Step 1: Update imports**

In `src/lib.rs`, change the core imports to:

```rust
use sgbr_core::commute::display::format_see_you_soon;
use sgbr_core::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::{StopArrivals, stop_arrivals, timeline_scale_max};
use sgbr_core::lta::client::fetch_arrivals;
```

And the generated import to include the new structs:

```rust
use generated::{AppWindow, ArrivalTag, CommuteRow, EditStop, Screen, StopLane, StopResult};
```

- [ ] **Step 2: Replace `card_label`, `card_status`, `active_minutes`, `minutes_status`**

Replace those four functions (lines ~237–282) with the nested versions. `card_label` now uses `display_label` (which already prefers the user label, falling back to the first stop's name). The cached stop names live on `CommuteStop.name`, so we no longer need the catalog to label.

```rust
/// Card label: the commute's own label, or its first stop's name (+N).
fn card_label(commute: &Commute) -> String {
    commute.display_label()
}

fn see_you_soon(commute: &Commute, now: OffsetDateTime) -> String {
    commute
        .next_window_start(now)
        .map(format_see_you_soon)
        .unwrap_or_default()
}

/// Off-window summary line: "N stops · M buses · …".
fn inactive_summary(commute: &Commute, now: OffsetDateTime) -> String {
    let stops = commute.stops.len();
    let buses: usize = commute.stops.iter().map(|s| s.buses.len()).sum();
    let see = see_you_soon(commute, now);
    format!("{see} · {stops} stops · {buses} buses")
}
```

- [ ] **Step 3: Replace `rebuild_rows` to build inactive rows synchronously (no lanes)**

```rust
fn empty_lanes() -> ModelRc<StopLane> {
    ModelRc::new(VecModel::from(Vec::<StopLane>::new()))
}

fn rebuild_rows(window: &AppWindow, store: &CommuteStore) {
    let now = now_sgt();
    let mut rows: Vec<CommuteRow> = Vec::new();
    for (i, c) in store.commutes.iter().enumerate() {
        let active = c.is_active_at(now);
        rows.push(CommuteRow {
            label: SharedString::from(card_label(c)),
            status: SharedString::from(if active {
                "active now".to_owned()
            } else {
                inactive_summary(c, now)
            }),
            active,
            index: i32::try_from(i).unwrap_or(-1),
            lanes: empty_lanes(),
            scale_max: 15,
        });
    }
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}
```

(Note: the Slint field `scale-max` is accessed in Rust as `scale_max`.)

- [ ] **Step 4: Replace `spawn_arrivals` to fetch per active stop and build lanes**

Build `StopArrivals` per stop of each active commute (one `fetch_arrivals` per stop, filtered to that commute's tracked buses), then map to `StopLane`/`ArrivalTag` and compute `scale_max` per commute.

```rust
/// Build the timeline lanes + scale for one active commute (blocking; off-UI).
fn commute_lanes(commute: &Commute, now: OffsetDateTime) -> (Vec<StopLane>, i32) {
    let mut stop_arrivals_all: Vec<StopArrivals> = Vec::new();
    for stop in &commute.stops {
        let arrivals = match fetch_arrivals(ACCOUNT_KEY, &stop.code) {
            Ok(resp) => stop_arrivals(&stop.code, &stop.name, &stop.buses, &resp, now),
            Err(_) => StopArrivals {
                code: stop.code.clone(),
                name: stop.name.clone(),
                items: Vec::new(),
            },
        };
        stop_arrivals_all.push(arrivals);
    }
    let scale = timeline_scale_max(&stop_arrivals_all);
    let lanes: Vec<StopLane> = stop_arrivals_all
        .iter()
        .map(|sa| StopLane {
            name: SharedString::from(sa.name.as_str()),
            code: SharedString::from(sa.code.as_str()),
            tags: ModelRc::new(VecModel::from(
                sa.items
                    .iter()
                    .map(|it| ArrivalTag {
                        buses: SharedString::from(it.buses.join("·")),
                        minutes: i32::try_from(it.minutes).unwrap_or(0),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    (lanes, i32::try_from(scale).unwrap_or(15))
}

fn spawn_arrivals(window: &AppWindow, store: &CommuteStore) {
    if ACCOUNT_KEY.is_empty() {
        return;
    }
    let now = now_sgt();
    if !store.commutes.iter().any(|c| c.is_active_at(now)) {
        return;
    }
    let commutes = store.commutes.clone();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let now = now_sgt();
        let mut rows: Vec<CommuteRow> = Vec::new();
        for (i, c) in commutes.iter().enumerate() {
            let active = c.is_active_at(now);
            let (lanes, scale) = if active {
                commute_lanes(c, now)
            } else {
                (Vec::new(), 15)
            };
            rows.push(CommuteRow {
                label: SharedString::from(card_label(c)),
                status: SharedString::from(if active {
                    "active now".to_owned()
                } else {
                    inactive_summary(c, now)
                }),
                active,
                index: i32::try_from(i).unwrap_or(-1),
                lanes: ModelRc::new(VecModel::from(lanes)),
                scale_max: scale,
            });
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                w.set_rows(ModelRc::new(VecModel::from(rows)));
            }
        });
    });
}
```

- [ ] **Step 5: Update all `rebuild_rows`/`spawn_arrivals` call sites**

They previously took a `catalog` argument; now they don't. Update call sites in `spawn_refresh_if_stale`, `handle_save`, `run_app`, `on_delete` to call `rebuild_rows(&w, &store)` / `spawn_arrivals(&w, &store)` (drop the catalog arg and the surrounding `with_catalog` wrappers where they only served labelling). Keep the catalog for the stop-search screen and the editor service chips.

- [ ] **Step 6: Build (still expect editor errors), then fix list-only errors**

Run: `cargo build`
Expected: errors now only from the editor functions (`populate_form`, `handle_save`, `on_stop_picked`) handled in Task 4. Confirm the list/rows code compiles.

(No separate commit yet — Task 4 completes the building state.)

---

## Task 4: Accordion editor state (app glue)

**Files:**
- Modify: `src/lib.rs`
- Modify: `ui/app.slint` (editor screen markup + properties/callbacks)

Goal: the editor edits a label, days, window, and a list of stops (each with toggle-chip buses). Rust owns the working stop list and rebuilds the Slint `form-stops` model on every change.

- [ ] **Step 1: Editor state type + helpers (Rust)**

Add near the top of `src/lib.rs` (after the type aliases):

```rust
/// One stop being edited: its code/name, the full service list at that stop, and
/// which services are currently selected (parallel to `services`).
#[derive(Clone)]
struct EditStopState {
    code: String,
    name: String,
    services: Vec<String>,
    selected: Vec<bool>,
}

type FormStops = Rc<RefCell<Vec<EditStopState>>>;
```

Add a builder that pushes the Rust state into the Slint `form-stops` model:

```rust
fn push_form_stops(window: &AppWindow, stops: &[EditStopState]) {
    let model: Vec<EditStop> = stops
        .iter()
        .map(|s| EditStop {
            code: SharedString::from(s.code.as_str()),
            name: SharedString::from(s.name.as_str()),
            services: ModelRc::new(VecModel::from(
                s.services.iter().map(SharedString::from).collect::<Vec<_>>(),
            )),
            selected: ModelRc::new(VecModel::from(s.selected.clone())),
        })
        .collect();
    window.set_form_stops(ModelRc::new(VecModel::from(model)));
}
```

- [ ] **Step 2: Rewrite `populate_form`**

`populate_form` now loads label/days/window and the stop list (selecting the buses already tracked):

```rust
fn populate_form(
    window: &AppWindow,
    commute: Option<&Commute>,
    index: i32,
    catalog: Option<&BusCatalog>,
    form_stops: &FormStops,
) {
    let mut stops: Vec<EditStopState> = Vec::new();
    if let Some(c) = commute {
        window.set_form_label(SharedString::from(c.label.clone().unwrap_or_default()));
        set_days(window, c.days);
        window.set_start_hour(i32::from(c.start.hour));
        window.set_start_minute(i32::from(c.start.minute));
        window.set_end_hour(i32::from(c.end.hour));
        window.set_end_minute(i32::from(c.end.minute));
        for st in &c.stops {
            let services: Vec<String> = catalog
                .map(|k| k.services(&st.code).iter().map(ToString::to_string).collect())
                .unwrap_or_default();
            // ensure tracked buses appear even if catalog lacks them
            let mut services = services;
            for b in &st.buses {
                if !services.iter().any(|s| s == b) {
                    services.push(b.clone());
                }
            }
            let selected = services.iter().map(|s| st.buses.contains(s)).collect();
            stops.push(EditStopState {
                code: st.code.clone(),
                name: st.name.clone(),
                services,
                selected,
            });
        }
    } else {
        window.set_form_label(SharedString::new());
        set_days(window, Weekdays(0));
        window.set_start_hour(8);
        window.set_start_minute(0);
        window.set_end_hour(9);
        window.set_end_minute(0);
    }
    *form_stops.borrow_mut() = stops.clone();
    push_form_stops(window, &stops);
    window.set_editing_index(index);
    window.set_error_text(SharedString::new());
}
```

- [ ] **Step 3: Rewrite `handle_save`**

```rust
fn handle_save(window: &AppWindow, store: &Store, path: &Path, form_stops: &FormStops) {
    let label = window.get_form_label().to_string();
    let label = if label.trim().is_empty() { None } else { Some(label) };
    let days = read_weekdays(window);
    let start = time_of_day(window.get_start_hour(), window.get_start_minute());
    let end = time_of_day(window.get_end_hour(), window.get_end_minute());

    let stops: Vec<CommuteStop> = form_stops
        .borrow()
        .iter()
        .map(|s| CommuteStop {
            code: s.code.clone(),
            name: s.name.clone(),
            buses: s
                .services
                .iter()
                .zip(s.selected.iter())
                .filter(|(_, on)| **on)
                .map(|(svc, _)| svc.clone())
                .collect(),
        })
        .collect();

    let commute = match Commute::new(label, days, start, end, stops) {
        Ok(c) => c,
        Err(e) => {
            window.set_error_text(SharedString::from(e.to_string()));
            return;
        }
    };

    let mut s = store.borrow_mut();
    let target = usize::try_from(window.get_editing_index())
        .ok()
        .filter(|i| *i < s.commutes.len());
    if let Some(i) = target {
        if let Some(slot) = s.commutes.get_mut(i) {
            *slot = commute;
        }
    } else {
        s.commutes.push(commute);
    }
    persist(&s, path);
    maybe_start_service(&s);
    drop(s);

    populate_form(window, None, -1, None, form_stops);
    rebuild_rows(window, &store.borrow());
    spawn_arrivals(window, &store.borrow());
    window.set_screen(Screen::List);
}
```

- [ ] **Step 4: New editor callbacks — `toggle_bus`, `remove_stop`, and stop-picked appends**

In `run_app`, after constructing `form_stops` (`let form_stops: FormStops = Rc::new(RefCell::new(Vec::new()));`), wire:

```rust
    // Toggle a bus chip on stop `si`, service index `bi`.
    let w = window.as_weak();
    let fs = Rc::clone(&form_stops);
    window.on_toggle_bus(move |si, bi| {
        if let Some(w) = w.upgrade() {
            let (si, bi) = (si as usize, bi as usize);
            {
                let mut stops = fs.borrow_mut();
                if let Some(stop) = stops.get_mut(si) {
                    if let Some(sel) = stop.selected.get_mut(bi) {
                        *sel = !*sel;
                    }
                }
            }
            push_form_stops(&w, &fs.borrow());
        }
    });

    let w = window.as_weak();
    let fs = Rc::clone(&form_stops);
    window.on_remove_stop(move |si| {
        if let Some(w) = w.upgrade() {
            let si = si as usize;
            {
                let mut stops = fs.borrow_mut();
                if si < stops.len() {
                    stops.remove(si);
                }
            }
            push_form_stops(&w, &fs.borrow());
        }
    });
```

Rewrite `on_stop_picked` to **append** a stop to the editor list (instead of setting a single line/stop):

```rust
    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    let fs = Rc::clone(&form_stops);
    window.on_stop_picked(move |code| {
        if let Some(w) = w.upgrade() {
            let (name, services) = with_catalog(&c, |cat| {
                let name = cat
                    .and_then(|k| k.stop(&code))
                    .map_or_else(String::new, |s| s.name.clone());
                let services: Vec<String> = cat
                    .map(|k| k.services(&code).iter().map(ToString::to_string).collect())
                    .unwrap_or_default();
                (name, services)
            });
            let selected = vec![false; services.len()];
            fs.borrow_mut().push(EditStopState {
                code: code.to_string(),
                name,
                services,
                selected,
            });
            push_form_stops(&w, &fs.borrow());
            w.set_screen(Screen::Editor);
        }
    });
```

Update the `on_save`, `on_edit`, `on_new_commute` closures to pass `&form_stops` (and drop the catalog arg from `handle_save`). `on_edit`/`on_new_commute` call `populate_form(..., &form_stops)`.

Remove now-unused helpers: `services_model`, and the old single-stop form properties usage. Remove the `set_form_line`/`set_form_stop_code`/`set_form_stop_name`/`set_stop_services` calls (those properties are removed from Slint in Step 5).

- [ ] **Step 5: Rewrite the editor screen markup in `ui/app.slint`**

Replace the editor screen's form properties and body:

- Remove properties: `form-stop-code`, `form-stop-name`, `form-line`, `stop-services`.
- Add properties: `in-out property <string> form-label;`, `in property <[EditStop]> form-stops;`.
- Add callbacks: `callback toggle-bus(int, int);`, `callback remove-stop(int);`. Keep `stop-picked(string)`, `pick-time`, `save`, `delete`, etc.
- Editor body (inside the ScrollView VerticalLayout), in order:
  1. **Label** — `TextField { placeholder: "Label (optional)"; text <=> root.form-label; }`
  2. **Active days** — the existing 7 `DayToggle`s.
  3. **Window** — the existing two `TimeStepper`s.
  4. **Stops & buses** — section label, then:
     ```slint
     for st[si] in root.form-stops: StopEditorCard {
         name: st.name;
         code: st.code;
         services: st.services;
         selected: st.selected;
         remove => { root.remove-stop(si); }
         toggle(bi) => { root.toggle-bus(si, bi); }
     }
     GhostButton {
         text: "+ Add stop";
         clicked => { root.search-query = ""; root.screen = Screen.search; }
     }
     ```
  5. The existing error text + `PrimaryButton { text: "Save commute"; }`.

The stop-search screen's `stop-picked` handler should NOT also flip to editor (Rust's `on_stop_picked` now sets the screen). Keep `sf.defocus()` then `root.stop-picked(r.code)` and remove the explicit `root.screen = Screen.editor;` there (Rust sets it).

- [ ] **Step 6: Rewrite the list card to render the timeline**

In `ui/app.slint`, replace the `for row in root.rows: CommuteCard { ... }` usage so an active row shows the timeline. Either extend `CommuteCard` with lanes or inline. Recommended: render a `CommuteCard` for the header + status, and when `row.active`, a column of `TimelineLane`s underneath inside the same tappable card. Minimal approach — replace the `for` body with:

```slint
for row in root.rows: Rectangle {
    border-radius: 18px;
    background: row.active ? Palette.hero : Palette.surface;
    border-width: 1px;
    border-color: row.active ? Palette.accent-solid : Palette.hairline;
    VerticalLayout {
        padding: 14px;
        spacing: 8px;
        HorizontalLayout {
            Text { horizontal-stretch: 1; text: row.label; font-size: Type.body; font-weight: 700; color: Palette.text; overflow: elide; }
            if !row.active: Text { text: "›"; color: Palette.text-dim; }
        }
        if !row.active: Text { text: row.status; font-size: Type.caption; color: Palette.text-dim; wrap: word-wrap; }
        if row.active: VerticalLayout {
            spacing: 6px;
            for lane in row.lanes: TimelineLane {
                name: lane.name;
                tags: lane.tags;
                scale-max: row.scale-max;
            }
        }
    }
    TouchArea {
        clicked => { root.edit(row.index); root.screen = Screen.editor; }
    }
}
```

Import `TimelineLane`, `StopEditorCard` from `components.slint` at the top of `app.slint`.

- [ ] **Step 7: Build the whole workspace**

Run: `cargo build`
Expected: SUCCESS (host target). Fix any remaining compile errors (Slint property name mismatches use `-`/`_` conversion: Slint `form-stops` ↔ Rust `set_form_stops`/`get_form_stops`; `toggle-bus` ↔ `on_toggle_bus`).

- [ ] **Step 8: Lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean. Run `cargo fmt --all` then `cargo fmt --all -- --check` (clean).

- [ ] **Step 9: Full test suite**

Run: `cargo test`
Expected: PASS (sgbr-core green; app crate builds).

- [ ] **Step 10: Commit**

```bash
git add src/lib.rs ui/app.slint ui/components.slint
git commit -m "feat(ui): per-stop timeline list + accordion multi-stop editor"
```

---

## Task 5: Desktop smoke check (where a display is available)

- [ ] **Step 1: Run the desktop app**

Run: `cargo run` (needs a Wayland/X display). Seed a couple of commutes via the editor: add a label, pick days, set the window, add 1–2 stops, toggle buses, save. Confirm:
- List shows a card per commute; off-window shows "see you soon · N stops · M buses".
- Editing reopens with the stops + selected buses intact.
- Removing a stop and toggling buses updates the card on save.
- (Live timeline only renders with a valid `LTA_API_ACCOUNT_KEY` and an active window — without a key the active card shows lanes with empty axes, which is expected.)

If no display is available in this environment, record that visual verification is deferred to the user's device and rely on `cargo build`/`clippy`/`test` from Task 4.

---

## Self-Review notes

- **Spec coverage:** timeline scale (Task 1), Slint timeline + editor components (Task 2), active-card timeline + off-window summary (Task 3), accordion editor with multi-stop/multi-bus + nested save (Task 4). Restores building workspace (Task 4 Step 7).
- **Type consistency:** Slint `CommuteRow { label, status, active, index, lanes, scale-max }`, `StopLane { name, code, tags }`, `ArrivalTag { buses, minutes }`, `EditStop { code, name, services, selected }` are used identically in `app.slint`, `components.slint`, and `src/lib.rs` (Rust sees `scale_max`, `set_form_stops`, `on_toggle_bus`).
- **Carry-overs:** `format_active_notification`/`format_stop_line` are consumed by Plan 3 (Android), not here. `format_live_update` removal also happens in Plan 3.
```
