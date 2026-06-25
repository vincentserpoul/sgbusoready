//! Parse LTA `OData` pages (`{"value":[...]}`) and invert routes into a map.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::bus_catalog::model::BusStop;
use crate::error::CoreError;

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

/// Parse one `BusStops` page into [`BusStop`]s.
pub fn parse_stops_page(json: &str) -> Result<Vec<BusStop>, CoreError> {
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

/// Parse one `BusRoutes` page into `(stop_code, service_no)` pairs.
pub fn parse_routes_page(json: &str) -> Result<Vec<(String, String)>, CoreError> {
    let page: RoutesPage =
        serde_json::from_str(json).map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(page
        .value
        .into_iter()
        .map(|r| (r.stop, r.service))
        .collect())
}

/// Sort key for a service number: leading digits as a number, then the whole
/// string — so "2" < "15" < "151" < "151A", and non-numeric (e.g. "NR7") sort last.
pub fn service_sort_key(service: &str) -> (u32, String) {
    let digits: String = service.chars().take_while(char::is_ascii_digit).collect();
    let num = digits.parse::<u32>().unwrap_or(u32::MAX);
    (num, service.to_owned())
}

/// Invert `(stop, service)` pairs into `stop → sorted, deduped services`.
pub fn build_services_by_stop(pairs: Vec<(String, String)>) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (stop, service) in pairs {
        map.entry(stop).or_default().push(service);
    }
    for services in map.values_mut() {
        services.sort_by_key(|s| service_sort_key(s));
        services.dedup();
    }
    map
}

#[cfg(test)]
mod tests {
    use super::{build_services_by_stop, parse_routes_page, parse_stops_page, service_sort_key};

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

    #[test]
    fn service_sort_is_numeric_aware() {
        let mut v = vec![
            "151".to_owned(),
            "2".to_owned(),
            "15".to_owned(),
            "151A".to_owned(),
        ];
        v.sort_by_key(|s| service_sort_key(s));
        assert_eq!(v, vec!["2", "15", "151", "151A"]);
    }

    #[test]
    fn service_sort_non_numeric_sorts_last() {
        let mut v = vec!["NR7".to_owned(), "2".to_owned(), "15".to_owned()];
        v.sort_by_key(|s| service_sort_key(s));
        assert_eq!(v, vec!["2", "15", "NR7"]);
    }

    #[test]
    fn inverts_and_dedups() {
        let pairs = vec![
            ("83139".to_owned(), "52".to_owned()),
            ("83139".to_owned(), "15".to_owned()),
            ("83139".to_owned(), "15".to_owned()),
        ];
        let map = build_services_by_stop(pairs);
        assert_eq!(
            map.get("83139"),
            Some(&vec!["15".to_owned(), "52".to_owned()])
        );
    }
}
