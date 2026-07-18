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

## Overview

v0.5.0 is **EON web screens + Users/RBAC admin** — it completes web-UI parity with every mockup screen: a full Users admin page, a Power & RTC schedule card, and an OLED config page with a server-rendered live preview of the actual framebuffer. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan.

## What's New

- **Users admin page** (`/users`, admin-only) — full CRUD, role assignment, "Reset password," and a "Locked" badge/"Unlock" action for accounts that hit the failed-login lockout. Refuses to delete or demote the sole remaining admin (checked atomically, not as a separate check-then-write step, so two concurrent requests can't both slip past the guard). Temporary passwords (on create or reset) now show up in a persistent, copyable panel instead of only in a toast that could disappear before it's read.
- **Power & RTC schedule** (`/system`, EON-only) — a full multi-entry, day-of-week wake/sleep table, not just a single wake/sleep pair. The PCF8563 RTC has exactly one hardware alarm slot, so the daemon resolves the configured schedule down to "what's the single next occurrence" and re-arms that — once at startup, on every schedule edit, and again right before any self-triggered sleep.
- **Display / OLED config** (`/display`, EON-only) — drag-to-reorder screen rotation, per-screen enable, timing sliders, and a **live preview that's a real render**, not a simulation: the same `draw_screen` function the physical panel uses renders into an in-memory framebuffer, streamed to the browser over the existing WebSocket connection.
- Fan curves, temperature units, RTC schedule, and OLED config are all DB-backed with live-apply — editing any of them takes effect on the running daemon immediately, no restart.
- Dashboard status strip and sidebar now match their mockups: severity-colored CPU-temp dot, IP address, uptime, and the device hostname under the "argonone" brand mark — none of which were wired up before this release.

## Known Limitations

- **Not yet verified on real Argon EON hardware.** Every feature in this release is covered by 170 automated tests (including EON-board-gated routes, exercised via a `Board::Eon` test fixture — never a live server) and manual smoke-testing against a locally-running server, but that live testing only ever ran as `Board::NoCase`: this dev machine is non-Linux, and `hardware::detect` has no override to force `Board::Eon` at runtime off real hardware. Neither the RTC wake/sleep scheduling nor the OLED live preview has ever driven an actual PCF8563 or SSD1306. This is the same bar v0.1.0–v0.4.0 were held to before tagging, and it hasn't been met yet for v0.5.0. Treat this release as functionally complete but hardware-unverified until that pass happens.

## Deploying

No new migrations since v0.4.0 — `users`, `settings`, and `audit_log` already had every column this release needed.

```sh
cargo install argonone-rs
# or cross-compile / copy the binary to the Pi — see README.md, or use scripts/deploy.sh
sudo systemctl restart argonone-rs
```

See [README.md](README.md) for full deployment instructions.

## What's Next

v0.6.0 closes the remaining dashboard data-surface gaps (network throughput, load average, swap), adds HTTPS, an audit log viewer, and self-service password rotation. See [docs/ROADMAP.md](docs/ROADMAP.md#v060--https-dashboard-data-surface-gaps-hardening).
