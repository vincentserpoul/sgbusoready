//! Window logic: is a commute live right now, and when does it next open?
//! All functions take an injected `now` so every branch is unit-testable.

use time::{OffsetDateTime, PrimitiveDateTime};

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

    /// The next moment strictly after `now` at which this commute's window
    /// opens, scanning today plus the next 7 days. Returns `None` only if no
    /// days are selected or the start time is out of range (neither happens for
    /// a `Commute` built via [`Commute::new`]).
    #[must_use]
    pub fn next_window_start(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        let start_time = self.start.to_time()?;
        let mut date = now.date();
        // Today plus the next 7 calendar days covers every weekly recurrence.
        for _ in 0..8 {
            if self.days.contains(date.weekday()) {
                let candidate =
                    PrimitiveDateTime::new(date, start_time).assume_offset(now.offset());
                if candidate > now {
                    return Some(candidate);
                }
            }
            date = date.next_day()?;
        }
        None
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

    #[test]
    fn next_start_is_today_when_before_window() {
        // Monday 07:00 -> today 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-22 07:00:00 +8)),
            Some(datetime!(2026-06-22 08:00:00 +8))
        );
    }

    #[test]
    fn next_start_skips_to_next_selected_day_after_window() {
        // Monday 09:30 (after today's window) -> Tuesday 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-22 09:30:00 +8)),
            Some(datetime!(2026-06-23 08:00:00 +8))
        );
    }

    #[test]
    fn next_start_skips_unselected_days() {
        // Friday 10:00 -> skip Sat/Sun (unselected) -> Monday 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-26 10:00:00 +8)),
            Some(datetime!(2026-06-29 08:00:00 +8))
        );
    }

    #[test]
    fn next_start_at_exact_start_returns_next_occurrence() {
        // Exactly 08:00 Monday: window is open now, so "next start" is Tuesday.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_window_start(datetime!(2026-06-22 08:00:00 +8)),
            Some(datetime!(2026-06-23 08:00:00 +8))
        );
    }
}
