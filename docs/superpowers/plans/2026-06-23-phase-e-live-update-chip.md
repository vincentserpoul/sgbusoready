# Phase E — API-36 Live Update chip promotion (scoping)

> Optional polish on top of the shipped commute Live Update. Promotes the existing
> ongoing notification to an Android 16 **Live Update**, so the next-bus arrival
> surfaces as a glanceable **status-bar chip** and is elevated on the lock screen /
> always-on display — without changing the data or the 15s refresh loop.

**Supersedes** the one-paragraph Phase E stub in
`docs/superpowers/plans/2026-06-23-android-commute-live-update.md`.

## Context / why now

The "should I rush for the bus?" question is best answered at a glance. Today the
arrivals live in a plain ongoing notification — only visible after pulling down the
shade. Android 16 (API 36) Live Updates render an eligible ongoing notification as a
compact status-bar chip (e.g. `15 · 3 min`) and surface it prominently on the lock
screen / AOD. This is presentation-only; the foreground service, fetch, render, and
boundary alarm are untouched.

## Prerequisites — ALL already met (the plan's old blocker is gone)

Probed 2026-06-23 on this machine + the Pixel 6a:
- `compileSdk = 36`, `targetSdk = 36` already set (`android/app/build.gradle.kts:8,13`).
- `~/Android/Sdk/platforms/android-36/android.jar` installed; `android/.env.build`
  already points `ANDROID_JAR` at it.
- Device is **API 36** (`ro.build.version.sdk = 36`), so it can render the chip.

So Phase E is now purely a few notification-builder lines + one permission + a dep bump.

## Eligibility audit of the current notification

`CommuteService.buildNotification` already satisfies every Live Update rule except the
explicit promotion request:

| Requirement | Current state |
|---|---|
| Style ∈ {Standard, BigText, Call, Progress, Metric} | ✅ `BigTextStyle` |
| `ongoing` (FLAG_ONGOING_EVENT) | ✅ `setOngoing(true)` |
| `contentTitle` set | ✅ "Next buses" |
| No `customContentView` / RemoteViews | ✅ none |
| Not a group summary | ✅ |
| Not `setColorized(true)` | ✅ never called |
| Channel importance ≠ IMPORTANCE_MIN | ✅ `IMPORTANCE_DEFAULT` |
| **`setRequestPromotedOngoing(true)`** | ❌ to add |
| **`POST_PROMOTED_NOTIFICATIONS` permission** | ❌ to add |

## Changes

1. **Manifest** (`android/app/src/main/AndroidManifest.xml`): add
   `<uses-permission android:name="android.permission.POST_PROMOTED_NOTIFICATIONS" />`
   (non-runtime; no grant prompt).

2. **Dependency** (`android/app/build.gradle.kts`): bump
   `androidx.core:core-ktx` `1.13.1` → **`1.17.0`** (first stable exposing
   `NotificationCompat.Builder#setRequestPromotedOngoing` / `setShortCriticalText` /
   `ProgressStyle`; verify latest 1.17.x at implementation time).

3. **`CommuteService.buildNotification`**: on API ≥ 36, add
   `.setRequestPromotedOngoing(true)` and `.setShortCriticalText(<chip text>)`.
   Guard with `Build.VERSION.SDK_INT >= 36` so pre-36 stays a plain ongoing
   notification (minSdk 24 fallback). Apply the same to `NotificationHelper.showNow`
   only if we keep the Phase-B test path promoted (optional).

4. **Chip short text** — keep it minimal, derive in Kotlin from the existing body's
   first line (`"Bus 15 · 3 min · 11 min"` → chip `"15 · 3 min"`). No Rust change →
   **no `.so` rebuild**. (Alternative: a dedicated `CommuteNative.renderChip` JNI
   export for exact formatting, at the cost of a `cargo-ndk` rebuild — not worth it.)

### Dependency alternative (if 1.17.0 is undesirable)
Keep `core 1.13.1` and build the promoted notification with the **platform**
`android.app.Notification.Builder` directly under the `SDK_INT >= 36` guard
(`setRequestPromotedOngoing`, `setShortCriticalText`), keeping `NotificationCompat`
for the fallback path. Costs a second builder code path; avoids any newer/alpha core.
**Recommended: take the dep bump (option A)** for one clean code path.

## Verification gate (on-device, the Pixel 6a, API 36)

Reuse the Phase D boundary harness (seed an active commute → start the service):
1. **Status-bar chip** shows the short text (e.g. `15 · 3 min`) while the window is open.
2. Notification is **elevated on the lock screen / AOD**.
3. Chip + notification **clear at window end** (unchanged service stop).
4. Confirm the build still assembles and the app launches (no core-bump regressions).
5. If the chip doesn't appear, check Settings → the app's "Promoted notifications"
   toggle (deep-link via `Settings.ACTION_MANAGE_APP_PROMOTED_NOTIFICATIONS`); the
   system can demote ineligible/over-posting notifications.

## Effort & risk

- **Effort: small** (~1–2 h incl. on-device check). 1 permission line, 1 dep bump,
  ~5 lines in `buildNotification`, a short-text helper. No Rust/`.so` change.
- **Risks:** (a) core `1.17.0` API names — verify `setRequestPromotedOngoing` /
  `setShortCriticalText` compile against the chosen version; (b) the OS may still
  withhold the chip if the user disabled promoted notifications or the notification
  over-posts; (c) keep the pre-36 fallback path so older devices are unaffected.
- **Out of scope:** `ProgressStyle`/`MetricStyle` countdown visuals (a nice follow-up
  once the basic chip is verified).
