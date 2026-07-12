# Changelog

All notable changes to this project are documented here, grouped by
release. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows the policy in [docs/ROADMAP.md](docs/ROADMAP.md)
(`0.MINOR.0` per milestone, `0.MINOR.PATCH` for unplanned fixes against
the current milestone).

This file is the permanent, cumulative log across every version. For the
prose write-up of just the *current* unreleased work (what a tag's
GitHub Release page will show), see [RELEASE_NOTES.md](RELEASE_NOTES.md).
Once a version is tagged, that release's notes are archived permanently
under [docs/releases/](docs/releases/).

## [Unreleased]

Everything so far is planning/design work — no daemon code has shipped
yet. See [docs/ROADMAP.md](docs/ROADMAP.md) for what "released" will
mean starting at `v0.1.0`.

### Added

- Research: existing Argon40 Python stack fully audited (I2C/GPIO
  hardware surface, config file formats, daemon behavior), proposed
  Rust daemon architecture, and web UI/UX research
  (`docs/research-rust-backend-webui.md`).
- Research: forced first-run admin setup, three-tier RBAC, SQLite
  persistence strategy, and systemd service/privilege design for
  Ubuntu 26.04 on Raspberry Pi (`docs/research-auth-persistence-service.md`).
- Nine interactive, animated HTML mockups covering every planned screen
  — setup, login, dashboard, fan curve editor, storage/RAID, OLED
  display, users, system settings (`docs/mockups/`).
- `docs/ROADMAP.md` — seven dependency-ordered implementation milestones
  (`v0.1.0` → `v0.7.0`), each citing the research section it's grounded in.
- CI workflow: `fmt` → `clippy` → `test` → `audit` gating build jobs for
  aarch64 Raspberry Pi (primary/blocking) and macOS (secondary/dev-only,
  non-blocking).
- Release workflow: tag-triggered (`v*.*.*`), cross-compiled Pi release
  binary, GitHub Release titled `Release <tag>`.
- GitHub Pages deployment workflow: publishes `docs/mockups/` on every
  push to `main` that touches it, via the native Actions-artifact deploy
  (no `gh-pages` branch to keep in sync by hand).
- `CHANGELOG.md`, `RELEASE_NOTES.md`, and a per-tag archive under
  `docs/releases/` — three-tier release documentation, with
  `scripts/cut-release.sh` to archive/reset mechanically.
- An original `argonone` OLED boot/rotation screen — `RPI` rotated 90°,
  detected Raspberry Pi model large and horizontal, `argonone` signature
  — replacing Argon40's `logo1v5.bin` splash entirely.
- Full decode of Argon40's OLED `.bin` asset format (backgrounds and
  fonts), including the font format's inverted bit order relative to
  backgrounds, documented for the future Rust OLED blitter.
- Legal/licensing research: confirmed no license exists on any Argon40
  script or asset (including their own official GitHub mirror), and
  that reimplementing the hardware protocol is unproblematic separate
  from that (Argon40's ToS has no reverse-engineering clause; real
  precedent exists on their own community forum).
- HTTPS research: tiered plan (Tailscale-issued certs primary,
  `rustls-acme`/TLS-ALPN-01 for a custom domain, plain HTTP as an
  explicit opt-out) — no self-signed-cert theater, since that doesn't
  actually satisfy "trusted by browsers."
- Initial Rust project scaffold (`Cargo.toml`, `Cargo.lock`, `src/main.rs`).
- `HardwareBackend` trait (`FanBackend`/`PowerButtonBackend`) with no-op
  fallbacks, so the daemon never crashes on a Pi without the case
  attached (v0.1.0, W§1.4).
- I2C fan controller backend (`0x1a`) with register-vs-legacy capability
  auto-detection, matching `argonregister_checksupport` (v0.1.0, W§1.1).
- Board auto-detection (Argon ONE vs EON) by probing for the OLED
  (`0x3c`) and RTC (`0x51`) addresses at runtime, not an install-time
  flag (v0.1.0, W§2.6).
- GPIO power-button monitor (BCM pin 4, character-device v2 uAPI via the
  `gpiod` crate), classifying pulse width into reboot/shutdown/OLED-
  switch actions — replaces the old sysfs/RPi.GPIO code paths outright
  rather than replicating them (v0.1.0, W§1.1).

### Changed

- Revised the original "self-signed cert on first run" TLS plan after
  confirming it doesn't satisfy "browser-trusted" — replaced with the
  tiered Tailscale/rustls-acme/HTTP plan above.
- Closed eleven gaps found by re-reading both research docs against each
  other: API/WebSocket contract, runtime board detection, live
  fan-curve-reload mechanism, a server-enforced fan-curve safety floor,
  CI test/dependency-audit coverage, the previously-open frontend-stack
  decision (resolved: htmx + minijinja), the setup-wizard exposure
  window, DB backup/restore, and a `.deb` packaging plan.
- Corrected the identity of `logo1v5.bin`: it's not Argon40's corporate
  logo, it decodes to "ONE V5" (a product/version splash for the Argon
  ONE V5 case) — same trademark caution applies, description is now
  accurate.

### Fixed

- `docs/mockups/00-index.html`: a duplicate CSS grid declaration on
  nested elements (`.flow li` and its only child `.flow a` both declaring
  the same `display:grid`) was squeezing each screen-flow row into a
  56px-wide column, wrapping text and stranding the role tag. Verified
  the fix by rendering both themes with a headless browser.

[Unreleased]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/53094f5...HEAD
