# SG Bus Ready — Design

**Date:** 2026-06-21
**Status:** Approved (pending written-spec review)

## 1. Summary

A Singapore live bus-arrival mobile app for **iOS and Android**, built to be as
**Rust-maximal** as the platforms allow. Users save favourites (one bus line +
one stop), see live arrival times, pin a home-screen widget per favourite, and
arm a one-shot "next bus is X away" reminder.

The feature set is **not novel** — BusComing, Bustro, SG BusNow, Singabus and
SG BusLeh already cover live timing + widgets + arrival reminders, all on the
same LTA DataMall feed. The point of this project is the **Rust-only build** and
the engineering quality, not a market gap.

## 2. Product scope (MVP)

### In scope

1. **Search & add favourites**
   - Search by **5-digit stop code** or **bus number**. No GPS, no map.
   - A favourite = **one bus line + one stop**.
   - Favourites can be **renamed** and **reordered**; no hard limit (soft warning if very large).

2. **Live arrivals (in-app)**
   - Open a favourite/stop → show **next 3** buses.
   - **Auto-refresh ~15s** + **local per-second countdown interpolation** between fetches, with an "as of HH:MM:SS" stamp.
   - Graceful states: no service / last bus gone / API unreachable (show stamp + stale indicator).

3. **One-shot reminders (fire-and-forget)**
   - Per favourite: "notify me when the next bus is **X** away." Default **5 min**, configurable in **minutes or seconds**.
   - At arm time: read the current estimate, schedule a **local notification**, fire once, auto-disarm.
   - Labelled "based on the estimate when set." Optional single re-check when the app is foregrounded before firing. **No background polling.**

4. **Home-screen widget**
   - **One favourite per widget instance**; shows next 1–3 times.
   - Refresh: **periodic** (OS-throttled, ~5–15 min, "as of" stamp) **+ tap-to-refresh** (opens/pings the app for a fresh fetch).

### Out of scope (post-MVP backlog)

- GPS / nearby list / map
- Always-on or scheduled-window background notifications
- Lock Screen / Dynamic Island / Live Activities
- MRT, route planning, alight-here alerts
- Multi-favourite list widget

## 3. Data source

- **LTA DataMall** Bus Arrival API (real-time ETA, up to 3 next buses per service per stop; ISO timestamps).
- **Static datasets** (Bus Stops, Bus Routes — ~5–6k stops) power search and "which buses serve this stop." MVP plan: **bundle a snapshot in-app** and refresh periodically.
- **No backend.** The DataMall `AccountKey` ships embedded in the app. This is extractable and risks key abuse/revocation — **accepted for the MVP**. Escalation path if abused: a thin proxy (breaks "server-free," deferred).

## 4. Tech architecture — "B1"

**Pure-Rust core + Slint UI + thin native bridges only where the OS forces it.**

### Why Slint
On the user's priorities — fast launch, leanness, reactivity — the WebView
options (Tauri, Dioxus-mobile) lose: they carry WebView resident memory and
cold-start cost, and "Rust-only" there means a Rust→WASM frontend inside a
WebView. Slint renders natively on the GPU with a <300 KiB reactive runtime and
is **Rust-only on mobile by design**, so it matches both the performance goals
and the Rust-maximal goal. Native SwiftUI/Compose UI (approach A) would be the
performance ceiling but means writing the app UI twice and is not "a Rust app."

### Components

| Component | Tech | Unsafe? |
|---|---|---|
| `sgbr-core` — LTA client, parsing, search index, favourites store, reminder/countdown logic | Pure safe Rust | `unsafe_code = deny` |
| `sgbr-ui` — search / favourites / live-arrivals screens | Slint + Rust | `unsafe_code = deny` |
| Android bridge — notification scheduling + widget data | Rust → JNI (`jni` crate) → Kotlin (`NotificationManager`, `AlarmManager`, Glance/RemoteViews) | unsafe allowed locally, `// SAFETY:` required |
| iOS bridge — notification scheduling + widget data | Rust → Obj-C (`objc2`/Swift shim) → `UNUserNotificationCenter`; widget via App Group → WidgetKit (SwiftUI) | unsafe allowed locally, `// SAFETY:` required |

The **widget UI is native on both platforms** (WidgetKit SwiftUI; Android
Glance) in *every* possible approach — it cannot be Slint. The Rust core writes
a small "next arrivals" snapshot to shared storage (iOS App Group container /
Android SharedPreferences or file) that the widget reads.

### Data flow (reminder, the subtle one)
1. User arms a reminder on a favourite with threshold X.
2. `sgbr-core` fetches the current ETA, computes fire time = `eta - X`.
3. Bridge schedules a single local notification at that time. Core marks the reminder armed.
4. OS fires it once; reminder auto-disarms. (Optional: if app is foregrounded before fire time, re-fetch once and reschedule.)

### Error handling
- Network/API failures: show last good data + "as of" stamp + stale indicator; never crash (lints forbid `unwrap`/`panic`/`indexing_slicing`).
- Reminder scheduling failure surfaces a visible error on the favourite.
- All fallible paths return `Result` with `thiserror` types.

## 5. Risks & de-risking

| Risk | Severity | Mitigation |
|---|---|---|
| **Slint iOS maturity** (newer than Android port; 1.15 added safe-area/keyboard) | High | **Spike first** on a real iPhone before full build |
| Native bridges (JNI/Obj-C) are hand-rolled (no plugin ecosystem) | Medium | Keep bridges tiny + well-bounded; spike covers them |
| Mobile build tooling rougher than Tauri | Medium | Document the Android (`android-activity`) + iOS (Xcode + Rust staticlib) flows in the plan |
| Embedded AccountKey abuse | Low (MVP) | Accepted; proxy escalation path noted |
| Widget OS refresh budget (iOS ~40–70/day) | Low | Periodic + tap model already respects it |

### Spike (first implementation step — gates full commitment)
A green spike proves B1 end-to-end:
1. Slint "hello arrivals" on a **real iPhone + real Android device**.
2. One **live LTA fetch** rendered in Slint.
3. One **local notification** fired via the native bridge on each platform.
4. One **static home-screen widget** on each OS reading a value the Rust core wrote.

## 6. Quality bar

Ported from the `youtun4` project and **verified active** (`cargo clippy` +
`cargo fmt --check` clean on toolchain 1.96.0):

- Workspace `[lints]`: clippy `all = deny`; `pedantic/cargo/perf/style/...= warn`; denied `unwrap_used`, `expect_used`, `panic`, `unimplemented`, `unreachable`, `indexing_slicing`, `float_arithmetic`, `cast_possible_truncation/sign_loss/precision_loss`, `print_stdout`, `print_stderr`, `dbg_macro`.
- `clippy.toml` complexity thresholds; test-only relaxations.
- Pinned `rust-toolchain.toml` (1.96.0) with Android + iOS targets.
- `deny.toml` (advisories / licenses / bans / sources), `.taplo.toml`, `_typos.toml`.
- **To port in the plan:** `Justfile`, pre-commit hooks, `.github` CI, `cargo-vet` supply-chain, pinned dev-tool versions.

## 7. Open items for the implementation plan

- Crate split: `sgbr-core`, `sgbr-ui`, `sgbr-bridge-android`, `sgbr-bridge-ios`.
- HTTP client choice (`reqwest` vs `ureq`) and async runtime footprint on mobile.
- Static-dataset bundling + refresh strategy.
- Favourites persistence format + location per platform.
- The dev-tooling port listed in §6.
