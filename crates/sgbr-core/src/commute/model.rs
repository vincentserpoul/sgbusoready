//! Commute data model with validation and serde-native representation.

use serde::{Deserialize, Serialize};
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

#[cfg(test)]
mod tests {
    use super::TimeOfDay;
    use super::Weekdays;
    use time::Weekday::{Monday, Saturday, Sunday, Tuesday};

    #[test]
    fn orders_by_hour_then_minute() {
        assert!(TimeOfDay { hour: 8, minute: 0 } < TimeOfDay { hour: 8, minute: 30 });
        assert!(TimeOfDay { hour: 8, minute: 59 } < TimeOfDay { hour: 9, minute: 0 });
        assert_eq!(
            TimeOfDay { hour: 8, minute: 0 },
            TimeOfDay { hour: 8, minute: 0 }
        );
    }

    #[test]
    fn converts_to_time_when_valid() {
        let t = TimeOfDay { hour: 8, minute: 5 }.to_time().expect("valid time");
        assert_eq!(t, time::macros::time!(08:05:00));
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(TimeOfDay { hour: 24, minute: 0 }.to_time().is_none());
        assert!(TimeOfDay { hour: 0, minute: 60 }.to_time().is_none());
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
}
