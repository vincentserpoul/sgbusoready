# SG Bus Ready — Commutes + Live Update design

**Date:** 2026-06-22
**Status:** Approved (design), pending implementation plan
**Supersedes (scope):** Narrows the locked MVP (`2026-06-21-sg-bus-ready-design.md`). Favourites, the in-app live-arrival list, and on-demand reminders are dropped/deferred in favour of the commutes model below.

## Summary

The app shrinks to one job: **manage a list of commutes, and surface live bus arrivals while a commute is happening.**

A **commute** is a recurring (bus line + bus stop + weekdays + time-of-day window). The user configures commutes in-app. While a commute's window is open, the app shows live arrivals on an Android **Live Update** (a foreground-service-driven ongoing notification, promoted to the API-36 Live Update chip). Outside any window, that surface is simply absent; the in-app list shows a per-row **"see you soon"** with the next occurrence.

This replaces the home-screen widget for live data: widgets are OS-throttled (~10–15 min) and are the wrong tool for "frequently updated values during a known window." A foreground service can update every few seconds while it runs.

## Platform scope

- **Android-only for this slice.** Target Android 16 / API 36 (user's Pixel 6a), with a plain ongoing foreground-service notification as the baseline and Live Update promotion on API 36+.
- **iOS deferred** (no Mac available yet). The Rust core is designed so iOS Live Activities (ActivityKit) can slot in later, replacing the Android service + notification glue.

## Data model

A **Commute**:

| Field | Type | Notes |
|---|---|---|
| `line` | string | bus service no., e.g. `14` |
| `stop` | string | LTA bus stop code, e.g. `83139` |
| `days` | set of weekday | e.g. Mon–Fri |
| `start` | time-of-day | e.g. `08:00` |
| `end` | time-of-day | e.g. `09:00`; assume `end > start` within a single day (no overnight windows in this slice) |
| `label` | string (optional) | defaults to `"<line> @ <stop>"` |

The commute **list** is persisted locally (no backend) as a small file (JSON or TOML) in app storage. Operations: add / edit / delete / reorder.

## Surfaces

### 1. In-app screen (Slint)
The only place commutes are configured. A list of commutes + an add/edit form. Each row shows status:
- **Active now:** live arrivals (mirrors the Live Update).
- **Inactive:** **"see you soon · next <day> <start>"**.

"See you soon" lives here — always glanceable, regardless of window state.

### 2. Live Update (Android)
Appears **only while a commute window is open**:
- Content: `<label>` + **next 3 arrivals** as minute countdowns, e.g. `Bus 14 · 3 min · 11 min · 19 min`.
- An action to open the app / tap-to-refresh.
- One Live Update per currently-active commute (overlapping windows → multiple concurrent Live Updates).
- Auto-dismissed at window `end`. Off-window: absent.

## Scheduling & lifecycle (no continuous polling)

Use **AlarmManager** to wake only at window boundaries — no always-on process.

1. For each commute, schedule an **exact alarm at the next window `start`** (respecting `days`).
2. At `start`: a **foreground service** launches, posts the Live Update, begins refreshing.
3. At `end` (a second alarm / service-internal timer): the service stops, the Live Update is dismissed, and the **next occurrence's** start-alarm is scheduled.
4. Editing/deleting a commute reschedules its alarms.
5. On device reboot, a `BOOT_COMPLETED` receiver re-arms all alarms.

## Refresh cadence & content

- While active, the foreground service fetches LTA every **~15s** via the existing `sgbr-core` client and re-posts the notification.
- Between fetches, the displayed countdown interpolates down per-second using the existing `minutes_until`.
- Content shows the **next 3** arrivals.

## Architecture (Rust-maximal)

- **`sgbr-core` (Rust, mostly existing):** already has LTA fetch + `ServiceArrivals` + `minutes_until`. **Add:**
  - a pure-Rust **commute model** and **window logic** — `is_active_at(now)`, `next_window_start(now)` — time-injected, no platform deps, fully unit-testable.
  - pure-Rust **settings persistence** (load/save the commute list).
  - a display-formatting helper that turns `ServiceArrivals` into the Live Update strings.
- **Slint UI:** settings list + add/edit form, bound to the core model.
- **Android glue (Kotlin + JNI):** AlarmManager scheduling, the foreground service, and posting/updating the notification (`NotificationCompat` + API-36 Live Update promotion). The service calls into Rust for fetch + formatting; Rust returns display strings.
- **iOS (later):** same core; Live Activities (ActivityKit) replace the foreground service + notification.

This builds directly on the Spike #2 plan (Gradle + `cargo-ndk` + JNI), which already introduces the notification bridge and AlarmManager groundwork.

## Testing

- **Rust unit tests:**
  - window logic: active/inactive across weekdays and times near `start`/`end` boundaries; `next_window_start` across day rollovers.
  - settings round-trip (serialize → deserialize → equal).
  - display formatting from `ServiceArrivals`.
- **On-device manual verification:** configure a commute starting in ~1 min; confirm the Live Update appears, updates every ~15s, and dismisses at `end`.

## Out of scope (this slice)

- Overnight windows (`end` crossing midnight).
- Home-screen widget for live data (replaced by Live Update).
- Favourites, in-app live-arrival browsing, on-demand reminders.
- iOS implementation.
- GPS / nearby / map.
