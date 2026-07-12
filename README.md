# argonone-rs

[![CI](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml)

A rust based monitoring and management system for argon one cases of raspberry pi

## Status

[v0.1.0](docs/ROADMAP.md#v010--core-hardware-daemon-argon-one-parity) —
core hardware daemon — is implemented and verified on real Argon ONE
hardware (not yet tagged/released). CLI/systemd only, no web server yet:
I2C fan control with capability auto-detection, GPIO power-button
monitoring, sysinfo collection, board auto-detection (ONE vs EON), and
config-file compat with the original Python daemon. Every hardware
access goes through a `HardwareBackend` trait with a no-op fallback, so
the daemon runs (and is testable) without the case attached. See
[docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan and
[CHANGELOG.md](CHANGELOG.md) for what's landed so far.

## Usage

```sh
argonone-rs service   # run the daemon (fan loop + power button monitor)
argonone-rs status    # one-shot: board, fan, CPU/RAM/temp, disks, RAID, IP
argonone-rs shutdown  # signal the case MCU, then power off
argonone-rs fanoff    # turn the fan off and exit
```

The legacy uppercase spellings (`SERVICE`/`SHUTDOWN`/`FANOFF`) used by the
original Python daemon's scripts and systemd units also work unchanged.
A systemd unit is provided at
[packaging/systemd/argonone-rs.service](packaging/systemd/argonone-rs.service).

## Docs

- [docs/ROADMAP.md](docs/ROADMAP.md) — milestone plan (v0.1.0 → v0.7.0) sequencing the research below into implementation order.
- [docs/research-rust-backend-webui.md](docs/research-rust-backend-webui.md) — what the existing Argon40 Python stack does, proposed Rust daemon architecture, and web UI/UX research (target: homelab/NAS self-hosters).
- [docs/research-auth-persistence-service.md](docs/research-auth-persistence-service.md) — forced first-run admin setup, multi-user RBAC, SQLite persistence, and systemd service install for Ubuntu 26.04 on Raspberry Pi.
- [docs/mockups/](docs/mockups/00-index.html) — interactive HTML mockups of the web UI (setup, login, dashboard, fan curve editor, storage/RAID, OLED display, users, system settings). Open `00-index.html` in a browser to start.
- [CHANGELOG.md](CHANGELOG.md) — cumulative log of every change, by version. [RELEASE_NOTES.md](RELEASE_NOTES.md) covers just the current unreleased cycle; past releases are archived under [docs/releases/](docs/releases/README.md).
