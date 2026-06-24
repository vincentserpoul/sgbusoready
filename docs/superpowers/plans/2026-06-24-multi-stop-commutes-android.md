# Multi-stop Commutes — Android + Branding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans / subagent-driven-development. Rust changes verified by `cargo build`/`clippy`/`test`; Android (Kotlin/manifest/JNI) verified by the NDK+Gradle build where the toolchain is present, else compile-only + on-device deferral. Steps use checkbox (`- [ ]`).

**Goal:** Bring the Android layer onto the nested multi-stop model (per-stop Live Update notification, time-first buses-bracketed), rename the application id `com.sgbusoready` → `com.sgbuscommute` and the user-facing label to **SG Bus Commute**, and ship a new SG-flag launcher icon.

**Architecture:** `render_active` (Rust JNI) switches from the old flat `format_live_update` to the core `StopArrivals` view model + `format_active_notification` (one line per stop, union of active commutes via `active_stop_plans`). The obsolete `format_live_update` is removed from `sgbr-core`. The Kotlin package directory, all `package` declarations, the JNI export symbols, `load_app_class` binary names, and gradle `applicationId`/`namespace` move to `com.sgbuscommute`. The launcher icon becomes a red/white flag split with a white front bus and a red route-dot line, as adaptive vector layers plus rasterised mipmap PNGs.

**Tech Stack:** Rust JNI (`jni` crate), Kotlin + AndroidX, Gradle/AGP + cargo-ndk, Android adaptive icons. This is **Plan 3 of 3** (core + UI already merged on `feat/multi-stop-commutes`).

**Build toolchain (from project memory):** JDK 21 (`/usr/lib/jvm/java-21-openjdk`); `source android/.env.build`; `cargo ndk -t arm64-v8a -P 35 -o android/app/src/main/jniLibs build`; `(cd android && ./gradlew assembleDebug)`. `ANDROID_JAR`/`LTA_API_ACCOUNT_KEY` via `android/.env.build`.

---

## File Structure

- `crates/sgbr-core/src/commute/display.rs` — **remove** `format_live_update` + its tests.
- `src/android_bridge.rs` — **rewrite** `render_active`; rename JNI exports + `load_app_class` names + doc to `com.sgbuscommute`.
- `src/lib.rs` — rename the two JNI export fn names to `com.sgbuscommute`.
- `android/app/src/main/kotlin/com/sgbusoready/**` → **move to** `…/com/sgbuscommute/**`; update `package` lines.
- `android/app/src/main/kotlin/com/sgbuscommute/CommuteService.kt` — notification title + `chipText` for the new format.
- `android/app/build.gradle.kts` — `namespace`/`applicationId` → `com.sgbuscommute`.
- `android/app/src/main/AndroidManifest.xml` — `android:label` → `SG Bus Commute`.
- `android/app/src/main/res/drawable/ic_launcher_background.xml`, `ic_launcher_foreground.xml` — new flag/bus art.
- `android/app/src/main/res/mipmap-*/ic_launcher.png`, `ic_launcher_round.png` — regenerated from the new SVG.

---

## Task 1: Remove obsolete `format_live_update` (core)

**Files:** `crates/sgbr-core/src/commute/display.rs`

- [ ] **Step 1:** Delete the `format_live_update` function and its three tests (`live_update_lists_up_to_three_countdowns`, `live_update_shows_due_for_zero_or_negative`, `live_update_handles_no_buses`). Update the test module `use super::...` to drop `format_live_update`.
- [ ] **Step 2:** `cargo test -p sgbr-core` (PASS), `cargo clippy -p sgbr-core --all-targets -- -D warnings` (clean).
- [ ] **Step 3:** Commit: `refactor(core): drop obsolete format_live_update`.

> Note: this leaves `src/android_bridge.rs` referencing it until Task 2; do Task 2 in the same working session before any android build.

---

## Task 2: Per-stop notification render (Rust JNI)

**Files:** `src/android_bridge.rs`

- [ ] **Step 1:** Replace the imports:
```rust
use sgbr_core::commute::display::format_active_notification;
use sgbr_core::commute::schedule::{active_stop_plans, next_alarm_at};
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::{StopArrivals, stop_arrivals};
use sgbr_core::lta::client::fetch_arrivals;
```
(Drop `active_commutes_at`, `service_arrivals`, `format_live_update`.)

- [ ] **Step 2:** Replace `render_active`:
```rust
/// Render the Live Update body across all active commutes: one line per distinct
/// stop (union of tracked buses), time-first with buses bracketed. Empty string
/// => nothing active (caller stops the service).
fn render_active(files_dir: &str, now: OffsetDateTime) -> String {
    let store = CommuteStore::load(&store_path(files_dir)).unwrap_or_default();
    let plans = active_stop_plans(&store.commutes, now);
    if plans.is_empty() {
        return String::new();
    }
    let stops: Vec<StopArrivals> = plans
        .iter()
        .map(|plan| match fetch_arrivals(ACCOUNT_KEY, &plan.code) {
            Ok(resp) => stop_arrivals(&plan.code, &plan.name, &plan.buses, &resp, now),
            Err(e) => {
                log::warn!("fetch stop {} failed: {e}", plan.code);
                StopArrivals {
                    code: plan.code.clone(),
                    name: plan.name.clone(),
                    items: Vec::new(),
                }
            }
        })
        .collect();
    format_active_notification(&stops)
}
```

- [ ] **Step 3:** Rename the JNI exports and `load_app_class` names from `com_sgbusoready`/`com.sgbusoready` to `com_sgbuscommute`/`com.sgbuscommute` (also the module doc comment line). Specifically:
  - `Java_com_sgbusoready_CommuteNative_renderActive` → `Java_com_sgbuscommute_CommuteNative_renderActive`
  - `Java_com_sgbusoready_CommuteNative_nextAlarmEpochMillis` → `…com_sgbuscommute…`
  - `"com.sgbusoready.AlarmScheduler"` → `"com.sgbuscommute.AlarmScheduler"` (and `CommuteService`, `InsetsHelper`, `TimePicker`).

- [ ] **Step 4:** `cargo build` (host; `android_bridge` is cfg-android so not compiled here, but the workspace must still build) and `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5:** Commit: `feat(android): per-stop Live Update render + appid rename in JNI`.

---

## Task 3: Rename JNI exports in `src/lib.rs`

**Files:** `src/lib.rs`

- [ ] **Step 1:** Rename `Java_com_sgbusoready_CommuteNative_onTimePicked` → `Java_com_sgbuscommute_CommuteNative_onTimePicked` and `…onBackPressed` likewise.
- [ ] **Step 2:** `cargo build` + `cargo clippy --workspace --all-targets -- -D warnings` (clean).
- [ ] **Step 3:** Commit: `feat(android): rename lib.rs JNI exports to com.sgbuscommute`.

---

## Task 4: Kotlin package rename + gradle + manifest

**Files:** Kotlin sources, `build.gradle.kts`, `AndroidManifest.xml`

- [ ] **Step 1:** Move the Kotlin package directory:
```bash
git mv android/app/src/main/kotlin/com/sgbusoready android/app/src/main/kotlin/com/sgbuscommute
```
- [ ] **Step 2:** In every `.kt` under the new dir, change the first line `package com.sgbusoready` → `package com.sgbuscommute`. (No cross-file imports use the package name, so no other edits needed. `System.loadLibrary("sgbusoready")` in `CommuteNative.kt` stays — that's the cdylib name, unchanged.)
- [ ] **Step 3:** `android/app/build.gradle.kts`: set `namespace = "com.sgbuscommute"` and `applicationId = "com.sgbuscommute"`.
- [ ] **Step 4:** `AndroidManifest.xml`: change `android:label="SG Bus Ready"` → `android:label="SG Bus Commute"`. (Component names are relative `.MainActivity` etc., resolved against the new namespace — no change needed.)
- [ ] **Step 5:** Commit: `feat(android): rename package/appid to com.sgbuscommute, label SG Bus Commute`.

---

## Task 5: Notification title + chip for the new format

**Files:** `android/app/src/main/kotlin/com/sgbuscommute/CommuteService.kt`

- [ ] **Step 1:** Change the placeholder title `"SG Bus Ready"` → `"SG Bus Commute"` (the initial `startForegroundCompat(buildNotification("SG Bus Commute", "Updating…"))`).
- [ ] **Step 2:** Replace `chipText` to suit the new per-stop line format (`"Opp Blk 123: 2m (14), 4m (14e·16)"`). The chip should show the soonest time + buses:
```kotlin
/** Compact status-bar chip, e.g. line "Opp Blk 123: 2m (14), 4m (14e)" -> "2m (14)". */
private fun chipText(body: String): String {
    val firstLine = body.substringBefore('\n')
    // Everything after the stop name + ": ", first arrival group only.
    val afterColon = firstLine.substringAfter(": ", "")
    return afterColon.substringBefore(", ").ifEmpty { firstLine }
}
```
- [ ] **Step 3:** Commit: `feat(android): notification title + chip for per-stop format`.

---

## Task 6: Launcher icon (SG flag + bus + route dots)

**Files:** `ic_launcher_background.xml`, `ic_launcher_foreground.xml`, `mipmap-*/ic_launcher*.png`

- [ ] **Step 1:** Replace `ic_launcher_background.xml` with the flag split (red top ~⅔, white bottom), 108-viewport:
```xml
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp" android:height="108dp"
    android:viewportWidth="108" android:viewportHeight="108">
    <path android:fillColor="#FFFFFF" android:pathData="M0,0h108v108h-108z" />
    <path android:fillColor="#E2231A" android:pathData="M0,0h108v73h-108z" />
</vector>
```

- [ ] **Step 2:** Replace `ic_launcher_foreground.xml` with the white bus (red windshield/headlights/wheel-bars) + red route dots, kept within the central safe zone (~ x 30–78, y 26–92 of 108):
```xml
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp" android:height="108dp"
    android:viewportWidth="108" android:viewportHeight="108">
    <!-- bus body -->
    <path android:fillColor="#FFFFFF" android:pathData="M40,28 h28 a8,8 0 0 1 8,8 v22 a8,8 0 0 1 -8,8 h-28 a8,8 0 0 1 -8,-8 v-22 a8,8 0 0 1 8,-8 z" />
    <!-- windshield (red) -->
    <path android:fillColor="#E2231A" android:pathData="M40,36 h28 v10 q-14,4 -28,0 z" />
    <!-- headlights (red) -->
    <path android:fillColor="#E2231A" android:pathData="M42,52 h6 v3 h-6 z M60,52 h6 v3 h-6 z" />
    <!-- wheel bars (red) -->
    <path android:fillColor="#E2231A" android:pathData="M41,60 h9 v3 h-9 z M58,60 h9 v3 h-9 z" />
    <!-- route line + dots (red), on the white strip -->
    <path android:fillColor="#E2231A" android:pathData="M34,84 h40 v2 h-40 z" />
    <path android:fillColor="#E2231A" android:pathData="M34,85 m-4,0 a4,4 0 1 0 8,0 a4,4 0 1 0 -8,0 z" />
    <path android:fillColor="#E2231A" android:pathData="M74,85 m-4,0 a4,4 0 1 0 8,0 a4,4 0 1 0 -8,0 z" />
    <path android:fillColor="#E2231A" android:pathData="M54,85 m-2.5,0 a2.5,2.5 0 1 0 5,0 a2.5,2.5 0 1 0 -5,0 z" />
</vector>
```

- [ ] **Step 3:** Author a composed full-icon SVG (flag + bus + dots, full-bleed) at `/tmp/.../icon.svg` and rasterise to each mipmap density for both `ic_launcher.png` and `ic_launcher_round.png` (round can reuse the square source; the launcher masks it):
  - mdpi 48, hdpi 72, xhdpi 96, xxhdpi 144, xxxhdpi 192.
```bash
for d in "mdpi 48" "hdpi 72" "xhdpi 96" "xxhdpi 144" "xxxhdpi 192"; do
  set -- $d; px=$2
  rsvg-convert -w $px -h $px /tmp/.../icon.svg \
    -o android/app/src/main/res/mipmap-$1/ic_launcher.png
  cp android/app/src/main/res/mipmap-$1/ic_launcher.png \
     android/app/src/main/res/mipmap-$1/ic_launcher_round.png
done
```
- [ ] **Step 4:** Commit: `feat(android): SG-flag launcher icon (bus + route dots)`.

---

## Task 7: Android build verification

- [ ] **Step 1:** Build the Rust cdylib for arm64 and assemble the debug APK (toolchain present):
```bash
source android/.env.build
cargo ndk -t arm64-v8a -P 35 -o android/app/src/main/jniLibs build
(cd android && ./gradlew assembleDebug)
```
Expected: BUILD SUCCESSFUL; the cdylib exports `Java_com_sgbuscommute_CommuteNative_*` (verify with `nm -D android/app/src/main/jniLibs/arm64-v8a/libsgbusoready.so | grep com_sgbuscommute`).

- [ ] **Step 2:** If the toolchain/SDK is unavailable in this environment, record that and rely on `cargo build`/`clippy`/`test` (host) + on-device verification by the user. Deploy/visual checks (notification text, launcher icon, label) are done on the Pixel by the user:
```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell pm grant com.sgbuscommute android.permission.POST_NOTIFICATIONS
```

---

## Self-Review notes

- **Spec coverage:** per-stop notification (Tasks 1–2), appid+package rename (Tasks 2–4), label rename (Task 4), notification title/chip (Task 5), launcher icon (Task 6), build verification (Task 7).
- **Rename completeness:** JNI exports (android_bridge.rs + lib.rs), `load_app_class` strings, Kotlin `package` + dir, gradle `namespace`/`applicationId`. `System.loadLibrary("sgbusoready")` and `lib_name=sgbusoready` stay (cdylib name unchanged). Manifest component names are relative.
- **Carry-over:** none — this is the final plan. After it, the full feature is complete on `feat/multi-stop-commutes`.
