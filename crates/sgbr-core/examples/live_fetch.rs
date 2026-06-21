//! Manual smoke test: `LTA_ACCOUNT_KEY=... cargo run -p sgbr-core --example live_fetch -- 83139`
//!
//! Prints the next-bus countdowns for one stop using live DataMall data.
#![allow(clippy::print_stdout, reason = "manual example binary")]

use std::env;

use sgbr_core::lta::arrival::service_arrivals;
use sgbr_core::lta::client::fetch_arrivals;
use time::OffsetDateTime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = env::var("LTA_ACCOUNT_KEY")
        .map_err(|_| "set LTA_ACCOUNT_KEY in the environment")?;
    let stop = env::args().nth(1).unwrap_or_else(|| "83139".to_owned());

    let response = fetch_arrivals(&key, &stop)?;
    let now = OffsetDateTime::now_utc();
    for svc in service_arrivals(&response, now) {
        // `print_stdout` is denied in library code, but examples are binaries
        // outside the workspace lint table; this is fine here.
        println!("bus {}: {:?} min", svc.service_no, svc.minutes);
    }
    Ok(())
}
