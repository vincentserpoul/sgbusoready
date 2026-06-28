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

use generated::{AppWindow, ArrivalTag, CommuteRow, EditStop, Screen, StopLane, StopResult};

#[cfg(target_os = "android")]
mod android_bridge;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sgbr_core::bus_catalog::fetch::fetch_catalog;
use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::bus_catalog::search::search as catalog_search;
use sgbr_core::bus_catalog::store as catalog_store;
use sgbr_core::commute::display::format_see_you_soon;
use sgbr_core::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::{StopArrivals, stop_arrivals};
use sgbr_core::lta::client::fetch_arrivals;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use time::macros::offset;
use time::{OffsetDateTime, Weekday};

/// Catalog is shared with a background refresh thread, so it's `Arc<Mutex>`.
type Catalog = Arc<Mutex<Option<BusCatalog>>>;
type Store = Rc<RefCell<CommuteStore>>;

/// One stop being edited: its code/name, the full service list at that stop, and
/// which services are currently selected (parallel to `services`).
#[derive(Clone)]
struct EditStopState {
    code: String,
    name: String,
    services: Vec<String>,
    selected: Vec<bool>,
}

/// The editor's working list of stops, owned by Rust and rebuilt into the Slint
/// `form-stops` model on every mutation.
type FormStops = Rc<RefCell<Vec<EditStopState>>>;

/// Push the Rust editor stop-state into the Slint `form-stops` model.
fn push_form_stops(window: &AppWindow, stops: &[EditStopState]) {
    let model: Vec<EditStop> = stops
        .iter()
        .map(|s| EditStop {
            code: SharedString::from(s.code.as_str()),
            name: SharedString::from(s.name.as_str()),
            services: ModelRc::new(VecModel::from(
                s.services
                    .iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
            )),
            selected: ModelRc::new(VecModel::from(s.selected.clone())),
        })
        .collect();
    window.set_form_stops(ModelRc::new(VecModel::from(model)));
}

/// LTA `DataMall` `AccountKey`, injected at build time (empty if unset → no refresh).
const ACCOUNT_KEY: &str = match option_env!("LTA_API_ACCOUNT_KEY") {
    Some(k) => k,
    None => "",
};

/// Wall-clock "now" in Singapore time (UTC+8, no DST).
fn now_sgt() -> OffsetDateTime {
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
                // Rows label from cached stop names, so they need no relabel; just
                // refresh the stop-search now the catalog is available.
                refresh_search(&w, &catalog);
            }
        });
    });
}

fn catalog_path(store_path: &Path) -> PathBuf {
    store_path.with_file_name("bus_catalog.json")
}

/// Card label: the commute's own label, or its first stop's name (+N).
fn card_label(commute: &Commute) -> String {
    commute.display_label()
}

fn see_you_soon(commute: &Commute, now: OffsetDateTime) -> String {
    commute
        .next_window_start(now)
        .map(format_see_you_soon)
        .unwrap_or_default()
}

/// Off-window summary line, e.g. "see you soon · next Mon 08:00 · 2 stops · 4 buses".
fn inactive_summary(commute: &Commute, now: OffsetDateTime) -> String {
    let stops = commute.stops.len();
    let buses: usize = commute.stops.iter().map(|s| s.buses.len()).sum();
    let see = see_you_soon(commute, now);
    format!("{see} · {stops} stops · {buses} buses")
}

fn empty_lanes() -> ModelRc<StopLane> {
    ModelRc::new(VecModel::from(Vec::<StopLane>::new()))
}

/// Skeleton lanes for an active commute: one lane per stop with its name but no
/// arrival tags yet, so the timeline structure shows immediately while the live
/// fetch (`spawn_arrivals`) is in flight.
fn skeleton_lanes(commute: &Commute) -> ModelRc<StopLane> {
    let stops: Vec<StopArrivals> = commute
        .stops
        .iter()
        .map(|s| StopArrivals {
            code: s.code.clone(),
            name: s.name.clone(),
            items: Vec::new(),
        })
        .collect();
    lanes_model(&stops)
}

/// Build all list rows synchronously (no network): active rows get skeleton lanes
/// until `spawn_arrivals` fills in live tags; inactive rows get the summary line.
fn rebuild_rows(window: &AppWindow, store: &CommuteStore) {
    let now = now_sgt();
    let mut rows: Vec<CommuteRow> = Vec::new();
    for (i, c) in store.commutes.iter().enumerate() {
        let active = c.is_active_at(now);
        rows.push(CommuteRow {
            label: SharedString::from(card_label(c)),
            status: SharedString::from(if active {
                String::new()
            } else {
                inactive_summary(c, now)
            }),
            active,
            index: i32::try_from(i).unwrap_or(-1),
            lanes: if active {
                skeleton_lanes(c)
            } else {
                empty_lanes()
            },
            scale_max: i32::from(c.scale_minutes),
        });
    }
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Plain, `Send` row data computed off the UI thread; converted to the Slint
/// `CommuteRow` (which holds non-`Send` `ModelRc`s) back on the UI thread.
struct RowData {
    label: String,
    status: String,
    active: bool,
    index: i32,
    stops: Vec<StopArrivals>,
    scale: i32,
}

/// Fetch each stop's arrivals for one active commute (blocking; off-UI only) and
/// the commute's fixed timeline scale. One `fetch_arrivals` per stop, filtered to
/// buses.
fn commute_stop_arrivals(commute: &Commute, now: OffsetDateTime) -> (Vec<StopArrivals>, i32) {
    let mut all: Vec<StopArrivals> = Vec::new();
    for stop in &commute.stops {
        let arrivals = match fetch_arrivals(ACCOUNT_KEY, &stop.code) {
            Ok(resp) => stop_arrivals(&stop.code, &stop.name, &stop.buses, &resp, now),
            Err(_) => StopArrivals {
                code: stop.code.clone(),
                name: stop.name.clone(),
                items: Vec::new(),
            },
        };
        all.push(arrivals);
    }
    (all, i32::from(commute.scale_minutes))
}

/// Build the Slint timeline lanes for a commute's stops (UI thread — makes `ModelRc`s).
fn lanes_model(stops: &[StopArrivals]) -> ModelRc<StopLane> {
    let lanes: Vec<StopLane> = stops
        .iter()
        .map(|sa| StopLane {
            name: SharedString::from(sa.name.as_str()),
            code: SharedString::from(sa.code.as_str()),
            tags: ModelRc::new(VecModel::from(
                sa.items
                    .iter()
                    .map(|it| ArrivalTag {
                        buses: SharedString::from(it.buses.join("·")),
                        minutes: i32::try_from(it.minutes).unwrap_or(0),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    ModelRc::new(VecModel::from(lanes))
}

/// For each active commute, fetch live arrivals on a background thread and
/// replace the list rows with timeline lanes (no-op without a key / when none
/// are active). Inactive rows keep their summary line.
fn spawn_arrivals(window: &AppWindow, store: &CommuteStore) {
    if ACCOUNT_KEY.is_empty() {
        return;
    }
    let now = now_sgt();
    if !store.commutes.iter().any(|c| c.is_active_at(now)) {
        return;
    }
    let commutes = store.commutes.clone();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let now = now_sgt();
        let mut data: Vec<RowData> = Vec::new();
        for (i, c) in commutes.iter().enumerate() {
            let active = c.is_active_at(now);
            let (stops, scale) = if active {
                commute_stop_arrivals(c, now)
            } else {
                (Vec::new(), 15)
            };
            data.push(RowData {
                label: card_label(c),
                status: if active {
                    String::new()
                } else {
                    inactive_summary(c, now)
                },
                active,
                index: i32::try_from(i).unwrap_or(-1),
                stops,
                scale,
            });
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                let rows: Vec<CommuteRow> = data
                    .into_iter()
                    .map(|d| CommuteRow {
                        label: SharedString::from(d.label),
                        status: SharedString::from(d.status),
                        active: d.active,
                        index: d.index,
                        lanes: lanes_model(&d.stops),
                        scale_max: d.scale,
                    })
                    .collect();
                w.set_rows(ModelRc::new(VecModel::from(rows)));
            }
        });
    });
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

/// The services at `code` from the catalog (empty when unavailable).
fn stop_services(catalog: Option<&BusCatalog>, code: &str) -> Vec<String> {
    catalog
        .map(|c| c.services(code).iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

/// Populate the editor form from an existing commute (edit) or reset it (new).
/// `form_stops` is the Rust-owned working stop list, kept in sync with the UI.
fn populate_form(
    window: &AppWindow,
    commute: Option<&Commute>,
    index: i32,
    catalog: Option<&BusCatalog>,
    form_stops: &FormStops,
) {
    let mut stops: Vec<EditStopState> = Vec::new();
    if let Some(c) = commute {
        window.set_form_label(SharedString::from(c.label.clone().unwrap_or_default()));
        set_days(window, c.days);
        window.set_start_hour(i32::from(c.start.hour));
        window.set_start_minute(i32::from(c.start.minute));
        window.set_end_hour(i32::from(c.end.hour));
        window.set_end_minute(i32::from(c.end.minute));
        window.set_form_scale(i32::from(c.scale_minutes));
        for st in &c.stops {
            let mut services = stop_services(catalog, &st.code);
            // Keep tracked buses visible even if the catalog lacks them.
            for b in &st.buses {
                if !services.iter().any(|s| s == b) {
                    services.push(b.clone());
                }
            }
            let selected = services.iter().map(|s| st.buses.contains(s)).collect();
            stops.push(EditStopState {
                code: st.code.clone(),
                name: st.name.clone(),
                services,
                selected,
            });
        }
    } else {
        window.set_form_label(SharedString::new());
        set_days(window, Weekdays(0));
        window.set_start_hour(8);
        window.set_start_minute(0);
        window.set_end_hour(9);
        window.set_end_minute(0);
        window.set_form_scale(i32::from(Commute::DEFAULT_SCALE_MINUTES));
    }
    form_stops.borrow_mut().clone_from(&stops);
    push_form_stops(window, &stops);
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

    let scale = u16::try_from(window.get_form_scale()).unwrap_or(Commute::DEFAULT_SCALE_MINUTES);
    let commute = match Commute::new(label, days, start, end, stops) {
        Ok(c) => c.with_scale_minutes(scale),
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
