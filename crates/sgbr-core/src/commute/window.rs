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
        // Every future candidate reuses `now`'s UTC offset. Singapore (the
        // target market) has no DST, so this is exact; in a DST zone a window
        // could be off by an hour across a transition.
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

    /// If the commute is active at `now`, the [`OffsetDateTime`] of today's
    /// window close (`end`); otherwise `None`.
    #[must_use]
    pub fn current_window_end(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        if !self.is_active_at(now) {
            return None;
        }
        let end_time = self.end.to_time()?;
        Some(PrimitiveDateTime::new(now.date(), end_time).assume_offset(now.offset()))
    }

    /// The next moment this commute changes state: its window close if active
    /// now, otherwise its next window open. `None` only when `next_window_start`
    /// is `None` (no days / invalid time — neither happens via `Commute::new`).
    #[must_use]
    pub fn next_boundary(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        if self.is_active_at(now) {
            self.current_window_end(now)
        } else {
            self.next_window_start(now)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
    use time::Weekday::{Friday, Monday, Tuesday};
    use time::macros::datetime;

    fn mon_fri_8_to_9() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday, Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned()],
            }],
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

    #[test]
    fn current_window_end_some_when_active() {
        // Monday 08:30 -> active, window ends 09:00 the same day.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.current_window_end(datetime!(2026-06-22 08:30:00 +8)),
            Some(datetime!(2026-06-22 09:00:00 +8))
        );
    }

    #[test]
    fn current_window_end_none_when_inactive() {
        // Monday 07:00 -> not active -> no current window.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.current_window_end(datetime!(2026-06-22 07:00:00 +8)),
            None
        );
    }

    #[test]
    fn next_boundary_is_end_when_active() {
        // Active Monday 08:30 -> next boundary is this window's end 09:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_boundary(datetime!(2026-06-22 08:30:00 +8)),
            Some(datetime!(2026-06-22 09:00:00 +8))
        );
    }

    #[test]
    fn next_boundary_is_next_start_when_inactive() {
        // Inactive Monday 07:00 -> next boundary is today's start 08:00.
        let c = mon_fri_8_to_9();
        assert_eq!(
            c.next_boundary(datetime!(2026-06-22 07:00:00 +8)),
            Some(datetime!(2026-06-22 08:00:00 +8))
        );
    }
}
