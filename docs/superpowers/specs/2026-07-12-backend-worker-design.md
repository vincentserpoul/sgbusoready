# Backend Worker — Design

**Date:** 2026-07-12
**Status:** Approved (brainstorming)

## Summary

SG Bus Commute currently ships the LTA `DataMall` `AccountKey` embedded in the app
(accepted risk for the MVP — see `2026-06-21-sg-bus-ready-design.md` §3). Add a
small **backend worker** that owns the key server-side, fetches from `DataMall`
once on behalf of every device, and **pushes** live arrivals to devices over
Server-Sent Events instead of each device polling `DataMall` directly. This is a
**friends-scale** service (a handful of devices, not a public product) — the
design optimises for "cheap, boring, hard to abuse" over horizontal scale.

This doc covers the whole feature; it lands in phases. **Phase 0** (this commit)
prepares `sgbr-core`: gates the direct-`DataMall` code behind a feature, extracts
shared render logic, and adds the protocol/auth/SSE/client modules that both the
app and the future worker crate depend on. No server exists yet — Phases 1-3 add
`crates/sgbr-worker` (axum), Phases 4-5 rewire the app and Android to talk to it,
Phase 6 adds deployment artifacts.

## Goals

- **Hide the LTA key.** No device ships or can extract an `AccountKey`; only the
  worker process holds it.
- **Shared caching/quota.** One `DataMall` fetch per distinct stop/dataset serves
  every connected device, instead of N devices each polling independently —
  friendlier to LTA's rate limits as the device count grows.
- **SSE push instead of polling.** Devices open one long-lived connection per
  active session and receive change-only events; the worker owns the ~15s
  `DataMall` refresh cadence, not each device.
- **Friends-scale.** Sized for a small, known set of devices (invite-gated), not
  public sign-up. No horizontal scaling, no multi-tenant isolation beyond what
  the invite/token model already gives.
- **Stateless stop streams.** The worker only ever knows "this connection wants
  these stop codes + service numbers, right now" (the `stops` query string). It
  never learns a device's commute schedule, labels, or history.
- **Commutes never leave the device.** `commutes.json` (label, days, window,
  stops) stays local, as today; the worker's world model is bus stops, routes,
  and live arrivals only — nothing about *when* or *why* a device is watching a
  stop.

## Locked decisions

- **`axum` worker crate.** New `crates/sgbr-worker` (Phase 1+), the one place in
  the workspace allowed to pull in `tokio`/`axum`. `sgbr-core` stays blocking
  `ureq` + std threads — **no tokio in `sgbr-core`**, so the same core keeps
  compiling for the Slint desktop/Android app without an async runtime.
- **Ed25519 device key + invite code → bearer token.** A device generates an
  Ed25519 keypair locally, signs a fixed message over an out-of-band invite
  code, and exchanges that for an opaque bearer token
  (`POST /v1/devices`, see wire protocol below). The invite code is single-use;
  the private key never leaves the device. This avoids running any real user
  system (no passwords, no email) while still gating registration.
- **Worker-only client (post Phase 4).** Once the app is rewired, it drops the
  ability to talk to `DataMall` directly for normal operation and always goes
  through the worker. The `lta-upstream` feature (this phase) stays available
  in `sgbr-core` for local development and for the worker's own upstream fetch
  — it just stops being compiled into the shipped app.
- **Oracle free-tier ARM VM + rootless Podman quadlets + Cloudflare Tunnel.**
  Deployment target: a single Always-Free Ampere A1 VM, the worker container run
  as a rootless Podman quadlet (systemd-managed, auto-restart), exposed to the
  internet via a Cloudflare Tunnel (no inbound firewall rules, free TLS).
  Chosen for zero infra cost at friends-scale.
- **Public GHCR image, `FROM scratch`.** The worker container is a statically
  linked binary on a `scratch` base, published to GitHub Container Registry.
  Public is fine — the image contains no secrets (the `AccountKey` and admin
  token are injected as runtime env/secrets by the quadlet, not baked in).

## Dev builds

Both the desktop binary and the Android cdylib pick up the worker's base URL at
build time via `option_env!("SGBR_WORKER_URL")` (`src/lib.rs`'s `WORKER_URL`,
consumed by `feed.rs`/`rows.rs`/`android_bridge.rs`); an unset/empty value means
"no server configured" and the invite screen reports it rather than trying to
connect.

- **Android dev builds** run against a worker on the dev machine: set
  `SGBR_WORKER_URL=http://127.0.0.1:8080` (in the git-ignored repo-root `.env`,
  which `android/.env.build` already sources with `set -a`/`. .env`/`set +a` —
  no build-plumbing changes needed) and `adb reverse tcp:8080 tcp:8080` so the
  on-device app reaches the host's `localhost:8080`.
- **Production builds** set `SGBR_WORKER_URL` to the public `https://` tunnel
  URL from the Phase 6 deployment.

## Wire protocol

| Endpoint | Auth | Behavior |
|---|---|---|
| `POST /v1/devices` | invite | `{invite_code, pubkey(b64 Ed25519), signature(b64 over "sgbr-register-v1:{invite_code}")}` → `201 {device_id, token(64-hex)}`; 403 bad sig/used/unknown invite |
| `GET /v1/arrivals?stops=83139:15.14e,83141:16` | Bearer | SSE; `event: arrivals`, `data: {"code":"83139","items":[{"minutes":2,"buses":["15"]}]}`; per-connection change-only events; `: hb` heartbeat every 25s |
| `GET /v1/bus-stops`, `GET /v1/bus-routes` | Bearer | Full datasets, `ETag: "stops-{fetched_at_unix}"` / `"routes-{fetched_at_unix}"`, If-None-Match→304, 503 before first fetch |
| `GET /v1/healthz` | none | status |
| `POST/GET /v1/admin/invites`, `GET/DELETE /v1/admin/devices[/{id}]` | X-Admin-Token | invite mint/list, device list/revoke |

The `stops` query format is `"{code}:{svc}.{svc},{code}:{svc}"` — one entry per
stop code, its tracked service numbers joined by `.`, entries joined by `,`
(e.g. `83139:15.14e,83141:16`). This is exactly the distinct-stop union
`sgbr-core::commute::schedule::active_stop_plans` already computes today for the
direct-`DataMall` path; the worker path reuses the same `StopPlan`s, just
formats them into a query string instead of issuing N `DataMall` calls.

## Phase 0 scope (this change)

- `sgbr-core/Cargo.toml` gains an off-by-default `lta-upstream` feature. Only
  network-touching modules (`lta::client`, `bus_catalog::fetch`) are gated
  behind it; models, parsing, arrival maths, and storage stay ungated and
  shared by both the direct-`DataMall` path and the worker path.
  `examples/live_fetch` requires the feature. The root app crate temporarily
  keeps `lta-upstream` enabled (`TODO(phase-4)` marker) so every phase stays
  green until the app is rewired in Phase 4.
- `lta::arrival` is split into a pure `arrival_items` (stop-agnostic grouping)
  and a thin `stop_arrivals` wrapper, so the worker can reuse the exact same
  grouping logic per stop without a display name attached. `ArrivalItem` /
  `StopArrivals` gain `Serialize`/`Deserialize` — they are now wire types too
  (`StopUpdate.items`).
- A new ungated `sgbr-core::worker` module holds everything **shared** by the
  app and the future `sgbr-worker` crate: the wire DTOs (`protocol.rs`), the
  pure Ed25519 registration helpers (`auth.rs`), a dependency-free incremental
  SSE line parser (`sse.rs`), a blocking worker-API client
  (`client.rs`: `register` / `fetch_catalog` / `run_stream`), and atomic
  on-disk credential storage (`credentials.rs`).
- `CoreError` gains `Auth` (registration/token rejected) and `Crypto`
  (keygen/signing failure) variants.

## Out of scope (this slice)

- The `sgbr-worker` axum crate itself, the SSE broadcast/registry, and the
  admin endpoints (Phases 1-3).
- Rewiring the desktop app or Android to actually call the worker (Phases 4-5).
- Containerfile, quadlet units, and the deploy runbook (Phase 6).

## Affected files (this phase, non-exhaustive)

- `crates/sgbr-core/Cargo.toml` — `lta-upstream` feature, new deps
  (`ed25519-dalek`, `getrandom`, `base64`).
- `crates/sgbr-core/src/lta/mod.rs`, `crates/sgbr-core/src/bus_catalog/mod.rs` —
  feature-gate the network-touching submodules.
- `crates/sgbr-core/src/lta/arrival.rs` — `arrival_items` extraction, serde
  derives.
- `crates/sgbr-core/src/lta/client.rs` — `fetch_arrivals_with_agent`.
- `crates/sgbr-core/src/bus_catalog/parse.rs` — shared `assemble_catalog` used
  by both the upstream and worker-client catalog fetches.
- `crates/sgbr-core/src/worker/{protocol,auth,sse,client,credentials}.rs` — new.
- `crates/sgbr-core/src/error.rs` — `CoreError::Auth`, `CoreError::Crypto`.
- `Cargo.toml` (root) — temporary `lta-upstream` passthrough on the app's
  `sgbr-core` dependency.
- `deny.toml`, `.supply-chain/` — allowlist/exemptions for the new deps.
