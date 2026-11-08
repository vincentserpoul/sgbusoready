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
mod feed;
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
use sgbr_core::error::CoreError;
use sgbr_core::worker::credentials::WorkerCredentials;
use slint::{ComponentHandle, Model, SharedString, Timer, TimerMode};
use time::OffsetDateTime;
use time::macros::offset;

use catalog::{Catalog, catalog_path, refresh_search, spawn_refresh_if_stale, with_catalog};
use editor::{
    EditStopState, FormStops, populate_form, push_form_stops, read_weekdays, stop_services,
    time_of_day,
};
use feed::feed;
use hero::update_hero;
use rows::refresh_arrivals;

type Store = Rc<RefCell<CommuteStore>>;

/// Worker base URL, injected at build time (empty if unset → no server
/// configured, the invite screen reports it rather than trying to connect).
pub(crate) const WORKER_URL: &str = match option_env!("SGBR_WORKER_URL") {
    Some(u) => u,
    None => "",
};

/// Wall-clock "now" in Singapore time (UTC+8, no DST).
pub(crate) fn now_sgt() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(offset!(+8))
}

/// Where worker device credentials are cached — `worker.json`, a sibling of
/// the commute store (mirrors `catalog::catalog_path`'s idiom).
pub(crate) fn credentials_path(store_path: &Path) -> PathBuf {
    store_path.with_file_name("worker.json")
}

/// Load cached worker credentials, if any (missing/unreadable/corrupt → `None`
/// — the caller falls back to the invite screen rather than erroring).
pub(crate) fn load_credentials(path: &Path) -> Option<WorkerCredentials> {
    sgbr_core::worker::credentials::load(path).ok()
}

/// Stash of the editor window so the native time-picker / back JNI callbacks can
/// reach it (set once in `run_app`).
#[cfg(target_os = "android")]
static EDITOR_WINDOW: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);

/// Whether the list screen is showing (kept in sync from Slint), so the native
/// Back handler knows whether to navigate to the list or let the app finish.
static ON_LIST: AtomicBool = AtomicBool::new(true);

thread_local! {
    /// The "repaint from the store" closure, parked here (rather than
    /// captured directly by the feed listener below) because it closes over
    /// `Store` (`Rc<RefCell<CommuteStore>>`), which is `!Send` — but
    /// `ArrivalsFeed::add_listener` and `slint::invoke_from_event_loop` both
    /// require `Send` closures, since the feed calls listeners from its
    /// background stream thread. Set once in `run_app`; the feed listener
    /// (itself capture-free, so trivially `Send`) hops to the UI thread via
    /// `invoke_from_event_loop` and, from there — safely on the same thread
    /// this was set from — invokes it through the thread-local.
    static ON_ARRIVALS_CHANGED: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
}

/// Run the parked `ON_ARRIVALS_CHANGED` closure, if one has been set. Must
/// only be called on the UI thread.
fn run_arrivals_changed() {
    ON_ARRIVALS_CHANGED.with(|cell| {
        if let Some(f) = cell.borrow_mut().as_mut() {
            f();
        }
    });
}

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

/// Re-render everything the store drives on the list screen: the hero line and
/// the commute rows with their live arrivals.
fn repaint(window: &AppWindow, store: &CommuteStore, path: &Path) {
    update_hero(window, store);
    refresh_arrivals(window, store, path);
}

/// The worker answered the arrivals stream with `401`/`403`: this device was
/// revoked (or the worker lost its store). The cached credentials are dead
/// weight — drop them and send the user back to the invite screen instead of
/// silently showing "no buses" forever.
fn handle_revoked(window: &AppWindow, store_path: &Path) {
    if !window.get_registered() {
        return;
    }
    let _ = std::fs::remove_file(credentials_path(store_path));
    window.set_registered(false);
    window.set_invite_error(SharedString::from(
        "this device is no longer registered — enter a new invite code",
    ));
    window.set_screen(Screen::Invite);
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
    repaint(window, &store.borrow(), path);
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
    let registered = load_credentials(&credentials_path(&store_path)).is_some();
    window.set_registered(registered);
    if !registered {
        window.set_screen(Screen::Invite);
    }
    repaint(&window, &store.borrow(), &store_path);
    with_catalog(&catalog, |cat| {
        populate_form(&window, None, -1, cat, &form_stops);
    });
    maybe_start_service(&store.borrow());
    window.set_catalog_loading(with_catalog(&catalog, |c| c.is_none()));
    spawn_refresh_if_stale(&catalog, &window, &store_path);

    // Park the store-capturing "repaint" closure for the feed listener below
    // (see `ON_ARRIVALS_CHANGED`'s doc comment for why it can't be captured
    // directly by a `Send` listener/event-loop closure).
    let weak = window.as_weak();
    let repaint_store = Rc::clone(&store);
    let repaint_path = Rc::clone(&store_path);
    ON_ARRIVALS_CHANGED.with(|cell| {
        *cell.borrow_mut() = Some(Box::new(move || {
            if let Some(w) = weak.upgrade() {
                repaint(&w, &repaint_store.borrow(), &repaint_path);
                // Checked *after* `refresh_arrivals`, whose `set_plans` is what
                // clears the rejection once a fresh token is on disk.
                if feed().auth_rejected() {
                    handle_revoked(&w, &repaint_path);
                }
            }
        }));
    });
    // Event-driven repaint on top of the 15s timer below: every applied feed
    // snapshot hops back to the UI thread and re-renders from the cache
    // (still no network on this path). Both closures here are capture-free,
    // so they trivially satisfy `Send`.
    feed().add_listener(|| {
        let _ = slint::invoke_from_event_loop(run_arrivals_changed);
    });

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
            repaint(&w, &s.borrow(), &p);
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

    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    let p = Rc::clone(&store_path);
    window.on_register_invite(move |code| {
        let Some(w) = w.upgrade() else {
            return;
        };
        let trimmed = code.trim();
        if trimmed.is_empty() {
            w.set_invite_error(SharedString::from("enter an invite code"));
            return;
        }
        if WORKER_URL.is_empty() {
            w.set_invite_error(SharedString::from("no server configured in this build"));
            return;
        }
        w.set_invite_busy(true);
        w.set_invite_error(SharedString::new());
        let weak = w.as_weak();
        let catalog = Arc::clone(&c);
        let store_path: PathBuf = (*p).clone();
        let code = trimmed.to_owned();
        std::thread::spawn(
            move || match sgbr_core::worker::client::register(WORKER_URL, &code) {
                Ok(creds) => {
                    let saved = creds.save(&credentials_path(&store_path));
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = weak.upgrade() {
                            w.set_invite_busy(false);
                            // The invite is consumed server-side either way;
                            // hiding a failed save would strand the device at
                            // the next launch with no way back in.
                            if let Err(e) = saved {
                                w.set_invite_error(SharedString::from(format!(
                                    "registered, but saving credentials failed: {e}"
                                )));
                                return;
                            }
                            w.set_invite_error(SharedString::new());
                            w.set_registered(true);
                            w.set_screen(Screen::List);
                            spawn_refresh_if_stale(&catalog, &w, &store_path);
                            run_arrivals_changed();
                        }
                    });
                }
                Err(err) => {
                    let message = match err {
                        CoreError::Auth(_) => "invite not accepted — check the code",
                        CoreError::Http(_)
                        | CoreError::Parse(_)
                        | CoreError::NoArrival
                        | CoreError::InvalidTimestamp(_)
                        | CoreError::Io(_)
                        | CoreError::Crypto(_) => "cannot reach the server",
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = weak.upgrade() {
                            w.set_invite_busy(false);
                            w.set_invite_error(SharedString::from(message));
                        }
                    });
                }
            },
        );
    });

    // Every 15s: retry a missing/stale catalog fetch (a no-op once it has
    // landed), and refresh the list's hero + live arrivals while it is showing.
    let arrivals_timer = Timer::default();
    let w = window.as_weak();
    let s = Rc::clone(&store);
    let p = Rc::clone(&store_path);
    let c = Arc::clone(&catalog);
    arrivals_timer.start(TimerMode::Repeated, Duration::from_secs(15), move || {
        let Some(w) = w.upgrade() else {
            return;
        };
        spawn_refresh_if_stale(&c, &w, &p);
        if ON_LIST.load(Ordering::Relaxed) {
            repaint(&w, &s.borrow(), &p);
        }
    });

    window.run()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sgbr_core::worker::credentials::WorkerCredentials;

    use super::{credentials_path, load_credentials};

    #[test]
    fn credentials_path_is_worker_json_beside_the_commute_store() {
        let path = credentials_path(Path::new("/data/app/files/commutes.json"));
        assert_eq!(path, Path::new("/data/app/files/worker.json"));
    }

    #[test]
    fn credentials_save_then_load_round_trips_through_the_helper_path() {
        let dir = std::env::temp_dir().join(format!("sgbr_app_creds_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store_path = dir.join("commutes.json");
        let creds = WorkerCredentials {
            token: "t".repeat(64),
            device_id: "device-1".to_owned(),
            secret_seed_b64: "c2VlZA==".to_owned(),
            pubkey_b64: "cHVia2V5".to_owned(),
        };
        creds
            .save(&credentials_path(&store_path))
            .expect("save credentials");
        let loaded = load_credentials(&credentials_path(&store_path))
            .expect("credentials just saved must load");
        assert_eq!(loaded, creds);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_credentials_missing_file_is_none() {
        let path = std::env::temp_dir().join(format!(
            "sgbr_app_creds_missing_{}/worker.json",
            std::process::id()
        ));
        assert!(load_credentials(&path).is_none());
    }
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
