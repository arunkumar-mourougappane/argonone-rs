# Changelog

All notable changes to this project are documented here, grouped by release. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows the policy in [docs/ROADMAP.md](docs/ROADMAP.md) (`0.MINOR.0` per milestone, `0.MINOR.PATCH` for unplanned fixes against the current milestone).

This file is the permanent, cumulative log across every version. For the prose write-up of just the *current* unreleased work (what a tag's GitHub Release page will show), see [RELEASE_NOTES.md](RELEASE_NOTES.md). Once a version is tagged, that release's notes are archived permanently under [docs/releases/](docs/releases/).

## [Unreleased]

## [v0.6.0] - 2026-07-19

HTTPS, dashboard data-surface gaps, hardening — every v0.6.0 roadmap item, a full-fidelity mockup-parity pass, and a comprehensive post-implementation bug sweep across all eight feature areas. See [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

### Added

- HTTPS (`src/https.rs`, W§4.4): mode dispatch between plain HTTP, Tailscale-issued certs (`tailscale cert` + daemon-owned renewal via `axum-server`'s `RustlsConfig`), and `rustls-acme`/TLS-ALPN-01 for a custom domain. `SessionManagerLayer`'s `Secure` cookie flag now follows the active mode instead of a hardcoded `false`. Tailscale mode also surfaces the real on-disk cert's issuer/expiry/auto-renew status (parsed via `x509-parser`) and a manual "Re-issue now" action, in `templates/system.html`'s HTTPS card. **Not yet verified on real Tailscale/ACME infrastructure** — needs an actual Tailscale-joined device and a real domain to confirm both flows end-to-end.
- Audit log viewer (W§3.6): admin-only, paginated `GET /audit` with actor/action filters and per-category action-badge coloring — `src/db/audit.rs`, `src/web/audit.rs`, `templates/audit.html`. `audit_log` has been populated since v0.5.0; this is the first screen that reads it back.
- IR remote learn/program (W§3.2): `FanBackend::learn_ir_code`/`program_ir_code` over I2C register `0x82`, wired into `/system`'s IR card. **Unverified against real hardware** — the only documentation for that register is one line ("IR code (block write)"), so this is a best-effort reconstruction (a listen-window sentinel write, then a block read) pending confirmation on a real case.
- Setup-wizard exposure window (A§1.1): a one-time, console-printed setup token required on `/setup`, regenerated every boot while no admin exists yet, consumed atomically in the same transaction as the winning admin insert so a losing racer's request can't replay it.
- Network throughput, load average, and swap (W§3.3 Tier 1) — `sysinfo::read_load_avg`, `MemInfo::swap_used_percent`, `NetUsage::sample_rates` (diffing `/proc/net/dev` against the default-routed interface), surfaced on both `GET /api/status` and `/api/ws`.
- Full dashboard card-grid rebuild (`src/web/dashboard.rs`, `templates/dashboard.html`), matching `03-dashboard.html`: Fan control, Power & RTC (EON), Network (with a client-side rolling sparkline), Storage, Display (EON, a live OLED thumbnail), System, and Signed-in-as — each card reusing its own dedicated page's existing data rather than a second copy of that logic. Pulls v0.7.0's card-grid contents forward a milestone. `rtc_schedule::next_wake` generalized into `next_occurrence`/`next_sleep` for the Power & RTC card's "next sleep" row.
- Self-service password change (W§3.6): a sidebar account-menu link to the existing `/account/change-password` route, with context-aware copy for the voluntary-vs.-forced flow, a Cancel button, and a post-change login notice — previously only reachable via the forced `must_change_pw` redirect.
- Status ribbon's Fan column: a fixed-width ▲/▼ trend indicator (derived from `fan_state`'s `target_pct` vs. current speed) replacing a variable-width "· ramping" suffix that reflowed the whole ribbon whenever it appeared or disappeared.
- Sidebar/shell fidelity pass: nav icons, brand mark with hostname, a collapsible avatar account-menu, and the mockups' shared motion language (`fadeUp` entrance stagger, pulsing live-status dot, flash-on-update, card hover-lift), all gated behind `prefers-reduced-motion`. Status strip restructured to a flush full-width bar matching the mockups, not an inset card.
- `WS_TICK_INTERVAL` reduced 2s → 1s (sparkline point count doubled to match) so the dashboard's live charts read as a smooth trend rather than a handful of visible segments.
- No UPS/battery, NIC link-speed, or MCU-firmware-register data exists anywhere in this codebase, so the mockups' rows depending on those are intentionally omitted rather than fabricated (noted inline in `docs/ROADMAP.md`).

### Fixed

Mockup-fidelity pass (fan curve chart, and cross-page consistency):

- Fan/HDD curve chart: temperature axis started at 0°C instead of the mockup's 30°C, wasting a third of the chart on temperatures no default curve uses; saved points below the new 30°C floor then rendered off-canvas (x was unclamped); and a fixed SVG height combined with `preserveAspectRatio="none"` distorted data points into ellipses at any card width other than exactly 620px. Fixed the axis range, clamped rendered (not table-displayed) x-coordinates, and replaced the fixed height with `aspect-ratio`.
- `fan_curve.html`/`storage.html`/`oled.html`/`users.html` had drifted from their own mockups (missing icons, no entrance/hover motion) predating the dashboard/sidebar fidelity pass; `storage.html`'s RAID device chips were also discarding `RaidDevice.spare` when building the role label. All reconciled per-file against each page's own mockup.
- Cross-page CSS drift left by fixing pages independently: `audit.html`/`system.html` missing the shared `fadeUp` entrance animation; three different form-control background tokens in use for the same concept across pages; four pages locally redeclaring a byte-identical `@keyframes fadeUp`; `audit.html`'s avatar sizing invented rather than matching its own mockup.

Bug-check sweep (whole-codebase correctness review, not diff-scoped):

- Login lockout had a TOCTOU race: concurrent requests could each read the failed-attempt count before any of them recorded a new failure, letting an attacker exceed `MAX_FAILED_ATTEMPTS` via parallel guesses. Closed with a per-`Backend` async mutex serializing the whole check-then-record sequence.
- `PUT /api/fan/curve/{cpu,hdd}` validated the safety floor against client-submitted point order, but `FanCurve::speed_for` (and `curve_store::load`'s `ORDER BY temp_c DESC`) assumes descending order — an unsorted submission could pass validation in the order it arrived, then evaluate differently (and unsafely) after the next restart. Points are now sorted before validating.
- The one-time setup token's persistence failure was swallowed silently, and `/setup`'s "no token on file" check read as "no token required" — a DB write failure at boot could have opened first-run admin claiming to anyone on the LAN. `generate_and_store_setup_token` now surfaces the error, and the missing-token check fails closed.
- `POST /api/system/ir/learn` called the ~2-second blocking I2C listen window directly inside its async handler, stalling the whole tokio runtime (dashboard, WebSocket ticks, every other user's requests) for the entire window. Offloaded to `tokio::task::spawn_blocking`.
- A RAID member marked `(F)` (failed) in `/proc/mdstat` was only ever checked against the `(S)` (spare) marker, so it fell through to "active sync" — the storage page showed a dead disk as healthy. `RaidDevice` now tracks `failed` distinctly, with a new crit-styled "faulty" devchip.
- Board auto-detection (`Board::One` vs `Board::Eon`) treated any single I2C probe error as "address absent," so a momentary bus glitch at boot could permanently misdetect a real EON as a plain ONE for that entire boot. Probes now retry up to three times before giving up.
- The dashboard's storage card picked a disk's RAID level label via an unordered `HashMap` lookup — a disk with members in more than one array could show a different array's level on every restart, depending on that process's randomized hash-iteration order. Replaced with an ordered `Vec` scan that always picks the same (first) match.
- The WebSocket "ramping" indicator computed its target from the CPU curve alone, understating it whenever the HDD curve was the one actually pinning the fan speed higher (`max(cpu_target, hdd_floor)`, matching the real control loop). The control loop now publishes its per-poll disk temperature over a new watch channel so the web layer can fold the HDD floor in without re-shelling `smartctl` on every tick.
- The one-shot `SHUTDOWN`/`FANOFF` CLI commands opened `/dev/i2c-1` on their own file descriptor with no coordination against the running daemon's own I2C access, despite being invoked independently of it (e.g. a systemd shutdown hook). Added `hardware::lockfile`, a cross-process `flock`-backed advisory lock now held around every I2C operation.
- `filesystem_belongs_to_device`'s unanchored prefix match misattributed a longer device name's partitions to a shorter one sharing a prefix (`sdaa1` to `sda`; `nvme0n10p1` to `nvme0n1` — a two-digit NVMe namespace). Now checks the byte after the prefix against Linux's own partition-naming convention.
- `learn_ir_code` used an all-zero sentinel to mark "nothing captured yet," making a genuinely-learned all-zero code indistinguishable from no capture at all. Switched to a fixed non-zero sentinel.

## [v0.5.0] - 2026-07-17

### Added

- Users admin page (v0.5.0, mirrors `07-users-rbac.html`): full CRUD — `GET/POST /api/users`, `DELETE /api/users/{id}`, `PUT /api/users/{id}/role`, all admin-only — plus the `/users` page itself. Refuses to delete or demote the last remaining admin, and refuses self-delete, so user management can't lock itself out. New `src/db/users.rs` mirrors `db/settings.rs`'s query-style conventions; no migration needed, the `users` table already had `first_name`/`last_name`/`created_at`/`last_login_at` sitting unused by the auth-hot-path `User` struct.
- Power & RTC schedule card on `/system` (v0.5.0, EON-only, mirrors `08-system-settings.html#power`): a full multi-entry, day-of-week wake/sleep schedule, replacing the old single-wake/single-sleep `/etc/argonrtc.conf` model. `GET/PUT /api/rtc/schedule`, `viewer+`/`operator+`, 404s on non-EON boards. `RtcBackend::set_wake_alarm` now takes an optional weekday — the PCF8563 has exactly one alarm slot (fires "every day" or one specific weekday, never an arbitrary multi-day set), so new `src/rtc_schedule.rs` resolves a full schedule down to the single next occurrence to arm: once at startup, on every schedule edit, and again before every self-triggered sleep (nothing's running to reprogram the alarm once the Pi is actually powered off). Sleep matching now checks day-of-week too, not just hour:minute.
- OLED config page (v0.5.0, `/display`, EON-only, mirrors `06-oled-display.html`): drag-to-reorder screen rotation list, per-screen enable toggle, switch-duration/screensaver-timeout sliders, and a panel-enable switch, all auto-saving via `GET/PUT /api/oled/config`. The live preview is a real render, not a simulation — new `src/oled/framebuffer.rs` adds an in-memory `DrawTarget` so the same `draw_screen` function the physical panel uses renders into memory instead; `GET /api/oled/preview` returns the currently-selected screen's packed 1bpp pixels as a plain JSON byte array (no image-encoding dependency needed for a 1024-byte 128×64 frame). `service::render_oled_tick` publishes the selected screen on a watch channel whenever the *selection* changes; `/api/ws` forwards it as `{"type":"oled_screen","name":...}` per the documented contract, driving the preview's refresh without polling.
- `OledConfig`/`RtcSchedule` are now DB-backed (`db/settings.rs::load_oled_config`/`save_oled_config`, `load_rtc_schedule`/`save_rtc_schedule`), mirroring fan curves/units — the config files stay the fallback default until something's actually been saved. Both push live-apply updates through `service::run`'s existing `tokio::sync::watch` pattern.
- Sidebar nav visibility (Display link, Power & RTC card) now depends on `board == Eon`, threaded through every authenticated page's template context.
- v0.5.0 verified on real Argon ONE hardware: board auto-detection correctly identified `Board::One`, the fan curve editor's live response on the actual fan, and the Users admin page end-to-end (login, create/delete users, role changes, password reset, lockout/unlock). RTC scheduling and OLED config remain unverified — both are EON-gated and an Argon ONE has neither chip present.

### Fixed

- `audit_log` gap: `settings.update_units` never wrote an audit entry despite `fan_curve.update`/`user.reset_password` already doing so. Closed alongside the new user-management actions, which all audit-log consistently now.
- Users: a newly-created account's temporary password was generated server-side but never shown to the admin — the response was discarded after checking `resp.ok`, same gap in the reset-password handler. Both now surface it in a persistent, dismissible "cred-reveal" panel with a copy button.
- RTC: disabling the schedule (or deleting its last Wake entry) never called `clear_alarm()` — the physical PCF8563 alarm stayed armed regardless of what the UI/DB said. `apply_rtc_wake_alarm` now clears the alarm on both the disabled and no-wake-entries paths.
- OLED: every control (screen toggle, drag-reorder, sliders, panel-enable) mutated the UI optimistically before saving, but a failed `PUT` never rolled any of it back. `saveConfig()` now takes a pre-mutation snapshot and restores it (DOM included) on failure.
- Users: `delete`/`update_role`'s last-admin guard was a separate check-then-act (a count query, then the write) — a TOCTOU race under concurrent requests. Now a single atomic SQL statement, the `COUNT(*)` guard folded into the same `DELETE`/`UPDATE`.
- Users: no "Locked" badge or "Unlock" action existed despite the mockup designing for it. Added `UserRow.is_locked`, a Locked badge, `POST /api/users/{id}/unlock`, and network-error (not just HTTP-error) handling on every `fetch()` in `users.html`.
- CSS: `base.html`'s `.card{max-width:440px}` (meant only for the centered login/setup/change-password cards) leaked into every page reusing the `.card` class — `users.html` and `oled.html` never set their own `max-width` (unlike `fan_curve.html`/`system.html`/`storage.html`, which happened to), so the Users table and every Display-page card rendered squeezed into a fraction of the available width. Scoped to `.center-page`.
- CSS: the Users page's "Create" button had a stray `margin-top` (meant for stacked form-submit buttons elsewhere) misaligning it from the input row it shares.
- Dashboard status strip had drifted from its mockup since v0.3.0's bare shell: flat text instead of the stacked label/value/severity-dot layout, a CPU% stat the mockup doesn't show, and no IP address or Uptime. Restyled to match, added `sysinfo::read_uptime_secs`/`read_local_ip` reaching the web layer, dropped CPU%.
- Sidebar brand block never rendered the hostname despite the mockup showing it under "argonone" (e.g. `rpi01.lan`) — added `sysinfo::read_hostname`, sent once over `/api/ws` on connect.

## [v0.4.0] - 2026-07-16

Core dashboard: fan control, storage, system — the highest-value milestone, replacing `argonone-fanconfig.sh`/`argon-unitconfig.sh` and friends with the web UI. See [docs/ROADMAP.md](docs/ROADMAP.md) for what's next.

### Added

- Core dashboard (v0.4.0, W§2.5/§2.7/§2.8, W§3.4): draggable-SVG fan curve editor (CPU/HDD tabs), Storage & RAID page, and a System page (units toggle + firmware/service info) — the web UI's first real feature screens, replacing `argonone-fanconfig.sh`/`argon-unitconfig.sh` and friends.
- `GET/PUT /api/fan/curve/{cpu,hdd}` and `GET/PUT /api/settings/units` — `viewer+` reads, `operator+` writes; edits apply to the running fan control loop live via `tokio::sync::watch` channels, without restarting the daemon or losing hysteresis state (W§2.7).
- Server-enforced fan-curve safety floor (W§2.8): rejects any curve implying less than 25% fan at or above 75°C, checked at every configured breakpoint at/above the ceiling (not just one point), so an unsafe *gap* between two otherwise-safe points is caught too.
- The HDD curve is now actually applied to fan control, not just editable — `max(cpu_curve_speed, hdd_curve_speed)` each poll, matching the documented "the higher value wins" behavior.
- New sysinfo surface: per-disk S.M.A.R.T. temperature (`smartctl`), whole-disk enumeration (`lsblk`), and richer `/proc/mdstat` RAID parsing (level, size, working/failed/spare disk counts, member device list) — previously only a coarse name+state summary.
- Fan curves and the temperature-unit setting are now DB-backed (`fan_curve_points`/`settings` tables), replacing the config-file source of truth per the plan already recorded when those files were first read (v0.1.0) — `argonone-rs status` reads the same values the running daemon applies, so the two can't drift.
- Shared authenticated app-shell template (sidebar navigation, status strip) factored out of the v0.3.0 dashboard shell and reused across all four authenticated pages.
- `scripts/deploy.sh`/`scripts/deploy-local.sh`: script the manual cross-compile → scp → systemd-install sequence from README's Troubleshooting section, guarding against the real failure modes hit during on-hardware deployment (I2C conflicts, a Plymouth boot stall, and a redeploy not actually picking up the new binary).
- A shared toast notification component (`app_shell.html`) for save/action success feedback, matching every mockup's pattern — previously the running app had no positive confirmation anywhere, only inline error text on failure. Wired into the fan curve editor's save and the System page's units toggle.
- A live "now: NN°C → NN%" indicator on the fan curve chart (badge plus a marker line/dot), driven by the existing `stats`/`fan_state` WebSocket messages, matching `04-fan-curve-editor.html`'s current-operating-point marker.
- Storage & RAID page: a Role column classifying each disk as a RAID member or `NN% full`, plus severity-banded (good/warn/crit) coloring on the usage progress bar and temperature reading, matching `05-storage-raid.html`.
- v0.4.0 verified end-to-end on real Argon ONE/EON hardware: the fan curve editor applying live to the real fan, the 25%-at-75°C safety floor, the Storage & RAID page against actual attached disks and a real RAID array, and the units toggle propagating across every display, all confirmed on-device.

### Fixed

- Web temperature displays didn't all honor the configured unit setting — some pages/panels stayed hardcoded to Celsius after switching to Fahrenheit. All temperature output now goes through one shared `TempUnit::convert_c`/`suffix` conversion.
- Storage page disk-usage matching compared a mount path (e.g. `/`) against a device name (e.g. `sda`), which essentially never matches — usage always showed `—`. Fixed by matching against `df`'s `Filesystem` column instead, via a new `filesystem_belongs_to_device` leaf-component matcher (also reused for RAID-membership matching).
- `.btn` had a blanket `width:100%` meant only for the single-column auth forms (login/setup/change-password), but was reused unscoped elsewhere — Save/Reset buttons on the fan curve page would have stretched full-width, and `.btn.ghost` was never actually styled. Scoped the full-width rule to those forms and the sidebar logout button, and added the missing ghost variant.
- The System page's Celsius/Fahrenheit selector only changed text color on the active option, unlike every other selector control in the app; restored the mockup's raised background-pill highlight.
- `GET /api/settings/units` was missing entirely — only `PUT` was wired, despite both being documented `viewer+`/`operator+` in the API contract (W§2.5). A viewer had no way to read the setting except via the broader `GET /api/status` payload.

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

[Unreleased]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/v0.4.0...HEAD
[v0.4.0]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/v0.3.0...v0.4.0
[v0.3.0]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/v0.2.0...v0.3.0
[v0.2.0]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/v0.1.0...v0.2.0
[v0.1.0]: https://github.com/arunkumar-mourougappane/argonone-rs/compare/53094f5...v0.1.0
