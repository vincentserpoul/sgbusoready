# Deploy runbook — SG Bus Commute worker on Oracle Cloud

Design context: `docs/superpowers/specs/2026-07-12-backend-worker-design.md`
(this file is ops-only; that doc is authoritative on the *why*). Steps marked
⚑ are manual, one-time, and done outside this repo.

Target: Oracle Cloud Always-Free `VM.Standard.A1.Flex` ARM VM, running the
worker as a rootless Podman quadlet, fronted by a Cloudflare Tunnel (zero
inbound ports besides SSH).

## 1. ⚑ Provision the VM

Oracle Cloud (OCI) free-tier signup, then create an instance:

- Shape: `VM.Standard.A1.Flex`, 2 OCPU / 12 GB (within the Always-Free ARM
  allowance).
- Image: Ubuntu 24.04 Minimal, aarch64.
- Region: `ap-singapore-1` (closest to `DataMall`).
- Add your SSH public key at creation.
- Leave the default security list / NSG as-is — no extra ingress rules. The
  VM never needs an open inbound port; the Cloudflare Tunnel dials out.

## 2. ⚑ VM prep

SSH in as the default Ubuntu user, then:

```bash
sudo apt update && sudo apt install -y podman uidmap slirp4netns

# Dedicated, sudo-less login user to run the rootless quadlets as.
sudo useradd --create-home --shell /bin/bash sgbr
sudo loginctl enable-linger sgbr
```

`enable-linger` keeps `sgbr`'s systemd `--user` instance (and therefore the
quadlet services) running without an active login session.

From here on, run everything as the `sgbr` user (`sudo -iu sgbr` or SSH in
directly once you've set up its key).

## 3. ⚑ Cloudflare Tunnel

- Domain already on Cloudflare.
- Zero Trust dashboard → Networks → Tunnels → Create a tunnel →
  **Cloudflare Connector (Docker/Podman)** style, remotely-managed, name it
  `sgbr`.
- Public hostname: `bus.<your-domain>` → service `http://sgbr-worker:8080`
  (that hostname resolves inside the `sgbr` Podman network because
  `deploy/sgbr-worker.container` sets `ContainerName=sgbr-worker`).
- Copy the tunnel token shown in the "Install and run a connector" step
  (Docker tab) — just the token argument's value, not the whole command.

## 4. Build & push the image (from the dev machine)

One-time toolchain install (see the comment on `just build-worker-arm` in the
root `Justfile`):

```bash
rustup target add aarch64-unknown-linux-musl
cargo install cargo-zigbuild
# zig itself, e.g. on Arch:
sudo pacman -S zig
```

⚑ `podman login ghcr.io` (a GitHub PAT with `write:packages`) before the
first push.

```bash
just build-worker-image push-worker-image
```

⚑ After the first push, open the package settings for `sgbr-worker` on GHCR
and mark it **public** (design doc: the image contains no secrets — the LTA
key and admin token are injected at runtime, never baked in).

The default image ref is `ghcr.io/vincentserpoul/sgbr-worker:latest`
(`worker_image` in the root `Justfile`, and `Image=` in
`deploy/sgbr-worker.container`) — edit both if you publish under a different
GHCR namespace.

## 5. VM install

As the `sgbr` user on the VM:

```bash
mkdir -p ~/sgbr/data
```

Write `~/sgbr/env` from `deploy/env.example`, then lock it down:

```bash
chmod 600 ~/sgbr/env
```

Fill in real values: `LTA_API_ACCOUNT_KEY` from DataMall,
`SGBR_ADMIN_TOKEN=$(openssl rand -hex 24)`, and keep
`SGBR_BIND=0.0.0.0:8080` / `SGBR_DATA_DIR=/data` as shipped in the example
(they must match the quadlet's bind port and `Volume=` mount point).

Write `~/sgbr/cloudflared-env` from `deploy/cloudflared-env.example` with the
real tunnel token, and `chmod 600` it too.

Make `~/sgbr/data` writable by the worker's in-container UID (the image runs
as `65534:65534`, not the `sgbr` user's own rootless-mapped UID 0):

```bash
podman unshare chown 65534:65534 ~/sgbr/data
```

Copy the three quadlet units into the rootless search path:

```bash
mkdir -p ~/.config/containers/systemd
cp deploy/sgbr.network deploy/sgbr-worker.container deploy/cloudflared.container \
  ~/.config/containers/systemd/
```

Reload and start:

```bash
systemctl --user daemon-reload
systemctl --user start sgbr-worker cloudflared
systemctl --user enable podman-auto-update.timer
```

(`enable`, not `start` — `WantedBy=default.target` in the quadlets already
means `daemon-reload` picks them up on future boots; `enable` here is only
for the stock `podman-auto-update.timer`, which is a plain systemd timer
unit, not a quadlet.)

## 6. Logs / ops

```bash
journalctl --user -u sgbr-worker -f
journalctl --user -u cloudflared -f
```

Mint an invite code (env vars are already inside the container from
`~/sgbr/env`, and the CLI's default `--url http://127.0.0.1:8080` matches
`SGBR_BIND=0.0.0.0:8080` since it's run *inside* the container's own network
namespace):

```bash
podman exec sgbr-worker /sgbr-worker invite new
```

## 7. Smoke test

```bash
curl -s https://bus.<your-domain>/v1/healthz | jq
```

Device registration (`POST /v1/devices`) needs a real Ed25519 signature over
`sgbr-register-v1:{invite_code}` — the app does this itself on first launch
via the invite screen; this curl is only illustrative of the wire shape
(`crates/sgbr-core/src/worker/protocol.rs::RegisterRequest`/`RegisterResponse`):

```bash
curl -s https://bus.<your-domain>/v1/devices \
  -H 'content-type: application/json' \
  -d '{"invite_code":"<code>","pubkey":"<base64>","signature":"<base64>"}' | jq
# -> 201 {"token":"<64-hex>","device_id":"<id>"}
```

SSE stream, once you have a device token:

```bash
curl -N -H "Authorization: Bearer <token>" \
  "https://bus.<your-domain>/v1/arrivals?stops=83139:15"
```

Expect an `event: arrivals` frame within ~2s and a `: hb` comment line every
~25s (`HEARTBEAT_INTERVAL` in `crates/sgbr-worker/src/routes/arrivals.rs`).
If events only arrive in bursts instead of streaming, disable Brotli and
Rocket Loader on the `bus.<your-domain>` hostname in Cloudflare (both can
buffer SSE responses).

## 8. App builds

- **Production**: set `SGBR_WORKER_URL=https://bus.<your-domain>` in the
  repo-root `.env` before `just build-android` (`android/.env.build` already
  sources it; see the design doc's "Dev builds" section).
- **Dev**: `SGBR_WORKER_URL=http://127.0.0.1:8080` in `.env`, plus
  `adb reverse tcp:8080 tcp:8080` so the on-device app reaches the worker
  running on your dev machine.

## 9. Update flow

```bash
just build-worker-image push-worker-image
```

The VM picks up the new image within `podman-auto-update.timer`'s window
(daily, by default — see `podman-auto-update(1)`), or immediately with:

```bash
podman auto-update
```
