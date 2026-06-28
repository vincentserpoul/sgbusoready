//! List rows and timeline lanes: building the Slint `CommuteRow`/`StopLane`
//! models, synchronously (skeletons) and from live arrivals (off-thread fetch).

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use time::OffsetDateTime;

use sgbr_core::commute::display::format_see_you_soon;
use sgbr_core::commute::model::Commute;
use sgbr_core::commute::store::CommuteStore;
use sgbr_core::lta::arrival::{StopArrivals, stop_arrivals};
use sgbr_core::lta::client::fetch_arrivals;

use crate::{ACCOUNT_KEY, AppWindow, ArrivalTag, CommuteRow, StopLane, now_sgt};

/// Card label: the commute's own label, or its first stop's name (+N).
fn card_label(commute: &Commute) -> String {
    commute.display_label()
}

fn see_you_soon(commute: &Commute, now: OffsetDateTime) -> String {
    commute
        .next_window_start(now)
        .map(format_see_you_soon)
        .unwrap_or_default()
}

/// Off-window summary line, e.g. "see you soon · next Mon 08:00 · 2 stops · 4 buses".
fn inactive_summary(commute: &Commute, now: OffsetDateTime) -> String {
    let stops = commute.stops.len();
    let buses: usize = commute.stops.iter().map(|s| s.buses.len()).sum();
    let see = see_you_soon(commute, now);
    format!("{see} · {stops} stops · {buses} buses")
}

fn empty_lanes() -> ModelRc<StopLane> {
    ModelRc::new(VecModel::from(Vec::<StopLane>::new()))
}

/// Skeleton lanes for an active commute: one lane per stop with its name but no
/// arrival tags yet, so the timeline structure shows immediately while the live
/// fetch (`spawn_arrivals`) is in flight.
fn skeleton_lanes(commute: &Commute) -> ModelRc<StopLane> {
    let stops: Vec<StopArrivals> = commute
        .stops
        .iter()
        .map(|s| StopArrivals {
            code: s.code.clone(),
            name: s.name.clone(),
            items: Vec::new(),
        })
        .collect();
    lanes_model(&stops)
}

/// Build all list rows synchronously (no network): active rows get skeleton lanes
/// until `spawn_arrivals` fills in live tags; inactive rows get the summary line.
pub fn rebuild_rows(window: &AppWindow, store: &CommuteStore) {
    let now = now_sgt();
    let mut rows: Vec<CommuteRow> = Vec::new();
    for (i, c) in store.commutes.iter().enumerate() {
        let active = c.is_active_at(now);
        rows.push(CommuteRow {
            label: SharedString::from(card_label(c)),
            status: SharedString::from(if active {
                String::new()
            } else {
                inactive_summary(c, now)
            }),
            active,
            index: i32::try_from(i).unwrap_or(-1),
            lanes: if active {
                skeleton_lanes(c)
            } else {
                empty_lanes()
            },
            scale_max: i32::from(c.scale_minutes),
        });
    }
    window.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Plain, `Send` row data computed off the UI thread; converted to the Slint
/// `CommuteRow` (which holds non-`Send` `ModelRc`s) back on the UI thread.
struct RowData {
    label: String,
    status: String,
    active: bool,
    index: i32,
    stops: Vec<StopArrivals>,
    scale: i32,
}

/// Fetch each stop's arrivals for one active commute (blocking; off-UI only) and
/// the commute's fixed timeline scale. One `fetch_arrivals` per stop, filtered to
/// buses.
fn commute_stop_arrivals(commute: &Commute, now: OffsetDateTime) -> (Vec<StopArrivals>, i32) {
    let mut all: Vec<StopArrivals> = Vec::new();
    for stop in &commute.stops {
        let arrivals = match fetch_arrivals(ACCOUNT_KEY, &stop.code) {
            Ok(resp) => stop_arrivals(&stop.code, &stop.name, &stop.buses, &resp, now),
            Err(_) => StopArrivals {
                code: stop.code.clone(),
                name: stop.name.clone(),
                items: Vec::new(),
            },
        };
        all.push(arrivals);
    }
    (all, i32::from(commute.scale_minutes))
}

/// Build the Slint timeline lanes for a commute's stops (UI thread — makes `ModelRc`s).
fn lanes_model(stops: &[StopArrivals]) -> ModelRc<StopLane> {
    let lanes: Vec<StopLane> = stops
        .iter()
        .map(|sa| StopLane {
            name: SharedString::from(sa.name.as_str()),
            code: SharedString::from(sa.code.as_str()),
            tags: ModelRc::new(VecModel::from(
                sa.items
                    .iter()
                    .map(|it| ArrivalTag {
                        buses: SharedString::from(it.buses.join("·")),
                        minutes: i32::try_from(it.minutes).unwrap_or(0),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    ModelRc::new(VecModel::from(lanes))
}

/// For each active commute, fetch live arrivals on a background thread and
/// replace the list rows with timeline lanes (no-op without a key / when none
/// are active). Inactive rows keep their summary line.
pub fn spawn_arrivals(window: &AppWindow, store: &CommuteStore) {
    if ACCOUNT_KEY.is_empty() {
        return;
    }
    let now = now_sgt();
    if !store.commutes.iter().any(|c| c.is_active_at(now)) {
        return;
    }
    let commutes = store.commutes.clone();
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let now = now_sgt();
        let mut data: Vec<RowData> = Vec::new();
        for (i, c) in commutes.iter().enumerate() {
            let active = c.is_active_at(now);
            let (stops, scale) = if active {
                commute_stop_arrivals(c, now)
            } else {
                (Vec::new(), 15)
            };
            data.push(RowData {
                label: card_label(c),
                status: if active {
                    String::new()
                } else {
                    inactive_summary(c, now)
                },
                active,
                index: i32::try_from(i).unwrap_or(-1),
                stops,
                scale,
            });
        }
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = weak.upgrade() {
                let rows: Vec<CommuteRow> = data
                    .into_iter()
                    .map(|d| CommuteRow {
                        label: SharedString::from(d.label),
                        status: SharedString::from(d.status),
                        active: d.active,
                        index: d.index,
                        lanes: lanes_model(&d.stops),
                        scale_max: d.scale,
                    })
                    .collect();
                w.set_rows(ModelRc::new(VecModel::from(rows)));
            }
        });
    });
}
