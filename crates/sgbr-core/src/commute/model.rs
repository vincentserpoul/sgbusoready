//! Commute data model with validation and serde-native representation.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Time, Weekday};

/// A minute-resolution time of day (`hour` 0–23, `minute` 0–59).
///
/// Ordering is lexicographic by `hour` then `minute` (derived), which is the
/// natural chronological order within a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl TimeOfDay {
    /// Convert to a [`time::Time`], or `None` if the fields are out of range.
    #[must_use]
    pub fn to_time(self) -> Option<Time> {
        Time::from_hms(self.hour, self.minute, 0).ok()
    }
}

/// A set of weekdays stored as a 7-bit mask: bit `n` (0 = Monday … 6 = Sunday)
/// corresponds to `Weekday::number_days_from_monday()`. Serializes as its `u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Weekdays(pub u8);

impl Weekdays {
    /// Build a set from a slice of weekdays. Duplicates collapse.
    #[must_use]
    pub fn from_days(days: &[Weekday]) -> Self {
        let mut mask = 0u8;
        for day in days {
            mask |= 1u8 << day.number_days_from_monday();
        }
        Self(mask)
    }

    /// Is `day` in the set?
    #[must_use]
    pub const fn contains(self, day: Weekday) -> bool {
        self.0 & (1u8 << day.number_days_from_monday()) != 0
    }

    /// True when no days are selected.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Why a [`Commute`] failed validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommuteError {
    #[error("commute must select at least one day")]
    NoDays,
    #[error("commute start or end time is out of range")]
    InvalidTime,
    #[error("commute end time must be after its start time")]
    EndNotAfterStart,
    #[error("commute must have at least one stop")]
    NoStops,
    #[error("commute stop code must not be empty")]
    StopEmptyCode,
    #[error("commute stop must track at least one bus")]
    StopNoBuses,
}

/// One stop within a commute and the buses tracked there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommuteStop {
    /// LTA bus stop code, e.g. `"83139"`.
    pub code: String,
    /// Cached display name, e.g. `"Opp Blk 123"`.
    pub name: String,
    /// Service numbers tracked at this stop, e.g. `["14", "14e"]` (>= 1).
    pub buses: Vec<String>,
}

/// A recurring commute: a set of stops (each with its own tracked buses), on a
/// set of weekdays, within a single-day time window (`start` < `end`, no
/// overnight wrap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commute {
    /// Optional user label; falls back to a stop-derived label.
    pub label: Option<String>,
    /// Days the window is active.
    pub days: Weekdays,
    /// Window open time (inclusive).
    pub start: TimeOfDay,
    /// Window close time (exclusive).
    pub end: TimeOfDay,
    /// Stops tracked by this commute (>= 1).
    pub stops: Vec<CommuteStop>,
}

impl Commute {
    /// Construct a validated commute. See [`CommuteError`] for failure modes.
    pub fn new(
        label: Option<String>,
        days: Weekdays,
        start: TimeOfDay,
        end: TimeOfDay,
        stops: Vec<CommuteStop>,
    ) -> Result<Self, CommuteError> {
        if days.is_empty() {
            return Err(CommuteError::NoDays);
        }
        if start.to_time().is_none() || end.to_time().is_none() {
            return Err(CommuteError::InvalidTime);
        }
        if end <= start {
            return Err(CommuteError::EndNotAfterStart);
        }
        if stops.is_empty() {
            return Err(CommuteError::NoStops);
        }
        for stop in &stops {
            if stop.code.is_empty() {
                return Err(CommuteError::StopEmptyCode);
            }
            if stop.buses.is_empty() {
                return Err(CommuteError::StopNoBuses);
            }
        }
        Ok(Self {
            label,
            days,
            start,
            end,
            stops,
        })
    }

    /// The timeline axis length, in minutes: the window duration (`end - start`).
    /// `Commute::new` guarantees `end > start`, so this is always >= 1.
    #[must_use]
    pub fn scale_minutes(&self) -> u16 {
        let start = u16::from(self.start.hour) * 60 + u16::from(self.start.minute);
        let end = u16::from(self.end.hour) * 60 + u16::from(self.end.minute);
        end.saturating_sub(start)
    }

    /// The label to show. Falls back to the first stop's name, suffixed with
    /// `" +N"` when there is more than one stop.
    #[must_use]
    pub fn display_label(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        match self.stops.split_first() {
            Some((first, [])) => first.name.clone(),
            Some((first, rest)) => format!("{} +{}", first.name, rest.len()),
            // Unreachable for a validated commute: `Commute::new` rejects empty `stops`.
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TimeOfDay;
    use super::Weekdays;
    use super::{Commute, CommuteError, CommuteStop};
    use time::Weekday::{Friday, Monday, Saturday, Sunday, Tuesday};

    fn stop(code: &str, name: &str, buses: &[&str]) -> CommuteStop {
        CommuteStop {
            code: code.to_owned(),
            name: name.to_owned(),
            buses: buses.iter().map(|b| (*b).to_owned()).collect(),
        }
    }

    fn weekday_commute() -> Commute {
        Commute::new(
            None,
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![stop("83139", "Opp Blk 123", &["14"])],
        )
        .expect("valid commute")
    }

    #[test]
    fn orders_by_hour_then_minute() {
        assert!(
            TimeOfDay { hour: 8, minute: 0 }
                < TimeOfDay {
                    hour: 8,
                    minute: 30
                }
        );
        assert!(
            TimeOfDay {
                hour: 8,
                minute: 59
            } < TimeOfDay { hour: 9, minute: 0 }
        );
        assert_eq!(
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 8, minute: 0 }
        );
    }

    #[test]
    fn converts_to_time_when_valid() {
        let t = TimeOfDay { hour: 8, minute: 5 }
            .to_time()
            .expect("valid time");
        assert_eq!(t, time::macros::time!(08:05:00));
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(
            TimeOfDay {
                hour: 24,
                minute: 0
            }
            .to_time()
            .is_none()
        );
        assert!(
            TimeOfDay {
                hour: 0,
                minute: 60
            }
            .to_time()
            .is_none()
        );
    }

    #[test]
    fn contains_only_listed_days() {
        let wd = Weekdays::from_days(&[Monday, Tuesday]);
        assert!(wd.contains(Monday));
        assert!(wd.contains(Tuesday));
        assert!(!wd.contains(Saturday));
        assert!(!wd.contains(Sunday));
    }

    #[test]
    fn empty_contains_nothing_and_reports_empty() {
        let wd = Weekdays::from_days(&[]);
        assert!(wd.is_empty());
        assert!(!wd.contains(Monday));
    }

    #[test]
    fn weekdays_round_trip_as_u8() {
        let wd = Weekdays::from_days(&[Monday, Saturday]);
        let json = serde_json::to_string(&wd).expect("serialize");
        let back: Weekdays = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(wd, back);
    }

    #[test]
    fn label_defaults_to_single_stop_name() {
        let c = weekday_commute();
        assert_eq!(c.display_label(), "Opp Blk 123");
    }

    #[test]
    fn label_defaults_to_first_stop_plus_count() {
        let c = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![
                stop("83139", "Opp Blk 123", &["14"]),
                stop("17009", "Bef Clementi Stn", &["96"]),
            ],
        )
        .expect("valid commute");
        assert_eq!(c.display_label(), "Opp Blk 123 +1");
    }

    #[test]
    fn custom_label_overrides_default() {
        let c = Commute::new(
            Some("Morning work".to_owned()),
            Weekdays::from_days(&[Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            vec![stop("83139", "Opp Blk 123", &["14"])],
        )
        .expect("valid commute");
        assert_eq!(c.display_label(), "Morning work");
    }

    #[test]
    fn rejects_no_days() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::NoDays)
        ));
    }

    #[test]
    fn rejects_end_not_after_start() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 9, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::EndNotAfterStart)
        ));
    }

    #[test]
    fn rejects_out_of_range_time() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay {
                    hour: 24,
                    minute: 0
                },
                vec![stop("83139", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::InvalidTime)
        ));
    }

    #[test]
    fn rejects_no_stops() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![],
            ),
            Err(CommuteError::NoStops)
        ));
    }

    #[test]
    fn rejects_empty_stop_code() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("", "Opp Blk 123", &["14"])],
            ),
            Err(CommuteError::StopEmptyCode)
        ));
    }

    #[test]
    fn rejects_stop_with_no_buses() {
        assert!(matches!(
            Commute::new(
                None,
                Weekdays::from_days(&[Monday]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                vec![stop("83139", "Opp Blk 123", &[])],
            ),
            Err(CommuteError::StopNoBuses)
        ));
    }

    #[test]
    fn commute_serde_round_trip() {
        let c = weekday_commute();
        let json = serde_json::to_string(&c).expect("serialize");
        let back: Commute = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(c, back);
    }

    #[test]
    fn scale_minutes_is_window_duration() {
        // 08:00–09:00 → 60 minutes.
        assert_eq!(weekday_commute().scale_minutes(), 60);
        // 09:30–10:00 → 30 minutes.
        let c = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay {
                hour: 9,
                minute: 30,
            },
            TimeOfDay {
                hour: 10,
                minute: 0,
            },
            vec![stop("83139", "Opp Blk 123", &["14"])],
        )
        .expect("valid commute");
        assert_eq!(c.scale_minutes(), 30);
        // Smallest valid window → 1 minute.
        let c = Commute::new(
            None,
            Weekdays::from_days(&[Monday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 8, minute: 1 },
            vec![stop("83139", "Opp Blk 123", &["14"])],
        )
        .expect("valid commute");
        assert_eq!(c.scale_minutes(), 1);
    }

    #[test]
    fn ignores_legacy_scale_minutes_field() {
        // Older persisted commutes carry a `scale_minutes` field; it is now derived
        // from the window, so a stored value is simply ignored on load.
        let legacy = r#"{
            "label": null,
            "days": 3,
            "start": { "hour": 8, "minute": 0 },
            "end": { "hour": 9, "minute": 0 },
            "stops": [{ "code": "83139", "name": "Opp Blk 123", "buses": ["14"] }],
            "scale_minutes": 45
        }"#;
        let c: Commute = serde_json::from_str(legacy).expect("deserialize legacy");
        assert_eq!(c.scale_minutes(), 60);
    }
}
