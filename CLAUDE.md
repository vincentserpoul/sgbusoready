# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SG Bus Ready shows live Singapore bus arrivals for saved commutes. The app is a Slint UI in Rust that runs on desktop and Android (Kotlin glue under `android/`). A small axum backend, `sgbr-worker`, holds the LTA `DataMall` `AccountKey` and pushes arrivals to devices over Server-Sent Events.

Design docs live in `docs/superpowers/specs/`; the deploy runbook is `deploy/README.md`.

## Build & Development Commands

Uses `just` as task runner (see `Justfile` for all recipes):

```bash
just check          # Quick compile check
just build          # Debug build
just build-release  # Release build
just test           # Run tests with nextest
just test-one NAME  # Run a single test by name
just test-doc       # Run doctests
just clippy         # Run clippy lints
just clippy-fix     # Auto-fix clippy issues
just setup          # One-time: install git hooks (prek)
just hooks          # Run all pre-commit hooks on the whole repo
just fmt            # Format with rustfmt
just fmt-check      # Check formatting
just coverage       # Code coverage with llvm-cov
just dev            # Quick local iteration: check + clippy + test
just ci             # Full CI suite: fmt-check + clippy + test + doc-check + deny + audit + machete
just run-worker     # Run the worker locally against real LTA (needs .env)
just clippy-android # Lint the Android-only code (needs android/.env.build + NDK)
just run-android    # Build, install and launch on a connected device
```

## Workspace Architecture

Three workspace members with a clear dependency hierarchy:

- **`crates/sgbr-core`** — Pure Rust core library. LTA `DataMall` models and client (behind the `lta-upstream` feature), bus catalog and stop search, the commute model and its persistence, and the worker protocol (auth, SSE, client). No platform or UI code.

- **`crates/sgbr-worker`** — axum backend. Device registration, bus catalog, the live arrivals SSE stream, and admin endpoints. Enables `lta-upstream` on the core. Ships as a scratch container image (`crates/sgbr-worker/Containerfile`).

- **`sgbusoready`** (workspace root: `src/`, `ui/`, `build.rs`) — the Slint app. `src/main.rs` is the desktop entry point; `src/lib.rs` is shared and also built as a `cdylib` for Android. `src/android_bridge.rs` is the JNI bridge to the Kotlin code in `android/`.

The root manifest is itself a package, so always pass `--workspace` to cargo (the `just` recipes do).

## Lint & Code Quality Rules

Strict lint configuration in workspace `Cargo.toml`, kept identical to the `youtun4` repo:

- **`unsafe_code = "deny"`** — the only exception is the Android JNI exports, each with an `#[allow(unsafe_code, reason = "...")]`
- **`unwrap_used = "deny"`, `expect_used = "deny"`** — use proper error handling (`.ok()`, `?`, `map_err`)
- **`panic = "deny"`, `unreachable = "deny"`, `unimplemented = "deny"`**
- **`print_stdout = "deny"`, `print_stderr = "deny"`, `dbg_macro = "deny"`** — use `tracing` instead
- **`indexing_slicing = "deny"`** — use `.get()` with bounds checking
- **`float_arithmetic = "deny"`, `cast_possible_truncation = "deny"`, `cast_sign_loss = "deny"`**
- Pedantic clippy enabled across the board
- Levels marked `warn` only warn locally; `just clippy` and CI add `-D warnings`, so nothing with a warning can be merged
- `#[allow]` needs a `reason`
- Tests are relaxed: `clippy.toml` allows `unwrap`, `expect`, `panic`, `dbg`, `print`, and indexing in tests

## Toolchain

- Rust 1.100.0, pinned in `rust-toolchain.toml` — the single source of truth; CI installs from it
- Edition 2024
- Android and iOS targets are listed in `rust-toolchain.toml` and installed automatically

## Pre-commit Hooks

Configured in `.pre-commit-config.yaml` (uses `prek`; run `just setup` once to install): file hygiene checks, conventional commits + gitlint (commit-msg), yamllint, taplo, typos, gitleaks, actionlint, zizmor, cargo fmt, cargo clippy. CI runs the same hooks in the `hooks` job of `ci.yml`.

## Secrets

The LTA `DataMall` `AccountKey` and the worker admin token must never be committed. They are read from `.env` locally and injected at runtime in production (`deploy/env.example`).
