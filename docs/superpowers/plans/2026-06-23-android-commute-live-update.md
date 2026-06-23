# Android Commute Live Update — On-Device Implementation Plan

> **For agentic workers:** This plan is **interactive** (developer + Pixel 6a + `adb`) and **staged**. Phases run in order; a phase's on-device gate must be green before the next starts. The pure-Rust logic is already merged and tested; what remains is platform glue (Gradle/Kotlin/JNI) that can only be verified on-device. REQUIRED SUB-SKILL when executing: superpowers:executing-plans (interactive, with the developer driving sudo/device steps). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Ship the commute Live Update on Android: a Gradle + `cargo-ndk` app that, while a commute's window is open, runs a foreground service which refreshes an **ongoing notification** every ~15s with that commute's live arrivals (rendered by `sgbr-core`), armed by AlarmManager at window boundaries — plus a Slint settings screen to manage commutes.

**Architecture:** Keep the proven pure-Rust Slint app on `NativeActivity` (Spike #1). A Gradle project packages the Rust `cdylib` (built by `cargo-ndk` into `jniLibs/`) and adds Kotlin only where the OS forces it: a `NotificationHelper`, a `CommuteService` (foreground), an `AlarmReceiver`/`BootReceiver`, and a thin `AlarmScheduler`. All decisions stay in Rust, exposed as JNI functions the Kotlin glue calls: which commutes are live now, the live-arrival text, and the next alarm time. Settings persist via `CommuteStore::load`/`save` to the app's internal files dir.

**Tech Stack:** existing crates + `slint` android backend; `cargo-ndk` 4.1.2; Gradle (wrapper-pinned) + AGP + Kotlin; `jni` crate; `AlarmManager`/`NotificationManager`/foreground `Service`. JDK 21 for the build.

**Pure-Rust foundation already merged (consumed by this plan):**
- `commute::model` — `Commute`, `TimeOfDay`, `Weekdays`, validation.
- `commute::window` — `is_active_at`, `next_window_start`, `current_window_end`, `next_boundary`.
- `commute::schedule` — `active_commutes_at(&[Commute], now)`, `next_alarm_at(&[Commute], now)`.
- `commute::display` — `format_live_update(line, &[i64])`, `format_see_you_soon(next_start)`.
- `commute::store` — `CommuteStore::{to_json, from_json, load(&Path), save(&Path)}`.
- `lta::client::fetch_arrivals(account_key, bus_stop_code)` → `BusArrivalResponse`; `lta::arrival::service_arrivals(&resp, now)`.

**Spec:** `docs/superpowers/specs/2026-06-22-commutes-live-update-design.md`
**Supersedes:** the Glance-widget half of `docs/superpowers/plans/2026-06-21-android-spike-2-gradle-notification-widget.md` (Phase C dropped; Phase A/B mechanism reused below).

**Environment facts (probed 2026-06-23):** Pixel 6a `bluejay`, Android 16 / API 36, connected. `cargo-ndk` 4.1.2, native target `aarch64-linux-android` installed. NDK r29 at `/opt/android-ndk` (caps native build at API 35). Only JDK 26 installed → **Phase 0 installs JDK 21**. SDK at `/opt/android-sdk` has only the non-standard `android-37.0`; a Spike-#1 shim at `~/.android-sdk-shim` exposes `android-35`. We target **compileSdk/targetSdk 35, minSdk 24**, native build API ≤ 35. The API-36 Live Update *chip promotion* is deferred to the optional Phase E.

---

## Phase 0 — Toolchain (interactive, dev machine)

**Why first:** released Gradle/AGP cannot run on JDK 26, and AGP needs a clean integer SDK platform. This phase produces a build environment that just works, so Phase A isn't fighting two problems at once.

### Task 0.1: Install JDK 21 and a build env file

- [ ] **Step 1 (developer, sudo):** Install JDK 21.

```bash
sudo pacman -S --needed jdk21-openjdk
```
Verify: `ls -d /usr/lib/jvm/java-21-openjdk` exists. **Do NOT** change the system default (leave 26 default); we point only the build at 21.

- [ ] **Step 2: Write a sourceable env file** `android/.env.build` (git-ignored) so every Gradle/cargo-ndk invocation is consistent:

```bash
export JAVA_HOME=/usr/lib/jvm/java-21-openjdk
export ANDROID_NDK_ROOT=/opt/android-ndk
# ANDROID_HOME / ANDROID_SDK_ROOT set in Task 0.2 once a clean SDK exists.
```
Add `android/.env.build` to `.gitignore`. (It contains no secrets, but it's machine-specific.)

### Task 0.2: A Gradle-friendly SDK platform (android-35)

AGP is pickier than `cargo-apk`. Try the existing shim first; fall back to a clean Google SDK.

- [ ] **Step 1: Try the existing shim.** Point the build at the Spike-#1 shim and see if AGP accepts it in Phase A. Append to `android/.env.build`:

```bash
export ANDROID_HOME=$HOME/.android-sdk-shim
export ANDROID_SDK_ROOT=$HOME/.android-sdk-shim
```
Verify: `ls $ANDROID_HOME/platforms/android-35/android.jar` exists. If it does, proceed; AGP's acceptance is confirmed at the Phase A gate. If `android.jar` is missing under `android-35`, do Step 2.

- [ ] **Step 2 (fallback, if the shim's `android-35` lacks `android.jar` or Phase A's AGP rejects it):** Install a clean Google command-line SDK.

```bash
mkdir -p ~/Android/Sdk/cmdline-tools
# Download "Command line tools only" (Linux) from developer.android.com, unzip so that:
#   ~/Android/Sdk/cmdline-tools/latest/bin/sdkmanager  exists
export ANDROID_HOME=$HOME/Android/Sdk
export ANDROID_SDK_ROOT=$HOME/Android/Sdk
yes | $ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager --licenses
$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager \
  "platforms;android-35" "build-tools;35.0.0" "platform-tools"
```
Update `android/.env.build`'s `ANDROID_HOME`/`ANDROID_SDK_ROOT` to `~/Android/Sdk`. Keep `ANDROID_NDK_ROOT=/opt/android-ndk` (NDK r29 stays).

- [ ] **Step 3: Confirm the toolchain.**

```bash
source android/.env.build
java -version            # must show 21
cargo ndk --version      # 4.1.2
gradle --version         # runs without a JVM error under JAVA_HOME=21
ls $ANDROID_HOME/platforms
```
Gate: `java -version` is 21; `gradle --version` prints a version with no `Unsupported class file major version`/JVM error; `platforms` lists `android-35`.

> No commit in Phase 0 (only `.gitignore` + an ignored env file). Commit the `.gitignore` change with Phase A.

---

## Phase A — Gradle + cargo-ndk + Slint (NativeActivity) on device

Reproduce the Spike #1 screen, but built by the production Gradle structure. **Gate before any Kotlin features.**

### Task A1: Scaffold the Gradle project

**Files (new, under `android/`):** `settings.gradle.kts`, `build.gradle.kts`, `gradle.properties`, `app/build.gradle.kts`, `app/src/main/AndroidManifest.xml`. Also add `android/.gradle/`, `android/.env.build`, `android/app/build/`, `android/app/src/main/jniLibs/` to `.gitignore`.

- [ ] **Step 1: `android/settings.gradle.kts`**

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

- [ ] **Step 2: `android/build.gradle.kts`**

```kotlin
plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
```
> If the Phase A build reports AGP/Gradle incompatibility, the wrapper version (Task A3 Step 1) is the lever — bump the wrapper to the Gradle that AGP 8.7.3 requires (8.9+), not the AGP here.

- [ ] **Step 3: `android/gradle.properties`**

```properties
org.gradle.jvmargs=-Xmx2048m
android.useAndroidX=true
kotlin.code.style=official
```

- [ ] **Step 4: `android/app/build.gradle.kts`**

```kotlin
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.serpoul.sgbusready"
    compileSdk = 35
    ndkVersion = "29.0.14206865"   // /opt/android-ndk Pkg.Revision

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
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    // jniLibs default srcDir is src/main/jniLibs — cargo-ndk writes there.
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
}
```

- [ ] **Step 5: `android/app/src/main/AndroidManifest.xml`** (Phase A: pure-Rust, no Kotlin yet → `hasCode="false"`)

```xml
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
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
> Phases B–D add Kotlin → flip to `android:hasCode="true"` (remove the attribute) and add the permissions/components noted there.

### Task A2: Build the cdylib into jniLibs

- [ ] **Step 1:** From the repo root, with the env sourced:

```bash
source android/.env.build
cargo ndk -t arm64-v8a -P 35 -o android/app/src/main/jniLibs build
```
Expected: `android/app/src/main/jniLibs/arm64-v8a/libsgbusoready.so` exists. `-p 35` pins the native API level to 35 (NDK r29 max). No Rust change needed — reuses the existing `[lib] crate-type=["rlib","cdylib"]` and `android_main`.

### Task A3: Assemble, install, verify on device

- [ ] **Step 1: Generate the wrapper (pin Gradle), then build**

```bash
source android/.env.build
cd android && gradle wrapper --gradle-version 8.10.2 && ./gradlew assembleDebug
```
Expected: `android/app/build/outputs/apk/debug/app-debug.apk`. If AGP errors on the SDK platform, that's the SDK-layout issue → Phase 0 Task 0.2 Step 2 (clean SDK). If it errors on Gradle/AGP/JDK compatibility, adjust the wrapper Gradle version.

- [ ] **Step 2: Install and launch**

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell monkey -p com.serpoul.sgbusready -c android.intent.category.LAUNCHER 1
adb exec-out screencap -p > /tmp/sgbr_gradle.png
```
**Gate (must be green to proceed):** the SAME screen as Spike #1 — stop `83139`, service `15` with `8 min, 15 min`. This proves Gradle + cargo-ndk + Slint. Inspect `/tmp/sgbr_gradle.png`.

- [ ] **Step 3: Commit**

```bash
git add android/ .gitignore
git commit -m "feat(android): Gradle + cargo-ndk project building the Slint app"
```

---

## Phase B — JNI ongoing notification (prove the bridge)

Prove Rust can post a notification through a Kotlin helper. This is the mechanism Phase C reuses for the Live Update.

### Task B1: Kotlin notification helper

**Files:** `android/app/src/main/kotlin/com/serpoul/sgbusready/NotificationHelper.kt`. Manifest: remove `android:hasCode="false"`, add `<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />`.

- [ ] **Step 1: Helper with a JNI-friendly static method**

```kotlin
package com.serpoul.sgbusready

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import androidx.core.app.NotificationCompat

object NotificationHelper {
    const val CHANNEL_ID = "sgbr_commute"
    const val NOTIF_ID = 1

    @JvmStatic
    fun ensureChannel(context: Context) {
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Commute arrivals", NotificationManager.IMPORTANCE_DEFAULT)
        )
    }

    @JvmStatic
    fun showNow(context: Context, title: String, text: String) {
        ensureChannel(context)
        val nm = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val notif = NotificationCompat.Builder(context, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true)
            .build()
        nm.notify(NOTIF_ID, notif)
    }
}
```

- [ ] **Step 2: Call it once from Rust at startup (spike)**

Add `jni` to the Android-only deps in the root `Cargo.toml`:
```toml
[target.'cfg(target_os = "android")'.dependencies]
slint = { version = "1.15", features = ["backend-android-activity-06"] }
jni = "0.21"
```
In `src/lib.rs` `android_main`, after `slint::android::init(app)`, obtain the `JavaVM` + activity `Context` from the `AndroidApp` and call `NotificationHelper.showNow(...)`. Sketch (verify exact `android-activity` 0.6 accessors on-device):

```rust
// SAFETY: documented platform-bridge exception (per design doc).
// 1. let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
// 2. let mut env = vm.attach_current_thread()?;
// 3. let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
// 4. let title = env.new_string("SG Bus Ready")?;
//    let text  = env.new_string("Notification bridge OK")?;
// 5. env.call_static_method(
//        "com/serpoul/sgbusready/NotificationHelper", "showNow",
//        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
//        &[(&activity).into(), (&title).into(), (&text).into()])?;
```
> The exact `AndroidApp` pointer accessors are the part that needs on-device iteration; the on-device notification appearing IS the test. Keep this behind a small `fn post_test_notification(app)` that logs (via `android_logger`/`log`) and swallows errors so a JNI misstep can't crash the UI.

- [ ] **Step 3: Build, install, verify**

```bash
source android/.env.build
cargo ndk -t arm64-v8a -P 35 -o android/app/src/main/jniLibs build
cd android && ./gradlew assembleDebug && adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell monkey -p com.serpoul.sgbusready -c android.intent.category.LAUNCHER 1
```
Grant POST_NOTIFICATIONS if prompted (Android 13+: `adb shell pm grant com.serpoul.sgbusready android.permission.POST_NOTIFICATIONS`).
**Gate:** an ongoing notification "SG Bus Ready / Notification bridge OK" appears.

- [ ] **Step 4: Commit** the Kotlin helper + Rust JNI bridge + Cargo.toml/manifest changes.

---

## Phase C — Foreground service + AlarmManager + periodic Live Update

The heart of the feature. The foreground service refreshes the ongoing notification every ~15s while a window is open; AlarmManager wakes the app only at window boundaries; Rust makes every decision.

### Task C1: Rust JNI entry points (the decision surface)

**Files:** new `src/android_bridge.rs` (compiled only on Android), `pub mod android_bridge;` guarded by `#[cfg(target_os="android")]` in `src/lib.rs`. AccountKey injected at build time via `env!("LTA_SDK_ACCOUNT_KEY")` (never committed).

These are `#[no_mangle] extern "C"` JNI functions (named `Java_com_serpoul_sgbusready_<Class>_<method>`). All heavy logic delegates to the already-tested `sgbr-core`.

- [ ] **Step 1: Settings path + render function**

```rust
//! JNI surface for the Android Kotlin glue. Pure decisions live in sgbr-core;
//! this file only marshals JNI <-> Rust and performs the (blocking) fetch.
//! SAFETY: this whole module is the documented platform-bridge unsafe surface.

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jlong, jstring};
use std::path::PathBuf;
use time::OffsetDateTime;

use sgbr_core::commute::display::{format_live_update, format_see_you_soon};
use sgbr_core::commute::schedule::{active_commutes_at, next_alarm_at};
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::service_arrivals;
use sgbr_core::lta::client::fetch_arrivals;

const ACCOUNT_KEY: &str = env!("LTA_SDK_ACCOUNT_KEY");

fn store_path(files_dir: &str) -> PathBuf {
    let mut p = PathBuf::from(files_dir);
    p.push("commutes.json");
    p
}

/// Render the current Live Update body for all active commutes, one per line.
/// Empty string => nothing active => caller should stop the service.
fn render_active(files_dir: &str, now: OffsetDateTime) -> String {
    let store = CommuteStore::load(&store_path(files_dir)).unwrap_or_default();
    let active = active_commutes_at(&store.commutes, now);
    let mut lines: Vec<String> = Vec::new();
    for c in active {
        match fetch_arrivals(ACCOUNT_KEY, &c.stop) {
            Ok(resp) => {
                let mins = service_arrivals(&resp, now)
                    .into_iter()
                    .find(|s| s.service_no == c.line)
                    .map(|s| s.minutes)
                    .unwrap_or_default();
                lines.push(format_live_update(&c.line, &mins));
            }
            Err(_) => lines.push(format_live_update(&c.line, &[])),
        }
    }
    lines.join("\n")
}
```
> `unwrap_or_default()` / `unwrap_or_default` here are on `Result`/`Option` in a `#[cfg(target_os="android")]` module that is NOT under the workspace strict lints in the same way — but to stay clean, this file lives in the root `sgbusoready` crate (the Slint app), which already scopes Slint-generated lint allows. Add a small `#![allow(clippy::unwrap_used, clippy::expect_used, reason = "android JNI bridge: errors are swallowed to never crash the service")]` at the top of `android_bridge.rs` ONLY, with a reason — never in `sgbr-core`.

- [ ] **Step 2: The JNI exports**

```rust
/// Java: CommuteNative.renderActive(filesDir, epochSecs) -> String
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_serpoul_sgbusready_CommuteNative_renderActive(
    mut env: JNIEnv,
    _class: JClass,
    files_dir: JString,
    epoch_secs: jlong,
) -> jstring {
    let dir: String = env.get_string(&files_dir).map(Into::into).unwrap_or_default();
    let now = OffsetDateTime::from_unix_timestamp(epoch_secs)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(time::macros::offset!(+8)); // SGT
    let body = render_active(&dir, now);
    env.new_string(body).map(|s| s.into_raw()).unwrap_or(JObject::null().into_raw())
}

/// Java: CommuteNative.nextAlarmEpochMillis(filesDir, epochSecs) -> long (-1 = none)
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_serpoul_sgbusready_CommuteNative_nextAlarmEpochMillis(
    mut env: JNIEnv,
    _class: JClass,
    files_dir: JString,
    epoch_secs: jlong,
) -> jlong {
    let dir: String = env.get_string(&files_dir).map(Into::into).unwrap_or_default();
    let now = OffsetDateTime::from_unix_timestamp(epoch_secs)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(time::macros::offset!(+8));
    let store = CommuteStore::load(&store_path(&dir)).unwrap_or_default();
    match next_alarm_at(&store.commutes, now) {
        Some(dt) => dt.unix_timestamp() * 1000,
        None => -1,
    }
}
```
> `now` is passed in from Kotlin as `System.currentTimeMillis()/1000` so the device clock is the single source of truth; Rust converts to SGT (`+8`) to match the commute times the user configured. Confirm the `jni` 0.21 `into_raw`/`JString` API on-device; iterate against compile errors.

- [ ] **Step 3:** `cargo clippy -p sgbr-core --all-targets -- -D warnings` still clean (this file is in the root crate, not `sgbr-core`). Build the `.so` with the key:
```bash
source android/.env.build
LTA_SDK_ACCOUNT_KEY (read from repo-root .env by android/.env.build) cargo ndk -t arm64-v8a -P 35 -o android/app/src/main/jniLibs build
```

### Task C2: Kotlin foreground service, scheduler, receivers

**Files:** `CommuteNative.kt` (external fn declarations), `CommuteService.kt`, `AlarmScheduler.kt`, `AlarmReceiver.kt`, `BootReceiver.kt`. Manifest: add permissions + components.

- [ ] **Step 1: `CommuteNative.kt`** — the JNI declarations + lib load.

```kotlin
package com.serpoul.sgbusready

object CommuteNative {
    init { System.loadLibrary("sgbusoready") }
    external fun renderActive(filesDir: String, epochSecs: Long): String
    external fun nextAlarmEpochMillis(filesDir: String, epochSecs: Long): Long
}
```

- [ ] **Step 2: `CommuteService.kt`** — foreground service with a ~15s refresh loop.

```kotlin
package com.serpoul.sgbusready

import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import androidx.core.app.NotificationCompat

class CommuteService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            if (refresh()) handler.postDelayed(this, 15_000L)
            else stopSelf()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        NotificationHelper.ensureChannel(this)
        startForeground(NotificationHelper.NOTIF_ID, build("SG Bus Ready", "Updating…"))
        handler.post(tick)
        // After (re)starting, re-arm the next boundary alarm.
        AlarmScheduler.arm(this)
        return START_STICKY
    }

    /** @return true if something is still active (keep ticking), false to stop. */
    private fun refresh(): Boolean {
        val now = System.currentTimeMillis() / 1000
        val body = CommuteNative.renderActive(filesDir.absolutePath, now)
        if (body.isEmpty()) return false
        val first = body.substringBefore('\n')
        NotificationHelper.show(this, build("Next buses", body, first))
        return true
    }

    private fun build(title: String, text: String, ticker: String = text) =
        NotificationCompat.Builder(this, NotificationHelper.CHANNEL_ID)
            .setContentTitle(title)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentText(ticker)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setOngoing(true)
            .build()

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        fun start(context: Context) {
            context.startForegroundService(Intent(context, CommuteService::class.java))
        }
    }
}
```
> Add a `NotificationHelper.show(context, notification)` overload (post a prebuilt `Notification` via `notify(NOTIF_ID, n)`) alongside the Phase B `showNow`.

- [ ] **Step 3: `AlarmScheduler.kt`** — set an exact alarm at the next boundary.

```kotlin
package com.serpoul.sgbusready

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

object AlarmScheduler {
    fun arm(context: Context) {
        val now = System.currentTimeMillis() / 1000
        val at = CommuteNative.nextAlarmEpochMillis(context.filesDir.absolutePath, now)
        if (at < 0) return
        val am = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        val pi = PendingIntent.getBroadcast(
            context, 0, Intent(context, AlarmReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
    }
}
```
> On Android 12+ exact alarms may need `SCHEDULE_EXACT_ALARM`/`USE_EXACT_ALARM`. Declare `USE_EXACT_ALARM` (allowed for an alarm-clock-like app) or gracefully fall back to `setAndAllowWhileIdle` if `canScheduleExactAlarms()` is false. Decide at the gate; start with `USE_EXACT_ALARM`.

- [ ] **Step 4: `AlarmReceiver.kt` + `BootReceiver.kt`**

```kotlin
package com.serpoul.sgbusready

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        // A boundary passed: (re)start the service to re-evaluate active commutes.
        // The service re-arms the next alarm and stops itself if nothing is active.
        CommuteService.start(context)
    }
}

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) AlarmScheduler.arm(context)
    }
}
```

- [ ] **Step 5: Manifest** — permissions + components (inside `<application>`):

```xml
<uses-permission android:name="android.permission.INTERNET" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<uses-permission android:name="android.permission.USE_EXACT_ALARM" />
<uses-permission android:name="android.permission.RECEIVE_BOOT_COMPLETED" />
<!-- inside <application>: -->
<service android:name=".CommuteService" android:exported="false"
         android:foregroundServiceType="dataSync" />
<receiver android:name=".AlarmReceiver" android:exported="false" />
<receiver android:name=".BootReceiver" android:exported="true">
    <intent-filter><action android:name="android.intent.action.BOOT_COMPLETED" /></intent-filter>
</receiver>
```

- [ ] **Step 6: Arm on app launch.** In `android_main` (after init), call `AlarmScheduler.arm` via JNI once (so configuring commutes then closing the app still schedules them). Reuse the Phase B JNI call pattern, target `AlarmScheduler.arm(Context)` (static).

### Task C3: On-device verification

- [ ] **Step 1: Seed a commute that starts in ~1 minute.** Until the settings UI (Phase D) exists, push a `commutes.json` directly:

```bash
# Build now+1min / now+6min in SGT HH:MM and your weekday, then:
cat > /tmp/commutes.json <<'JSON'
{ "commutes": [ { "line": "<your-line>", "stop": "<your-stop>",
  "days": <weekday-bitmask>, "start": {"hour": <h>, "minute": <m>},
  "end": {"hour": <h2>, "minute": <m2>}, "label": null } ] }
JSON
adb push /tmp/commutes.json /data/local/tmp/commutes.json
adb shell run-as com.serpoul.sgbusready cp /data/local/tmp/commutes.json files/commutes.json
```
(`days` bitmask: bit0=Mon … bit6=Sun, e.g. Monday = 1.)

- [ ] **Step 2:** Launch the app (arms the alarm), or trigger the receiver directly to start immediately:
```bash
adb shell am broadcast -n com.serpoul.sgbusready/.AlarmReceiver
```
**Gate:** within the window, an ongoing notification shows `Bus <line> · N min · …`, refreshing ~every 15s; at the window `end` it disappears (service stops). Check `adb logcat` for the bridge if not.

- [ ] **Step 3: Commit** Phase C (Rust bridge + Kotlin service/scheduler/receivers + manifest).

> **Do not commit your real `LTA_SDK_ACCOUNT_KEY`.** It's supplied via the build env only.

---

## Phase D — Slint settings UI

Manage commutes in-app; persist with `CommuteStore::save`; re-arm the alarm on save.

### Task D1: Settings model in the Slint app

**Files:** extend `ui/app.slint` (a commutes list + add/edit form); extend `src/lib.rs` to load/save the store from `app.internal_data_path()` and bind to Slint. Each inactive row shows `format_see_you_soon(next_window_start(now))`; active rows show `format_live_update`.

- [ ] **Step 1:** On startup, `let dir = app.internal_data_path()`; `CommuteStore::load(dir/commutes.json)`; map each commute to a Slint row with label + status string (active → live arrivals; inactive → see-you-soon). Reuse `service_arrivals` for active rows (real fetch on a worker thread, off the UI thread).
- [ ] **Step 2:** Add/edit form: line (text), stop (text), weekday toggles (7), start/end (hour+minute). On save: build `Commute::new(...)`, push/replace into the store, `store.save(...)`, then arm the alarm via the `AlarmScheduler.arm` JNI call. Surface `CommuteError` validation messages inline.
- [ ] **Step 3:** Delete + reorder (move up/down) mutate `store.commutes` and re-save.

- [ ] **Step 4: Build, install, verify.**
**Gate:** add a commute in-app → it persists across relaunch (`adb shell run-as com.serpoul.sgbusready cat files/commutes.json`); an inactive commute shows "see you soon · next …"; saving a commute whose window is open shortly triggers the Live Update at the boundary.

- [ ] **Step 5: Commit** the settings UI.

---

## Phase E — (Optional, later) API-36 Live Update chip promotion

Only once a clean `android-36` SDK is installed. Bump `compileSdk = 36`; on the ongoing notification, set the API-36 promoted-ongoing request so it surfaces as the status-bar/lock-screen Live Update chip. Verify the chip appears on the Android 16 device; keep the plain ongoing notification as the pre-36 fallback. Out of scope for the working feature.

---

## Self-Review Notes

- **Spec coverage:** commutes managed in-app (Phase D); ongoing/Live-Update notification during each active window with next-3 arrivals + ~15s refresh (Phase C, using `format_live_update` + `service_arrivals` + `fetch_arrivals`); per-row "see you soon" off-window in-app (Phase D, `format_see_you_soon`); AlarmManager wakes only at boundaries via `next_alarm_at`, BOOT re-arm (Phase C); one notification per active commute rendered as lines (Phase C `render_active`); Android-only, API-36 chip deferred (Phase E) — matches the spec's baseline/promotion split.
- **Honest-about-risk:** the genuinely uncertain spots are the JNI accessor sequence (B2/C1, gated by on-device behavior, errors swallowed so they can't crash), the SDK-platform acceptance by AGP (Phase 0 fallback to a clean SDK), and JDK/Gradle/AGP version cohesion (Phase 0 + wrapper lever). These are flagged with verification gates and fallbacks rather than asserted.
- **Secrets:** `LTA_SDK_ACCOUNT_KEY` is injected via build env (`env!`), never committed; `commutes.json` lives in app-private storage.
- **Lints:** all swallow-error `unwrap_or_default` lives only in the Android `android_bridge.rs` of the root crate with a scoped, reasoned `#![allow]`; `sgbr-core` stays strict and untouched.
- **Type consistency:** Kotlin `CommuteNative.renderActive/nextAlarmEpochMillis(filesDir, epochSecs)` ↔ the Rust `Java_..._renderActive/nextAlarmEpochMillis` exports; `CommuteService`/`AlarmScheduler`/`AlarmReceiver`/`BootReceiver` names match the manifest entries; `NotificationHelper.{CHANNEL_ID,NOTIF_ID,ensureChannel,show,showNow}` consistent across B and C.
