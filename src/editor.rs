//! Commute editor form: the Rust-owned working stop list and the helpers that
//! sync it to/from the Slint editor screen.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};
use time::Weekday;

use sgbr_core::bus_catalog::model::BusCatalog;
use sgbr_core::commute::model::{Commute, TimeOfDay, Weekdays};

use crate::{AppWindow, EditStop};

/// One stop being edited: its code/name, the full service list at that stop, and
/// which services are currently selected (parallel to `services`).
#[derive(Clone)]
pub struct EditStopState {
    pub code: String,
    pub name: String,
    pub services: Vec<String>,
    pub selected: Vec<bool>,
}

/// The editor's working list of stops, owned by Rust and rebuilt into the Slint
/// `form-stops` model on every mutation.
pub type FormStops = Rc<RefCell<Vec<EditStopState>>>;

/// Push the Rust editor stop-state into the Slint `form-stops` model.
pub fn push_form_stops(window: &AppWindow, stops: &[EditStopState]) {
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

pub fn read_weekdays(window: &AppWindow) -> Weekdays {
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

pub fn time_of_day(hour: i32, minute: i32) -> TimeOfDay {
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
pub fn stop_services(catalog: Option<&BusCatalog>, code: &str) -> Vec<String> {
    catalog
        .map(|c| c.services(code).iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

/// Populate the editor form from an existing commute (edit) or reset it (new).
/// `form_stops` is the Rust-owned working stop list, kept in sync with the UI.
pub fn populate_form(
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
    }
    form_stops.borrow_mut().clone_from(&stops);
    push_form_stops(window, &stops);
    window.set_editing_index(index);
    window.set_error_text(SharedString::new());
}
