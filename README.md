# SG Bus Ready

[![CI](https://github.com/vincentserpoul/sgbusoready/actions/workflows/ci.yml/badge.svg)](https://github.com/vincentserpoul/sgbusoready/actions/workflows/ci.yml)
[![Security](https://github.com/vincentserpoul/sgbusoready/actions/workflows/security.yml/badge.svg)](https://github.com/vincentserpoul/sgbusoready/actions/workflows/security.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)

Live Singapore bus arrivals for your saved commutes. Built with Rust and Slint, for desktop and Android.

## How it works

- **App**: a Slint UI. Commutes (label, days, time window, stops) are stored on the device and never leave it.
- **Worker**: a small axum backend that holds the LTA `DataMall` `AccountKey`, fetches arrivals once for all devices, and pushes them over Server-Sent Events.

## Project Structure

```text
sgbusoready/
├── crates/
│   ├── sgbr-core/      # Core library (LTA client, bus catalog, commutes, worker protocol)
│   └── sgbr-worker/    # axum backend
├── src/                # Slint app (desktop binary + Android cdylib)
├── ui/                 # Slint markup
├── android/            # Kotlin glue and Gradle project
├── deploy/             # Worker deploy runbook and Podman quadlets
└── Cargo.toml          # Workspace configuration
```

## Development

### Prerequisites

- Rust, at the version pinned in `rust-toolchain.toml` (rustup installs it automatically)
- [`just`](https://github.com/casey/just)
- [`prek`](https://github.com/j178/prek) for the git hooks
- `cargo-nextest`, `cargo-deny`, `cargo-audit` and `cargo-machete` for `just ci`
- On Linux, `libfontconfig1-dev` and `pkg-config` for the Slint desktop build

### Common tasks

```bash
just setup    # One-time: install the git hooks
just dev      # check + clippy + test
just ci       # Everything CI runs locally
just hooks    # Run all pre-commit hooks on the whole repo
cargo run     # Run the desktop app
```

Run `just` with no arguments to list every recipe. The worker is deployed following [`deploy/README.md`](deploy/README.md).

## Code Quality

- Zero-warning clippy policy with pedantic lints (`[workspace.lints]` in `Cargo.toml`)
- `unsafe` is denied outside the Android JNI exports
- Pre-commit hooks enforced in CI
- cargo-audit, cargo-deny, gitleaks and Trivy on every PR and weekly

See [`SECURITY.md`](SECURITY.md) to report a vulnerability.

## License

[GPL-3.0-only](LICENSE)
