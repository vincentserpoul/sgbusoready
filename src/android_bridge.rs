//! Android JNI bridge.
//!
//! Two directions:
//! - **Kotlin → Rust** (`Java_com_sgbusoready_CommuteNative_*`): the
//!   foreground service / scheduler ask Rust what to show and when to wake.
//!   The `JNIEnv` is supplied and the calling thread is a normal Java thread,
//!   so no class-loader dance is needed.
//! - **Rust → Kotlin** (`arm_alarms`): called from `android_main`'s native
//!   thread, which resolves `FindClass` with the *system* class loader, so app
//!   classes must be loaded via the Activity's class loader (`load_app_class`).
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

use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{jlong, jstring};
use jni::{AttachGuard, JNIEnv, JavaVM};
use jni::errors::Error as JniError;
use time::OffsetDateTime;
use time::macros::offset;

use sgbr_core::commute::display::format_live_update;
use sgbr_core::commute::schedule::{active_commutes_at, next_alarm_at};
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::service_arrivals;
use sgbr_core::lta::client::fetch_arrivals;

/// LTA DataMall AccountKey, injected at build time from `LTA_API_ACCOUNT_KEY`
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

/// Render the Live Update body for every active commute, one per line.
/// Empty string => nothing active (caller stops the service).
fn render_active(files_dir: &str, now: OffsetDateTime) -> String {
    let store = CommuteStore::load(&store_path(files_dir)).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    for c in active_commutes_at(&store.commutes, now) {
        let mins = match fetch_arrivals(ACCOUNT_KEY, &c.stop) {
            Ok(resp) => service_arrivals(&resp, now)
                .into_iter()
                .find(|s| s.service_no == c.line)
                .map(|s| s.minutes)
                .unwrap_or_default(),
            Err(e) => {
                log::warn!("fetch {} @ {} failed: {e}", c.line, c.stop);
                Vec::new()
            }
        };
        lines.push(format_live_update(&c.line, &mins));
    }
    lines.join("\n")
}

/// Resolve an app class by binary name through the Activity's class loader.
/// Needed for Rust→Kotlin calls from native threads (see module docs).
fn load_app_class<'a>(
    env: &mut AttachGuard<'a>,
    activity: &JObject,
    binary_name: &str,
) -> Result<JClass<'a>, JniError> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name = env.new_string(binary_name)?;
    let class_obj = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    Ok(JClass::from(class_obj))
}

/// Re-arm the next boundary alarm via `AlarmScheduler.arm(context)`.
fn arm_alarms_inner() -> Result<(), JniError> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process JavaVM, valid for the process life.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: ndk-context holds the running Activity (a Context) jobject.
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let scheduler = load_app_class(&mut env, &activity, "com.sgbusoready.AlarmScheduler")?;
    let call = env.call_static_method(
        &scheduler,
        "arm",
        "(Landroid/content/Context;)V",
        &[JValue::Object(&activity)],
    );
    if call.is_err() {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    call?;
    Ok(())
}

/// Arm the commute alarms based on the saved store. Errors are logged only.
pub fn arm_alarms() {
    match arm_alarms_inner() {
        Ok(()) => log::info!("jni: alarms armed"),
        Err(e) => log::error!("jni: arm alarms failed: {e:?}"),
    }
}

fn status_bar_top_dp_inner() -> Result<i32, JniError> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process JavaVM, valid for the process life.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: ndk-context holds the running Activity (a Context) jobject.
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let helper = load_app_class(&mut env, &activity, "com.sgbusoready.InsetsHelper")?;
    let value = env.call_static_method(
        &helper,
        "statusBarTopDp",
        "(Landroid/content/Context;)I",
        &[JValue::Object(&activity)],
    )?;
    value.i()
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
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: the stashed NativeActivity jobject is a global ref held for the
    // activity's lifetime; a dialog requires the Activity, not the Application.
    let activity = unsafe { JObject::from_raw(ACTIVITY_PTR.load(Ordering::Relaxed) as jni::sys::jobject) };
    let helper = load_app_class(&mut env, &activity, "com.sgbusoready.TimePicker")?;
    let jtag = env.new_string(tag)?;
    let call = env.call_static_method(
        &helper,
        "show",
        "(Landroid/content/Context;Ljava/lang/String;II)V",
        &[
            JValue::Object(&activity),
            JValue::Object(&jtag),
            JValue::Int(hour),
            JValue::Int(minute),
        ],
    );
    if call.is_err() {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    call?;
    Ok(())
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
pub extern "C" fn Java_com_sgbusoready_CommuteNative_renderActive(
    mut env: JNIEnv,
    _class: JClass,
    files_dir: JString,
    epoch_secs: jlong,
) -> jstring {
    ensure_logger();
    let dir: String = match env.get_string(&files_dir) {
        Ok(s) => s.into(),
        Err(_) => String::new(),
    };
    let body = render_active(&dir, unix_to_sgt(epoch_secs));
    match env.new_string(body) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// `CommuteNative.nextAlarmEpochMillis(filesDir, epochSecs) -> long` (-1 = none)
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_sgbusoready_CommuteNative_nextAlarmEpochMillis(
    mut env: JNIEnv,
    _class: JClass,
    files_dir: JString,
    epoch_secs: jlong,
) -> jlong {
    ensure_logger();
    let dir: String = match env.get_string(&files_dir) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    let store = CommuteStore::load(&store_path(&dir)).unwrap_or_default();
    match next_alarm_at(&store.commutes, unix_to_sgt(epoch_secs)) {
        Some(dt) => dt.unix_timestamp() * 1000,
        None => -1,
    }
}
