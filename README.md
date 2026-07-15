# argonone-rs

[![CI](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml)
[![Release](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/release.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/release.yml)
[![crates.io](https://img.shields.io/crates/v/argonone-rs.svg)](https://crates.io/crates/argonone-rs)
[![Downloads](https://img.shields.io/crates/d/argonone-rs.svg)](https://crates.io/crates/argonone-rs)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Raspberry Pi](https://img.shields.io/badge/Raspberry%20Pi-A22846?logo=raspberrypi&logoColor=white)](https://www.raspberrypi.com/)

A Rust daemon and CLI for Argon ONE/EON Raspberry Pi cases — I2C fan control, GPIO power-button handling, and system monitoring, config-compatible with the original Argon40 Python daemon.

## Status

[v0.2.0](docs/ROADMAP.md#v020--eon-extras-oled--rtc) — EON extras (OLED dashboard + RTC wake/sleep scheduling), completing Python-parity for both case models — is released and verified on real Argon EON hardware. [v0.1.0](docs/ROADMAP.md#v010--core-hardware-daemon-argon-one-parity) (core hardware daemon: I2C fan control, GPIO power-button monitoring, sysinfo collection, board auto-detection) shipped first and remains verified on real Argon ONE hardware. Both are CLI/systemd-only — no web server yet. Every hardware access goes through a `HardwareBackend` trait with a no-op fallback, so the daemon runs (and is testable) without the case attached. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan and [CHANGELOG.md](CHANGELOG.md) for what's landed so far.

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

## Usage

```sh
argonone-rs service   # run the daemon (fan loop, power button, EON OLED/RTC)
argonone-rs status    # one-shot: board, fan, CPU/RAM/temp, disks, RAID, IP, RTC
argonone-rs shutdown  # signal the case MCU, then power off
argonone-rs fanoff    # turn the fan off and exit
```

The legacy uppercase spellings (`SERVICE`/`SHUTDOWN`/`FANOFF`) used by the original Python daemon's scripts and systemd units also work unchanged. A systemd unit is provided at [packaging/systemd/argonone-rs.service](packaging/systemd/argonone-rs.service).

On an Argon EON, the daemon also drives the OLED dashboard (screen rotation configured via `/etc/argoneonoled.conf`: `switchduration`, `screensaver`, `screenlist`, `enabled`) and the RTC wake/sleep schedule (`/etc/argonrtc.conf`: `enabled`, `wake=HH:MM`, `sleep=HH:MM`) — both config-file only for now, no web UI yet. On an Argon ONE or a bare Pi with no case, these are no-ops.

## Docs

- [docs/ROADMAP.md](docs/ROADMAP.md) — milestone plan (v0.1.0 → v0.7.0).
- [CHANGELOG.md](CHANGELOG.md) — cumulative log of every change, by version. [RELEASE_NOTES.md](RELEASE_NOTES.md) covers just the current unreleased cycle; past releases are archived under [docs/releases/](docs/releases/README.md).

### Planned / research (not yet implemented)

Everything below describes the future web UI (v0.3.0+), not the current CLI/systemd daemon:

- [docs/research-rust-backend-webui.md](docs/research-rust-backend-webui.md) — what the existing Argon40 Python stack does, proposed Rust daemon architecture, and web UI/UX research (target: homelab/NAS self-hosters).
- [docs/research-auth-persistence-service.md](docs/research-auth-persistence-service.md) — forced first-run admin setup, multi-user RBAC, SQLite persistence, and systemd service install for Ubuntu 26.04 on Raspberry Pi.
- [docs/mockups/](docs/mockups/00-index.html) — interactive HTML mockups of the web UI (setup, login, dashboard, fan curve editor, storage/RAID, OLED display, users, system settings). Open `00-index.html` in a browser to start.

## License

MIT — see [LICENSE](LICENSE). Copyright (c) 2026 Arunkumar Mourougappane.
