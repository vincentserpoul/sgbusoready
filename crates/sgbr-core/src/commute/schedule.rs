//! Decisions across the whole commute list: which are active now, and when the
//! scheduler should next wake. Single-commute timing lives in `window.rs`.

use time::OffsetDateTime;

use crate::commute::model::Commute;

/// The commutes whose window is open at `now`, in list order.
#[must_use]
pub fn active_commutes_at(commutes: &[Commute], now: OffsetDateTime) -> Vec<&Commute> {
    commutes.iter().filter(|c| c.is_active_at(now)).collect()
}

/// The earliest moment any commute next changes state — the time the scheduler
/// should set its next alarm for. `None` when the list is empty (or no commute
/// has a valid boundary).
#[must_use]
pub fn next_alarm_at(commutes: &[Commute], now: OffsetDateTime) -> Option<OffsetDateTime> {
    commutes.iter().filter_map(|c| c.next_boundary(now)).min()
}

#[cfg(test)]
mod tests {
    use super::{active_commutes_at, next_alarm_at};
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
    use time::Weekday::{Monday, Tuesday};
    use time::macros::datetime;

    fn morning() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday]),
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

    fn evening() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay {
                hour: 18,
                minute: 0,
            },
            TimeOfDay {
                hour: 19,
                minute: 0,
            },
            vec![CommuteStop {
                code: "84009".to_owned(),
                name: "Bef Clementi Stn".to_owned(),
                buses: vec!["67".to_owned()],
            }],
        )
        .expect("valid commute")
    }

    #[test]
    fn active_returns_only_live_commutes() {
        // Monday 08:30 -> morning active, evening not.
        let list = vec![morning(), evening()];
        let active = active_commutes_at(&list, datetime!(2026-06-22 08:30:00 +8));
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].stops[0].code, "83139");
    }

    #[test]
    fn active_empty_when_none_live() {
        let list = vec![morning(), evening()];
        let active = active_commutes_at(&list, datetime!(2026-06-22 12:00:00 +8));
        assert!(active.is_empty());
    }

    #[test]
    fn next_alarm_is_earliest_boundary() {
        // Monday 08:30: morning is active (boundary 09:00), evening inactive
        // (next start 18:00). Earliest boundary is 09:00.
        let list = vec![morning(), evening()];
        assert_eq!(
            next_alarm_at(&list, datetime!(2026-06-22 08:30:00 +8)),
            Some(datetime!(2026-06-22 09:00:00 +8))
        );
    }

    #[test]
    fn next_alarm_when_none_active_is_earliest_start() {
        // Monday 12:00: both inactive. morning next start = Tuesday 08:00,
        // evening next start = Monday 18:00. Earliest is Monday 18:00.
        let list = vec![morning(), evening()];
        assert_eq!(
            next_alarm_at(&list, datetime!(2026-06-22 12:00:00 +8)),
            Some(datetime!(2026-06-22 18:00:00 +8))
        );
    }

    #[test]
    fn next_alarm_empty_list_is_none() {
        let list: Vec<Commute> = vec![];
        assert_eq!(
            next_alarm_at(&list, datetime!(2026-06-22 08:30:00 +8)),
            None
        );
    }
}
