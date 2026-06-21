//! Typed mirror of the `DataMall` Bus Arrival JSON response.
//!
//! Only the fields used by the app are modelled; unknown fields are ignored.

use serde::Deserialize;

/// Top-level Bus Arrival response for one bus stop.
#[derive(Debug, Clone, Deserialize)]
pub struct BusArrivalResponse {
    /// The queried 5-digit bus stop code.
    #[serde(rename = "BusStopCode")]
    pub bus_stop_code: String,
    /// One entry per bus service that calls at this stop.
    #[serde(rename = "Services")]
    pub services: Vec<Service>,
}

/// Arrival info for a single bus service at the stop.
///
/// # `NextBus` coupling
/// `DataMall` sends exactly three `NextBus` fields (`NextBus`, `NextBus2`,
/// `NextBus3`). The `service_arrivals` function in `arrival.rs` mirrors this
/// with a fixed 3-element slot array `[&svc.next_bus, &svc.next_bus2,
/// &svc.next_bus3]`. If the API ever gains a fourth bus (`NextBus4`), add a
/// `next_bus4` field here **and** extend that slot array in
/// `arrival.rs::service_arrivals` to keep both sides in sync.
#[derive(Debug, Clone, Deserialize)]
pub struct Service {
    /// The public service number, e.g. `"15"` or `"67"`.
    #[serde(rename = "ServiceNo")]
    pub service_no: String,
    /// The next bus to arrive.
    #[serde(rename = "NextBus")]
    pub next_bus: NextBus,
    /// The bus after that (fields may be empty late at night).
    #[serde(rename = "NextBus2")]
    pub next_bus2: NextBus,
    /// The third bus (fields may be empty late at night).
    #[serde(rename = "NextBus3")]
    pub next_bus3: NextBus,
}

/// One predicted arrival. `estimated_arrival` is empty when no bus is expected.
#[derive(Debug, Clone, Deserialize)]
pub struct NextBus {
    /// RFC3339 timestamp (e.g. `2026-06-21T08:18:00+08:00`), or `""` if none.
    #[serde(rename = "EstimatedArrival")]
    pub estimated_arrival: String,
}

#[cfg(test)]
mod tests {
    use super::BusArrivalResponse;

    /// A trimmed but structurally faithful `DataMall` response sample.
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
    fn parses_sample_response() {
        let parsed: BusArrivalResponse = serde_json::from_str(SAMPLE).expect("sample should parse");
        assert_eq!(parsed.bus_stop_code, "83139");
        assert_eq!(parsed.services.len(), 1);
        let svc = parsed.services.first().expect("one service");
        assert_eq!(svc.service_no, "15");
        assert_eq!(svc.next_bus.estimated_arrival, "2026-06-21T08:18:00+08:00");
        assert_eq!(svc.next_bus3.estimated_arrival, "");
    }
}
