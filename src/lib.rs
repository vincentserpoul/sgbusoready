//! SG Bus Ready — shared app entry. The desktop binary and the Android cdylib
//! both call [`run_app`]; only the platform entry points differ.

mod generated {
    #![allow(trivial_numeric_casts, reason = "Slint-generated code")]
    #![allow(
        missing_debug_implementations,
        reason = "Slint-generated types do not derive Debug"
    )]
    #![allow(
        clippy::unwrap_used,
        reason = "Slint-generated code uses unwrap internally"
    )]
    #![allow(
        clippy::expect_used,
        reason = "Slint-generated code uses expect internally"
    )]
    #![allow(clippy::panic, reason = "Slint-generated code uses panic internally")]
    #![allow(
        clippy::indexing_slicing,
        reason = "Slint-generated code uses indexing internally"
    )]
    #![allow(
        clippy::float_arithmetic,
        reason = "Slint-generated code uses float arithmetic internally"
    )]
    #![allow(
        clippy::semicolon_outside_block,
        reason = "Slint-generated code formatting"
    )]
    #![allow(
        clippy::clone_on_ref_ptr,
        reason = "Slint-generated code clones ref-counted pointers"
    )]
    #![allow(clippy::todo, reason = "Slint-generated code contains todo! stubs")]
    #![allow(
        clippy::cognitive_complexity,
        reason = "Slint-generated render code is deeply nested"
    )]
    slint::include_modules!();
}

pub(crate) use generated::{
    AppWindow, ArrivalTag, CommuteRow, EditStop, Screen, StopLane, StopResult,
};

#[cfg(target_os = "android")]
mod android_bridge;
mod catalog;
mod editor;
mod hero;
mod rows;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sgbr_core::bus_catalog::store as catalog_store;
use sgbr_core::commute::model::{Commute, CommuteStop};
use sgbr_core::commute::store::CommuteStore;
use slint::{ComponentHandle, Model, SharedString, Timer, TimerMode};
use time::OffsetDateTime;
use time::macros::offset;

use catalog::{Catalog, catalog_path, refresh_search, spawn_refresh_if_stale, with_catalog};
use editor::{
    EditStopState, FormStops, populate_form, push_form_stops, read_weekdays, stop_services,
    time_of_day,
};
use hero::update_hero;
use rows::{rebuild_rows, spawn_arrivals};

type Store = Rc<RefCell<CommuteStore>>;

/// LTA `DataMall` `AccountKey`, injected at build time (empty if unset → no refresh).
pub(crate) const ACCOUNT_KEY: &str = match option_env!("LTA_API_ACCOUNT_KEY") {
    Some(k) => k,
    None => "",
};

/// Wall-clock "now" in Singapore time (UTC+8, no DST).
pub(crate) fn now_sgt() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(offset!(+8))
}

/// Stash of the editor window so the native time-picker / back JNI callbacks can
/// reach it (set once in `run_app`).
#[cfg(target_os = "android")]
static EDITOR_WINDOW: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);

/// Whether the list screen is showing (kept in sync from Slint), so the native
/// Back handler knows whether to navigate to the list or let the app finish.
static ON_LIST: AtomicBool = AtomicBool::new(true);

/// Open the native time picker for `tag` ("start"/"end"); no-op off Android.
#[allow(
    clippy::missing_const_for_fn,
    reason = "non-const on Android, where it calls into the JNI bridge"
)]
fn pick_time(tag: &str, hour: i32, minute: i32) {
    #[cfg(target_os = "android")]
    android_bridge::show_time_picker(tag, hour, minute);
    #[cfg(not(target_os = "android"))]
    let _ = (tag, hour, minute);
}

/// A small dp value as a Slint logical length (logical px ≈ dp on Android).
#[cfg(target_os = "android")]
#[allow(clippy::cast_precision_loss, reason = "small status-bar dp value")]
const fn dp_to_length(dp: i32) -> f32 {
    dp as f32
}

/// Deliver a native time-picker result (`tag` = "start"/"end") to the editor.
#[cfg(target_os = "android")]
#[allow(
    unsafe_code,
    reason = "Android JNI export; the sole unsafe surface per the platform-bridge exception"
)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_sgbuscommute_CommuteNative_onTimePicked<'local>(
    mut env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
    tag: jni::objects::JString<'local>,
    hour: jni::sys::jint,
    minute: jni::sys::jint,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let tag: String = tag
            .mutf8_chars(env)
            .map(|s| s.to_string())
            .unwrap_or_default();
        let _ = slint::invoke_from_event_loop(move || {
            if let Ok(guard) = EDITOR_WINDOW.lock()
                && let Some(w) = guard.as_ref().and_then(slint::Weak::upgrade)
            {
                if tag == "start" {
                    w.set_start_hour(hour);
                    w.set_start_minute(minute);
                } else {
                    w.set_end_hour(hour);
                    w.set_end_minute(minute);
                }
            }
        });
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

/// Handle system Back: if on a sub-screen, navigate to the list and return true
/// (consumed); on the list, return false so the app finishes.
#[cfg(target_os = "android")]
#[allow(
    unsafe_code,
    reason = "Android JNI export; the sole unsafe surface per the platform-bridge exception"
)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_sgbuscommute_CommuteNative_onBackPressed(
    _env: jni::EnvUnowned<'_>,
    _class: jni::objects::JClass<'_>,
) -> jni::sys::jboolean {
    if ON_LIST.load(Ordering::Relaxed) {
        return false;
    }
    let _ = slint::invoke_from_event_loop(|| {
        if let Ok(guard) = EDITOR_WINDOW.lock()
            && let Some(w) = guard.as_ref().and_then(slint::Weak::upgrade)
        {
            w.set_screen(Screen::List);
        }
    });
    true
}

fn persist(store: &CommuteStore, path: &Path) {
    #[cfg(target_os = "android")]
    if store.save(path).is_err() {
        log::warn!("commute store save failed");
    }
    #[cfg(not(target_os = "android"))]
    let _ = store.save(path);
    rearm_alarms();
}

#[allow(
    clippy::missing_const_for_fn,
    reason = "non-const on Android, where it calls into the JNI bridge"
)]
fn rearm_alarms() {
    #[cfg(target_os = "android")]
    android_bridge::arm_alarms();
}

/// If a commute is active right now, start the foreground service immediately so
/// its Live Update appears — the boundary alarm only fires at *future* window
/// starts, so an already-active commute would otherwise show no notification.
#[allow(
    clippy::missing_const_for_fn,
    reason = "non-const on Android, where it calls into the JNI bridge"
)]
fn maybe_start_service(store: &CommuteStore) {
    #[cfg(target_os = "android")]
    {
        let now = now_sgt();
        if !sgbr_core::commute::schedule::active_commutes_at(&store.commutes, now).is_empty() {
            android_bridge::start_commute_service();
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = store;
}

fn handle_save(window: &AppWindow, store: &Store, path: &Path, form_stops: &FormStops) {
    let label = window.get_form_label().to_string();
    let label = if label.trim().is_empty() {
        None
    } else {
        Some(label)
    };
    let days = read_weekdays(window);
    let start = time_of_day(window.get_start_hour(), window.get_start_minute());
    let end = time_of_day(window.get_end_hour(), window.get_end_minute());

    let stops: Vec<CommuteStop> = form_stops
        .borrow()
        .iter()
        .map(|s| CommuteStop {
            code: s.code.clone(),
            name: s.name.clone(),
            buses: s
                .services
                .iter()
                .zip(s.selected.iter())
                .filter(|(_, on)| **on)
                .map(|(svc, _)| svc.clone())
                .collect(),
        })
        .collect();

    let commute = match Commute::new(label, days, start, end, stops) {
        Ok(c) => c,
        Err(e) => {
            window.set_error_text(SharedString::from(e.to_string()));
            return;
        }
    };

    let mut s = store.borrow_mut();
    let target = usize::try_from(window.get_editing_index())
        .ok()
        .filter(|i| *i < s.commutes.len());
    if let Some(i) = target {
        if let Some(slot) = s.commutes.get_mut(i) {
            *slot = commute;
        }
    } else {
        s.commutes.push(commute);
    }
    persist(&s, path);
    maybe_start_service(&s);
    drop(s);

    populate_form(window, None, -1, None, form_stops);
    rebuild_rows(window, &store.borrow());
    update_hero(window, &store.borrow());
    spawn_arrivals(window, &store.borrow());
    window.set_screen(Screen::List);
}

/// Build the window, load the cached catalog, wire callbacks, run the loop.
///
/// # Errors
/// Returns any [`slint::PlatformError`] from window creation or the event loop.
pub fn run_app(store_path: PathBuf) -> Result<(), slint::PlatformError> {
    let store_path = Rc::new(store_path);
    let store: Store = Rc::new(RefCell::new(
        CommuteStore::load(&store_path).unwrap_or_default(),
    ));
    let catalog: Catalog = Arc::new(Mutex::new(
        catalog_store::load(&catalog_path(&store_path)).ok(),
    ));

    let window = AppWindow::new()?;
    let form_stops: FormStops = Rc::new(RefCell::new(Vec::new()));
    rebuild_rows(&window, &store.borrow());
    update_hero(&window, &store.borrow());
    with_catalog(&catalog, |cat| {
        populate_form(&window, None, -1, cat, &form_stops);
    });
    spawn_arrivals(&window, &store.borrow());
    maybe_start_service(&store.borrow());
    window.set_catalog_loading(with_catalog(&catalog, |c| c.is_none()));
    spawn_refresh_if_stale(&catalog, &window, &store_path);

    #[cfg(target_os = "android")]
    {
        window.set_top_inset(dp_to_length(android_bridge::status_bar_top_dp()));
        if let Ok(mut g) = EDITOR_WINDOW.lock() {
            *g = Some(window.as_weak());
        }
    }

    let w = window.as_weak();
    window.on_pick_time(move |tag| {
        if let Some(w) = w.upgrade() {
            let (h, m) = if tag == "start" {
                (w.get_start_hour(), w.get_start_minute())
            } else {
                (w.get_end_hour(), w.get_end_minute())
            };
            pick_time(&tag, h, m);
        }
    });

    window.on_screen_changed(|is_list| ON_LIST.store(is_list, Ordering::Relaxed));

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let p = Rc::clone(&store_path);
    let fs = Rc::clone(&form_stops);
    window.on_save(move || {
        if let Some(w) = w.upgrade() {
            handle_save(&w, &s, &p, &fs);
        }
    });

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let p = Rc::clone(&store_path);
    window.on_delete(move |index| {
        if let Some(w) = w.upgrade()
            && let Ok(i) = usize::try_from(index)
        {
            let mut store = s.borrow_mut();
            if i < store.commutes.len() {
                store.commutes.remove(i);
                persist(&store, &p);
            }
            drop(store);
            rebuild_rows(&w, &s.borrow());
            update_hero(&w, &s.borrow());
            spawn_arrivals(&w, &s.borrow());
        }
    });

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Arc::clone(&catalog);
    let fs = Rc::clone(&form_stops);
    window.on_edit(move |index| {
        if let Some(w) = w.upgrade()
            && let Ok(i) = usize::try_from(index)
        {
            let commute = s.borrow().commutes.get(i).cloned();
            with_catalog(&c, |cat| {
                populate_form(&w, commute.as_ref(), index, cat, &fs);
            });
        }
    });

    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    let fs = Rc::clone(&form_stops);
    window.on_new_commute(move || {
        if let Some(w) = w.upgrade() {
            with_catalog(&c, |cat| {
                populate_form(&w, None, -1, cat, &fs);
            });
        }
    });

    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    window.on_search_changed(move || {
        if let Some(w) = w.upgrade() {
            refresh_search(&w, &c);
        }
    });

    // Picking a stop appends it (with its services) to the editor's stop list.
    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    let fs = Rc::clone(&form_stops);
    window.on_stop_picked(move |code| {
        if let Some(w) = w.upgrade() {
            let (name, services) = with_catalog(&c, |cat| {
                let name = cat
                    .and_then(|k| k.stop(&code))
                    .map_or_else(String::new, |s| s.name.clone());
                (name, stop_services(cat, &code))
            });
            let selected = vec![false; services.len()];
            fs.borrow_mut().push(EditStopState {
                code: code.to_string(),
                name,
                services,
                selected,
            });
            push_form_stops(&w, &fs.borrow());
            w.set_screen(Screen::Editor);
        }
    });

    // Toggle a bus chip on stop `si`, service index `bi`.
    let w = window.as_weak();
    let fs = Rc::clone(&form_stops);
    window.on_toggle_bus(move |si, bi| {
        if let Some(w) = w.upgrade()
            && let (Ok(si), Ok(bi)) = (usize::try_from(si), usize::try_from(bi))
        {
            // Flip the working-state bool and capture the new value.
            let toggled = fs
                .borrow_mut()
                .get_mut(si)
                .and_then(|stop| stop.selected.get_mut(bi))
                .map(|sel| {
                    *sel = !*sel;
                    *sel
                });
            // Update only the affected chip in the live model — rebuilding the
            // whole `form-stops` model would recreate each StopEditorCard's
            // Flickable and snap its chip row back to the start.
            if let Some(value) = toggled
                && let Some(stop) = w.get_form_stops().row_data(si)
            {
                stop.selected.set_row_data(bi, value);
            }
        }
    });

    let w = window.as_weak();
    let fs = Rc::clone(&form_stops);
    window.on_remove_stop(move |si| {
        if let Some(w) = w.upgrade()
            && let Ok(si) = usize::try_from(si)
        {
            let mut stops = fs.borrow_mut();
            if si < stops.len() {
                stops.remove(si);
            }
            drop(stops);
            push_form_stops(&w, &fs.borrow());
        }
    });

    // Refresh live arrivals on the list every 15s while a commute is active.
    let arrivals_timer = Timer::default();
    let w = window.as_weak();
    let s = Rc::clone(&store);
    arrivals_timer.start(TimerMode::Repeated, Duration::from_secs(15), move || {
        if ON_LIST.load(Ordering::Relaxed)
            && let Some(w) = w.upgrade()
        {
            update_hero(&w, &s.borrow());
            spawn_arrivals(&w, &s.borrow());
        }
    });

    window.run()
}

/// Android entry point.
#[cfg(target_os = "android")]
#[allow(
    unsafe_code,
    reason = "Android requires a #[no_mangle] entry; this is the sole unsafe surface, per the platform-bridge exception in the design doc"
)]
#[unsafe(no_mangle)]
extern "Rust" fn android_main(app: slint::android::AndroidApp) {
    android_bridge::ensure_logger();
    android_bridge::set_activity_ptr(app.activity_as_ptr());
    let store_path = app.internal_data_path().map_or_else(
        || PathBuf::from("commutes.json"),
        |p| p.join("commutes.json"),
    );
    if slint::android::init(app).is_err() {
        return;
    }
    android_bridge::arm_alarms();
    let _ = run_app(store_path);
}
