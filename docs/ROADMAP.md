# Roadmap

Planned feature releases for **argonone-rs**, sequenced by dependency —
each milestone builds on what the previous one shipped, not by
importance alone. Bumps the middle (`0.MINOR.0`) version number per
milestone; any bug fixes found along the way ship as patch releases
(`0.MINOR.PATCH`) against the current milestone without pulling forward
unplanned scope. No `v1.0.0` criteria defined yet — that's a call to make
once the milestones below are field-tested on real hardware, not assumed
here.

Each item cites the research doc section it's grounded in
([`research-rust-backend-webui.md`](research-rust-backend-webui.md) = **W**,
[`research-auth-persistence-service.md`](research-auth-persistence-service.md) = **A**)
so the "why" behind a milestone item is one click away, not re-argued here.

---

## Phase 0 — Research & design (done)

Not a version, the groundwork everything below stands on: hardware
protocol + OLED asset format fully decoded (W§1.1–§1.4, §1.7), reuse/
reimplementation legality checked against Argon40's actual ToS (W§1.6),
Rust architecture and API/WebSocket contract designed (W§2), auth/RBAC/
persistence/systemd/HTTPS/packaging designed (A§1–§4.5), CI (`fmt` →
`clippy` → `test` → `audit` → build) and release workflows already live,
original OLED splash screen designed and ready to encode. Interactive
mockups for every planned screen exist in [`mockups/`](mockups/00-index.html).

---

## v0.1.0 — Core hardware daemon (Argon ONE parity)

CLI/systemd only, no web server yet. Gets a real, testable, on-hardware
daemon shipped before touching web/auth complexity — deliberately the
smallest useful slice, matching what `argononed.py` alone does today.

- I2C register bus + fan capability auto-detection (W§1.1, `argonregister`
  parity)
- Fan control loop: temp→speed curve, 30s poll, hysteresis on speed
  decrease (W§1.4) — hardcoded default curve, no persistence/editing yet
  (that's v0.4.0)
- GPIO power-button pulse-width monitor: reboot/shutdown/OLED-switch
  semantics (W§1.1)
- `HardwareBackend` trait with a no-op impl so a Pi without the case
  attached doesn't crash (W§1.4) — this is the seam v0.1.0's tests run
  against, not real I2C/GPIO
- Sysinfo collection: CPU/RAM/temp/disk/RAID via `/proc` + `smartctl` +
  `mdadm` (W§1.2)
- Board auto-detection: ONE vs EON via I2C probe, not install-time file
  presence (W§2.6) — EON-specific behavior (OLED/RTC) stays inert until
  v0.2.0 actually uses the signal
- Config file compat: read existing `/etc/argononed.conf` /
  `/etc/argononed-hdd.conf` / `/etc/argonunits.conf` formats unchanged
  (W§1.3) — this is the *only* source of truth until v0.3.0's SQLite
  migration replaces it
- CLI argv compat: `SERVICE` / `SHUTDOWN` / `FANOFF` (W§4, migration notes)
- systemd unit, `sd_notify` readiness (A§4.2, minus the web-specific bits)

**Not in scope**: OLED, RTC, web server, auth, persistence beyond config
files. Deliberately deferred, not forgotten.

---

## v0.2.0 — EON extras (OLED + RTC)

Completes Python-parity for both case models, still CLI/systemd only.

- OLED framebuffer + blitter: backgrounds (verbatim SSD1306 page format)
  and fonts (per-plane layout, inverted bit order) per the fully decoded
  format (W§1.7)
- Screen rotation state machine: switch duration, screensaver blank-
  after-idle (W§1.2)
- **Blocking decision before this ships**: fonts/backgrounds sourcing —
  Argon40's originals fetched at install time (not vendored, per W§1.5)
  as an interim measure, or regenerated from Adafruit's BSD-licensed GFX
  font (W§1.5's proposed path, proof-of-concept already done this
  session). Pick one before writing the blitter against it.
- Original `argonone` splash screen (`RPI` rotated + detected Pi model +
  signature) — asset already designed, just needs encoding + wiring to
  board detection (W§1.5 resolution)
- RTC register access (PCF8563) + wake/sleep scheduling (W§1.1)

**Not in scope**: web UI for any of this — screen rotation config and RTC
schedules stay config-file-driven until v0.3.0+.

---

## v0.3.0 — Web foundation: persistence, auth, live shell

The first version with a web server — but scoped to infrastructure, not
feature screens, so auth/session/DB plumbing is solid before building on
top of it. No screen in this milestone does more than prove the pipe
works end-to-end.

- SQLite schema + `sqlx::migrate!()` embedded migrations: `users`,
  `settings`, `fan_curve_points`, `audit_log` (A§2.3) — WAL mode,
  `synchronous=NORMAL` (A§3.3), file under systemd `StateDirectory=`
  (A§3.2)
- Forced first-run setup wizard, gated on empty `users` table (A§1.1)
- Argon2id auth, `axum-login` + `tower-sessions` (SQLite-backed) sessions,
  three-role RBAC (`admin`/`operator`/`viewer`) (A§2)
- Password recovery: admin-issued reset from Users page, CLI fallback for
  the no-admin-can-login case (A§1.2) — Users *page* itself is v0.5.0,
  but the reset mechanism and CLI subcommand land here since auth depends
  on it existing
- `axum` + `minijinja` + `htmx` server shell (W§3.5) — bare authenticated
  layout (sidebar, status strip) with no real content pages yet
  (mockups already built: `03-dashboard.html` etc.)
- WebSocket contract live: one shared connection, `stats`/`fan_state`
  message types (W§2.5) — status strip actually ticks, proving the
  pipe, even though nothing is configurable yet
- `GET /api/status` health endpoint (W§2.5)
- `must_change_pw` forced-change flow, failed-login throttle (A§2.2)

**Not in scope**: HTTPS (v0.6.0), fan curve editing, any settings screens.

---

## v0.4.0 — Core dashboard: fan control, storage, system

The highest-value milestone — this is what actually replaces
`argonone-fanconfig.sh` and friends with the web UI the mockups promise.

- Fan curve editor: draggable SVG points, CPU/HDD tabs, synced table
  (mirrors `04-fan-curve-editor.html`)
- `tokio::sync::watch` channel from the REST write handler into the live
  control loop — edits apply without restarting the daemon or losing
  hysteresis state (W§2.7)
- Server-enforced fan-curve safety floor, independent of stored config
  (W§2.8) — closes the "0% fan at 90°C" gap before the editor ships, not
  after
- Storage & RAID page (`05-storage-raid.html`)
- System page: units toggle, firmware/service info (`08-system-settings.html`,
  minus HTTPS/IR/RTC which land in later milestones)
- `PUT /api/fan/curve/{cpu,hdd}`, `GET/PUT /api/settings/units` per the
  API contract (W§2.5)

---

## v0.5.0 — EON web screens + Users/RBAC admin

Completes web-UI parity with every mockup screen.

- OLED config page: screen rotation drag-to-reorder, timing, **live
  preview rendered server-side from the actual framebuffer** (W§3.2 item
  4 — the "what's on the panel right now" feature flagged as
  genuinely hard to get today)
- RTC/Power schedule page
- Users admin page: full CRUD, role assignment, the "Reset password"
  action wired to the mechanism built in v0.3.0 (`07-users-rbac.html`)
- `audit_log` actually populated and worth having by this point — enough
  privileged multi-user actions exist to make it meaningful (A§2.3)

---

## v0.6.0 — HTTPS, dashboard data-surface gaps, hardening

Everything that makes the web UI safe to expose beyond localhost and
closes the researched-but-not-yet-built dashboard gaps.

- HTTPS: Tailscale-issued certs (`tailscale cert` + daemon-owned renewal)
  as the primary path, `rustls-acme`/TLS-ALPN-01 for the custom-domain
  path, `Secure` cookie tied to active mode (A§4.4) — mockup already
  built (`08-system-settings.html`'s HTTPS card)
- Network throughput, load average, swap — the Tier 1 dashboard gaps
  from W§3.3, already scoped and mocked on the dashboard
- IR remote config page (learn/store code)
- Setup-wizard exposure window: one-time console-printed setup token
  (A§1.1)

**Deferred, not planned**: per-core CPU, disk I/O throughput, OS-update
count (W§3.3 Tier 2 — real, just not scheduled yet); container
management, network topology, historical time-series (W§3.3 Tier 3 —
explicitly out of scope, not just unscheduled).

---

## v0.7.0 — Packaging & operability

Turns "a binary that works" into "a thing someone installs."

- `.deb` via `cargo-deb`: bundles the systemd unit, the `/dev/i2c-1` udev
  rule, and `argonone` system-user creation into generated maintainer
  scripts (A§4.5) — `config.txt` I2C enablement stays a documented manual
  step, deliberately not automated
- `argonone-rs admin backup` / `admin restore` using SQLite's online-backup
  API (A§3.4)
- Legacy config-file → SQLite one-time import, for anyone upgrading from
  a v0.1.0/v0.2.0 install or the original Python daemon (ties off the
  "config files are the only source of truth until v0.3.0" note above —
  this is where that migration actually has to land)
- Install docs verified against real Ubuntu 26.04-on-Pi hardware — several
  claims in A§4.3 are flagged "needs hardware verification, not assumed";
  this is where that verification actually has to happen before shipping

---

## Ongoing, not milestone-gated

- `cargo-audit` / CI already blocks on every push, not scheduled per
  version
- Fonts/backgrounds licensing resolution (W§1.5) blocks v0.2.0
  specifically, called out there rather than as its own line item
