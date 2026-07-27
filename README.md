# argonone-rs

[![CI](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml)
[![Release](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/release.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/release.yml)
[![crates.io](https://img.shields.io/crates/v/argonone-rs.svg)](https://crates.io/crates/argonone-rs)
[![Downloads](https://img.shields.io/crates/d/argonone-rs.svg)](https://crates.io/crates/argonone-rs)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Raspberry Pi](https://img.shields.io/badge/Raspberry%20Pi-A22846?logo=raspberrypi&logoColor=white)](https://www.raspberrypi.com/)

A Rust daemon, CLI, and web UI for Argon ONE/EON Raspberry Pi cases — I2C fan control, GPIO power-button handling, system monitoring, and a browser-based dashboard, config-compatible with the original Argon40 Python daemon.

## Status

Latest tagged release: **v0.6.2** (HTTPS over Tailscale-issued certs now verified end-to-end against a real Tailscale-joined device, and a listener-hang bug found in the process is fixed — a small unplanned fix on top of v0.6.1's Fahrenheit fan curve fix and v0.6.0's HTTPS/dashboard/hardening work; see [CHANGELOG.md](CHANGELOG.md#v062---2026-07-25)). See [docs/tailscale-setup.md](docs/tailscale-setup.md) for Tailscale HTTPS setup, validation, and troubleshooting.

**v0.7.0 (packaging & operability) is complete on `main`, not yet tagged**: `.deb` packaging via `cargo-deb`, `admin backup`/`admin restore`, one-time legacy config-file import, a diagnosable read-only-rootfs startup error, critical-state webhook notifications, dashboard card drag-reorder, and a system log viewer — every item verified on real Ubuntu 26.04-on-Pi hardware, including both install paths (`.deb` and manual). See [CHANGELOG.md](CHANGELOG.md#unreleased) and [RELEASE_NOTES.md](RELEASE_NOTES.md).

**ACME (Let's Encrypt/custom domain) HTTPS and IR remote learn/program are still not yet verified against real infrastructure/hardware** — both pass in-process tests but need a real domain and a real Argon case with an IR receiver to confirm end-to-end. Every hardware access goes through a `HardwareBackend` trait with a no-op fallback, so the daemon runs (and is testable) without the case attached.

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 milestone plan (each item annotated `Done`/`Not yet done` against the code) and [CHANGELOG.md](CHANGELOG.md) for the dated, cumulative record of what's landed.

## Installation

```sh
cargo install argonone-rs
```

### `.deb` package (Raspberry Pi / Debian, Ubuntu)

Each [GitHub Release](https://github.com/arunkumar-mourougappane/argonone-rs/releases) includes an `aarch64` `.deb`. It installs the binary, the systemd unit, an I2C udev rule, and creates a dedicated `argonone` system account (`dialout` group, for I2C/GPIO access) — no manual `useradd`/unit-file copying needed:

```sh
sudo dpkg -i argonone-rs-vX.Y.Z-linux-aarch64.deb
```

The service is enabled and started automatically. **I2C still needs to be enabled in `/boot/firmware/config.txt` first** (`dtparam=i2c_arm=on`, then reboot) — this is deliberately not automated by the package, since editing the bootloader config unattended is a bigger blast radius than a package install should silently take. See Troubleshooting below if the daemon starts but can't reach the I2C bus.

To build the `.deb` yourself: `cargo install cargo-deb`, then `cargo deb --target aarch64-unknown-linux-gnu` from the repo root (requires the cross-toolchain below).

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

On an Argon EON, the daemon also drives the OLED dashboard (screen rotation, timing, live preview — the **Display** page below) and the RTC wake/sleep schedule (a multi-entry, day-of-week table — the **System** page's Power & RTC card). Both are DB-backed, with the original `/etc/argoneonoled.conf`/`/etc/argonrtc.conf` formats only read as a fallback default (or via the one-time import below) until something's actually been saved through the web UI. On an Argon ONE or a bare Pi with no case, these are no-ops.

`argonone-rs service` also starts the web server (default `0.0.0.0:8080`, SQLite state at `/var/lib/argonone-rs/argonone.db` — both overridable via `ARGONONE_BIND`/`ARGONONE_DB_PATH` for local dev). First visit forces a one-time admin account setup wizard; after that it's a normal login. Admin CLI subcommands, all direct-to-database and bypassing the web layer entirely:

```sh
argonone-rs admin reset-password --username <name>        # recovery if the only admin is locked out
argonone-rs admin backup --output <path>                  # consistent live snapshot (VACUUM INTO), safe while running
argonone-rs admin restore --input <path>                  # restore a prior backup — stop the service first
argonone-rs admin import-legacy-config                     # one-time import of /etc/argon*.conf into the DB
```

Once logged in, the sidebar has **Dashboard** (full card grid — Fan, Power & RTC, Network, Storage, Display, System, Signed-in-as — drag-to-reorder via "Customize layout"), **Fan control** (draggable CPU/HDD curve editor — edits apply live, no restart), **Storage & RAID** (per-disk usage/temperature, array health), **Display** (EON-only OLED config + live preview), **System** (Celsius/Fahrenheit toggle, IR remote learn/program, HTTPS mode, firmware/service info), and, admin-only: **Users** (account management), **Audit log**, **Logs** (live `journalctl -u argonone-rs` tail), and **Notifications** (one webhook URL, fired on a CPU/disk-temp or RAID severity transition into critical). Fan curve/unit/IR/layout edits are operator+; HTTPS, Users, Logs, and Notifications are admin-only — viewers can look, not touch hardware, account, or system-visibility settings. The server always enforces at least 25% fan at or above 75°C regardless of what a curve requests.

### Troubleshooting

- **`sudo systemctl start argonone-rs` hangs, `systemctl list-jobs` shows it queued behind `plymouth-quit-wait.service` ("running" forever).** A headless-boot Plymouth quirk unrelated to this daemon — it can stall `multi-user.target`, which the unit is ordered after. Unstick it with `sudo systemctl stop plymouth-quit-wait.service`; the queued jobs (including `argonone-rs`) then proceed immediately.
- **`status=203/EXEC` / "Exec format error" in `journalctl -u argonone-rs`.** The binary at `/usr/bin/argonone-rs` is the wrong architecture — almost always caused by cross-compiling and then copying `target/release/argonone-rs` (the *host* build) instead of `target/aarch64-unknown-linux-gnu/release/argonone-rs` (the actual Pi binary). Verify with `file /usr/bin/argonone-rs` on the Pi — it should say `ELF 64-bit LSB pie executable, ARM aarch64`.
- **Upgrading a box that still has the original Python daemon installed.** `argononed.service` will compete with `argonone-rs` for the same I2C bus (`0x1a`) if both are enabled. Disable the old one first: `sudo systemctl disable --now argononed.service`.

## Docs

- [docs/ROADMAP.md](docs/ROADMAP.md) — milestone plan (v0.1.0 → v0.7.0).
- [CHANGELOG.md](CHANGELOG.md) — cumulative log of every change, by version. [RELEASE_NOTES.md](RELEASE_NOTES.md) covers just the current unreleased cycle; past releases are archived under [docs/releases/](docs/releases/README.md).
- [docs/tailscale-setup.md](docs/tailscale-setup.md) — setting up HTTPS via Tailscale-issued certs: prerequisites, enabling HTTPS Certificates on your tailnet, validating cert issuance and network reachability, and troubleshooting.

### Planned / research

The web UI now covers every mockup screen — setup/login/session/RBAC, the full (reorderable) dashboard card grid, fan control, storage/RAID, system (including HTTPS and IR remote), Power & RTC scheduling, OLED config, Users admin, audit log, system log viewer, and webhook notifications — as of v0.3.0 through v0.7.0 (see Usage above). No further milestone is planned yet; see [docs/ROADMAP.md](docs/ROADMAP.md)'s "Ongoing, not milestone-gated" section and its note on `v1.0.0` criteria, still an open call pending broader field use.

- [docs/research-rust-backend-webui.md](docs/research-rust-backend-webui.md) — what the existing Argon40 Python stack does, proposed Rust daemon architecture, and web UI/UX research (target: homelab/NAS self-hosters).
- [docs/research-auth-persistence-service.md](docs/research-auth-persistence-service.md) — forced first-run admin setup, multi-user RBAC, SQLite persistence, and systemd service install for Ubuntu 26.04 on Raspberry Pi.
- [docs/mockups/](docs/mockups/00-index.html) — interactive HTML mockups of the full web UI (setup, login, dashboard, fan curve editor, storage/RAID, OLED display, users, system settings, audit log, dashboard reorder, logs, notifications) — the target design; every screen is now real. Open `00-index.html` in a browser to start.

## License

MIT — see [LICENSE](LICENSE). Copyright (c) 2026 Arunkumar Mourougappane.
