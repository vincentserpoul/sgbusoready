//! User-facing string builders for the Live Update and the in-app list row.

use crate::lta::arrival::StopArrivals;
use time::macros::format_description;
use time::{Duration, OffsetDateTime};

/// Short weekday + time of the next window start, e.g. `"next Tue 08:00"`.
/// `next_start` is the value returned by
/// [`crate::commute::model::Commute::next_window_start`].
#[must_use]
pub fn format_next_time(next_start: OffsetDateTime) -> String {
    let fmt = format_description!("[weekday repr:short] [hour]:[minute]");
    // The descriptor only uses fields every `OffsetDateTime` has, so formatting
    // is infallible here; `unwrap_or_default` is a safe, lint-clean fallback.
    let when = next_start.format(&fmt).unwrap_or_default();
    format!("next {when}")
}

/// Build the in-app "see you soon" row for a commute that is not active now,
/// e.g. `"see you soon · next Tue 08:00"`.
#[must_use]
pub fn format_see_you_soon(next_start: OffsetDateTime) -> String {
    format!("see you soon · {}", format_next_time(next_start))
}

/// A coarse, human countdown for a positive number of `minutes`, e.g. `"45m"`,
/// `"2h 25m"`, `"2h"`, `"1d 3h"`. Non-positive input renders as `"now"`.
#[must_use]
pub fn format_countdown(minutes: i64) -> String {
    if minutes <= 0 {
        return "now".to_owned();
    }
    let days = minutes / 1440;
    let hours = (minutes % 1440) / 60;
    let mins = minutes % 60;
    if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if mins > 0 {
            format!("{hours}h {mins}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{mins}m")
    }
}

/// The two clock labels for a rolling timeline axis: the current time (left) and
/// `now + duration` (right), each `"HH:MM"` (24h, zero-padded). The right label
/// wraps past midnight naturally (e.g. `23:50` + 30m → `00:20`).
#[must_use]
pub fn format_timeline_labels(now: OffsetDateTime, duration_minutes: u16) -> (String, String) {
    let fmt = format_description!("[hour]:[minute]");
    let end = now + Duration::minutes(i64::from(duration_minutes));
    // The descriptor only uses fields every `OffsetDateTime` has, so formatting is
    // infallible; `unwrap_or_default` is a safe, lint-clean fallback.
    (
        now.format(&fmt).unwrap_or_default(),
        end.format(&fmt).unwrap_or_default(),
    )
}

/// Build the two-line Live Update block for one stop: the stop name on the first
/// line, then its arrivals time-first with buses bracketed on the second, e.g.
/// `"Opp Blk 123\n2m (14), 4m (14e·16), 11m (154)"`. A `minutes` value of `0` or
/// below renders as `"due"`. An empty stop's second line is `"no buses"`.
#[must_use]
pub fn format_stop_line(stop: &StopArrivals) -> String {
    let arrivals = if stop.items.is_empty() {
        "no buses".to_owned()
    } else {
        stop.items
            .iter()
            .map(|item| {
                let when = if item.minutes <= 0 {
                    "due".to_owned()
                } else {
                    format!("{}m", item.minutes)
                };
                format!("{when} ({})", item.buses.join("·"))
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("{}\n{arrivals}", stop.name)
}

/// Build the full Live Update body for the active commute(s): one two-line
/// [`format_stop_line`] block per stop, newline-separated.
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
    use super::{
        format_active_notification, format_countdown, format_next_time, format_see_you_soon,
        format_stop_line, format_timeline_labels,
    };
    use crate::lta::arrival::{ArrivalItem, StopArrivals};
    use time::macros::datetime;

    #[test]
    fn next_time_is_short_weekday_and_time() {
        assert_eq!(
            format_next_time(datetime!(2026-06-23 08:00:00 +8)),
            "next Tue 08:00"
        );
    }

    #[test]
    fn countdown_formats_coarse_human_durations() {
        assert_eq!(format_countdown(0), "now");
        assert_eq!(format_countdown(45), "45m");
        assert_eq!(format_countdown(145), "2h 25m");
        assert_eq!(format_countdown(120), "2h");
        assert_eq!(format_countdown(1440), "1d");
        assert_eq!(format_countdown(1500), "1d 1h");
    }

    #[test]
    fn timeline_labels_span_now_to_now_plus_duration() {
        // 09:35 + 30m → 10:05.
        assert_eq!(
            format_timeline_labels(datetime!(2026-06-28 09:35:00 +8), 30),
            ("09:35".to_owned(), "10:05".to_owned())
        );
    }

    #[test]
    fn timeline_labels_zero_pad_and_wrap_midnight() {
        // 08:05 zero-pads; 23:50 + 30m wraps to 00:20.
        assert_eq!(
            format_timeline_labels(datetime!(2026-06-28 08:05:00 +8), 15),
            ("08:05".to_owned(), "08:20".to_owned())
        );
        assert_eq!(
            format_timeline_labels(datetime!(2026-06-28 23:50:00 +8), 30),
            ("23:50".to_owned(), "00:20".to_owned())
        );
    }

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
            "Opp Blk 123\n2m (14), 4m (14e·16), 11m (154)"
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
        assert_eq!(format_stop_line(&stop), "Opp Blk 123\ndue (14)");
    }

    #[test]
    fn stop_line_handles_no_buses() {
        let stop = StopArrivals {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            items: vec![],
        };
        assert_eq!(format_stop_line(&stop), "Opp Blk 123\nno buses");
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
            "Opp Blk 123\n2m (14)\nBef Clementi Stn\n8m (96)"
        );
    }
}
