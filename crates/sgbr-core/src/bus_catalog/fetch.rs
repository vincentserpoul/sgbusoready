//! Fetch the full catalog from LTA `DataMall` (paginated `OData`), then assemble it.
//! Pagination is sequential (page count is unknown up front); this runs on a
//! background thread in the app, so wall-time isn't on any UI path.

use std::time::Duration;

use serde::Deserialize;
use time::OffsetDateTime;
use ureq::Agent;

use crate::bus_catalog::model::{BusCatalog, BusStop};
use crate::bus_catalog::parse::assemble_catalog;
use crate::error::CoreError;

const BUS_STOPS_URL: &str = "https://datamall2.mytransport.sg/ltaodataservice/BusStops";
const BUS_ROUTES_URL: &str = "https://datamall2.mytransport.sg/ltaodataservice/BusRoutes";
const PAGE_SIZE: usize = 500;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
struct StopsPage {
    value: Vec<RawStop>,
}

#[derive(Deserialize)]
struct RawStop {
    #[serde(rename = "BusStopCode")]
    code: String,
    #[serde(rename = "Description")]
    name: String,
    #[serde(rename = "RoadName")]
    road: String,
}

/// Parse one `BusStops` `OData` page into [`BusStop`]s.
fn parse_stops_page(json: &str) -> Result<Vec<BusStop>, CoreError> {
    let page: StopsPage =
        serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(page
        .value
        .into_iter()
        .map(|r| BusStop {
            code: r.code,
            name: r.name,
            road: r.road,
        })
        .collect())
}

#[derive(Deserialize)]
struct RoutesPage {
    value: Vec<RawRoute>,
}

#[derive(Deserialize)]
struct RawRoute {
    #[serde(rename = "ServiceNo")]
    service: String,
    #[serde(rename = "BusStopCode")]
    stop: String,
}

/// Parse one `BusRoutes` `OData` page into `(stop_code, service_no)` pairs.
fn parse_routes_page(json: &str) -> Result<Vec<(String, String)>, CoreError> {
    let page: RoutesPage =
        serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(page
        .value
        .into_iter()
        .map(|r| (r.stop, r.service))
        .collect())
}

/// `OData` page URL: `{base}?$skip={skip}`.
#[must_use]
pub fn page_url(base: &str, skip: usize) -> String {
    format!("{base}?$skip={skip}")
}

fn fetch_page(agent: &Agent, account_key: &str, url: &str) -> Result<String, CoreError> {
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
    agent: &Agent,
    account_key: &str,
    base: &str,
    parse: impl Fn(&str) -> Result<Vec<T>, CoreError>,
) -> Result<Vec<T>, CoreError> {
    let mut all: Vec<T> = Vec::new();
    let mut skip = 0;
    loop {
        let json = fetch_page(agent, account_key, &page_url(base, skip))?;
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
    let agent = Agent::new_with_config(
        Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build(),
    );
    let stops = fetch_all(&agent, account_key, BUS_STOPS_URL, parse_stops_page)?;
    let pairs = fetch_all(&agent, account_key, BUS_ROUTES_URL, parse_routes_page)?;
    Ok(assemble_catalog(stops, pairs, now))
}

#[cfg(test)]
mod tests {
    use super::{BUS_STOPS_URL, page_url, parse_routes_page, parse_stops_page};

    #[test]
    fn builds_skip_url() {
        assert_eq!(
            page_url(BUS_STOPS_URL, 500),
            "https://datamall2.mytransport.sg/ltaodataservice/BusStops?$skip=500"
        );
    }

    #[test]
    fn parses_stops_page() {
        let json = r#"{"value":[
            {"BusStopCode":"01012","RoadName":"Victoria St","Description":"Hotel Grand Pacific","Latitude":1.0,"Longitude":2.0}
        ]}"#;
        let stops = parse_stops_page(json).expect("parse stops");
        assert_eq!(stops.len(), 1);
        assert_eq!(
            stops.first().map(|s| s.name.as_str()),
            Some("Hotel Grand Pacific")
        );
        assert_eq!(stops.first().map(|s| s.road.as_str()), Some("Victoria St"));
    }

    #[test]
    fn parses_routes_page() {
        let json = r#"{"value":[
            {"ServiceNo":"15","BusStopCode":"83139","Direction":1,"StopSequence":5},
            {"ServiceNo":"52","BusStopCode":"83139","Direction":1,"StopSequence":9}
        ]}"#;
        let pairs = parse_routes_page(json).expect("parse routes");
        assert_eq!(
            pairs,
            vec![
                ("83139".to_owned(), "15".to_owned()),
                ("83139".to_owned(), "52".to_owned())
            ]
        );
    }
}
