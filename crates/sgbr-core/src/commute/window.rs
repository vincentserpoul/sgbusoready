//! Window logic: is a commute live right now, and when does it next open?
//! All functions take an injected `now` so every branch is unit-testable.

use time::OffsetDateTime;

use crate::commute::model::{Commute, TimeOfDay};

impl Commute {
    /// True when `now` falls on a selected day and within `[start, end)`.
    #[must_use]
    pub fn is_active_at(&self, now: OffsetDateTime) -> bool {
        if !self.days.contains(now.weekday()) {
            return false;
        }
        let current = TimeOfDay {
            hour: now.hour(),
            minute: now.minute(),
        };
        self.start <= current && current < self.end
    }
}

#[cfg(test)]
mod tests {
    use crate::commute::model::{Commute, TimeOfDay, Weekdays};
    use time::Weekday::{Friday, Monday, Tuesday};
    use time::macros::datetime;

    fn mon_fri_8_to_9() -> Commute {
        Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Monday, Tuesday, Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            None,
        )
        .expect("valid commute")
    }

    #[test]
    fn active_inside_window_on_selected_day() {
        // Monday 08:30 +8
        assert!(mon_fri_8_to_9().is_active_at(datetime!(2026-06-22 08:30:00 +8)));
    }

    #[test]
    fn inactive_before_start() {
        assert!(!mon_fri_8_to_9().is_active_at(datetime!(2026-06-22 07:59:00 +8)));
    }

    #[test]
    fn inactive_at_end_exclusive() {
        // 09:00 is the exclusive end -> not active.
        assert!(!mon_fri_8_to_9().is_active_at(datetime!(2026-06-22 09:00:00 +8)));
    }

    #[test]
    fn inactive_on_unselected_day() {
        // Saturday 2026-06-27 08:30 -> day not selected.
        assert!(!mon_fri_8_to_9().is_active_at(datetime!(2026-06-27 08:30:00 +8)));
    }
}
