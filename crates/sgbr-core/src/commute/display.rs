//! User-facing string builders for the Live Update and the in-app list row.

use time::OffsetDateTime;
use time::macros::format_description;

/// Build the Live Update line for one commute, e.g.
/// `"Bus 14 · 3 min · 11 min · 19 min"`. A `minutes` entry of `0` or below
/// renders as `"due"`. An empty slice renders `"Bus <line> · no buses"`.
#[must_use]
pub fn format_live_update(line: &str, minutes: &[i64]) -> String {
    if minutes.is_empty() {
        return format!("Bus {line} · no buses");
    }
    let parts: Vec<String> = minutes
        .iter()
        .map(|&m| {
            if m <= 0 {
                "due".to_owned()
            } else {
                format!("{m} min")
            }
        })
        .collect();
    format!("Bus {line} · {}", parts.join(" · "))
}

/// Build the in-app "see you soon" row for a commute that is not active now,
/// e.g. `"see you soon · next Tue 08:00"`. `next_start` is the value returned by
/// [`crate::commute::model::Commute::next_window_start`].
#[must_use]
pub fn format_see_you_soon(next_start: OffsetDateTime) -> String {
    let fmt = format_description!("[weekday repr:short] [hour]:[minute]");
    let when = next_start.format(&fmt).unwrap_or_default();
    format!("see you soon · next {when}")
}

#[cfg(test)]
mod tests {
    use super::{format_live_update, format_see_you_soon};
    use time::macros::datetime;

    #[test]
    fn live_update_lists_up_to_three_countdowns() {
        assert_eq!(
            format_live_update("14", &[3, 11, 19]),
            "Bus 14 · 3 min · 11 min · 19 min"
        );
    }

    #[test]
    fn live_update_shows_due_for_zero_or_negative() {
        assert_eq!(format_live_update("14", &[0, 5]), "Bus 14 · due · 5 min");
        assert_eq!(format_live_update("14", &[-2, 7]), "Bus 14 · due · 7 min");
    }

    #[test]
    fn live_update_handles_no_buses() {
        assert_eq!(format_live_update("14", &[]), "Bus 14 · no buses");
    }

    #[test]
    fn see_you_soon_formats_short_weekday_and_time() {
        // Tuesday 2026-06-23 08:00.
        assert_eq!(
            format_see_you_soon(datetime!(2026-06-23 08:00:00 +8)),
            "see you soon · next Tue 08:00"
        );
    }
}
