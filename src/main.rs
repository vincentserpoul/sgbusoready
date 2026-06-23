//! SG Bus Ready — desktop entry point. Delegates to the shared library, storing
//! commutes under the user's data dir so they persist across runs (mirroring the
//! app-private `files/commutes.json` used on Android).

use std::path::PathBuf;

fn desktop_store_path() -> PathBuf {
    let mut dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.push("sgbusoready");
    dir.push("commutes.json");
    dir
}

fn main() -> Result<(), slint::PlatformError> {
    sgbusoready::run_app(desktop_store_path())
}
