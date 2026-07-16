# Changelog

All notable changes to this project are documented here, grouped by release. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows the policy in [docs/ROADMAP.md](docs/ROADMAP.md) (`0.MINOR.0` per milestone, `0.MINOR.PATCH` for unplanned fixes against the current milestone).

This file is the permanent, cumulative log across every version. For the prose write-up of just the *current* unreleased work (what a tag's GitHub Release page will show), see [RELEASE_NOTES.md](RELEASE_NOTES.md). Once a version is tagged, that release's notes are archived permanently under [docs/releases/](docs/releases/).

## [Unreleased]

## [v0.4.0] - 2026-07-16

Core dashboard: fan control, storage, system — the highest-value milestone, replacing `argonone-fanconfig.sh`/`argon-unitconfig.sh` and friends with the web UI. See [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

### Added

- Core dashboard (v0.4.0, W§2.5/§2.7/§2.8, W§3.4): draggable-SVG fan curve editor (CPU/HDD tabs), Storage & RAID page, and a System page (units toggle + firmware/service info) — the web UI's first real feature screens, replacing `argonone-fanconfig.sh`/`argon-unitconfig.sh` and friends.
- `PUT /api/fan/curve/{cpu,hdd}` and `GET/PUT /api/settings/units`, operator+ gated; edits apply to the running fan control loop live via `tokio::sync::watch` channels, without restarting the daemon or losing hysteresis state (W§2.7).
- Server-enforced fan-curve safety floor (W§2.8): rejects any curve implying less than 25% fan at or above 75°C, checked at every configured breakpoint at/above the ceiling (not just one point), so an unsafe *gap* between two otherwise-safe points is caught too.
- The HDD curve is now actually applied to fan control, not just editable — `max(cpu_curve_speed, hdd_curve_speed)` each poll, matching the documented "the higher value wins" behavior.
- New sysinfo surface: per-disk S.M.A.R.T. temperature (`smartctl`), whole-disk enumeration (`lsblk`), and richer `/proc/mdstat` RAID parsing (level, size, working/failed/spare disk counts, member device list) — previously only a coarse name+state summary.
- Fan curves and the temperature-unit setting are now DB-backed (`fan_curve_points`/`settings` tables), replacing the config-file source of truth per the plan already recorded when those files were first read (v0.1.0) — `argonone-rs status` reads the same values the running daemon applies, so the two can't drift.
- Shared authenticated app-shell template (sidebar navigation, status strip) factored out of the v0.3.0 dashboard shell and reused across all four authenticated pages.
- `scripts/deploy.sh`/`scripts/deploy-local.sh`: script the manual cross-compile → scp → systemd-install sequence from README's Troubleshooting section, guarding against the real failure modes hit during on-hardware deployment (I2C conflicts, a Plymouth boot stall, and a redeploy not actually picking up the new binary).
- A shared toast notification component (`app_shell.html`) for save/action success feedback, matching every mockup's pattern — previously the running app had no positive confirmation anywhere, only inline error text on failure. Wired into the fan curve editor's save and the System page's units toggle.
- A live "now: NN°C → NN%" indicator on the fan curve chart (badge plus a marker line/dot), driven by the existing `stats`/`fan_state` WebSocket messages, matching `04-fan-curve-editor.html`'s current-operating-point marker.
- Storage & RAID page: a Role column classifying each disk as a RAID member or `NN% full`, plus severity-banded (good/warn/crit) coloring on the usage progress bar and temperature reading, matching `05-storage-raid.html`.
- **Not yet done**: verified on real hardware with actual disks/RAID attached — unlike v0.1.0-v0.3.0, this milestone's development pass ran without a case or block devices attached, so the fan safety floor, HDD-curve behavior, and `lsblk`/`smartctl`/`/proc/mdstat` parsing are covered by unit tests against synthetic/captured output only. That pass is still required before this milestone meets the bar every prior release did.

### Fixed

- Web temperature displays didn't all honor the configured unit setting — some pages/panels stayed hardcoded to Celsius after switching to Fahrenheit. All temperature output now goes through one shared `TempUnit::convert_c`/`suffix` conversion.
- Storage page disk-usage matching compared a mount path (e.g. `/`) against a device name (e.g. `sda`), which essentially never matches — usage always showed `—`. Fixed by matching against `df`'s `Filesystem` column instead, via a new `filesystem_belongs_to_device` leaf-component matcher (also reused for RAID-membership matching).
- `.btn` had a blanket `width:100%` meant only for the single-column auth forms (login/setup/change-password), but was reused unscoped elsewhere — Save/Reset buttons on the fan curve page would have stretched full-width, and `.btn.ghost` was never actually styled. Scoped the full-width rule to those forms and the sidebar logout button, and added the missing ghost variant.
- The System page's Celsius/Fahrenheit selector only changed text color on the active option, unlike every other selector control in the app; restored the mockup's raised background-pill highlight.

## [v0.3.0] - 2026-07-15

Web foundation: persistence, auth, live shell — the first version with a web server, scoped to infrastructure rather than feature screens. Still no HTTPS, fan curve editing, or settings screens. See [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

### Added

- Web foundation (A§1-2, W§2.5, §3.5): SQLite persistence (`users`/`settings`/`fan_curve_points`/`audit_log`, WAL mode, embedded `sqlx::migrate!()`), forced first-run admin setup wizard with the singleton-race guard, Argon2id auth via `axum-login` + SQLite-backed `tower-sessions`, three-role RBAC (`admin`/`operator`/`viewer`), a bare authenticated `axum` + `minijinja` + `htmx` shell, and a live `stats`/`fan_state` WebSocket over `htmx-ext-ws`.
- Password recovery mechanism (A§1.2): admin-issued reset (`POST /api/users/{id}/reset-password`) and a CLI fallback (`argonone-rs admin reset-password --username <u>`) for the no-admin-can-login case — the Users admin *page* itself is still v0.5.0.
- `must_change_pw` forced-change flow and a DB-backed failed-login throttle (5 attempts, 15-minute lockout, A§2.2).
- `GET /api/status` (auth-gated) doubling as a health check, reporting hardware presence alongside CPU/RAM/temp/fan stats.
- `htmx`/`htmx-ext-ws` vendored from upstream GitHub releases (not a CDN) into `assets/`, embedded into the binary — keeps the single-binary deploy story intact; see `assets/VENDORED.md` for provenance.
- systemd unit gains `StateDirectory=argonone-rs` for the new SQLite state file (A§3.2); the `argonone` service-account privilege drop stays deferred to v0.7.0's packaging scope.
- v0.3.0 verified end-to-end on real Argon ONE hardware: board auto-detection, the full setup/login/session flow, and `GET /api/status` returning live sysinfo all confirmed over the network from a browser on the LAN.

### Fixed

- CI: audit-check's `RUSTSEC-2023-0071` (rsa timing sidechannel) false positive, reachable only via `sqlx-mysql`'s always-pinned-but-never-compiled lockfile entry (we only enable sqlx's `sqlite` feature) — ignored with justification in `ci.yml` rather than left failing the job.

## [v0.2.0] - 2026-07-14

EON extras (OLED + RTC) — completes Python-parity for both case models, still CLI/systemd only, no web server. See [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

### Added

- EON OLED support (W§1.2, §1.7): screen-rotation state machine (`switchduration` cycling, screensaver blank-after-idle, power-button `OledSwitch` force-advance/wake), seven dashboard screens (clock, IP, CPU, RAM, storage, temp, RAID) plus an original splash screen (`RPI` rotated 90°, detected Pi model, `argonone` signature), and `/etc/argoneonoled.conf` config-file compat.
- **Blocking asset-sourcing decision resolved** (W§1.5): fonts/backgrounds are regenerated via the `embedded-graphics`/`ssd1306` crates' bundled, permissively-licensed fonts and primitive-drawn backgrounds rather than vendoring or fetching Argon40's originals — no bytes of Argon40's `.bin` assets used, and no reason to replicate their bespoke per-plane font packing since this project owns the whole render path.
- EON RTC support (W§1.1): PCF8563 register access with BCD encode/decode, daily wake-alarm programming, and a daily sleep (scheduled poweroff) check against the RTC's own clock — both driven by a new `/etc/argonrtc.conf` (`wake=`/`sleep=` `HH:MM`, `enabled=`), config-file-driven only per the v0.2.0 scope (no web UI yet).
- `argonstatus`-parity additions to the `status` command: RTC current time and configured wake/sleep schedule.
- `HardwareBackend` gains `OledBackend`/`RtcBackend` seams with no-op fallbacks, following the same pattern as the fan/button backends — inert on Argon ONE/no-case builds.
- v0.2.0 verified end-to-end on real Argon EON hardware: OLED dashboard, screen rotation, splash screen, and RTC wake/sleep scheduling all confirmed on-device.

### Fixed

- CI: `audit` job now grants `checks: write` so `rustsec/audit-check` can post its results as a GitHub check run — it previously succeeded locally but failed the job with "Resource not accessible by integration" under the default read-only `GITHUB_TOKEN`.
- Clippy: dead-code lint allowances scoped to `not(target_os = "linux")` for items only ever constructed/called by Linux-only hardware code (fan/button/board enum variants, the OLED render/splash modules), so macOS dev builds lint clean without weakening the Linux target's (CI's actual target) strict dead-code checking.

## [v0.1.0] - 2026-07-13

Core hardware daemon (Argon ONE parity) — CLI/systemd only, no web server yet. See [docs/ROADMAP.md](docs/ROADMAP.md) for what "released" means at this milestone and what's next.

### Added

- Research: existing Argon40 Python stack fully audited (I2C/GPIO hardware surface, config file formats, daemon behavior), proposed Rust daemon architecture, and web UI/UX research (`docs/research-rust-backend-webui.md`).
- Research: forced first-run admin setup, three-tier RBAC, SQLite persistence strategy, and systemd service/privilege design for Ubuntu 26.04 on Raspberry Pi (`docs/research-auth-persistence-service.md`).
- Nine interactive, animated HTML mockups covering every planned screen — setup, login, dashboard, fan curve editor, storage/RAID, OLED display, users, system settings (`docs/mockups/`).
- `docs/ROADMAP.md` — seven dependency-ordered implementation milestones (`v0.1.0` → `v0.7.0`), each citing the research section it's grounded in.
- CI workflow: `fmt` → `clippy` → `test` → `audit` gating build jobs for aarch64 Raspberry Pi (primary/blocking) and macOS (secondary/dev-only, non-blocking).
- Release workflow: tag-triggered (`v*.*.*`), cross-compiled Pi release binary, GitHub Release titled `Release <tag>`.
- GitHub Pages deployment workflow: publishes `docs/mockups/` on every push to `main` that touches it, via the native Actions-artifact deploy (no `gh-pages` branch to keep in sync by hand).
- `CHANGELOG.md`, `RELEASE_NOTES.md`, and a per-tag archive under `docs/releases/` — three-tier release documentation, with `scripts/cut-release.sh` to archive/reset mechanically.
- An original `argonone` OLED boot/rotation screen — `RPI` rotated 90°, detected Raspberry Pi model large and horizontal, `argonone` signature — replacing Argon40's `logo1v5.bin` splash entirely.
- Full decode of Argon40's OLED `.bin` asset format (backgrounds and fonts), including the font format's inverted bit order relative to backgrounds, documented for the future Rust OLED blitter.
- Legal/licensing research: confirmed no license exists on any Argon40 script or asset (including their own official GitHub mirror), and that reimplementing the hardware protocol is unproblematic separate from that (Argon40's ToS has no reverse-engineering clause; real precedent exists on their own community forum).
- HTTPS research: tiered plan (Tailscale-issued certs primary, `rustls-acme`/TLS-ALPN-01 for a custom domain, plain HTTP as an explicit opt-out) — no self-signed-cert theater, since that doesn't actually satisfy "trusted by browsers."
- Initial Rust project scaffold (`Cargo.toml`, `Cargo.lock`, `src/main.rs`).
- `HardwareBackend` trait (`FanBackend`/`PowerButtonBackend`) with no-op fallbacks, so the daemon never crashes on a Pi without the case attached (v0.1.0, W§1.4).
- I2C fan controller backend (`0x1a`) with register-vs-legacy capability auto-detection, matching `argonregister_checksupport` (v0.1.0, W§1.1).
- Board auto-detection (Argon ONE vs EON) by probing for the OLED (`0x3c`) and RTC (`0x51`) addresses at runtime, not an install-time flag (v0.1.0, W§2.6).
- GPIO power-button monitor (BCM pin 4, character-device v2 uAPI via the `gpiod` crate), classifying pulse width into reboot/shutdown/OLED-switch actions — replaces the old sysfs/RPi.GPIO code paths outright rather than replicating them (v0.1.0, W§1.1).
- Config file compat parsers for `/etc/argononed.conf`, `/etc/argononed-hdd.conf`, and `/etc/argonunits.conf`, unchanged from the Python daemon's plain-text formats so an existing install carries over without reformatting (v0.1.0, W§1.3).
- Fan control loop: 30s poll, temp→speed curve, hysteresis on speed decreases (held until sustained for a full poll window) to avoid audible fan flapping (v0.1.0, W§1.4).
- Sysinfo collection: CPU% (`/proc/stat` deltas), RAM (`/proc/meminfo`), CPU temp (thermal zone), disk usage (`df`), RAID status (`/proc/mdstat`), and local IP (UDP-connect trick, no packets sent) (v0.1.0, W§1.2).
- CLI: `SERVICE`/`SHUTDOWN`/`FANOFF` argv compat (matching the Python daemon's exact invocation spelling) plus a new `status` one-shot command (`argonstatus.py` pretty-printer parity), and the daemon orchestration wiring the fan loop, button monitor, and systemd `sd_notify` readiness together (v0.1.0, W§4, A§4.2).
- systemd unit (`packaging/systemd/argonone-rs.service`), `Type=notify`.
- v0.1.0 verified end-to-end on real Argon ONE hardware (Raspberry Pi, aarch64 Ubuntu 26.04 over `ssh euclides`): `cargo build`/`clippy`/`test`/`fmt` all clean natively on-device, and the no-op fallback path confirmed by running without I2C/GPIO permissions.
- Cross-compilation to `aarch64-unknown-linux-gnu` verified from a non-Pi host.
- `Cargo.toml` publish metadata (`license`, `description`, `repository`, `keywords`, `categories`, explicit `[[bin]]`) so `cargo publish` works cleanly; verified with `cargo publish --dry-run`.
- README: installation section (crates.io, build from source, cross-compile for Raspberry Pi) and a License section.
- Unit test coverage: 38 tests (up from 11) — legacy-uppercase CLI token normalization; `FanCurve`/`TempUnit` `load_or_default` (missing file, parse errors, unit parsing); fan-controller backend-failure and mid-hold target-change hysteresis edge cases; `hardware::detect()` and `board::detect()` no-op/no-case fallback paths; `NoopFan`/`NoopPowerButton`; pure parsing extracted from `sysinfo` (mem info, disk usage, RAID status, CPU temp) so it's testable without touching `/proc`/`/sys`; `service::handle_button_event`'s inert `OledSwitch` case and `spawn_system_command`'s success/failure/missing-binary paths. Added `tempfile` as a dev-dependency for the config-file tests.

### Changed

- Revised the original "self-signed cert on first run" TLS plan after confirming it doesn't satisfy "browser-trusted" — replaced with the tiered Tailscale/rustls-acme/HTTP plan above.
- Closed eleven gaps found by re-reading both research docs against each other: API/WebSocket contract, runtime board detection, live fan-curve-reload mechanism, a server-enforced fan-curve safety floor, CI test/dependency-audit coverage, the previously-open frontend-stack decision (resolved: htmx + minijinja), the setup-wizard exposure window, DB backup/restore, and a `.deb` packaging plan.
- Corrected the identity of `logo1v5.bin`: it's not Argon40's corporate logo, it decodes to "ONE V5" (a product/version splash for the Argon ONE V5 case) — same trademark caution applies, description is now accurate.

### Fixed

- `docs/mockups/00-index.html`: a duplicate CSS grid declaration on nested elements (`.flow li` and its only child `.flow a` both declaring the same `display:grid`) was squeezing each screen-flow row into a 56px-wide column, wrapping text and stranding the role tag. Verified the fix by rendering both themes with a headless browser.

[Unreleased]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/v0.1.0...HEAD
[v0.1.0]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/53094f5...v0.1.0
