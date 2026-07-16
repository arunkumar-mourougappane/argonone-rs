# argonone-rs

[![CI](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml)
[![Release](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/release.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/release.yml)
[![crates.io](https://img.shields.io/crates/v/argonone-rs.svg)](https://crates.io/crates/argonone-rs)
[![Downloads](https://img.shields.io/crates/d/argonone-rs.svg)](https://crates.io/crates/argonone-rs)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Raspberry Pi](https://img.shields.io/badge/Raspberry%20Pi-A22846?logo=raspberrypi&logoColor=white)](https://www.raspberrypi.com/)

A Rust daemon, CLI, and web UI for Argon ONE/EON Raspberry Pi cases — I2C fan control, GPIO power-button handling, system monitoring, and a browser-based dashboard, config-compatible with the original Argon40 Python daemon.

## Status

Latest tagged release: **v0.3.0** (web foundation — persistence, auth, live shell), verified on real Argon ONE/EON hardware. **v0.4.0** (fan curve editor, Storage & RAID, System pages) is implemented and route-tested on `main` but not yet tagged — it still needs a verification pass against real disks/RAID hardware. Every hardware access goes through a `HardwareBackend` trait with a no-op fallback, so the daemon runs (and is testable) without the case attached.

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 milestone plan (each item annotated `Done`/`Not yet done` against the code) and [CHANGELOG.md](CHANGELOG.md) for the dated, cumulative record of what's landed.

## Installation

```sh
cargo install argonone-rs
```

### Build from source

```sh
git clone https://github.com/arunkumar-mourougappane/argonone-rs
cd argonone-rs
cargo build --release
```

### Cross-compile for Raspberry Pi

From a non-Pi host, target `aarch64-unknown-linux-gnu` (64-bit Raspberry Pi OS):

```sh
rustup target add aarch64-unknown-linux-gnu
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-unknown-linux-gnu-gcc \
  cargo build --release --target aarch64-unknown-linux-gnu
```

Requires an `aarch64-unknown-linux-gnu` cross-toolchain on the host (e.g. `brew install aarch64-unknown-linux-gnu` on macOS).

### Deploying to a Pi

Two scripts, same install logic, pick based on where you're running from:

- **From your dev machine, over SSH** — [`scripts/deploy.sh`](scripts/deploy.sh) cross-compiles, ships the binary + systemd unit + `deploy-local.sh` itself to the Pi, and runs it there:

  ```sh
  scripts/deploy.sh <ssh-host>              # e.g. scripts/deploy.sh pi@192.168.1.50
  scripts/deploy.sh <ssh-host> --skip-build # reuse the last cross-compiled binary
  scripts/deploy.sh <ssh-host> --no-restart # copy files only, don't touch the running service
  scripts/deploy.sh <ssh-host> --yes        # don't prompt before disabling a conflicting argononed.service
  ```

- **Already on the Pi** (e.g. after `git clone`/`git pull`) — [`scripts/deploy-local.sh`](scripts/deploy-local.sh) builds natively and installs directly, no SSH involved:

  ```sh
  scripts/deploy-local.sh                 # cargo build --release, then install + restart
  scripts/deploy-local.sh --skip-build    # reuse the last local build
  ```

Both guard the real gotchas in Troubleshooting below (the old Python daemon's I2C conflict, the Plymouth boot-stall) and restart — not just `enable --now`, which no-ops on an already-running service — so a redeploy actually picks up the new binary.

Neither does the one-time hardware setup (enabling I2C in `/boot/firmware/config.txt`, which needs a reboot) — see Installation above for that, once, before the first deploy.

## Usage

```sh
argonone-rs service   # run the daemon (fan loop, power button, EON OLED/RTC)
argonone-rs status    # one-shot: board, fan, CPU/RAM/temp, disks, RAID, IP, RTC
argonone-rs shutdown  # signal the case MCU, then power off
argonone-rs fanoff    # turn the fan off and exit
```

The legacy uppercase spellings (`SERVICE`/`SHUTDOWN`/`FANOFF`) used by the original Python daemon's scripts and systemd units also work unchanged. A systemd unit is provided at [packaging/systemd/argonone-rs.service](packaging/systemd/argonone-rs.service).

On an Argon EON, the daemon also drives the OLED dashboard (screen rotation configured via `/etc/argoneonoled.conf`: `switchduration`, `screensaver`, `screenlist`, `enabled`) and the RTC wake/sleep schedule (`/etc/argonrtc.conf`: `enabled`, `wake=HH:MM`, `sleep=HH:MM`) — both config-file only for now, no settings *screen* yet. On an Argon ONE or a bare Pi with no case, these are no-ops.

`argonone-rs service` also starts the web server (default `0.0.0.0:8080`, SQLite state at `/var/lib/argonone-rs/argonone.db` — both overridable via `ARGONONE_BIND`/`ARGONONE_DB_PATH` for local dev). First visit forces a one-time admin account setup wizard; after that it's a normal login. Recovery, if the only admin gets locked out:

```sh
argonone-rs admin reset-password --username <name>
```

Once logged in, the sidebar has **Fan control** (draggable CPU/HDD curve editor — edits apply live, no restart), **Storage & RAID** (per-disk usage/temperature, array health), and **System** (Celsius/Fahrenheit toggle, firmware/service info). Fan curve and unit edits are operator/admin only — viewers can look, not touch hardware settings. The server always enforces at least 25% fan at or above 75°C regardless of what a curve requests.

### Troubleshooting

- **`sudo systemctl start argonone-rs` hangs, `systemctl list-jobs` shows it queued behind `plymouth-quit-wait.service` ("running" forever).** A headless-boot Plymouth quirk unrelated to this daemon — it can stall `multi-user.target`, which the unit is ordered after. Unstick it with `sudo systemctl stop plymouth-quit-wait.service`; the queued jobs (including `argonone-rs`) then proceed immediately.
- **`status=203/EXEC` / "Exec format error" in `journalctl -u argonone-rs`.** The binary at `/usr/local/bin/argonone-rs` is the wrong architecture — almost always caused by cross-compiling and then copying `target/release/argonone-rs` (the *host* build) instead of `target/aarch64-unknown-linux-gnu/release/argonone-rs` (the actual Pi binary). Verify with `file /usr/local/bin/argonone-rs` on the Pi — it should say `ELF 64-bit LSB pie executable, ARM aarch64`.
- **Upgrading a box that still has the original Python daemon installed.** `argononed.service` will compete with `argonone-rs` for the same I2C bus (`0x1a`) if both are enabled. Disable the old one first: `sudo systemctl disable --now argononed.service`.

## Docs

- [docs/ROADMAP.md](docs/ROADMAP.md) — milestone plan (v0.1.0 → v0.7.0).
- [CHANGELOG.md](CHANGELOG.md) — cumulative log of every change, by version. [RELEASE_NOTES.md](RELEASE_NOTES.md) covers just the current unreleased cycle; past releases are archived under [docs/releases/](docs/releases/README.md).

### Planned / research

The web UI's foundation (setup, login, session/RBAC) and its core dashboard (fan control, storage/RAID, system) are implemented as of v0.3.0/v0.4.0 — see Usage above. Still ahead: RTC/power scheduling and OLED config screens (v0.5.0), Users admin page (v0.5.0), and HTTPS (v0.6.0):

- [docs/research-rust-backend-webui.md](docs/research-rust-backend-webui.md) — what the existing Argon40 Python stack does, proposed Rust daemon architecture, and web UI/UX research (target: homelab/NAS self-hosters).
- [docs/research-auth-persistence-service.md](docs/research-auth-persistence-service.md) — forced first-run admin setup, multi-user RBAC, SQLite persistence, and systemd service install for Ubuntu 26.04 on Raspberry Pi.
- [docs/mockups/](docs/mockups/00-index.html) — interactive HTML mockups of the full web UI (setup, login, dashboard, fan curve editor, storage/RAID, OLED display, users, system settings) — the target design; setup, login, dashboard, fan curve editor, Storage & RAID, and the units/firmware slice of System are real, OLED display/Power & RTC/Users are still mockup-only. Open `00-index.html` in a browser to start.

## License

MIT — see [LICENSE](LICENSE). Copyright (c) 2026 Arunkumar Mourougappane.
