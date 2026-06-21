# Android Spike #2 — Gradle foundation + Notification + Widget

> **For agentic workers:** This plan is **interactive** (developer + phone) and **staged**. Phase A is the gating foundation — Phases B and C must not start until A renders on-device. Steps use checkbox (`- [ ]`) syntax. The Rust-side logic is TDD-able and dev-machine-verifiable; the Gradle/Kotlin/device steps need the NDK + phone and the developer.

**Goal:** Move the app from the throwaway `cargo-apk` build to a real **Gradle + `cargo-ndk`** project (the production structure), then prove the two native features that need Kotlin: a **one-shot local notification** (fire-and-forget reminder) and a **Glance home-screen widget** that displays a value the Rust core wrote.

**Architecture:** Keep the existing pure-Rust Slint app and **NativeActivity** (proven in Spike #1). A Gradle Android project packages the Rust `cdylib` (built by `cargo-ndk` into `jniLibs/`) and adds Kotlin only where the OS forces it: a `NotificationHelper` called from Rust via JNI, and a Glance `AppWidget`. The reminder *timing* logic lives in `sgbr-core` (testable); the OS scheduling call is the bridge.

**Tech Stack:** existing crates + `slint` android backend; `cargo-ndk`; Gradle (AGP) + Kotlin; `jni` crate; AndroidX `glance-appwidget`; `AlarmManager`/`NotificationManager`.

**Why staged (and why this is honest about risk):** The risky, version-sensitive part is the **Gradle + cargo-ndk + Slint NativeActivity** integration (Phase A). Once that renders on device, the notification (B) and widget (C) are "standard Android + one JNI call / one SharedPreferences read." So **Phase A is detailed and must be verified first**; B and C give concrete code but will firm up against A's exact structure. If A reveals the Slint-in-Gradle integration needs changes (e.g. GameActivity instead of NativeActivity), revisit B/C accordingly.

> **SDK note:** The AUR SDK used in Spike #1 had a non-standard `android-37.0` platform that needed a shim for cargo-apk. **Gradle/AGP is pickier about SDK layout** — strongly recommend installing a standard platform via Google cmdline-tools first: `sdkmanager "platforms;android-35" "build-tools;35.0.0" "platform-tools"`, and point `ANDROID_HOME` at that SDK. This avoids fighting AGP's SDK resolver.

---

## Phase A — Gradle + cargo-ndk + Slint (NativeActivity) on device

### Task A0: Prerequisites

- [ ] **Step 1: Install cargo-ndk and confirm a Gradle-friendly SDK**

```bash
cargo install cargo-ndk
# Standard SDK platform (see SDK note above):
sdkmanager "platforms;android-35" "build-tools;35.0.0" "platform-tools"
export ANDROID_HOME=<sdk-with-android-35>     # e.g. ~/Android/Sdk
export ANDROID_NDK_ROOT=/opt/android-ndk      # NDK r29 from Spike #1 is fine
```
Verify: `cargo ndk --version` works; `ls $ANDROID_HOME/platforms` shows `android-35`.

- [ ] **Step 2: Gradle** — install Gradle (`pacman -S gradle`) or use Android Studio's bundled Gradle. The plan uses the Gradle wrapper, so a one-time `gradle wrapper` is enough.

### Task A1: Scaffold the Gradle project

**Files (all new, under `android/`):**
- `android/settings.gradle.kts`, `android/build.gradle.kts`, `android/gradle.properties`
- `android/app/build.gradle.kts`
- `android/app/src/main/AndroidManifest.xml`

- [ ] **Step 1: Root Gradle files**

`android/settings.gradle.kts`:
```kotlin
pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositories { google(); mavenCentral() }
}
rootProject.name = "sgbusoready"
include(":app")
```

`android/build.gradle.kts`:
```kotlin
plugins {
    id("com.android.application") version "8.7.0" apply false
    id("org.jetbrains.kotlin.android") version "2.0.20" apply false
}
```
> Verify the AGP (`8.7.0`) and Kotlin (`2.0.20`) versions resolve with your installed Gradle; bump to whatever your Gradle supports if it complains.

`android/gradle.properties`:
```properties
org.gradle.jvmargs=-Xmx2048m
android.useAndroidX=true
kotlin.code.style=official
```

- [ ] **Step 2: App module Gradle**

`android/app/build.gradle.kts`:
```kotlin
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.serpoul.sgbusready"
    compileSdk = 35
    ndkVersion = "29.0.14206865"   // match /opt/android-ndk Pkg.Revision

    defaultConfig {
        applicationId = "com.serpoul.sgbusready"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a") }
    }
    buildTypes {
        getByName("debug") { isMinifyEnabled = false }
    }
    // jniLibs default srcDir is src/main/jniLibs — cargo-ndk writes there.
}

dependencies {
    // Phase C adds glance; Phase B needs no extra deps (JNI from Rust).
}
```

- [ ] **Step 3: Manifest declaring the existing NativeActivity**

`android/app/src/main/AndroidManifest.xml`:
```xml
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <application
        android:label="SG Bus Ready"
        android:hasCode="false">
        <activity
            android:name="android.app.NativeActivity"
            android:exported="true"
            android:configChanges="orientation|keyboardHidden|screenSize">
            <meta-data android:name="android.app.lib_name" android:value="sgbusoready" />
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
```
> `android:hasCode="false"` is correct for Phase A (no Kotlin yet). **Phase B/C add Kotlin → change to `android:hasCode="true"` (the default) and remove the attribute.** `POST_NOTIFICATIONS` is declared now but only used in Phase B.

### Task A2: Build the Rust cdylib into jniLibs and assemble the APK

- [ ] **Step 1: Build the .so with cargo-ndk**

From the repo root:
```bash
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build
```
Expected: `android/app/src/main/jniLibs/arm64-v8a/libsgbusoready.so` exists.
> This uses the existing `[lib] crate-type = ["rlib","cdylib"]` and the `cfg(target_os="android")` `android_main`. No code change needed from Spike #1.

- [ ] **Step 2: Generate the Gradle wrapper and build**

```bash
cd android && gradle wrapper && ./gradlew assembleDebug
```
Expected: `android/app/build/outputs/apk/debug/app-debug.apk`. If AGP errors on the SDK platform, that's the SDK-layout issue — install the standard `android-35` platform (SDK note).

- [ ] **Step 3: Install and verify on device**

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell monkey -p com.serpoul.sgbusready -c android.intent.category.LAUNCHER 1
adb exec-out screencap -p > /tmp/sgbr_gradle.png
```
**Success:** the SAME screen as Spike #1 — "Stop 83139", "15 — 8 min, 15 min". This proves the Gradle + cargo-ndk + Slint foundation. **Do not proceed to Phase B until this is green.**

- [ ] **Step 4: Commit**

```bash
git add android/ && git commit -m "feat(android): Gradle + cargo-ndk project building the Slint app"
```

---

## Phase B — One-shot local notification (fire-and-forget reminder)

The reminder *timing* is pure Rust (testable); the scheduling call crosses into Kotlin via JNI.

### Task B1: Reminder-delay logic in sgbr-core (TDD, dev-machine-verifiable)

**Files:** Create `crates/sgbr-core/src/reminder.rs`; add `pub mod reminder;` to `lib.rs`.

- [ ] **Step 1: Write the failing tests**

`crates/sgbr-core/src/reminder.rs`:
```rust
//! Fire-and-forget reminder timing. Pure logic: given the next bus's ETA in
//! minutes and a lead-time threshold, compute when to fire (in seconds from
//! now), or `None` if the bus is already within/under the threshold.

/// Seconds from now to fire a reminder, or `None` if it would be immediate/past.
#[must_use]
pub fn fire_delay_secs(eta_minutes: i64, threshold_secs: i64) -> Option<i64> {
    let eta_secs = eta_minutes.checked_mul(60)?;
    let delay = eta_secs.checked_sub(threshold_secs)?;
    if delay > 0 { Some(delay) } else { None }
}

#[cfg(test)]
mod tests {
    use super::fire_delay_secs;

    #[test]
    fn fires_before_arrival_by_threshold() {
        // bus in 11 min, threshold 5 min (300s) -> fire in 6 min (360s)
        assert_eq!(fire_delay_secs(11, 300), Some(360));
    }
    #[test]
    fn none_when_already_within_threshold() {
        // bus in 4 min, threshold 5 min -> already inside the window
        assert_eq!(fire_delay_secs(4, 300), None);
    }
    #[test]
    fn none_when_exactly_at_threshold() {
        assert_eq!(fire_delay_secs(5, 300), None);
    }
    #[test]
    fn supports_seconds_threshold() {
        // bus in 2 min (120s), threshold 30s -> fire in 90s
        assert_eq!(fire_delay_secs(2, 30), Some(90));
    }
}
```

- [ ] **Step 2: Run tests + clippy**

Run: `cargo nextest run -p sgbr-core reminder:: ; cargo clippy -p sgbr-core --all-targets -- -D warnings`
Expected: 4 tests pass; clippy clean.

- [ ] **Step 3: Commit**

```bash
git add crates/sgbr-core/src/reminder.rs crates/sgbr-core/src/lib.rs
git commit -m "feat(core): add fire_delay_secs reminder timing"
```

### Task B2: Kotlin notification helper

**Files:** Create `android/app/src/main/kotlin/com/serpoul/sgbusready/NotificationHelper.kt`. Flip `android:hasCode` to default (remove `="false"`) in the manifest.

- [ ] **Step 1: Kotlin helper with a JNI-friendly static method**

```kotlin
package com.serpoul.sgbusready

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat

object NotificationHelper {
    private const val CHANNEL_ID = "sgbr_reminders"

    @JvmStatic
    fun showNow(context: Context, title: String, text: String) {
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Bus reminders", NotificationManager.IMPORTANCE_HIGH)
        )
        val notif = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setAutoCancel(true)
            .build()
        nm.notify(1, notif)
    }
}
```
> Add `implementation("androidx.core:core-ktx:1.13.1")` to `app/build.gradle.kts` dependencies for `NotificationCompat`.

- [ ] **Step 2: Call it from Rust via JNI (spike: fire immediately on launch)**

For the spike, prove the bridge by firing a notification once at startup. In `src/lib.rs`, inside `android_main` (Android-only), after `slint::android::init`, obtain the `JavaVM` from the `AndroidApp` and call `NotificationHelper.showNow(...)` via the `jni` crate. Add `jni` to the `cfg(target_os="android")` deps.

```rust
// Sketch — exact JNI calls verified on-device. Uses android-activity's vm()/activity_as_ptr().
// 1. let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
// 2. let mut env = vm.attach_current_thread()?;
// 3. let activity = /* GlobalRef from app.activity_as_ptr() */;
// 4. env.call_static_method("com/serpoul/sgbusready/NotificationHelper", "showNow",
//        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
//        &[(&activity).into(), (&title).into(), (&text).into()])?;
```
> This JNI sequence is the part that needs on-device iteration — exact `AndroidApp` pointer accessors and the `Context` ref depend on the `android-activity` 0.6 API. Verify against `android_activity::AndroidApp` docs; treat the on-device notification appearing as the test.

- [ ] **Step 3: Build, install, verify** — rebuild the `.so` (`cargo ndk ...`), `./gradlew assembleDebug`, install, launch. **Success:** a heads-up notification "Next bus soon" appears. Grant the POST_NOTIFICATIONS runtime permission if prompted (Android 13+).

- [ ] **Step 4: Commit** the Kotlin helper + Rust JNI bridge.

> Real-app follow-up (not this spike): wire `fire_delay_secs` → `AlarmManager.setExactAndAllowWhileIdle` to a `BroadcastReceiver` that calls `showNow`, so it fires at the computed time even if the app is closed. The spike only proves the notification path works.

---

## Phase C — Glance home-screen widget

The widget is pure Kotlin (Glance) and reads a value the Rust core wrote to `SharedPreferences`.

### Task C1: Rust writes a widget snapshot

- [ ] **Step 1:** In `android_main` (or a small JNI-callable Rust fn), write the next-arrivals summary string (e.g. `"15: 8, 15 min"`) into `SharedPreferences("sgbr_widget", MODE_PRIVATE)` under key `"stop_83139"`, via JNI (same bridge mechanism as Phase B, calling a Kotlin `WidgetData.put(context, key, value)` helper). Keep it a single string for the spike.

### Task C2: Glance AppWidget

**Files:** `WidgetData.kt`, `BusWidget.kt`, `BusWidgetReceiver.kt`, widget XML + manifest `<receiver>`.

- [ ] **Step 1:** Add Glance deps to `app/build.gradle.kts`:
```kotlin
implementation("androidx.glance:glance-appwidget:1.1.0")
```
- [ ] **Step 2:** `WidgetData.kt` — `@JvmStatic fun put(context, key, value)` writing SharedPreferences; and a reader.
- [ ] **Step 3:** `BusWidget.kt` — a `GlanceAppWidget` whose `provideGlance` reads the SharedPreferences value and renders a `Text` with the favourite + timings.
- [ ] **Step 4:** `BusWidgetReceiver.kt` — `GlanceAppWidgetReceiver` exposing `BusWidget`; declare it in the manifest as a `<receiver>` with `android.appwidget.provider` meta-data + an `appwidget-info` XML (minWidth/Height, updatePeriodMillis).
- [ ] **Step 5: Build, install, add the widget** to the home screen, verify it shows the Rust-written value. **Success:** the widget displays "15: 8, 15 min" (or whatever the core wrote). This proves the Rust→shared-storage→native-widget data path.
- [ ] **Step 6: Commit.**

> Real-app follow-up: periodic + tap-to-refresh (call `BusWidget.update()` from a `CoroutineWorker`/on tap), and the OS refresh-budget handling — out of this spike.

---

## Self-Review

**Spec coverage (design-doc Android spike goals):**
- Gradle/Kotlin foundation (prereq for widget + notification) — Phase A. ✅
- Local notification (reminder path) — Phase B (B1 testable timing + B2 on-device fire). ✅
- Glance widget reading Rust-written data — Phase C. ✅
- Live LTA fetch on device — still deferred (needs INTERNET permission + key); add the permission + a fetch call when wiring real data. Noted, not silently dropped.

**Placeholder scan:** Phase A is concrete and verifiable. The two genuinely uncertain spots — the JNI call sequence (B2 Step 2) and exact AGP/Glance versions — are flagged with the on-device test as the gate, and a "verify against current API" instruction, rather than pretending certainty. This is intentional: the Gradle/JNI specifics can only be pinned against the live NDK/device, which this plan is executed with.

**Type/structure consistency:** `fire_delay_secs(eta_minutes, threshold_secs)` (B1) is the timing source the real-app AlarmManager wiring will consume. `NotificationHelper.showNow(context,title,text)` (B2) and `WidgetData.put(context,key,value)` (C1/C2) are the two Kotlin entry points the Rust JNI bridge calls — consistent across B and C. Package `com.serpoul.sgbusready` matches Spike #1's APK.

**Execution note:** Phase A and the on-device parts of B/C need the NDK + phone — run interactively. B1 (sgbr-core timing) is fully autonomous/TDD. Land and verify Phase A before B/C.
