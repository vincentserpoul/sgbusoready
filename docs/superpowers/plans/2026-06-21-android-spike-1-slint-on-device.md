# Android Spike #1 — Slint UI on a Real Device (installable APK)

> **For agentic workers:** This plan is **interactive** (developer + their phone), not fully autonomous: Tasks 3–4 require an Android NDK and a physical device that only the user has. Execute Tasks 1–2 normally (dev-machine-verifiable); pair with the user for Tasks 0, 3, 4. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Get the existing Slint UI running on the user's Android phone as an installable **debug APK**, retiring the biggest Android risk ("does Slint render on my real device + toolchain?") with the least machinery.

**Architecture:** Use **`cargo-apk`** to produce a pure-Rust NativeActivity APK from the existing app, reusing `sgbr-core` and the Slint UI **unchanged** (fixed in-code sample — no network, no permissions). The app is refactored so the window-building logic lives in `src/lib.rs::run_app()`, called by both the desktop `main()` and an Android `android_main()` entry point.

**Tech Stack:** Rust 2024 (1.96.0), `slint` 1.15 with the `backend-android-activity-06` feature (Android only), `cargo-apk`, Android SDK + NDK, `adb`.

**Explicitly NOT in this plan (→ Android Spike #2, a Gradle + `cargo-ndk` restructure):**
- JNI local-notification bridge (`NotificationManager`/`AlarmManager`)
- Glance / `RemoteViews` home-screen widget + Rust→widget shared data
- Live LTA fetch on device (needs `INTERNET` permission + the AccountKey)
These require Kotlin, which `cargo-apk` cannot host — hence the later Gradle plan.

**Why staged:** `cargo apk run` is one command to build+install+launch, giving the fastest proof that Slint works on the device. The widget/notification goals genuinely need Gradle, so they get their own plan. The small cargo-apk→Gradle rework between spikes is acceptable — a spike's job is to *learn*, not to be the final structure.

---

### Task 0: Prerequisites (developer machine + phone — user-verified)

This task is environment setup; only the user can complete and confirm it on their Arch Linux machine and phone. None of it is committed to the repo.

- [ ] **Step 1: Install the Android toolchain (Arch Linux)**

```bash
# adb + platform tools
sudo pacman -S --needed android-tools
# SDK + NDK: easiest via the AUR or Android Studio. Using sdkmanager:
#   yay -S android-sdk android-sdk-platform-tools android-ndk
# Confirm you have an NDK directory, e.g. /opt/android-ndk or ~/Android/Sdk/ndk/<version>
```

- [ ] **Step 2: Export the env vars cargo-apk needs**

```bash
export ANDROID_HOME="$HOME/Android/Sdk"          # adjust to your SDK path
export ANDROID_NDK_ROOT="$ANDROID_HOME/ndk/<version>"  # adjust to your NDK path
# Persist these in ~/.zshrc once confirmed working.
```
Verify: `echo "$ANDROID_HOME" && ls "$ANDROID_NDK_ROOT" && adb version`
Expected: both paths exist; `adb` prints a version.

- [ ] **Step 3: Install cargo-apk and confirm the Rust Android target**

```bash
cargo install cargo-apk
rustup target list --installed | grep aarch64-linux-android   # already pinned in rust-toolchain.toml
```
Expected: `cargo apk --help` works; the target is listed. (If `cargo-apk` proves stale against current NDKs, the maintained fork `cargo-apk2` is a drop-in fallback — note it and continue.)

- [ ] **Step 4: Connect the phone with USB debugging**

On the phone: enable Developer Options → USB debugging. Then:
```bash
adb devices
```
Expected: your device appears as `device` (authorize the RSA prompt on the phone if asked).
*(If you prefer to sideload the APK file manually instead of `adb install`, you can skip the cable — Task 4 produces an `.apk` you can transfer to the phone and open.)*

---

### Task 1: Refactor the app into a shared `run_app()` library (dev-machine-verifiable)

Move the window-building logic out of `main.rs` into `src/lib.rs` so both the desktop binary and the future Android entry point share it. **Desktop behaviour must be unchanged.**

**Files:**
- Create: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml` (add `[lib]` crate types)

- [ ] **Step 1: Add a library target to the root package**

In root `Cargo.toml`, directly under the `[package]` block, add:

```toml
[lib]
crate-type = ["rlib", "cdylib"]
```

(`rlib` lets `main.rs` call the library; `cdylib` is what `cargo-apk` packages for Android.)

- [ ] **Step 2: Create `src/lib.rs` with the shared UI logic**

Move the entire contents currently in `src/main.rs` (the `mod generated`, `use`s, `SAMPLE`, `timing_label`, and the body of `main`) into `src/lib.rs`, exposed as `run_app()`:

```rust
//! SG Bus Ready — shared app entry. The desktop binary and the Android
//! cdylib both call [`run_app`]; only the platform entry points differ.

mod generated {
    #![allow(trivial_numeric_casts, reason = "Slint-generated code")]
    #![allow(missing_debug_implementations, reason = "Slint-generated types do not derive Debug")]
    #![allow(clippy::unwrap_used, reason = "Slint-generated code uses unwrap internally")]
    #![allow(clippy::expect_used, reason = "Slint-generated code uses expect internally")]
    #![allow(clippy::panic, reason = "Slint-generated code uses panic internally")]
    #![allow(clippy::indexing_slicing, reason = "Slint-generated code uses indexing internally")]
    #![allow(clippy::float_arithmetic, reason = "Slint-generated code uses float arithmetic internally")]
    #![allow(clippy::semicolon_outside_block, reason = "Slint-generated code formatting")]
    #![allow(clippy::clone_on_ref_ptr, reason = "Slint-generated code clones ref-counted pointers")]
    #![allow(clippy::todo, reason = "Slint-generated code may contain a todo! stub")]
    slint::include_modules!();
}

use generated::{AppWindow, ServiceRow};
use sgbr_core::lta::arrival::{ServiceArrivals, service_arrivals};
use sgbr_core::lta::model::BusArrivalResponse;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

const SAMPLE: &str = r#"{
  "BusStopCode": "83139",
  "Services": [
    { "ServiceNo": "15",
      "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
      "NextBus2": { "EstimatedArrival": "2026-06-21T08:25:00+08:00" },
      "NextBus3": { "EstimatedArrival": "" } }
  ]
}"#;

fn timing_label(arrivals: &ServiceArrivals) -> String {
    if arrivals.minutes.is_empty() {
        return "no service".to_owned();
    }
    arrivals
        .minutes
        .iter()
        .map(|m| {
            if *m <= 0 {
                "Arr".to_owned()
            } else {
                format!("{m} min")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build the window from the fixed sample and run the Slint event loop.
///
/// # Errors
/// Returns any [`slint::PlatformError`] from window creation or the event loop.
pub fn run_app() -> Result<(), slint::PlatformError> {
    let now = time::macros::datetime!(2026-06-21 08:10:00 +8);
    let response: BusArrivalResponse = serde_json::from_str(SAMPLE).unwrap_or(BusArrivalResponse {
        bus_stop_code: String::new(),
        services: Vec::new(),
    });

    let rows: Vec<ServiceRow> = service_arrivals(&response, now)
        .iter()
        .map(|a| ServiceRow {
            service_no: SharedString::from(a.service_no.as_str()),
            timing: SharedString::from(timing_label(a).as_str()),
        })
        .collect();

    let window = AppWindow::new()?;
    window.set_rows(ModelRc::new(VecModel::from(rows)));
    window.run()
}
```

- [ ] **Step 3: Slim `src/main.rs` to a desktop entry point**

Replace the entire contents of `src/main.rs` with:

```rust
//! SG Bus Ready — desktop entry point. Delegates to the shared library.

fn main() -> Result<(), slint::PlatformError> {
    sgbusoready::run_app()
}
```

- [ ] **Step 4: Verify the desktop build is unchanged**

Run:
```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```
Expected: builds clean, clippy clean, 12 tests pass, fmt clean. (Optional manual: `cargo run` still opens the same window — Stop 83139, "15 — 8 min, 15 min".)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs
git commit -m "refactor: extract shared run_app() into library for desktop + android"
```

---

### Task 2: Add the Android entry point and cargo-apk config (dev-machine-verifiable for desktop; android compile needs NDK)

The Android-specific code is `#[cfg(target_os = "android")]`-gated, so the desktop build stays green even without an NDK.

**Files:**
- Modify: `src/lib.rs` (append the Android entry point)
- Modify: `Cargo.toml` (android slint feature + `[package.metadata.android]`)

- [ ] **Step 1: Add the `android_main` entry point**

Append to `src/lib.rs`:

```rust
/// Android entry point. `cargo-apk`'s NativeActivity glue calls this; we hand
/// the `AndroidApp` to Slint's backend, then run the shared UI.
#[cfg(target_os = "android")]
#[allow(
    unsafe_code,
    reason = "Android requires a #[no_mangle] entry; this is the sole unsafe surface, per the platform-bridge exception in the design doc"
)]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    if slint::android::init(app).is_err() {
        return;
    }
    let _ = run_app();
}
```

> **Verify against Slint 1.15 docs:** confirm the entry-point name/signature (`android_main(app: slint::android::AndroidApp)`) and `slint::android::init`. If 1.15 names the type `slint::android::android_activity::AndroidApp` or differs, adjust — the Android build in Task 3 is the real check.

- [ ] **Step 2: Add the Android Slint feature (Android target only)**

In root `Cargo.toml`, add a target-specific dependency that turns on the Android backend **only** when building for Android (the existing desktop `slint = "1.15"` line stays untouched):

```toml
[target.'cfg(target_os = "android")'.dependencies]
slint = { version = "1.15", features = ["backend-android-activity-06"] }
```

> If the Android build later complains about conflicting backends (desktop default backend + android), switch the base `slint` dep to `default-features = false` and select `backend-winit`/`renderer-femtovg` for non-android and `backend-android-activity-06`/`renderer-femtovg` for android. Try the simple form first.

- [ ] **Step 3: Add the cargo-apk manifest metadata**

In root `Cargo.toml`, add:

```toml
[package.metadata.android]
package = "com.serpoul.sgbusready"
build_targets = ["aarch64-linux-android"]
min_sdk_version = 24
target_sdk_version = 34

[package.metadata.android.application]
label = "SG Bus Ready"
```

(No `INTERNET` permission yet — this spike uses the fixed sample.)

- [ ] **Step 4: Verify the desktop build is still green**

Run:
```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```
Expected: all clean (the `#[cfg(target_os = "android")]` code is not compiled for the host, so desktop is unaffected).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs
git commit -m "feat(android): add android_main entry and cargo-apk config"
```

---

### Task 3: Build the Android APK (needs NDK — user's machine)

- [ ] **Step 1: Build the debug APK**

```bash
cargo apk build --lib --target aarch64-linux-android
```
Expected: produces an APK under `target/debug/apk/` (path is printed). If it fails on a **missing/incompatible NDK**, fix the NDK path/version from Task 0 and retry. If it fails on a **Slint backend feature/name**, apply the fallbacks noted in Task 2 Steps 1–2.

- [ ] **Step 2: Note the APK path for sideloading**

```bash
ls -la target/debug/apk/*.apk
```
Expected: one `.apk` file. This is the file to transfer to the phone if not using a cable.

---

### Task 4: Install and run on the device (user + phone — user-verified)

- [ ] **Step 1: Build, install, and launch in one step (cabled device)**

```bash
cargo apk run --lib --target aarch64-linux-android
```
Expected: the app installs and launches; the phone shows a window with **"Stop 83139"** and the row **"15 — 8 min, 15 min"**.

*Alternative (manual sideload, no cable):* transfer the `.apk` from Task 3 to the phone and open it (allow "install from unknown sources"), or `adb install target/debug/apk/<name>.apk`.

- [ ] **Step 2: Confirm on the device — LOOK at the screen**

The success criterion is visual: the Slint UI renders on the phone exactly like the desktop window. A blank/black screen or a crash is a failure — capture `adb logcat | grep -i -E 'slint|rust|sgbus'` and we debug from there.

- [ ] **Step 3: Record the outcome**

If it works: Android Spike #1 is GREEN — Slint renders on the real device. Proceed to Android Spike #2 (Gradle + JNI notification + Glance widget). If it fails, the logcat output tells us whether it's NDK, backend feature, or renderer — fix and retry; no code in the repo needs reverting.

---

## Self-Review

**Spec coverage (against the design doc's Android spike goals):**
- "Slint on a real Android device" — Tasks 1–4. ✅
- "One live LTA fetch on device" — **deferred to Spike #2** (needs INTERNET permission + key); noted explicitly, not a silent gap.
- "JNI notification" + "Glance widget" — **deferred to Spike #2** (need Gradle/Kotlin); noted explicitly.
- Easy install for the user — `cargo apk run` (one command) or sideload the produced `.apk`. ✅

**Placeholder scan:** No TBD/TODO. The two version-sensitive unknowns (Slint Android entry signature, backend feature flags) are given concrete best-known values **plus** a fallback and the exact verification gate (the Task 3 Android build). This is honest about environment-dependent specifics rather than a placeholder. ✅

**Type/structure consistency:** `run_app()` defined in `src/lib.rs` (Task 1) is called by both `main.rs` (Task 1 Step 3) and `android_main` (Task 2 Step 1). The `mod generated` + `ServiceRow`/`AppWindow`/`timing_label` move wholesale from `main.rs` to `lib.rs`, preserving the lint-scoping fix from the previous branch. Crate name `sgbusoready` → library path `sgbusoready::run_app` is correct (hyphen→nothing; the package name has no hyphen). ✅

**Execution note:** Tasks 1–2 are dev-machine-verifiable (desktop build/clippy/test/fmt) and suit autonomous or paired execution. Tasks 0, 3, 4 need the NDK + the physical phone and must be run with the user. Recommend executing this plan **interactively** rather than dispatching subagents.
