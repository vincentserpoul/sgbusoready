# SG Bus Ready — Polished UI + Bus-Stop Search (design)

**Status:** approved in brainstorming 2026-06-23. Supersedes the Phase-D spike UI.

## Context / goal

The app works end-to-end (commutes, Live Update, settings) but the settings UI is
raw `std-widgets` and commutes are entered by typing a bus line and a 5-digit stop
code from memory. This redesign does two things:

1. **A distinctive, polished visual language** ("Direction C": dark-first, rounded
   cards, an indigo→coral gradient accent, custom components, subtle motion).
2. **A bus-stop search flow** so a commute is created by *searching a stop by name*,
   then *picking a line that actually serves it* — backed by a cached LTA catalog
   with optimized fuzzy search.

The stored data model is unchanged: a `Commute` is still `{line, stop, days, start,
end, label}`. Search only changes how `line`/`stop` are chosen, and the catalog lets
the UI show stop **names** instead of bare codes.

Non-goals (YAGNI): GPS / nearest-stop, favorites, map view, choosing route direction
(arrivals need only `service_no` + `stop`), live arrivals inside the settings list
(those stay in the Live Update notification).

---

## Part 1 — Visual design system (Direction C)

Dark-first. A single Slint `Palette` global holds tokens; every screen and component
reads from it (no hardcoded colors), so a future light theme is a token swap.

**Color tokens**
- `bg` `#0A0B10`, `surface` `#171A22`, `surface-alt` `#1D2029`, `hairline` `#262A36`
- `text` `#F0F1F6`, `text-dim` `#AEB3C0`, `text-faint` `#6E7280`
- `accent-a` `#8B7BFF` (indigo), `accent-b` `#FF8AA0` (coral); **accent gradient**
  = 135° `accent-a → accent-b`; `on-accent` `#0A0B10`
- `hero` card gradient = 135° `#1D1B3A → #2A1F33` (active commute)
- `chip-muted` `rgba(255,255,255,.08)` / text `#C8CCD8`
- `danger` = `accent-b` (`#FF8AA0`) — destructive/validation text

**Typography** — bundle **Inter** (OFL, embedded via Slint) for brand consistency;
fall back to the system UI font. Scale: title 22/800 (gradient fill), section label
11/600 uppercase +0.6px tracking (`text-faint`), body 14/600–700, caption 11–12/500.

**Custom components** (replace std-widgets; one purpose each, token-driven):
`AppBar` (← / title / optional action), `CommuteCard` (normal + active/hero variant),
`Chip` (service + weekday, on/off states), `DayToggle` (circular), `TimeStepper`
(tap-to-edit HH:MM with ▲▼), `TextField` (rounded), `SearchField`, `StopResultRow`,
`PrimaryButton` (gradient pill), `FloatingAddButton`. `SpinBox`/`CheckBox`/`LineEdit`
from `std-widgets` are dropped.

**Motion** (Slint `animate`, ~120–180ms, ease-out): button/chip press (scale 0.97 +
opacity), chip toggle (background color), screen push (horizontal slide), card
appear. Subtle, never blocking input.

**Navigation** — no library: a `Screen` enum property (`List | Editor | StopSearch`)
on the root, with slide transitions. State lives in Rust; the `.slint` switches the
visible screen.

### Screens
- **List** — title, a `CommuteCard` per commute (active one is the gradient hero with
  the accent "N min" chip + muted later arrivals; inactive shows "see you soon ·
  next …"), a `FloatingAddButton`. Tapping a card → Editor (edit); "+" → Editor (new).
- **Editor** (full screen, pushed) — `AppBar` (← / "New|Edit commute" / Delete when
  editing); **Stop** row (tap → StopSearch) showing the chosen stop name+code;
  **Line at this stop** chips (from the catalog); **Active days** `DayToggle`s;
  **Window** two `TimeStepper`s; gradient **Save**; inline validation text.
- **StopSearch** (full screen, pushed) — `AppBar` (← / "Choose stop") + autofocused
  `SearchField`; live `StopResultRow` results (name, road · code). Select → back to
  Editor with the stop set and its service chips populated.

---

## Part 2 — Bus catalog (data layer, in `sgbr-core`)

New, pure-Rust, fully unit-testable. Lives beside `commute` and `lta`.

**Model** (`bus_catalog::model`)
- `BusStop { code: String, name: String, road: String }`
- `BusCatalog { stops: Vec<BusStop>, services_by_stop: HashMap<String, Vec<String>>,
  fetched_at: OffsetDateTime }` with helpers `stop(code)`, `services(code)`,
  `is_stale(now, ttl)`.

**Fetch** (`bus_catalog::fetch`) — both LTA endpoints are OData-paginated 500/page via
`?$skip=`:
- `BusStops` → `{BusStopCode, Description→name, RoadName→road}` (~5,000 / ~11 pages).
- `BusRoutes` → `{ServiceNo, BusStopCode}` rows (~26k / ~53 pages), inverted into
  `services_by_stop` (deduped, naturally-sorted service numbers).
- Pages fetched **concurrently** with a bounded pool (`std::thread::scope`, ~8 in
  flight) so a full build is a few seconds, not ~30. Stop paging when a page returns
  < 500 rows. Returns `CoreError` on transport/parse failure (partial data discarded).

**Store** (`bus_catalog::store`) — `load(&Path)` / `save(&Path)` (atomic, like
`CommuteStore`) to `bus_catalog.json` in the app files dir. **TTL ≈ 30 days**
(`const CATALOG_TTL: Duration`); the data changes rarely, so this avoids needless
network use while `is_stale` + a background refresh keep it current over time.

**Search** (`bus_catalog::search`) — `nucleo-matcher` (the Helix high-performance
fuzzy matcher) over a normalized index built once per catalog load:
- Each stop indexed by `name` (primary) and `code`. Fuzzy-rank the query across
  ~5,000 stops (sub-millisecond linear scan), return top ~30 `&BusStop` by score.
- Exact/prefix matches on the 5-digit **code** are boosted above fuzzy name hits, so
  typing "83139" jumps the stop to the top.

**Refresh lifecycle**
- On launch the UI loads the cached catalog instantly (if present) — search never
  waits on the network.
- If the cache is missing or `is_stale`, kick a **background** refresh on a worker
  thread; on success, atomically save + swap the in-memory catalog (and rebuild the
  search index). A failed refresh leaves the existing cache intact.
- **First run / offline:** no cache yet → StopSearch shows an "Updating bus stops…"
  state; a manual 5-digit-code field remains as a fallback so a commute can still be
  added without the catalog.

---

## Part 3 — Wiring & data flow

- `Commute` model is untouched. On save, `line`/`stop` come from the picker (or the
  manual fallback). List rows resolve `stop → name` via the catalog for display,
  falling back to the raw code if the catalog lacks it.
- The catalog is owned in `src/lib.rs` (an `Rc<RefCell<Option<BusCatalog>>>` alongside
  the existing `CommuteStore`), passed to the StopSearch/Editor bindings. Search runs
  on the UI thread per keystroke (fast enough) with a light debounce; the fetch is the
  only threaded part.
- Triggers: `android_main` / desktop `main` load the cached catalog and spawn the
  staleness-checked background refresh, mirroring how alarms are armed today.

**New dependencies:** `nucleo-matcher` (sgbr-core). Concurrency uses `std::thread`
(no new dep). Inter font embedded as a UI asset. No async runtime.

---

## Error handling & testing

- **Core (unit-tested):** OData page-URL building; `BusStops`/`BusRoutes` JSON parse;
  `services_by_stop` inversion + dedup/sort; `is_stale` boundaries; search ranking
  (name fuzzy, code prefix boost, ordering) against fixture JSON. Mirrors the existing
  `sgbr-core` test style; stays under the strict lint bar (no `unwrap`/`indexing`).
- **Fetch failures** degrade gracefully: keep the old cache; surface nothing scary in
  the UI (search just uses what's cached, or the "updating…" state).
- **UI** verified by desktop screenshots and on the Pixel 6a (the established loop):
  search returns sensible stops, the line chips match the stop, save persists, the
  list shows names, and the visual system matches Direction C.

## Implementation staging (for the plan)

1. **Catalog core** — `bus_catalog` model/fetch/store/search in `sgbr-core`, fully
   tested, no UI. (Independently shippable.)
2. **Design system** — `Palette` global + custom Slint components + Inter, swapped in
   behind the current screens.
3. **Editor + StopSearch + navigation** — the new flow consuming the catalog.
4. **Catalog refresh wiring** — load-on-start + background staleness refresh on both
   platforms; on-device verification.
