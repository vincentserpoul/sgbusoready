//! Convert raw `EstimatedArrival` strings into whole-minute countdowns.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::CoreError;
use crate::lta::model::BusArrivalResponse;

/// Whole minutes from `now` until `estimated_arrival`.
///
/// Returns [`CoreError::NoArrival`] for an empty string, and
/// [`CoreError::InvalidTimestamp`] for a non-RFC3339 value. A bus already due
/// or just departed yields `0` or a negative number; the caller decides how to
/// present that.
pub fn minutes_until(estimated_arrival: &str, now: OffsetDateTime) -> Result<i64, CoreError> {
    if estimated_arrival.is_empty() {
        return Err(CoreError::NoArrival);
    }
    let eta = OffsetDateTime::parse(estimated_arrival, &Rfc3339)
        .map_err(|e| CoreError::InvalidTimestamp(e.to_string()))?;
    Ok((eta - now).whole_minutes())
}

/// One service's next arrivals, reduced to whole-minute countdowns, ready for
/// display. `minutes` holds up to three entries; empty/invalid slots are
/// dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceArrivals {
    /// Public service number, e.g. `"15"`.
    pub service_no: String,
    /// Whole-minute countdowns for the next buses (0–3 entries).
    pub minutes: Vec<i64>,
}

/// Build display-ready arrivals for every service in a response, relative to
/// `now`. Slots with no/invalid timestamps are skipped (not errors).
#[must_use]
pub fn service_arrivals(
    response: &BusArrivalResponse,
    now: OffsetDateTime,
) -> Vec<ServiceArrivals> {
    response
        .services
        .iter()
        .map(|svc| {
            let slots = [&svc.next_bus, &svc.next_bus2, &svc.next_bus3];
            let minutes = slots
                .into_iter()
                .filter_map(|b| minutes_until(&b.estimated_arrival, now).ok())
                .collect();
            ServiceArrivals {
                service_no: svc.service_no.clone(),
                minutes,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::minutes_until;
    use crate::error::CoreError;
    use time::macros::datetime;

    #[test]
    fn computes_future_minutes() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        let mins = minutes_until("2026-06-21T08:18:00+08:00", now).expect("valid future arrival");
        assert_eq!(mins, 8);
    }

    #[test]
    fn truncates_to_whole_minutes() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        // 8m30s away -> 8 whole minutes.
        let mins = minutes_until("2026-06-21T08:18:30+08:00", now).expect("valid arrival");
        assert_eq!(mins, 8);
    }

    #[test]
    fn handles_different_offset_same_instant() {
        // now expressed in +08:00; arrival expressed in UTC for the same wall
        // clock instant 08:18 SGT == 00:18 UTC.
        let now = datetime!(2026-06-21 08:10:00 +8);
        let mins = minutes_until("2026-06-21T00:18:00+00:00", now).expect("valid arrival");
        assert_eq!(mins, 8);
    }

    #[test]
    fn empty_string_is_no_arrival() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        let err = minutes_until("", now).unwrap_err();
        assert!(matches!(err, CoreError::NoArrival));
    }

    #[test]
    fn garbage_is_invalid_timestamp() {
        let now = datetime!(2026-06-21 08:10:00 +8);
        let err = minutes_until("not-a-date", now).unwrap_err();
        assert!(matches!(err, CoreError::InvalidTimestamp(_)));
    }

    #[test]
    fn past_arrival_is_negative() {
        let now = datetime!(2026-06-21 08:20:00 +8);
        let mins = minutes_until("2026-06-21T08:18:00+08:00", now).expect("valid past arrival");
        assert_eq!(mins, -2);
    }
}

#[cfg(test)]
mod view_model_tests {
    use super::{ServiceArrivals, service_arrivals};
    use crate::lta::model::BusArrivalResponse;
    use time::macros::datetime;

    const SAMPLE: &str = r#"{
      "BusStopCode": "83139",
      "Services": [
        {
          "ServiceNo": "15",
          "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
          "NextBus2": { "EstimatedArrival": "2026-06-21T08:25:00+08:00" },
          "NextBus3": { "EstimatedArrival": "" }
        }
      ]
    }"#;

    #[test]
    fn drops_empty_slots_and_keeps_order() {
        let resp: BusArrivalResponse = serde_json::from_str(SAMPLE).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let out = service_arrivals(&resp, now);
        assert_eq!(
            out,
            vec![ServiceArrivals {
                service_no: "15".to_owned(),
                minutes: vec![8, 15],
            }]
        );
    }

    #[test]
    fn two_services_preserved_in_order() {
        const TWO_SERVICES: &str = r#"{
          "BusStopCode": "83139",
          "Services": [
            {
              "ServiceNo": "15",
              "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
              "NextBus2": { "EstimatedArrival": "" },
              "NextBus3": { "EstimatedArrival": "" }
            },
            {
              "ServiceNo": "65",
              "NextBus":  { "EstimatedArrival": "2026-06-21T08:22:00+08:00" },
              "NextBus2": { "EstimatedArrival": "2026-06-21T08:30:00+08:00" },
              "NextBus3": { "EstimatedArrival": "" }
            }
          ]
        }"#;
        let resp: BusArrivalResponse = serde_json::from_str(TWO_SERVICES).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let out = service_arrivals(&resp, now);
        assert_eq!(out.len(), 2);
        let mut iter = out.into_iter();
        assert_eq!(
            iter.next().expect("first service"),
            ServiceArrivals {
                service_no: "15".to_owned(),
                minutes: vec![8],
            }
        );
        assert_eq!(
            iter.next().expect("second service"),
            ServiceArrivals {
                service_no: "65".to_owned(),
                minutes: vec![12, 20],
            }
        );
    }

    #[test]
    fn all_empty_slots_yields_empty_minutes() {
        const ALL_EMPTY: &str = r#"{
          "BusStopCode": "83139",
          "Services": [
            {
              "ServiceNo": "99",
              "NextBus":  { "EstimatedArrival": "" },
              "NextBus2": { "EstimatedArrival": "" },
              "NextBus3": { "EstimatedArrival": "" }
            }
          ]
        }"#;
        let resp: BusArrivalResponse = serde_json::from_str(ALL_EMPTY).expect("parse");
        let now = datetime!(2026-06-21 08:10:00 +8);
        let out = service_arrivals(&resp, now);
        assert_eq!(
            out,
            vec![ServiceArrivals {
                service_no: "99".to_owned(),
                minutes: vec![],
            }]
        );
    }
}
