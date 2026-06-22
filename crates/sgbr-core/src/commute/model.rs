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
    #[error("commute line must not be empty")]
    EmptyLine,
    #[error("commute stop must not be empty")]
    EmptyStop,
    #[error("commute must select at least one day")]
    NoDays,
    #[error("commute end time must be after its start time")]
    EndNotAfterStart,
    #[error("commute start or end time is out of range")]
    InvalidTime,
}

/// A recurring commute: one bus line at one stop, on a set of weekdays, within
/// a single-day time window (`start` < `end`, no overnight wrap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commute {
    /// Bus service number, e.g. `"14"`.
    pub line: String,
    /// LTA bus stop code, e.g. `"83139"`.
    pub stop: String,
    /// Days the window is active.
    pub days: Weekdays,
    /// Window open time (inclusive).
    pub start: TimeOfDay,
    /// Window close time (exclusive).
    pub end: TimeOfDay,
    /// Optional user label; falls back to `"<line> @ <stop>"`.
    pub label: Option<String>,
}

impl Commute {
    /// Construct a validated commute. See [`CommuteError`] for failure modes.
    pub fn new(
        line: &str,
        stop: &str,
        days: Weekdays,
        start: TimeOfDay,
        end: TimeOfDay,
        label: Option<String>,
    ) -> Result<Self, CommuteError> {
        if line.is_empty() {
            return Err(CommuteError::EmptyLine);
        }
        if stop.is_empty() {
            return Err(CommuteError::EmptyStop);
        }
        if days.is_empty() {
            return Err(CommuteError::NoDays);
        }
        if start.to_time().is_none() || end.to_time().is_none() {
            return Err(CommuteError::InvalidTime);
        }
        if end <= start {
            return Err(CommuteError::EndNotAfterStart);
        }
        Ok(Self {
            line: line.to_owned(),
            stop: stop.to_owned(),
            days,
            start,
            end,
            label,
        })
    }

    /// The label to show, falling back to `"<line> @ <stop>"`.
    #[must_use]
    pub fn display_label(&self) -> String {
        match &self.label {
            Some(l) => l.clone(),
            None => format!("{} @ {}", self.line, self.stop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TimeOfDay;
    use super::Weekdays;
    use super::{Commute, CommuteError};
    use time::Weekday::{Friday, Monday, Saturday, Sunday, Tuesday};

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

    fn weekday_commute() -> Commute {
        Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Monday, Tuesday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            None,
        )
        .expect("valid commute")
    }

    #[test]
    fn label_defaults_to_line_at_stop() {
        let c = weekday_commute();
        assert_eq!(c.display_label(), "14 @ 83139");
    }

    #[test]
    fn custom_label_overrides_default() {
        let c = Commute::new(
            "14",
            "83139",
            Weekdays::from_days(&[Friday]),
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 9, minute: 0 },
            Some("Morning work".to_owned()),
        )
        .expect("valid commute");
        assert_eq!(c.display_label(), "Morning work");
    }

    #[test]
    fn rejects_empty_line_and_stop() {
        let days = Weekdays::from_days(&[Monday]);
        let start = TimeOfDay { hour: 8, minute: 0 };
        let end = TimeOfDay { hour: 9, minute: 0 };
        assert!(matches!(
            Commute::new("", "83139", days, start, end, None),
            Err(CommuteError::EmptyLine)
        ));
        assert!(matches!(
            Commute::new("14", "", days, start, end, None),
            Err(CommuteError::EmptyStop)
        ));
    }

    #[test]
    fn rejects_no_days() {
        assert!(matches!(
            Commute::new(
                "14",
                "83139",
                Weekdays::from_days(&[]),
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                None,
            ),
            Err(CommuteError::NoDays)
        ));
    }

    #[test]
    fn rejects_end_not_after_start() {
        let days = Weekdays::from_days(&[Monday]);
        assert!(matches!(
            Commute::new(
                "14",
                "83139",
                days,
                TimeOfDay { hour: 9, minute: 0 },
                TimeOfDay { hour: 9, minute: 0 },
                None,
            ),
            Err(CommuteError::EndNotAfterStart)
        ));
    }

    #[test]
    fn rejects_out_of_range_time() {
        let days = Weekdays::from_days(&[Monday]);
        assert!(matches!(
            Commute::new(
                "14",
                "83139",
                days,
                TimeOfDay { hour: 8, minute: 0 },
                TimeOfDay {
                    hour: 24,
                    minute: 0
                },
                None,
            ),
            Err(CommuteError::InvalidTime)
        ));
    }

    #[test]
    fn commute_serde_round_trip() {
        let c = weekday_commute();
        let json = serde_json::to_string(&c).expect("serialize");
        let back: Commute = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(c, back);
    }
}
