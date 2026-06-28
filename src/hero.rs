//! Home-screen hero header: the current time/weekday and an at-a-glance status
//! (a live commute, the next upcoming one with a countdown, or idle).

use slint::SharedString;
use time::macros::format_description;

use sgbr_core::commute::display::format_countdown;
use sgbr_core::commute::schedule::{HeroStatus, hero_status};
use sgbr_core::commute::store::CommuteStore;

use crate::{AppWindow, now_sgt};

/// Refresh the hero header from the store and the current time. Called on startup,
/// after edits, and on the 15s poll.
pub fn update_hero(window: &AppWindow, store: &CommuteStore) {
    let now = now_sgt();
    let time_fmt = format_description!("[hour]:[minute]");
    let day_fmt = format_description!("[weekday repr:short]");
    window.set_hero_time(SharedString::from(
        now.format(&time_fmt).unwrap_or_default(),
    ));
    window.set_hero_day(SharedString::from(now.format(&day_fmt).unwrap_or_default()));

    let status = match hero_status(&store.commutes, now) {
        HeroStatus::LiveNow { label } => format!("● Live now · {label}"),
        HeroStatus::Next { label, in_minutes } => {
            format!("Next · {label} in {}", format_countdown(in_minutes))
        }
        HeroStatus::Idle => "No upcoming commutes".to_owned(),
    };
    window.set_hero_status(SharedString::from(status));
}
