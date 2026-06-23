//! Android JNI bridge: calls into the Kotlin glue.
//!
//! SAFETY: this whole module is the documented platform-bridge unsafe surface
//! (per the design doc). It turns the raw `JavaVM` / Activity pointers that
//! `android-activity` published through `ndk-context` into `jni` handles. Each
//! unsafe block has a safety comment. Callers swallow errors so a JNI misstep
//! can never crash the UI thread.
#![allow(
    unsafe_code,
    reason = "Android JNI requires raw VM/Context pointers; the sole unsafe surface, per the platform-bridge exception in the design doc"
)]

use jni::objects::{JClass, JObject, JValue};
use jni::{AttachGuard, JavaVM};
use jni::errors::Error as JniError;

/// Resolve an app class by name through the Activity's class loader.
///
/// A native thread attached to the JVM (like the one `android_main` runs on)
/// resolves `FindClass` with the *system* class loader, which has no app
/// classes — so `env.find_class("com/serpoul/...")` throws
/// `ClassNotFoundException` even though the class is in the dex. Loading via the
/// Activity's class loader fixes that.
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

/// Post an ongoing notification through `NotificationHelper.showNow`.
fn show_notification(title: &str, text: &str) -> Result<(), JniError> {
    let ctx = ndk_context::android_context();
    log::info!("jni: vm={:?} context={:?}", ctx.vm(), ctx.context());
    // SAFETY: `ndk-context` holds the process JavaVM pointer published by
    // android-activity during init; it is valid for the life of the process.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: `ndk-context` holds the Activity (a Context) jobject for the
    // running NativeActivity; valid while the activity is alive.
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let helper = load_app_class(&mut env, &activity, "com.serpoul.sgbusready.NotificationHelper")?;
    let j_title = env.new_string(title)?;
    let j_text = env.new_string(text)?;
    let call = env.call_static_method(
        &helper,
        "showNow",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(&activity),
            JValue::Object(&j_title),
            JValue::Object(&j_text),
        ],
    );
    if call.is_err() {
        // Dump any pending Java exception (e.g. a Kotlin-side throw) to logcat.
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    call?;
    Ok(())
}

/// Fire a one-shot notification proving the Rust→Kotlin JNI bridge works.
/// Errors are logged (not propagated): a bridge failure must not crash the app.
pub fn post_test_notification() {
    match show_notification("SG Bus Ready", "Notification bridge OK") {
        Ok(()) => log::info!("jni: notification posted"),
        Err(e) => log::error!("jni: notification bridge failed: {e:?}"),
    }
}
