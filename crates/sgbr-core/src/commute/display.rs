//! User-facing string builders for the Live Update and the in-app list row.

use crate::lta::arrival::StopArrivals;
use time::OffsetDateTime;
use time::macros::format_description;

/// Build the in-app "see you soon" row for a commute that is not active now,
/// e.g. `"see you soon · next Tue 08:00"`. `next_start` is the value returned by
/// [`crate::commute::model::Commute::next_window_start`].
#[must_use]
pub fn format_see_you_soon(next_start: OffsetDateTime) -> String {
    let fmt = format_description!("[weekday repr:short] [hour]:[minute]");
    // The descriptor only uses fields every `OffsetDateTime` has, so formatting
    // is infallible here; `unwrap_or_default` is a safe, lint-clean fallback.
    let when = next_start.format(&fmt).unwrap_or_default();
    format!("see you soon · next {when}")
}

/// Build the Live Update line for one stop, time-first with buses bracketed:
/// `"Opp Blk 123: 2m (14), 4m (14e·16), 11m (154)"`. A `minutes` value of `0`
/// or below renders as `"due"`. An empty stop renders `"<name>: no buses"`.
#[must_use]
pub fn format_stop_line(stop: &StopArrivals) -> String {
    if stop.items.is_empty() {
        return format!("{}: no buses", stop.name);
    }
    let parts: Vec<String> = stop
        .items
        .iter()
        .map(|item| {
            let when = if item.minutes <= 0 {
                "due".to_owned()
            } else {
                format!("{}m", item.minutes)
            };
            format!("{when} ({})", item.buses.join("·"))
        })
        .collect();
    format!("{}: {}", stop.name, parts.join(", "))
}

/// Build the full Live Update body for the active commute(s): one
/// [`format_stop_line`] per stop, newline-separated.
#[must_use]
pub fn format_active_notification(stops: &[StopArrivals]) -> String {
    stops
        .iter()
        .map(format_stop_line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{format_active_notification, format_see_you_soon, format_stop_line};
    use crate::lta::arrival::{ArrivalItem, StopArrivals};
    use time::macros::datetime;

    #[test]
    fn see_you_soon_formats_short_weekday_and_time() {
        // Tuesday 2026-06-23 08:00.
        assert_eq!(
            format_see_you_soon(datetime!(2026-06-23 08:00:00 +8)),
            "see you soon · next Tue 08:00"
        );
    }

    #[test]
    fn see_you_soon_zero_pads_hour_and_minute() {
        // Monday 2026-06-22 08:05 -> both fields zero-padded.
        assert_eq!(
            format_see_you_soon(datetime!(2026-06-22 08:05:00 +8)),
            "see you soon · next Mon 08:05"
        );
    }

    #[test]
    fn stop_line_is_time_first_with_bracketed_buses() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![
                ArrivalItem {
                    minutes: 2,
                    buses: vec!["14".to_owned()],
                },
                ArrivalItem {
                    minutes: 4,
                    buses: vec!["14e".to_owned(), "16".to_owned()],
                },
                ArrivalItem {
                    minutes: 11,
                    buses: vec!["154".to_owned()],
                },
            ],
        };
        assert_eq!(
            format_stop_line(&stop),
            "Opp Blk 123: 2m (14), 4m (14e·16), 11m (154)"
        );
    }

    #[test]
    fn stop_line_shows_due_for_zero_minutes() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![ArrivalItem {
                minutes: 0,
                buses: vec!["14".to_owned()],
            }],
        };
        assert_eq!(format_stop_line(&stop), "Opp Blk 123: due (14)");
    }

    #[test]
    fn stop_line_handles_no_buses() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![],
        };
        assert_eq!(format_stop_line(&stop), "Opp Blk 123: no buses");
    }

    #[test]
    fn active_notification_joins_stop_lines_with_newlines() {
        let a = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![ArrivalItem {
                minutes: 2,
                buses: vec!["14".to_owned()],
            }],
        };
        let b = StopArrivals {
            code: "17009".to_owned(),
            name: "Bef Clementi Stn".to_owned(),
            items: vec![ArrivalItem {
                minutes: 8,
                buses: vec!["96".to_owned()],
            }],
        };
        assert_eq!(
            format_active_notification(&[a, b]),
            "Opp Blk 123: 2m (14)\nBef Clementi Stn: 8m (96)"
        );
    }
}
