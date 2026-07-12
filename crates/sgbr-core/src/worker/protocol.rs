//! Wire DTOs shared between the app and the worker's HTTP API.

use serde::{Deserialize, Serialize};

use crate::commute::schedule::StopPlan;
use crate::lta::arrival::ArrivalItem;

/// One stop's current arrival state, pushed as an `event: arrivals` SSE frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopUpdate {
    pub code: String,
    pub items: Vec<ArrivalItem>,
}

/// Body of `POST /v1/devices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub invite_code: String,
    pub pubkey: String,
    pub signature: String,
}

/// `201` response body of `POST /v1/devices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub token: String,
    pub device_id: String,
}

/// Format the `stops` query parameter for `GET /v1/arrivals`:
/// `"{code}:{svc}.{svc},{code}:{svc}"`.
#[must_use]
pub fn stream_query(plans: &[StopPlan]) -> String {
    plans
        .iter()
        .map(|plan| format!("{}:{}", plan.code, plan.buses.join(".")))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::stream_query;
    use crate::commute::schedule::StopPlan;

    #[test]
    fn formats_multi_stop_multi_service() {
        let plans = vec![
            StopPlan {
                code: "83139".to_owned(),
                name: "Opp Blk 123".to_owned(),
                buses: vec!["15".to_owned(), "14e".to_owned()],
            },
            StopPlan {
                code: "83141".to_owned(),
                name: "Aft Blk 456".to_owned(),
                buses: vec!["16".to_owned()],
            },
        ];
        assert_eq!(stream_query(&plans), "83139:15.14e,83141:16");
    }

    #[test]
    fn single_stop_single_service() {
        let plans = vec![StopPlan {
            code: "83139".to_owned(),
            name: "Opp Blk 123".to_owned(),
            buses: vec!["15".to_owned()],
        }];
        assert_eq!(stream_query(&plans), "83139:15");
    }

    #[test]
    fn empty_plans_is_empty_string() {
        assert_eq!(stream_query(&[]), "");
    }
}
