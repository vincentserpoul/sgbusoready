//! Blocking client for the `DataMall` Bus Arrival endpoint.

use std::time::Duration;

use ureq::Agent;

use crate::error::CoreError;
use crate::lta::model::BusArrivalResponse;

/// Global request timeout applied to every `fetch_arrivals` call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Base URL for the Bus Arrival v2 endpoint (no query string).
pub const BUS_ARRIVAL_URL: &str =
    "https://datamall2.mytransport.sg/ltaodataservice/BusArrivalv2";

/// Build the full request URL for a given stop code.
#[must_use]
pub fn arrival_url(base: &str, bus_stop_code: &str) -> String {
    format!("{base}?BusStopCode={bus_stop_code}")
}

/// Fetch and parse live arrivals for `bus_stop_code` using `account_key`.
///
/// The underlying HTTP request is bounded by a 10-second global timeout; a
/// stalled connection will be aborted rather than hanging the calling thread.
///
/// # Errors
/// Returns [`CoreError::Http`] on transport/status failure and
/// [`CoreError::Parse`] when the body is not the expected JSON.
pub fn fetch_arrivals(
    account_key: &str,
    bus_stop_code: &str,
) -> Result<BusArrivalResponse, CoreError> {
    let config = Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    let agent = Agent::new_with_config(config);
    let url = arrival_url(BUS_ARRIVAL_URL, bus_stop_code);
    let body = agent
        .get(&url)
        .header("AccountKey", account_key)
        .header("accept", "application/json")
        .call()
        .map_err(|e| CoreError::Http(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| CoreError::Http(e.to_string()))?;
    serde_json::from_str(&body).map_err(|e| CoreError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{arrival_url, BUS_ARRIVAL_URL};

    #[test]
    fn builds_query_url() {
        let url = arrival_url(BUS_ARRIVAL_URL, "83139");
        assert_eq!(
            url,
            "https://datamall2.mytransport.sg/ltaodataservice/BusArrivalv2?BusStopCode=83139"
        );
    }
}
