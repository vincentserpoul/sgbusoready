# Default recipe: list all available recipes
default:
    @just --list

# Format all code with cargo fmt (stable; toolchain is pinned in rust-toolchain.toml)
fmt:
    cargo fmt --all

# Check formatting without applying changes
fmt-check:
    cargo fmt --all -- --check

# Run clippy on the entire workspace (lints live in Cargo.toml [workspace.lints];
# the explicit flags mirror CI and keep parity if the table is ever loosened)
clippy:
    cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::todo -D clippy::unimplemented -W clippy::cognitive_complexity

# Run clippy and auto-fix where possible
clippy-fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged -- -D warnings

# Build the workspace (host)
build:
    cargo build --workspace --all-targets

# Build the workspace in release mode (host)
build-release:
    cargo build --workspace --release

# Type-check that the core crate cross-compiles to the mobile targets.
# (UI/bridge crates need NDK/Xcode; wire those into the plan.)
check-android:
    cargo check -p sgbr-core --target aarch64-linux-android

check-ios:
    cargo check -p sgbr-core --target aarch64-apple-ios

# Build the Android cdylib into android/app/src/main/jniLibs (arm64-v8a).
# Sources android/.env.build for SDK/NDK/ANDROID_JAR and the LTA key. Uses --lib
# to skip the desktop bin, which can't link Skia's `stdout` reference on Android.
build-android:
    #!/usr/bin/env bash
    set -euo pipefail
    source android/.env.build
    cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --lib

# Lint the Android target (the host clippy skips cfg(android) code). Run via
# cargo-ndk so the NDK C toolchain (for ring/skia) is configured.
clippy-android:
    #!/usr/bin/env bash
    set -euo pipefail
    source android/.env.build
    cargo ndk -t arm64-v8a clippy --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic

# Build the cdylib, assemble the debug APK, install on a connected device and launch.
run-android: build-android
    #!/usr/bin/env bash
    set -euo pipefail
    source android/.env.build
    ./android/gradlew -p android assembleDebug
    adb install -r android/app/build/outputs/apk/debug/app-debug.apk
    adb shell am start -n com.sgbuscommute/.MainActivity

# Run all tests with nextest
test:
    cargo nextest run --workspace

# Run all tests with standard cargo test (includes doctests)
test-doc:
    cargo test --workspace --doc

# Run a specific test by name
test-one NAME:
    cargo nextest run --workspace -E 'test({{ NAME }})'

# Check the workspace compiles without producing binaries
check:
    cargo check --workspace --all-targets

# Generate and open documentation
doc:
    cargo doc --workspace --no-deps --open

# Check docs build without warnings
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --quiet

# Run cargo audit for known vulnerabilities
audit:
    cargo audit

# Run cargo deny for license and advisory checks
deny:
    cargo deny check

# Detect unused dependencies
machete:
    cargo machete

# Supply-chain audit (cargo-vet)
vet:
    cargo vet

# Run code coverage with llvm-cov
coverage:
    cargo llvm-cov --workspace nextest

# Run code coverage and generate an HTML report
coverage-html:
    cargo llvm-cov --workspace nextest --html
    @echo "Report at target/llvm-cov/html/index.html"

# Clean build artifacts
clean:
    cargo clean

# Run the full CI-style check suite
ci: fmt-check clippy test doc-check deny audit machete

# Check, lint, and test (quick local iteration)
dev: check clippy test

# Update dependencies
update:
    cargo update

# Show the dependency tree
tree:
    cargo tree --workspace

# Run typos checker
typos:
    typos

# Format TOML files with taplo
taplo:
    taplo format

# Install zizmor (GitHub Actions security linter) locally via cargo
zizmor-install:
    cargo install zizmor --locked

# Run zizmor against all workflow files
zizmor:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v zizmor >/dev/null 2>&1; then
        echo "zizmor is not installed. Run 'just zizmor-install' first."
        exit 1
    fi
    zizmor .github/workflows/*.yml
