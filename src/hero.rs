//! Home-screen hero status: an at-a-glance relative status — a live commute, the
//! next upcoming one with a countdown, or idle. Deliberately *not* an absolute
//! clock: that would duplicate (and drift from) the OS status-bar clock.

use slint::SharedString;

use sgbr_core::commute::display::format_countdown;
use sgbr_core::commute::schedule::{HeroStatus, hero_status};
use sgbr_core::commute::store::CommuteStore;

use crate::{AppWindow, now_sgt};

/// Refresh the hero status from the store. Called on startup, after edits, and on
/// the 15s poll (so the countdown stays roughly current).
pub fn update_hero(window: &AppWindow, store: &CommuteStore) {
    let status = match hero_status(&store.commutes, now_sgt()) {
        HeroStatus::LiveNow { label } => format!("● Live now · {label}"),
        HeroStatus::Next { label, in_minutes } => {
            format!("Next · {label} in {}", format_countdown(in_minutes))
        }
        HeroStatus::Idle => "No upcoming commutes".to_owned(),
    };
    window.set_hero_status(SharedString::from(status));
}
