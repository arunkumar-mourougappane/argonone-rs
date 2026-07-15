<!--
  Current/unreleased release notes only — this is what `gh release create
  --notes-file RELEASE_NOTES.md` publishes to the GitHub Release page
  when a `v*.*.*` tag is pushed (see .github/workflows/release.yml).

  Process for cutting a release (order matters — tag while this file
  still has real content, archive/reset only after):
    1. Make sure this file describes what's actually shipping.
    2. Move the CHANGELOG.md [Unreleased] section to a new [vX.Y.Z] - date
       heading (add a fresh empty [Unreleased] above it).
    3. Commit, tag (`git tag vX.Y.Z`), push the tag — release.yml reads
       this file exactly as it is in that tagged commit.
    4. Only after tagging: run `scripts/cut-release.sh vX.Y.Z` — archives
       this file's content to docs/releases/vX.Y.Z.md and resets it back
       to the template below, in a new commit on main.
  See docs/releases/README.md for the full archive convention.
-->

# Release Notes — v0.2.0

## Overview

v0.2.0 is **EON extras (OLED + RTC)**: completes Python-parity for both Argon case models. Still CLI/systemd-only, no web server — screen rotation and RTC schedules stay config-file-driven until v0.3.0's web foundation lands. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan.

## What's New

- **OLED dashboard** — screen-rotation state machine (configurable switch duration, screensaver blank-after-idle, power-button force-advance/wake), seven live screens (clock, IP, CPU, RAM, storage, temperature, RAID), and an original splash screen (`RPI` rotated 90°, detected Pi model, `argonone` signature).
- **Fonts/backgrounds sourcing resolved** — none of it built from Argon40's original assets. Dashboard backgrounds are drawn as plain rectangles/labels and glyphs come from `embedded-graphics`'s bundled, permissively-licensed fonts, instead of vendoring or fetching Argon40's `.bin` files.
- **RTC (PCF8563)** — daily wake-alarm programming and a daily sleep (scheduled poweroff) check driven off the RTC's own battery-backed clock, both config-file driven (`/etc/argoneonoled.conf`, `/etc/argonrtc.conf`).
- **`status` command** — now reports RTC time and the configured wake/sleep schedule alongside the existing CPU/RAM/disk/RAID/IP output.
- **`HardwareBackend` gains `OledBackend`/`RtcBackend`** — same no-op-fallback pattern as the fan/button seams, inert on Argon ONE/no-case builds.

## Verified on Hardware

v0.2.0 has been run end-to-end on a real Argon EON case: the OLED dashboard, screen rotation (switch timing, screensaver blank, button force-advance), the splash screen, and RTC wake/sleep scheduling were all confirmed on-device.

## Getting Started

```sh
cargo install argonone-rs
argonone-rs status
```

See [README.md](README.md) for full installation, build-from-source, and systemd setup instructions, and the [Usage](README.md#usage) section for the EON OLED/RTC config file formats.

## What's Next

v0.3.0 starts the web server: SQLite persistence, forced first-run setup, and Argon2id auth — infrastructure only, no feature screens yet. See [docs/ROADMAP.md](docs/ROADMAP.md#v030--web-foundation-persistence-auth-live-shell).
