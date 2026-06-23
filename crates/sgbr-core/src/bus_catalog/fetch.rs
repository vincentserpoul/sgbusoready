//! Fetch the full catalog from LTA `DataMall` (paginated `OData`), then assemble it.
//! Pagination is sequential (page count is unknown up front); this runs on a
//! background thread in the app, so wall-time isn't on any UI path.

use std::time::Duration;

use time::OffsetDateTime;
use ureq::Agent;

use crate::bus_catalog::model::BusCatalog;
use crate::bus_catalog::parse::{build_services_by_stop, parse_routes_page, parse_stops_page};
use crate::error::CoreError;

const BUS_STOPS_URL: &str = "https://datamall2.mytransport.sg/ltaodataservice/BusStops";
const BUS_ROUTES_URL: &str = "https://datamall2.mytransport.sg/ltaodataservice/BusRoutes";
const PAGE_SIZE: usize = 500;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// `OData` page URL: `{base}?$skip={skip}`.
#[must_use]
pub fn page_url(base: &str, skip: usize) -> String {
    format!("{base}?$skip={skip}")
}

fn fetch_page(account_key: &str, url: &str) -> Result<String, CoreError> {
    let config = Agent::config_builder().timeout_global(Some(REQUEST_TIMEOUT)).build();
    let agent = Agent::new_with_config(config);
    agent
        .get(url)
        .header("AccountKey", account_key)
        .header("accept", "application/json")
        .call()
        .map_err(|e| CoreError::Http(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| CoreError::Http(e.to_string()))
}

fn fetch_all<T>(
    account_key: &str,
    base: &str,
    parse: impl Fn(&str) -> Result<Vec<T>, CoreError>,
) -> Result<Vec<T>, CoreError> {
    let mut all: Vec<T> = Vec::new();
    let mut skip = 0;
    loop {
        let json = fetch_page(account_key, &page_url(base, skip))?;
        let page = parse(&json)?;
        let count = page.len();
        all.extend(page);
        if count < PAGE_SIZE {
            break;
        }
        skip += PAGE_SIZE;
    }
    Ok(all)
}

/// Fetch and assemble the whole catalog; `now` stamps `fetched_at_unix`.
///
/// # Errors
/// Returns [`CoreError::Http`]/[`CoreError::Parse`] on any page failure (partial
/// data is discarded — the caller keeps its existing cache).
pub fn fetch_catalog(account_key: &str, now: OffsetDateTime) -> Result<BusCatalog, CoreError> {
    let stops = fetch_all(account_key, BUS_STOPS_URL, parse_stops_page)?;
    let pairs = fetch_all(account_key, BUS_ROUTES_URL, parse_routes_page)?;
    Ok(BusCatalog {
        stops,
        services_by_stop: build_services_by_stop(pairs),
        fetched_at_unix: now.unix_timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::{page_url, BUS_STOPS_URL};

    #[test]
    fn builds_skip_url() {
        assert_eq!(
            page_url(BUS_STOPS_URL, 500),
            "https://datamall2.mytransport.sg/ltaodataservice/BusStops?$skip=500"
        );
    }
}
