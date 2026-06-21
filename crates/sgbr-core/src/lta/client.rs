//! Blocking client for the `DataMall` Bus Arrival endpoint.

use crate::error::CoreError;
use crate::lta::model::BusArrivalResponse;

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
/// # Errors
/// Returns [`CoreError::Http`] on transport/status failure and
/// [`CoreError::Parse`] when the body is not the expected JSON.
pub fn fetch_arrivals(
    account_key: &str,
    bus_stop_code: &str,
) -> Result<BusArrivalResponse, CoreError> {
    let url = arrival_url(BUS_ARRIVAL_URL, bus_stop_code);
    let body = ureq::get(&url)
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
