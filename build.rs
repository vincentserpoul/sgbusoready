// Build script — allowed to use `expect` because a compile failure here must
// abort the build immediately; there is no meaningful recovery path.
#![allow(clippy::expect_used, reason = "build script abort-on-failure is correct")]

fn main() {
    slint_build::compile("ui/app.slint").expect("slint compile");
}
