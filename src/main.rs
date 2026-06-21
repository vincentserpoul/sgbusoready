//! SG Bus Ready — desktop entry point. Delegates to the shared library.

fn main() -> Result<(), slint::PlatformError> {
    sgbusoready::run_app()
}
