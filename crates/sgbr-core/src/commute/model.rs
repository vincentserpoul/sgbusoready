//! Commute data model with validation and serde-native representation.

use serde::{Deserialize, Serialize};
use time::Time;

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

#[cfg(test)]
mod tests {
    use super::TimeOfDay;

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
}
