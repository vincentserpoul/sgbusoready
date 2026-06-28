//! Decisions across the whole commute list: which are active now, and when the
//! scheduler should next wake. Single-commute timing lives in `window.rs`.

use time::OffsetDateTime;

use crate::commute::model::Commute;

/// The commutes whose window is open at `now`, in list order.
#[must_use]
pub fn active_commutes_at(commutes: &[Commute], now: OffsetDateTime) -> Vec<&Commute> {
    commutes.iter().filter(|c| c.is_active_at(now)).collect()
}

/// At-a-glance status for the home-screen header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeroStatus {
    /// A commute window is open right now.
    LiveNow { label: String },
    /// The soonest upcoming commute and the minutes until its window opens.
    Next { label: String, in_minutes: i64 },
    /// No commutes, or none with an upcoming window.
    Idle,
}

/// Pick the home-screen hero status: a live commute if one is open now, else the
/// soonest upcoming commute with a countdown, else idle.
#[must_use]
pub fn hero_status(commutes: &[Commute], now: OffsetDateTime) -> HeroStatus {
    if let Some(c) = active_commutes_at(commutes, now).first() {
        return HeroStatus::LiveNow {
            label: c.display_label(),
        };
    }
    let soonest = commutes
        .iter()
        .filter_map(|c| c.next_window_start(now).map(|start| (c, start)))
        .min_by_key(|(_, start)| *start);
    match soonest {
        Some((c, start)) => HeroStatus::Next {
            label: c.display_label(),
            in_minutes: (start - now).whole_minutes(),
        },
        None => HeroStatus::Idle,
    }
}

/// The earliest moment any commute next changes state — the time the scheduler
/// should set its next alarm for. `None` when the list is empty (or no commute
/// has a valid boundary).
#[must_use]
pub fn next_alarm_at(commutes: &[Commute], now: OffsetDateTime) -> Option<OffsetDateTime> {
    commutes.iter().filter_map(|c| c.next_boundary(now)).min()
}

/// A stop to refresh while active, with the union of buses tracked there by
/// every currently-active commute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPlan {
    /// LTA bus stop code.
    pub code: String,
    /// Cached display name (from the first commute that referenced this stop).
    pub name: String,
    /// Union of tracked buses across active commutes, deduped, first-seen order.
    pub buses: Vec<String>,
}

/// The distinct stops across all commutes active at `now`, each carrying the
/// union of buses tracked there. One LTA arrival call per returned stop covers
/// every active commute. Empty when nothing is active.
#[must_use]
pub fn active_stop_plans(commutes: &[Commute], now: OffsetDateTime) -> Vec<StopPlan> {
    let mut plans: Vec<StopPlan> = Vec::new();
    for commute in commutes.iter().filter(|c| c.is_active_at(now)) {
        for stop in &commute.stops {
            if let Some(existing) = plans.iter_mut().find(|p| p.code == stop.code) {
                for bus in &stop.buses {
                    if !existing.buses.contains(bus) {
                        existing.buses.push(bus.clone());
                    }
                }
            } else {
                plans.push(StopPlan {
                    code: stop.code.clone(),
                    name: stop.name.clone(),
                    buses: stop.buses.clone(),
                });
            }
        }
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::{HeroStatus, active_commutes_at, hero_status, next_alarm_at};
    use crate::commute::model::{Commute, CommuteStop, TimeOfDay, Weekdays};
    use time::Weekday::{Monday, Tuesday};
    use time::macros::datetime;

    #[test]
    fn hero_live_when_a_window_is_open() {
        let list = vec![morning(), evening()];
        assert_eq!(
            hero_status(&list, datetime!(2026-06-22 08:30:00 +8)),
            HeroStatus::LiveNow {
                label: "Opp Blk 123".to_owned()
            }
        );
    }

    #[test]
    fn hero_next_picks_soonest_with_countdown() {
        // Monday 12:00: evening starts Monday 18:00 (in 6h), morning Tuesday 08:00.
        let list = vec![morning(), evening()];
        assert_eq!(
            hero_status(&list, datetime!(2026-06-22 12:00:00 +8)),
            HeroStatus::Next {
                label: "Bef Clementi Stn".to_owned(),
                in_minutes: 360,
            }
        );
    }

    #[test]
    fn hero_idle_for_empty_list() {
        assert_eq!(
            hero_status(&[], datetime!(2026-06-22 12:00:00 +8)),
            HeroStatus::Idle
        );
    }

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

    fn two_stop_morning() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![
                CommuteStop {
                    code: "83139".to_owned(),
                    name: "Opp Blk 123".to_owned(),
                    buses: vec!["14".to_owned(), "14e".to_owned()],
                },
                CommuteStop {
                    code: "17009".to_owned(),
                    name: "Bef Clementi Stn".to_owned(),
                    buses: vec!["96".to_owned()],
                },
            ],
        )
        .expect("valid commute")
    }

    #[test]
    fn active_stop_plans_lists_distinct_stops_when_active() {
        let list = vec![two_stop_morning()];
        let plans = super::active_stop_plans(&list, datetime!(2026-06-22 08:30:00 +8));
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].code, "83139");
        assert_eq!(plans[0].buses, vec!["14".to_owned(), "14e".to_owned()]);
        assert_eq!(plans[1].code, "17009");
    }

    #[test]
    fn active_stop_plans_empty_when_inactive() {
        let list = vec![two_stop_morning()];
        let plans = super::active_stop_plans(&list, datetime!(2026-06-22 12:00:00 +8));
        assert!(plans.is_empty());
    }

    #[test]
    fn active_stop_plans_unions_buses_across_commutes_for_same_stop() {
        // Two commutes both active Monday 08:30, both tracking stop 83139 with
        // overlapping + distinct buses -> union, deduped, first-seen order.
        let a = two_stop_morning(); // 83139: 14, 14e ; 17009: 96
        let b = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![CommuteStop {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["14".to_owned(), "154".to_owned()],
            }],
        )
        .expect("valid commute");
        let list = vec![a, b];
        let plans = super::active_stop_plans(&list, datetime!(2026-06-22 08:30:00 +8));
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].code, "83139");
        assert_eq!(
            plans[0].buses,
            vec!["14".to_owned(), "14e".to_owned(), "154".to_owned()]
        );
    }
}
