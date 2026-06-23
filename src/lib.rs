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

use generated::{AppWindow, CommuteRow};

#[cfg(target_os = "android")]
mod android_bridge;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use sgbr_core::commute::display::format_see_you_soon;
use sgbr_core::commute::model::{Commute, TimeOfDay, Weekdays};
use sgbr_core::commute::store::CommuteStore;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use time::macros::offset;
use time::{OffsetDateTime, Weekday};

/// Wall-clock "now" in Singapore time (UTC+8, no DST) — the offset the user's
/// commute windows are expressed in.
fn now_sgt() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(offset!(+8))
}

/// Build the per-row status line: live commutes read "active now", otherwise
/// the next window opening rendered as "see you soon · next …".
fn status_for(commute: &Commute, now: OffsetDateTime) -> String {
    if commute.is_active_at(now) {
        "active now".to_owned()
    } else {
        commute
            .next_window_start(now)
            .map(format_see_you_soon)
            .unwrap_or_default()
    }
}

/// Re-render the commute list from the store into the window's `rows` model.
fn rebuild_rows(window: &AppWindow, store: &CommuteStore) {
    let now = now_sgt();
    let rows: Vec<CommuteRow> = store
        .commutes
        .iter()
        .map(|c| CommuteRow {
            label: SharedString::from(c.display_label()),
            status: SharedString::from(status_for(c, now)),
        })
        .collect();
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Collect the seven weekday checkboxes into a [`Weekdays`] bitmask.
fn read_weekdays(window: &AppWindow) -> Weekdays {
    let mut days: Vec<Weekday> = Vec::new();
    if window.get_form_day_mon() {
        days.push(Weekday::Monday);
    }
    if window.get_form_day_tue() {
        days.push(Weekday::Tuesday);
    }
    if window.get_form_day_wed() {
        days.push(Weekday::Wednesday);
    }
    if window.get_form_day_thu() {
        days.push(Weekday::Thursday);
    }
    if window.get_form_day_fri() {
        days.push(Weekday::Friday);
    }
    if window.get_form_day_sat() {
        days.push(Weekday::Saturday);
    }
    if window.get_form_day_sun() {
        days.push(Weekday::Sunday);
    }
    Weekdays::from_days(&days)
}

/// A [`TimeOfDay`] from two `SpinBox` values (already range-bounded by the widget).
fn time_of_day(hour: i32, minute: i32) -> TimeOfDay {
    TimeOfDay {
        hour: u8::try_from(hour).unwrap_or(0),
        minute: u8::try_from(minute).unwrap_or(0),
    }
}

/// Populate the form from an existing commute (edit) or reset it (new).
fn populate_form(window: &AppWindow, commute: Option<&Commute>, editing_index: i32) {
    if let Some(c) = commute {
        window.set_form_line(SharedString::from(c.line.as_str()));
        window.set_form_stop(SharedString::from(c.stop.as_str()));
        window.set_form_start_hour(i32::from(c.start.hour));
        window.set_form_start_minute(i32::from(c.start.minute));
        window.set_form_end_hour(i32::from(c.end.hour));
        window.set_form_end_minute(i32::from(c.end.minute));
        window.set_form_day_mon(c.days.contains(Weekday::Monday));
        window.set_form_day_tue(c.days.contains(Weekday::Tuesday));
        window.set_form_day_wed(c.days.contains(Weekday::Wednesday));
        window.set_form_day_thu(c.days.contains(Weekday::Thursday));
        window.set_form_day_fri(c.days.contains(Weekday::Friday));
        window.set_form_day_sat(c.days.contains(Weekday::Saturday));
        window.set_form_day_sun(c.days.contains(Weekday::Sunday));
    } else {
        window.set_form_line(SharedString::new());
        window.set_form_stop(SharedString::new());
        window.set_form_start_hour(8);
        window.set_form_start_minute(0);
        window.set_form_end_hour(9);
        window.set_form_end_minute(0);
        window.set_form_day_mon(false);
        window.set_form_day_tue(false);
        window.set_form_day_wed(false);
        window.set_form_day_thu(false);
        window.set_form_day_fri(false);
        window.set_form_day_sat(false);
        window.set_form_day_sun(false);
    }
    window.set_editing_index(editing_index);
    window.set_error_text(SharedString::new());
}

/// Persist the store (logging any I/O error on Android), then re-arm the
/// boundary alarm so schedule changes take effect immediately.
fn persist(store: &CommuteStore, path: &Path) {
    #[cfg(target_os = "android")]
    if store.save(path).is_err() {
        log::warn!("commute store save failed");
    }
    #[cfg(not(target_os = "android"))]
    let _ = store.save(path);
    rearm_alarms();
}

/// Re-arm the next commute alarm. No-op off Android (no `AlarmManager`).
#[allow(
    clippy::missing_const_for_fn,
    reason = "non-const on Android, where it calls into the JNI bridge"
)]
fn rearm_alarms() {
    #[cfg(target_os = "android")]
    android_bridge::arm_alarms();
}

/// Validate the form and insert/replace the commute, or surface the error.
fn handle_save(window: &AppWindow, store: &Rc<RefCell<CommuteStore>>, path: &Path) {
    let line = window.get_form_line().to_string();
    let stop = window.get_form_stop().to_string();
    let days = read_weekdays(window);
    let start = time_of_day(window.get_form_start_hour(), window.get_form_start_minute());
    let end = time_of_day(window.get_form_end_hour(), window.get_form_end_minute());

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

    populate_form(window, None, -1);
    rebuild_rows(window, &store.borrow());
}

/// Apply a mutation to the store by index (delete / reorder), then re-render.
fn mutate_at(
    window: &AppWindow,
    store: &Rc<RefCell<CommuteStore>>,
    path: &Path,
    index: i32,
    op: impl FnOnce(&mut Vec<Commute>, usize),
) {
    let Ok(i) = usize::try_from(index) else {
        return;
    };
    let mut s = store.borrow_mut();
    if i >= s.commutes.len() {
        return;
    }
    op(&mut s.commutes, i);
    persist(&s, path);
    drop(s);

    populate_form(window, None, -1);
    rebuild_rows(window, &store.borrow());
}

/// Build the settings window, wire its callbacks to `store_path`, and run the
/// Slint event loop. The store is loaded once and kept in sync on every edit.
///
/// # Errors
/// Returns any [`slint::PlatformError`] from window creation or the event loop.
pub fn run_app(store_path: PathBuf) -> Result<(), slint::PlatformError> {
    let store_path = Rc::new(store_path);
    let store = Rc::new(RefCell::new(
        CommuteStore::load(&store_path).unwrap_or_default(),
    ));
    let window = AppWindow::new()?;
    rebuild_rows(&window, &store.borrow());
    populate_form(&window, None, -1);

    let w_save = window.as_weak();
    let store_save = Rc::clone(&store);
    let path_save = Rc::clone(&store_path);
    window.on_save(move || {
        if let Some(w) = w_save.upgrade() {
            handle_save(&w, &store_save, &path_save);
        }
    });

    let w_del = window.as_weak();
    let store_del = Rc::clone(&store);
    let path_del = Rc::clone(&store_path);
    window.on_delete(move |index| {
        if let Some(w) = w_del.upgrade() {
            mutate_at(&w, &store_del, &path_del, index, |v, i| {
                v.remove(i);
            });
        }
    });

    let w_up = window.as_weak();
    let store_up = Rc::clone(&store);
    let path_up = Rc::clone(&store_path);
    window.on_move_up(move |index| {
        if let Some(w) = w_up.upgrade() {
            mutate_at(&w, &store_up, &path_up, index, |v, i| {
                if i > 0 {
                    v.swap(i, i - 1);
                }
            });
        }
    });

    let w_down = window.as_weak();
    let store_down = Rc::clone(&store);
    let path_down = Rc::clone(&store_path);
    window.on_move_down(move |index| {
        if let Some(w) = w_down.upgrade() {
            mutate_at(&w, &store_down, &path_down, index, |v, i| {
                if i + 1 < v.len() {
                    v.swap(i, i + 1);
                }
            });
        }
    });

    let w_edit = window.as_weak();
    let store_edit = Rc::clone(&store);
    window.on_edit(move |index| {
        if let Some(w) = w_edit.upgrade()
            && let Ok(i) = usize::try_from(index)
        {
            let commute = store_edit.borrow().commutes.get(i).cloned();
            populate_form(&w, commute.as_ref(), index);
        }
    });

    let w_new = window.as_weak();
    window.on_new_commute(move || {
        if let Some(w) = w_new.upgrade() {
            populate_form(&w, None, -1);
        }
    });

    window.run()
}

/// Android entry point. The `NativeActivity` glue calls this; we hand the
/// `AndroidApp` to Slint's backend, then run the shared settings UI.
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
    // Arm commute alarms from the saved store so windows fire even if the app
    // is later closed (the foreground service re-arms thereafter).
    android_bridge::arm_alarms();
    let _ = run_app(store_path);
}
