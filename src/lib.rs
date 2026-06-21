//! SG Bus Ready — shared app entry. The desktop binary and the Android cdylib
//! both call [`run_app`]; only the platform entry points differ.

mod generated {
    #![allow(trivial_numeric_casts, reason = "Slint-generated code")]
    #![allow(
        missing_debug_implementations,
        reason = "Slint-generated types do not derive Debug"
    )]
    #![allow(
        clippy::unwrap_used,
        reason = "Slint-generated code uses unwrap internally"
    )]
    #![allow(
        clippy::expect_used,
        reason = "Slint-generated code uses expect internally"
    )]
    #![allow(clippy::panic, reason = "Slint-generated code uses panic internally")]
    #![allow(
        clippy::indexing_slicing,
        reason = "Slint-generated code uses indexing internally"
    )]
    #![allow(
        clippy::float_arithmetic,
        reason = "Slint-generated code uses float arithmetic internally"
    )]
    #![allow(
        clippy::semicolon_outside_block,
        reason = "Slint-generated code formatting"
    )]
    #![allow(
        clippy::clone_on_ref_ptr,
        reason = "Slint-generated code clones ref-counted pointers"
    )]
    #![allow(clippy::todo, reason = "Slint-generated code contains todo! stubs")]
    slint::include_modules!();
}

use generated::{AppWindow, ServiceRow};

use sgbr_core::lta::arrival::{ServiceArrivals, service_arrivals};
use sgbr_core::lta::model::BusArrivalResponse;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

const SAMPLE: &str = r#"{
  "BusStopCode": "83139",
  "Services": [
    { "ServiceNo": "15",
      "NextBus":  { "EstimatedArrival": "2026-06-21T08:18:00+08:00" },
      "NextBus2": { "EstimatedArrival": "2026-06-21T08:25:00+08:00" },
      "NextBus3": { "EstimatedArrival": "" } }
  ]
}"#;

fn timing_label(arrivals: &ServiceArrivals) -> String {
    if arrivals.minutes.is_empty() {
        return "no service".to_owned();
    }
    arrivals
        .minutes
        .iter()
        .map(|m| {
            if *m <= 0 {
                "Arr".to_owned()
            } else {
                format!("{m} min")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build the window from the fixed sample and run the Slint event loop.
///
/// # Errors
/// Returns any [`slint::PlatformError`] from window creation or the event loop.
pub fn run_app() -> Result<(), slint::PlatformError> {
    // Fixed reference time so the sample always shows positive countdowns.
    let now = time::macros::datetime!(2026-06-21 08:10:00 +8);
    let response: BusArrivalResponse = serde_json::from_str(SAMPLE).unwrap_or(BusArrivalResponse {
        bus_stop_code: String::new(),
        services: Vec::new(),
    });

    let rows: Vec<ServiceRow> = service_arrivals(&response, now)
        .iter()
        .map(|a| ServiceRow {
            service_no: SharedString::from(a.service_no.as_str()),
            timing: SharedString::from(timing_label(a).as_str()),
        })
        .collect();

    let window = AppWindow::new()?;
    window.set_rows(ModelRc::new(VecModel::from(rows)));
    window.run()
}
