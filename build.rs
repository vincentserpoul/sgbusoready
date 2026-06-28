// Build script — allowed to use `expect` because a compile failure here must
// abort the build immediately; there is no meaningful recovery path.
#![allow(
    clippy::expect_used,
    reason = "build script abort-on-failure is correct"
)]

fn main() {
    // The Android bridge bakes the LTA AccountKey in via `option_env!`, which the
    // compiler does NOT otherwise treat as a build input — without this, changing
    // the key won't trigger a rebuild of the crate that reads it.
    println!("cargo:rerun-if-env-changed=LTA_API_ACCOUNT_KEY");
    slint_build::compile("ui/app.slint").expect("slint compile");
}
