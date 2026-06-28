//! Android JNI bridge.
//!
//! Two directions:
//! - **Kotlin → Rust** (`Java_com_sgbuscommute_CommuteNative_*`): the
//!   foreground service / scheduler ask Rust what to show and when to wake.
//!   The `JNIEnv` is supplied and the calling thread is a normal Java thread,
//!   so no class-loader dance is needed.
//! - **Rust → Kotlin** (`arm_alarms`): called from `android_main`'s native
//!   thread. With jni 0.22 + android-activity the thread context class loader is
//!   set for us, so app classes resolve via `Env::load_class` (which uses that
//!   loader) — no manual Activity class-loader dance needed.
//!
//! SAFETY: this whole module is the documented platform-bridge unsafe surface
//! (per the design doc); each unsafe block has a safety comment. Errors are
//! swallowed/logged so a JNI misstep can never crash the app.
#![allow(
    unsafe_code,
    reason = "Android JNI requires raw VM/Context pointers; the sole unsafe surface, per the platform-bridge exception in the design doc"
)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use jni::errors::Error as JniError;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jlong;
use jni::{EnvUnowned, JavaVM, jni_sig, jni_str};
use time::OffsetDateTime;
use time::macros::offset;

use sgbr_core::commute::display::format_active_notification;
use sgbr_core::commute::schedule::{active_stop_plans, next_alarm_at};
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::{StopArrivals, stop_arrivals};
use sgbr_core::lta::client::fetch_arrivals;

/// LTA `DataMall` `AccountKey`, injected at build time from `LTA_API_ACCOUNT_KEY`
/// (read from the repo-root `.env` by `android/.env.build`). Empty if unset,
/// in which case fetches fail gracefully and rows show "no buses".
const ACCOUNT_KEY: &str = match option_env!("LTA_API_ACCOUNT_KEY") {
    Some(k) => k,
    None => "",
};

/// Initialise logcat logging (tag `sgbr`). Idempotent — safe to call from any
/// entry point (the activity, the service's JNI calls), so logs work whichever
/// component spawned the process.
pub fn ensure_logger() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("sgbr"),
    );
}

/// Singapore is UTC+8 with no DST; commute times are wall-clock SGT.
fn unix_to_sgt(epoch_secs: jlong) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(epoch_secs)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset!(+8))
}

fn store_path(files_dir: &str) -> PathBuf {
    let mut p = PathBuf::from(files_dir);
    p.push("commutes.json");
    p
}

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

/// Re-arm the next boundary alarm via `AlarmScheduler.arm(context)`.
fn arm_alarms_inner() -> Result<(), JniError> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process JavaVM, valid for the process life.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    vm.attach_current_thread(|env| {
        // SAFETY: ndk-context holds the running Activity (a Context) jobject.
        let activity = unsafe { JObject::from_raw(env, ctx.context().cast()) };
        let scheduler = env.load_class(jni_str!("com.sgbuscommute.AlarmScheduler"))?;
        env.call_static_method(
            &scheduler,
            jni_str!("arm"),
            jni_sig!("(Landroid/content/Context;)V"),
            &[JValue::Object(&activity)],
        )?;
        Ok(())
    })
}

/// Arm the commute alarms based on the saved store. Errors are logged only.
pub fn arm_alarms() {
    match arm_alarms_inner() {
        Ok(()) => log::info!("jni: alarms armed"),
        Err(e) => log::error!("jni: arm alarms failed: {e:?}"),
    }
}

fn start_commute_service_inner() -> Result<(), JniError> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process JavaVM, valid for the process life.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    vm.attach_current_thread(|env| {
        // SAFETY: ndk-context holds a running Context jobject (the Application).
        let context = unsafe { JObject::from_raw(env, ctx.context().cast()) };
        let service = env.load_class(jni_str!("com.sgbuscommute.CommuteService"))?;
        env.call_static_method(
            &service,
            jni_str!("start"),
            jni_sig!("(Landroid/content/Context;)V"),
            &[JValue::Object(&context)],
        )?;
        Ok(())
    })
}

/// Start the foreground service now (used when a commute is already active at
/// save/launch, since the boundary alarm only fires at future window starts).
pub fn start_commute_service() {
    if let Err(e) = start_commute_service_inner() {
        log::error!("jni: start service failed: {e:?}");
    }
}

fn status_bar_top_dp_inner() -> Result<i32, JniError> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process JavaVM, valid for the process life.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    vm.attach_current_thread(|env| {
        // SAFETY: ndk-context holds the running Activity (a Context) jobject.
        let activity = unsafe { JObject::from_raw(env, ctx.context().cast()) };
        let helper = env.load_class(jni_str!("com.sgbuscommute.InsetsHelper"))?;
        let value = env.call_static_method(
            &helper,
            jni_str!("statusBarTopDp"),
            jni_sig!("(Landroid/content/Context;)I"),
            &[JValue::Object(&activity)],
        )?;
        value.i()
    })
}

/// The status-bar height in dp (Slint logical units); 0 on failure.
pub fn status_bar_top_dp() -> i32 {
    status_bar_top_dp_inner().unwrap_or(0)
}

/// The `NativeActivity` jobject (stashed in `android_main`). `ndk-context`'s
/// `context()` is the *Application*, but a dialog needs the *Activity*.
static ACTIVITY_PTR: AtomicUsize = AtomicUsize::new(0);

/// Stash the `NativeActivity` pointer for code that needs the Activity (dialogs).
pub fn set_activity_ptr(ptr: *mut c_void) {
    ACTIVITY_PTR.store(ptr as usize, Ordering::Relaxed);
}

fn show_time_picker_inner(tag: &str, hour: i32, minute: i32) -> Result<(), JniError> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process JavaVM, valid for the process life.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    vm.attach_current_thread(|env| {
        // SAFETY: the stashed NativeActivity jobject is a global ref held for the
        // activity's lifetime; a dialog requires the Activity, not the Application.
        let activity = unsafe {
            JObject::from_raw(
                env,
                ACTIVITY_PTR.load(Ordering::Relaxed) as jni::sys::jobject,
            )
        };
        let helper = env.load_class(jni_str!("com.sgbuscommute.TimePicker"))?;
        let jtag = env.new_string(tag)?;
        env.call_static_method(
            &helper,
            jni_str!("show"),
            jni_sig!("(Landroid/content/Context;Ljava/lang/String;II)V"),
            &[
                JValue::Object(&activity),
                JValue::Object(&jtag),
                JValue::Int(hour),
                JValue::Int(minute),
            ],
        )?;
        Ok(())
    })
}

/// Show the native Android `TimePickerDialog`; the result comes back via the
/// `CommuteNative.onTimePicked` JNI export. `tag` is "start" or "end".
pub fn show_time_picker(tag: &str, hour: i32, minute: i32) {
    if let Err(e) = show_time_picker_inner(tag, hour, minute) {
        log::error!("jni: show time picker failed: {e:?}");
    }
}

// ---------------------------------------------------------------------------
// Kotlin → Rust JNI exports (called by CommuteNative / the foreground service).
// ---------------------------------------------------------------------------

/// `CommuteNative.renderActive(filesDir, epochSecs) -> String`
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_sgbuscommute_CommuteNative_renderActive<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    files_dir: JString<'local>,
    epoch_secs: jlong,
) -> JString<'local> {
    ensure_logger();
    env.with_env(|env| -> jni::errors::Result<JString<'local>> {
        let dir: String = files_dir.mutf8_chars(env)?.to_string();
        let body = render_active(&dir, unix_to_sgt(epoch_secs));
        env.new_string(body)
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}

/// `CommuteNative.nextAlarmEpochMillis(filesDir, epochSecs) -> long` (-1 = none)
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_sgbuscommute_CommuteNative_nextAlarmEpochMillis<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    files_dir: JString<'local>,
    epoch_secs: jlong,
) -> jlong {
    ensure_logger();
    env.with_env(|env| -> jni::errors::Result<jlong> {
        let dir: String = match files_dir.mutf8_chars(env) {
            Ok(s) => s.to_string(),
            Err(_) => return Ok(-1),
        };
        let store = CommuteStore::load(&store_path(&dir)).unwrap_or_default();
        Ok(
            match next_alarm_at(&store.commutes, unix_to_sgt(epoch_secs)) {
                Some(dt) => dt.unix_timestamp() * 1000,
                None => -1,
            },
        )
    })
    .resolve::<jni::errors::LogErrorAndDefault>()
}
