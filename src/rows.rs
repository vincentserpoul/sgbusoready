//! List rows and timeline lanes: building the Slint `CommuteRow`/`StopLane`
//! models, synchronously from the process-global arrivals feed (`feed.rs`) —
//! no network on the UI thread.

use std::path::Path;

use slint::{ModelRc, SharedString, VecModel};
use time::OffsetDateTime;

use sgbr_core::commute::display::{format_next_time, format_timeline_labels};
use sgbr_core::commute::model::Commute;
use sgbr_core::commute::schedule::active_stop_plans;
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::{ArrivalItem, StopArrivals};

use crate::feed::feed;
use crate::{
    AppWindow, ArrivalTag, CommuteRow, StopLane, WORKER_URL, credentials_path, load_credentials,
    now_sgt,
};

/// Card label: the commute's own label, or its first stop's name (+N).
fn card_label(commute: &Commute) -> String {
    commute.display_label()
}

/// The 7 weekday flags (Mon..Sun) of a commute's active days, for the day pills.
fn day_flags(commute: &Commute) -> Vec<bool> {
    use time::Weekday::{Friday, Monday, Saturday, Sunday, Thursday, Tuesday, Wednesday};
    [
        Monday, Tuesday, Wednesday, Thursday, Friday, Saturday, Sunday,
    ]
    .iter()
    .map(|d| commute.days.contains(*d))
    .collect()
}

/// "next <Day> HH:MM" for an inactive card, or empty if there is no next window.
fn next_time_line(commute: &Commute, now: OffsetDateTime) -> String {
    commute
        .next_window_start(now)
        .map(format_next_time)
        .unwrap_or_default()
}

/// "N stops · M buses" for an inactive card.
fn counts_line(commute: &Commute) -> String {
    let stops = commute.stops.len();
    let buses: usize = commute.stops.iter().map(|s| s.buses.len()).sum();
    format!("{stops} stops · {buses} buses")
}

fn empty_lanes() -> ModelRc<StopLane> {
    ModelRc::new(VecModel::from(Vec::<StopLane>::new()))
}

/// Skeleton lanes for an active commute: one lane per stop with its name but no
/// arrival tags yet, so the timeline structure shows immediately while the live
/// fetch (`spawn_arrivals`) is in flight.
fn skeleton_lanes(commute: &Commute, now: OffsetDateTime) -> ModelRc<StopLane> {
    let stops: Vec<StopArrivals> = commute
        .stops
        .iter()
        .map(|s| StopArrivals {
            code: s.code.clone(),
            name: s.name.clone(),
            items: Vec::new(),
        })
        .collect();
    let (start, end) = format_timeline_labels(now, commute.scale_minutes());
    lanes_model(&stops, i32::from(commute.scale_minutes()), &start, &end)
}

/// Build all list rows synchronously (no network): active rows get skeleton lanes
/// until `spawn_arrivals` fills in live tags; inactive rows get the summary line.
pub fn rebuild_rows(window: &AppWindow, store: &CommuteStore) {
    let now = now_sgt();
    let today = i32::from(now.weekday().number_days_from_monday());
    let mut rows: Vec<CommuteRow> = Vec::new();
    for (i, c) in store.commutes.iter().enumerate() {
        let active = c.is_active_at(now);
        rows.push(CommuteRow {
            label: SharedString::from(card_label(c)),
            active,
            index: i32::try_from(i).unwrap_or(-1),
            lanes: if active {
                skeleton_lanes(c, now)
            } else {
                empty_lanes()
            },
            scale_max: i32::from(c.scale_minutes()),
            days: ModelRc::new(VecModel::from(day_flags(c))),
            today,
            next_time: SharedString::from(next_time_line(c, now)),
            counts: SharedString::from(counts_line(c)),
        });
    }
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Keep only the arrival groups (and, within each group, the buses) that are
/// in `tracked`; drop a group left with no buses after filtering. Order is
/// preserved (the feed's snapshots are already ascending by minute).
fn filter_items(items: &[ArrivalItem], tracked: &[String]) -> Vec<ArrivalItem> {
    items
        .iter()
        .filter_map(|item| {
            let buses: Vec<String> = item
                .buses
                .iter()
                .filter(|b| tracked.contains(b))
                .cloned()
                .collect();
            if buses.is_empty() {
                None
            } else {
                Some(ArrivalItem {
                    minutes: item.minutes,
                    buses,
                })
            }
        })
        .collect()
}

/// Build one active commute's stop lanes from the feed's cached snapshots
/// (no network — the feed's background stream thread keeps them warm), each
/// filtered down to the buses that commute-stop actually tracks.
fn commute_stop_arrivals_from_feed(commute: &Commute) -> (Vec<StopArrivals>, i32) {
    let stops: Vec<StopArrivals> = commute
        .stops
        .iter()
        .map(|stop| StopArrivals {
            code: stop.code.clone(),
            name: stop.name.clone(),
            items: filter_items(&feed().items_for(&stop.code), &stop.buses),
        })
        .collect();
    (stops, i32::from(commute.scale_minutes()))
}

/// Assign each arrival a stagger row so pills that would overlap horizontally are
/// stacked on separate rows. Pill width is approximated as ~18% of the axis, so
/// arrivals closer than that many minutes go on different rows (capped at 3 rows;
/// extras reuse the top row). Returns the per-item rows and the row count (>= 1).
fn stagger_rows(minutes: &[i64], scale_max: u16) -> (Vec<i32>, i32) {
    const MAX_ROWS: usize = 3;
    // A pill occupies roughly this fraction of the axis width; arrivals closer
    // than that go on separate rows.
    let threshold = (i64::from(scale_max) * 22 / 100).max(1);
    // (original index, minute), processed in ascending-minute order.
    let mut order: Vec<(usize, i64)> = minutes.iter().copied().enumerate().collect();
    order.sort_by_key(|&(_, m)| m);
    let mut last: Vec<i64> = Vec::new(); // last minute placed on each row
    let mut rows = vec![0i32; minutes.len()];
    for (idx, m) in order {
        let free = last.iter().position(|&lastm| m - lastm >= threshold);
        let r = match free {
            Some(r) => {
                if let Some(l) = last.get_mut(r) {
                    *l = m;
                }
                r
            }
            None if last.len() < MAX_ROWS => {
                last.push(m);
                last.len() - 1
            }
            None => {
                let r = MAX_ROWS - 1;
                if let Some(l) = last.get_mut(r) {
                    *l = m;
                }
                r
            }
        };
        if let Some(slot) = rows.get_mut(idx) {
            *slot = i32::try_from(r).unwrap_or(0);
        }
    }
    (rows, i32::try_from(last.len().max(1)).unwrap_or(1))
}

/// Build the Slint timeline lanes for a commute's stops (UI thread — makes
/// `ModelRc`s). `scale_max` is the window duration in minutes, used to stagger
/// overlapping pills.
fn lanes_model(
    stops: &[StopArrivals],
    scale_max: i32,
    start_label: &str,
    end_label: &str,
) -> ModelRc<StopLane> {
    let scale = u16::try_from(scale_max.max(0)).unwrap_or(0);
    let lanes: Vec<StopLane> = stops
        .iter()
        .map(|sa| {
            let minutes: Vec<i64> = sa.items.iter().map(|it| it.minutes).collect();
            let (item_rows, row_count) = stagger_rows(&minutes, scale);
            StopLane {
                name: SharedString::from(sa.name.as_str()),
                code: SharedString::from(sa.code.as_str()),
                start_label: SharedString::from(start_label),
                end_label: SharedString::from(end_label),
                row_count,
                tags: ModelRc::new(VecModel::from(
                    sa.items
                        .iter()
                        .enumerate()
                        .map(|(i, it)| ArrivalTag {
                            buses: SharedString::from(it.buses.join("·")),
                            minutes: i32::try_from(it.minutes).unwrap_or(0),
                            row: item_rows.get(i).copied().unwrap_or(0),
                        })
                        .collect::<Vec<_>>(),
                )),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(lanes))
}

/// Recompute the desired stop/bus subscription set from the active commutes
/// and hand it to the arrivals feed, then rebuild every list row from the
/// feed's cached snapshots — synchronously, on the UI thread. No network
/// happens here: `feed().set_plans` only (re)starts/reconnects the
/// background stream thread when the plan actually changes; reading the
/// cache back is a couple of `HashMap` lookups.
pub fn refresh_arrivals(window: &AppWindow, store: &CommuteStore, store_path: &Path) {
    let now = now_sgt();
    let plans = active_stop_plans(&store.commutes, now);
    let creds = load_credentials(&credentials_path(store_path));
    feed().set_plans(plans, creds.as_ref(), WORKER_URL);

    let today = i32::from(now.weekday().number_days_from_monday());
    let mut rows: Vec<CommuteRow> = Vec::new();
    for (i, c) in store.commutes.iter().enumerate() {
        let active = c.is_active_at(now);
        let (stops, scale) = if active {
            commute_stop_arrivals_from_feed(c)
        } else {
            (Vec::new(), i32::from(c.scale_minutes()))
        };
        let (start_label, end_label) = if active {
            format_timeline_labels(now, c.scale_minutes())
        } else {
            (String::new(), String::new())
        };
        rows.push(CommuteRow {
            label: SharedString::from(card_label(c)),
            active,
            index: i32::try_from(i).unwrap_or(-1),
            lanes: lanes_model(&stops, scale, &start_label, &end_label),
            scale_max: scale,
            days: ModelRc::new(VecModel::from(day_flags(c))),
            today,
            next_time: SharedString::from(next_time_line(c, now)),
            counts: SharedString::from(counts_line(c)),
        });
    }
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}

#[cfg(test)]
mod tests {
    use super::{ArrivalItem, filter_items, stagger_rows};

    fn item(minutes: i64, buses: &[&str]) -> ArrivalItem {
        ArrivalItem {
            minutes,
            buses: buses.iter().map(|b| (*b).to_owned()).collect(),
        }
    }

    fn owned(buses: &[&str]) -> Vec<String> {
        buses.iter().map(|b| (*b).to_owned()).collect()
    }

    #[test]
    fn filter_items_keeps_only_tracked_buses_within_each_group() {
        let items = vec![item(3, &["14", "15"]), item(9, &["16"])];
        let filtered = filter_items(&items, &owned(&["15"]));
        assert_eq!(filtered, vec![item(3, &["15"])]);
    }

    #[test]
    fn filter_items_drops_a_group_left_with_no_tracked_buses() {
        let items = vec![item(3, &["14"]), item(9, &["15"])];
        let filtered = filter_items(&items, &owned(&["15"]));
        assert_eq!(filtered, vec![item(9, &["15"])]);
    }

    #[test]
    fn filter_items_preserves_order() {
        let items = vec![item(3, &["15"]), item(9, &["16"]), item(20, &["15"])];
        let filtered = filter_items(&items, &owned(&["15", "16"]));
        assert_eq!(
            filtered,
            vec![item(3, &["15"]), item(9, &["16"]), item(20, &["15"])]
        );
    }

    #[test]
    fn well_spread_arrivals_share_the_bottom_row() {
        // 30-min axis → ~5-min threshold; 0/7/20 are all far enough apart.
        let (rows, count) = stagger_rows(&[0, 7, 20], 30);
        assert_eq!(rows, vec![0, 0, 0]);
        assert_eq!(count, 1);
    }

    #[test]
    fn close_arrivals_stack_on_separate_rows() {
        // 90-min axis → ~16-min threshold; 3 and 7 are too close → row 0 and 1.
        let (rows, count) = stagger_rows(&[3, 7], 90);
        assert_eq!(rows, vec![0, 1]);
        assert_eq!(count, 2);
    }

    #[test]
    fn overflow_beyond_three_rows_reuses_the_top_row() {
        let (rows, count) = stagger_rows(&[0, 1, 2, 3], 90);
        assert_eq!(rows, vec![0, 1, 2, 2]);
        assert_eq!(count, 3);
    }

    #[test]
    fn empty_lane_reports_one_row() {
        let (rows, count) = stagger_rows(&[], 30);
        assert!(rows.is_empty());
        assert_eq!(count, 1);
    }
}
