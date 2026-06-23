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

use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::bus_catalog::search::search as catalog_search;
use sgbr_core::bus_catalog::store as catalog_store;
use sgbr_core::commute::display::format_see_you_soon;
use sgbr_core::commute::model::{Commute, TimeOfDay, Weekdays};
use sgbr_core::commute::store::CommuteStore;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use time::macros::offset;
use time::{OffsetDateTime, Weekday};

type Catalog = Rc<RefCell<Option<BusCatalog>>>;
type Store = Rc<RefCell<CommuteStore>>;

/// Wall-clock "now" in Singapore time (UTC+8, no DST).
fn now_sgt() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(offset!(+8))
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

    let cat = catalog.borrow();
    populate_form(window, None, -1, cat.as_ref());
    rebuild_rows(window, &store.borrow(), cat.as_ref());
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
    let catalog: Catalog = Rc::new(RefCell::new(
        catalog_store::load(&catalog_path(&store_path)).ok(),
    ));

    let window = AppWindow::new()?;
    rebuild_rows(&window, &store.borrow(), catalog.borrow().as_ref());
    populate_form(&window, None, -1, catalog.borrow().as_ref());
    window.set_catalog_loading(catalog.borrow().is_none());

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Rc::clone(&catalog);
    let p = Rc::clone(&store_path);
    window.on_save(move || {
        if let Some(w) = w.upgrade() {
            handle_save(&w, &s, &c, &p);
        }
    });

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Rc::clone(&catalog);
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
            rebuild_rows(&w, &s.borrow(), c.borrow().as_ref());
        }
    });

    let w = window.as_weak();
    let s = Rc::clone(&store);
    let c = Rc::clone(&catalog);
    window.on_edit(move |index| {
        if let Some(w) = w.upgrade()
            && let Ok(i) = usize::try_from(index)
        {
            let commute = s.borrow().commutes.get(i).cloned();
            populate_form(&w, commute.as_ref(), index, c.borrow().as_ref());
        }
    });

    let w = window.as_weak();
    let c = Rc::clone(&catalog);
    window.on_new_commute(move || {
        if let Some(w) = w.upgrade() {
            populate_form(&w, None, -1, c.borrow().as_ref());
        }
    });

    let w = window.as_weak();
    let c = Rc::clone(&catalog);
    window.on_search_changed(move || {
        if let Some(w) = w.upgrade() {
            let query = w.get_search_query().to_string();
            let cat = c.borrow();
            let results: Vec<StopResult> = cat
                .as_ref()
                .map(|k| {
                    catalog_search(k, &query, 30)
                        .into_iter()
                        .map(|s| StopResult {
                            code: SharedString::from(s.code.as_str()),
                            name: SharedString::from(s.name.as_str()),
                            road: SharedString::from(s.road.as_str()),
                        })
                        .collect()
                })
                .unwrap_or_default();
            w.set_search_results(ModelRc::new(VecModel::from(results)));
            w.set_catalog_loading(cat.is_none());
        }
    });

    let w = window.as_weak();
    let c = Rc::clone(&catalog);
    window.on_stop_picked(move |code| {
        if let Some(w) = w.upgrade() {
            let cat = c.borrow();
            let name = cat
                .as_ref()
                .and_then(|k| k.stop(&code))
                .map_or_else(String::new, |s| s.name.clone());
            w.set_form_stop_code(code.clone());
            w.set_form_stop_name(SharedString::from(name));
            w.set_stop_services(services_model(cat.as_ref(), &code));
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
