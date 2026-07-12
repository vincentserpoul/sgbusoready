// Build script — allowed to use `expect` because a compile failure here must
// abort the build immediately; there is no meaningful recovery path.
#![allow(
    clippy::expect_used,
    reason = "build script abort-on-failure is correct"
)]

fn main() {
    // `lib.rs` bakes the worker base URL in via `option_env!`, which the
    // compiler does NOT otherwise treat as a build input — without this, changing
    // it won't trigger a rebuild of the crate that reads it.
    println!("cargo:rerun-if-env-changed=SGBR_WORKER_URL");
    slint_build::compile("ui/app.slint").expect("slint compile");
}
