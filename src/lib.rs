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

use generated::{AppWindow, CommuteRow, Screen, StopResult};

#[cfg(target_os = "android")]
mod android_bridge;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use sgbr_core::bus_catalog::fetch::fetch_catalog;
use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::bus_catalog::search::search as catalog_search;
use sgbr_core::bus_catalog::store as catalog_store;
use sgbr_core::commute::display::format_see_you_soon;
use sgbr_core::commute::model::{Commute, TimeOfDay, Weekdays};
use sgbr_core::commute::store::CommuteStore;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use time::macros::offset;
use time::{OffsetDateTime, Weekday};

/// Catalog is shared with a background refresh thread, so it's `Arc<Mutex>`.
type Catalog = Arc<Mutex<Option<BusCatalog>>>;
type Store = Rc<RefCell<CommuteStore>>;

/// LTA `DataMall` `AccountKey`, injected at build time (empty if unset → no refresh).
const ACCOUNT_KEY: &str = match option_env!("LTA_API_ACCOUNT_KEY") {
    Some(k) => k,
    None => "",
};

/// Wall-clock "now" in Singapore time (UTC+8, no DST).
fn now_sgt() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(offset!(+8))
}

/// Stash of the editor window so the native time-picker JNI callback can reach
/// it (set once in `run_app`).
#[cfg(target_os = "android")]
static EDITOR_WINDOW: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);

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
fn dp_to_length(dp: i32) -> f32 {
    dp as f32
}

/// Deliver a native time-picker result (`tag` = "start"/"end") to the editor.
#[cfg(target_os = "android")]
#[allow(
    unsafe_code,
    reason = "Android JNI export; the sole unsafe surface per the platform-bridge exception"
)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_serpoul_sgbusready_CommuteNative_onTimePicked(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    tag: jni::objects::JString,
    hour: jni::sys::jint,
    minute: jni::sys::jint,
) {
    let tag: String = env.get_string(&tag).map(Into::into).unwrap_or_default();
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
}

/// Run `f` with a borrow of the catalog, tolerating lock poisoning.
fn with_catalog<R>(catalog: &Catalog, f: impl FnOnce(Option<&BusCatalog>) -> R) -> R {
    match catalog.lock() {
        Ok(guard) => f(guard.as_ref()),
        Err(poisoned) => f(poisoned.into_inner().as_ref()),
    }
}

/// Re-run the current stop search and refresh the loading flag (used both on
/// keystroke and after a background catalog refresh lands).
fn refresh_search(window: &AppWindow, catalog: &Catalog) {
    let query = window.get_search_query().to_string();
    let results: Vec<StopResult> = with_catalog(catalog, |cat| {
        cat.map(|k| {
            catalog_search(k, &query, 30)
                .into_iter()
                .map(|s| StopResult {
                    code: SharedString::from(s.code.as_str()),
                    name: SharedString::from(s.name.as_str()),
                    road: SharedString::from(s.road.as_str()),
                })
                .collect()
        })
        .unwrap_or_default()
    });
    window.set_search_results(ModelRc::new(VecModel::from(results)));
    window.set_catalog_loading(with_catalog(catalog, |c| c.is_none()));
}

/// If the catalog is missing or stale (and a key is compiled in), fetch a fresh
/// one on a background thread, persist it, swap it in, and refresh the UI.
fn spawn_refresh_if_stale(catalog: &Catalog, window: &AppWindow, store_path: &Path) {
    if ACCOUNT_KEY.is_empty() {
        return;
    }
    let now = now_sgt();
    let needs = with_catalog(catalog, |c| c.is_none_or(|k| k.is_stale(now)));
    if !needs {
        return;
    }
    let catalog = Arc::clone(catalog);
    let weak = window.as_weak();
    let cat_path = catalog_path(store_path);
    let commutes_path = store_path.to_path_buf();
    std::thread::spawn(move || {
        let Ok(fresh) = fetch_catalog(ACCOUNT_KEY, OffsetDateTime::now_utc()) else {
            return;
        };
        let _ = catalog_store::save(&fresh, &cat_path);
        if let Ok(mut guard) = catalog.lock() {
            *guard = Some(fresh);
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                // The store is persisted on every change, so disk == memory; load
                // it to relabel rows with stop names now the catalog is available.
                let store = CommuteStore::load(&commutes_path).unwrap_or_default();
                with_catalog(&catalog, |cat| rebuild_rows(&w, &store, cat));
                refresh_search(&w, &catalog);
            }
        });
    });
}

fn catalog_path(store_path: &Path) -> PathBuf {
    store_path.with_file_name("bus_catalog.json")
}

/// Resolve a commute's stop code to a display name via the catalog (falls back
/// to the bare code) and build the card label.
fn card_label(catalog: Option<&BusCatalog>, commute: &Commute) -> String {
    let name = catalog
        .and_then(|c| c.stop(&commute.stop))
        .map_or_else(|| commute.stop.clone(), |s| s.name.clone());
    format!("Bus {} · {name}", commute.line)
}

fn card_status(commute: &Commute, now: OffsetDateTime) -> String {
    if commute.is_active_at(now) {
        format!("active · until {:02}:{:02}", commute.end.hour, commute.end.minute)
    } else {
        commute
            .next_window_start(now)
            .map(format_see_you_soon)
            .unwrap_or_default()
    }
}

fn rebuild_rows(window: &AppWindow, store: &CommuteStore, catalog: Option<&BusCatalog>) {
    let now = now_sgt();
    let mut rows: Vec<CommuteRow> = Vec::new();
    for (i, c) in store.commutes.iter().enumerate() {
        rows.push(CommuteRow {
            label: SharedString::from(card_label(catalog, c)),
            status: SharedString::from(card_status(c, now)),
            active: c.is_active_at(now),
            index: i32::try_from(i).unwrap_or(-1),
        });
    }
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}

fn read_weekdays(window: &AppWindow) -> Weekdays {
    let mut days: Vec<Weekday> = Vec::new();
    if window.get_day_mon() {
        days.push(Weekday::Monday);
    }
    if window.get_day_tue() {
        days.push(Weekday::Tuesday);
    }
    if window.get_day_wed() {
        days.push(Weekday::Wednesday);
    }
    if window.get_day_thu() {
        days.push(Weekday::Thursday);
    }
    if window.get_day_fri() {
        days.push(Weekday::Friday);
    }
    if window.get_day_sat() {
        days.push(Weekday::Saturday);
    }
    if window.get_day_sun() {
        days.push(Weekday::Sunday);
    }
    Weekdays::from_days(&days)
}

fn time_of_day(hour: i32, minute: i32) -> TimeOfDay {
    TimeOfDay {
        hour: u8::try_from(hour).unwrap_or(0),
        minute: u8::try_from(minute).unwrap_or(0),
    }
}

fn set_days(window: &AppWindow, days: Weekdays) {
    window.set_day_mon(days.contains(Weekday::Monday));
    window.set_day_tue(days.contains(Weekday::Tuesday));
    window.set_day_wed(days.contains(Weekday::Wednesday));
    window.set_day_thu(days.contains(Weekday::Thursday));
    window.set_day_fri(days.contains(Weekday::Friday));
    window.set_day_sat(days.contains(Weekday::Saturday));
    window.set_day_sun(days.contains(Weekday::Sunday));
}

fn services_model(catalog: Option<&BusCatalog>, code: &str) -> ModelRc<SharedString> {
    let services: Vec<SharedString> = catalog
        .map(|c| c.services(code).iter().map(SharedString::from).collect())
        .unwrap_or_default();
    ModelRc::new(VecModel::from(services))
}

/// Populate the editor form from an existing commute (edit) or reset it (new).
fn populate_form(window: &AppWindow, commute: Option<&Commute>, index: i32, catalog: Option<&BusCatalog>) {
    if let Some(c) = commute {
        window.set_form_line(SharedString::from(c.line.as_str()));
        window.set_form_stop_code(SharedString::from(c.stop.as_str()));
        let name = catalog
            .and_then(|k| k.stop(&c.stop))
            .map_or_else(String::new, |s| s.name.clone());
        window.set_form_stop_name(SharedString::from(name));
        window.set_stop_services(services_model(catalog, &c.stop));
        set_days(window, c.days);
        window.set_start_hour(i32::from(c.start.hour));
        window.set_start_minute(i32::from(c.start.minute));
        window.set_end_hour(i32::from(c.end.hour));
        window.set_end_minute(i32::from(c.end.minute));
    } else {
        window.set_form_line(SharedString::new());
        window.set_form_stop_code(SharedString::new());
        window.set_form_stop_name(SharedString::new());
        window.set_stop_services(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
        set_days(window, Weekdays(0));
        window.set_start_hour(8);
        window.set_start_minute(0);
        window.set_end_hour(9);
        window.set_end_minute(0);
    }
    window.set_editing_index(index);
    window.set_error_text(SharedString::new());
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

fn handle_save(window: &AppWindow, store: &Store, catalog: &Catalog, path: &Path) {
    let line = window.get_form_line().to_string();
    let stop = window.get_form_stop_code().to_string();
    let days = read_weekdays(window);
    let start = time_of_day(window.get_start_hour(), window.get_start_minute());
    let end = time_of_day(window.get_end_hour(), window.get_end_minute());

    let commute = match Commute::new(&line, &stop, days, start, end, None) {
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
    drop(s);

    with_catalog(catalog, |cat| {
        populate_form(window, None, -1, cat);
        rebuild_rows(window, &store.borrow(), cat);
    });
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
    with_catalog(&catalog, |cat| {
        rebuild_rows(&window, &store.borrow(), cat);
        populate_form(&window, None, -1, cat);
    });
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

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Arc::clone(&catalog);
    let p = Rc::clone(&store_path);
    window.on_save(move || {
        if let Some(w) = w.upgrade() {
            handle_save(&w, &s, &c, &p);
        }
    });

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Arc::clone(&catalog);
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
            with_catalog(&c, |cat| rebuild_rows(&w, &s.borrow(), cat));
        }
    });

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Arc::clone(&catalog);
    window.on_edit(move |index| {
        if let Some(w) = w.upgrade()
            && let Ok(i) = usize::try_from(index)
        {
            let commute = s.borrow().commutes.get(i).cloned();
            with_catalog(&c, |cat| populate_form(&w, commute.as_ref(), index, cat));
        }
    });

    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    window.on_new_commute(move || {
        if let Some(w) = w.upgrade() {
            with_catalog(&c, |cat| populate_form(&w, None, -1, cat));
        }
    });

    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    window.on_search_changed(move || {
        if let Some(w) = w.upgrade() {
            refresh_search(&w, &c);
        }
    });

    let w = window.as_weak();
    let c = Arc::clone(&catalog);
    window.on_stop_picked(move |code| {
        if let Some(w) = w.upgrade() {
            let name = with_catalog(&c, |cat| {
                cat.and_then(|k| k.stop(&code))
                    .map_or_else(String::new, |s| s.name.clone())
            });
            w.set_form_stop_code(code.clone());
            w.set_form_stop_name(SharedString::from(name));
            w.set_stop_services(with_catalog(&c, |cat| services_model(cat, &code)));
            w.set_form_line(SharedString::new());
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
fn android_main(app: slint::android::AndroidApp) {
    android_bridge::ensure_logger();
    android_bridge::set_activity_ptr(app.activity_as_ptr());
    let store_path = app
        .internal_data_path()
        .map(|p| p.join("commutes.json"))
        .unwrap_or_else(|| PathBuf::from("commutes.json"));
    if slint::android::init(app).is_err() {
        return;
    }
    android_bridge::arm_alarms();
    let _ = run_app(store_path);
}
