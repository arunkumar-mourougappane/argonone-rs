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

# Release Notes — v0.4.0

## Overview

v0.4.0 is **Core dashboard: fan control, storage, system** — the highest-value milestone, replacing `argonone-fanconfig.sh`/`argon-unitconfig.sh` and friends with the web UI the mockups promise. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan.

## What's New

- **Fan curve editor** (`/fan`) — draggable-SVG temperature→speed points, CPU/HDD tabs, a synced editable table, and a live "now: NN°C → NN%" indicator on the chart showing where the current operating point sits. Edits apply to the running control loop immediately via a `tokio::sync::watch` channel, without restarting the daemon or losing hysteresis state.
- **Server-enforced fan-curve safety floor** — rejects any curve implying less than 25% fan at or above 75°C, checked at every configured breakpoint at/above that ceiling (not just one point), so an unsafe *gap* between two otherwise-safe points is caught too. This is enforced independent of what an operator configures — a client-side-only check can't stop a direct API call from bypassing it.
- **The HDD curve is now actually applied**, not just editable — the daemon takes `max(cpu_curve_speed, hdd_curve_speed)` each poll, using the hottest of all detected disks' S.M.A.R.T. temperatures, matching the documented "the higher value wins" behavior.
- **Storage & RAID page** (`/storage`) — per-disk usage and S.M.A.R.T. temperature, a Role column (RAID member vs. `NN% full`), and severity-banded (good/warn/crit) coloring on the usage bar and temperature reading. RAID arrays show level, state, size, and working/failed/spare disk counts parsed from `/proc/mdstat`.
- **System page** (`/system`) — a Celsius/Fahrenheit toggle applied consistently across every temperature display in the app (dashboard, OLED, fan/storage pages), plus firmware/service info.
- **`GET/PUT /api/fan/curve/{cpu,hdd}`** and **`GET/PUT /api/settings/units`** — `viewer+` for reads, `operator+` for writes, per the documented API contract.
- Fan curves and the temperature-unit setting are now DB-backed, replacing the config-file source of truth; `argonone-rs status` reads the same values the running daemon applies, so the two can't drift.
- A shared toast notification component for save/action success feedback (fan curve save, units toggle) — the app previously had no positive confirmation anywhere, only inline error text on failure.
- `scripts/deploy.sh`/`scripts/deploy-local.sh` — script the cross-compile → scp → systemd-install sequence, guarding against the real failure modes hit during on-hardware deployment (I2C conflicts, a Plymouth boot stall, a redeploy not picking up the new binary).

## Verified on Hardware

v0.4.0 has been run end-to-end on a real Argon ONE/EON case: the fan curve editor (CPU/HDD tabs) applying live to the real fan, the 25%-at-75°C safety floor, the Storage & RAID page against actual attached disks and a real RAID array (`lsblk`/`smartctl`/`/proc/mdstat` parsing confirmed against real data, not just synthetic test fixtures), and the System page's C/F toggle propagating across the dashboard, OLED, and fan/storage displays were all confirmed on-device.

## Deploying

No new migrations or systemd unit changes since v0.3.0.

```sh
cargo install argonone-rs
# or cross-compile / copy the binary to the Pi — see README.md, or use scripts/deploy.sh
sudo systemctl restart argonone-rs
```

See [README.md](README.md) for full deployment instructions.

## What's Next

v0.5.0 adds the EON-specific web screens (OLED config, Power & RTC) and the Users/RBAC admin page. See [docs/ROADMAP.md](docs/ROADMAP.md#v050--eon-web-screens--usersrbac-admin).
