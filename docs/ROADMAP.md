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
  parity). Done — `src/hardware/i2c.rs::I2cFan`.
- Fan control loop: temp→speed curve, 30s poll, hysteresis on speed
  decrease (W§1.4) — hardcoded default curve, no persistence/editing yet
  (that's v0.4.0). Done — `src/fan/mod.rs`.
- GPIO power-button pulse-width monitor: reboot/shutdown/OLED-switch
  semantics (W§1.1). Done — `src/hardware/gpio.rs::GpiodPowerButton`.
- `HardwareBackend` trait with a no-op impl so a Pi without the case
  attached doesn't crash (W§1.4) — this is the seam v0.1.0's tests run
  against, not real I2C/GPIO. Done — `src/hardware/noop.rs`.
- Sysinfo collection: CPU/RAM/temp/disk/RAID via `/proc` + `smartctl` +
  `mdadm` (W§1.2). Done — `src/sysinfo/mod.rs`.
- Board auto-detection: ONE vs EON via I2C probe, not install-time file
  presence (W§2.6) — EON-specific behavior (OLED/RTC) stays inert until
  v0.2.0 actually uses the signal. Done — `src/hardware/board.rs`.
- Config file compat: read existing `/etc/argononed.conf` /
  `/etc/argononed-hdd.conf` / `/etc/argonunits.conf` formats unchanged
  (W§1.3) — this is the *only* source of truth until v0.3.0's SQLite
  migration replaces it. Done — `src/config/mod.rs`.
- CLI argv compat: `SERVICE` / `SHUTDOWN` / `FANOFF` (W§4, migration
  notes). Done — `src/cli.rs`.
- systemd unit, `sd_notify` readiness (A§4.2, minus the web-specific
  bits). Done — `packaging/systemd/argonone-rs.service`,
  `sd_notify::notify` in `src/service.rs`.

**Not in scope**: OLED, RTC, web server, auth, persistence beyond config
files. Deliberately deferred, not forgotten.

---

## v0.2.0 — EON extras (OLED + RTC)

Completes Python-parity for both case models, still CLI/systemd only.

- **Blocking decision resolved**: fonts/backgrounds are *not* Argon40's
  originals, vendored or fetched — regenerated instead from the
  `embedded-graphics`/`ssd1306` crates' bundled, permissively-licensed
  fonts and primitive-drawn (rects/labels) dashboard backgrounds (W§1.5's
  proposed path). Since this project owns the whole render path end to
  end, there's no reason to replicate Argon40's bespoke per-plane font
  packing (W§1.7) — that decode remains documented for reference, but the
  actual blitter is a plain `embedded-graphics::DrawTarget` implementation
  (`src/oled/render.rs`), driving the real SSD1306 panel via the `ssd1306`
  crate over I2C. Done.
- Screen rotation state machine: switch duration, screensaver blank-
  after-idle, power-button `OledSwitch` force-advance/wake (W§1.2). Done
  — `src/oled/mod.rs::Rotation`.
- Original `argonone` splash screen (`RPI` rotated 90° + detected Pi
  model + signature), wired to board detection (W§1.5 resolution). Done
  — `src/oled/splash.rs`.
- RTC register access (PCF8563) + wake/sleep scheduling (W§1.1): BCD
  time read, daily wake-alarm programming, and a daily sleep
  (scheduled-poweroff) check against the RTC's own clock. Done —
  `src/hardware/rtc.rs`, `/etc/argonrtc.conf`.
- Verified on real Argon EON hardware (2026-07-14) — OLED dashboard,
  screen rotation, splash screen, and RTC wake/sleep scheduling all
  confirmed on-device, matching the on-hardware verification bar v0.1.0
  shipped with.

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
  (A§3.2). Done — `src/db/mod.rs`, `migrations/0001_init.sql`.
- Forced first-run setup wizard, gated on empty `users` table (A§1.1),
  including the singleton-INSERT race guard from step 5. Done —
  `src/web/setup.rs`.
- Argon2id auth, `axum-login` + `tower-sessions` (SQLite-backed) sessions,
  three-role RBAC (`admin`/`operator`/`viewer`) (A§2). Done —
  `src/auth/mod.rs`.
- Password recovery: admin-issued reset (`POST
  /api/users/{id}/reset-password`, admin-only), CLI fallback
  (`argonone-rs admin reset-password --username <u>`) for the
  no-admin-can-login case (A§1.2) — Users *page* itself is still v0.5.0,
  the reset mechanism and CLI subcommand are done — `src/web/users.rs`,
  `src/admin.rs`.
- `axum` + `minijinja` + `htmx` server shell (W§3.5) — bare authenticated
  layout (sidebar, status strip), no real content pages yet. Done —
  `src/web/dashboard.rs`, `templates/`. `htmx`/`htmx-ext-ws` vendored
  from upstream releases (`assets/`, see `assets/VENDORED.md`) rather
  than CDN-loaded, matching the single-binary deploy story.
- WebSocket contract live: one shared connection, `stats`/`fan_state`
  message types (W§2.5) — status strip actually ticks over
  `htmx-ext-ws`. Done — `src/web/ws.rs`.
- `GET /api/status` health endpoint (W§2.5), auth-gated per the API
  table (`viewer+`). Done — `src/web/status.rs`.
- `must_change_pw` forced-change flow, failed-login throttle (A§2.2:
  5 attempts, 15-minute lockout). Done — `src/web/login.rs`,
  `src/auth/mod.rs`.
- systemd unit gains `StateDirectory=argonone-rs` for the SQLite file
  (A§3.2, A§4.2); the `argonone` service-account privilege drop
  (A§4.1) stays deferred to v0.7.0's `.deb` maintainer scripts, per the
  roadmap's original split — `User=root` still, documented inline.
- Verified on real Argon ONE hardware (2026-07-15) — board
  auto-detection, `GET /api/status` returning live sysinfo (real CPU
  temp/RAM%, not the no-op stub), and the full setup/login/session
  flow all confirmed over the network from a browser on the LAN,
  matching the on-hardware verification bar v0.1.0/v0.2.0 shipped
  with. (Deploy note: `systemd-networkd-wait-online`/`plymouth-quit-
  wait` can stall the unit's `After=network-online.target` boot
  ordering on a fresh headless image — unrelated to this daemon,
  documented as a one-off `systemctl stop plymouth-quit-wait.service`
  fix, not a unit-file defect.)

**Not in scope**: HTTPS (v0.6.0), fan curve editing, any settings screens.

---

## v0.4.0 — Core dashboard: fan control, storage, system

The highest-value milestone — this is what actually replaces
`argonone-fanconfig.sh` and friends with the web UI the mockups promise.

- Fan curve editor: draggable SVG points, CPU/HDD tabs, synced table
  (mirrors `04-fan-curve-editor.html`). Done — `templates/fan_curve.html`,
  `src/web/fan_curve.rs`.
- `tokio::sync::watch` channel from the REST write handler into the live
  control loop — edits apply without restarting the daemon or losing
  hysteresis state (W§2.7). Done — `FanController::set_curve`/
  `tick_with_floor` in `src/fan/mod.rs`, wired in `service::run`.
- Server-enforced fan-curve safety floor, independent of stored config
  (W§2.8) — closes the "0% fan at 90°C" gap before the editor ships, not
  after. Done — `FanCurve::violates_safety_floor` in `src/config/mod.rs`,
  checks every configured breakpoint at/above 75°C plus 75°C itself (not
  just a single point), so a curve with an unsafe *gap* between two
  otherwise-safe points is caught too.
- Storage & RAID page (`05-storage-raid.html`). Done —
  `templates/storage.html`, `src/web/storage.rs`; disk temperature via
  `smartctl` and per-array RAID detail (size, working/failed/spare
  counts, member devices) parsed from `/proc/mdstat` — both new sysinfo
  surface, not previously implemented despite W§1.2 documenting the
  approach.
- System page: units toggle, firmware/service info (`08-system-settings.html`,
  minus HTTPS/IR/RTC which land in later milestones). Done —
  `templates/system.html`, `src/web/system.rs`.
- `PUT /api/fan/curve/{cpu,hdd}`, `GET/PUT /api/settings/units` per the
  API contract (W§2.5). Done — `GET /api/settings/units` was initially
  missed (only `PUT` was wired), caught in a later audit and added,
  `viewer+` per the contract table.
- **Beyond the original bullet list**: the HDD curve is now actually
  *applied*, not just editable — the daemon takes `max(cpu_curve_speed,
  hdd_curve_speed)` each poll (`04-fan-curve-editor.html`'s documented
  "the higher value wins" behavior), using the hottest of all detected
  disks' S.M.A.R.T. temperatures. Without this the HDD tab would save
  successfully but have zero effect, which felt like a real gap to leave
  in rather than a defensible scope cut.
- Fan curves and the units setting are now DB-backed (`fan_curve_points`/
  `settings` tables), replacing the config-file source of truth per the
  plan already recorded in `src/config/mod.rs`'s own doc comment and
  A§3.4 — the legacy files stay readable only for a one-time import,
  still deferred to v0.7.0. `argonone-rs status` reads the same DB values
  the running daemon uses, so the two can't drift out of sync.
- **Not yet done**: verified on real hardware with actual disks/RAID
  attached (this dev pass ran on a Pi/Mac with no block devices to
  exercise `lsblk`/`smartctl`/`mdstat` against real data) — v0.4.0 needs
  that pass, same bar as v0.1.0-v0.3.0, before it's considered complete.

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
- Fonts/backgrounds licensing (W§1.5) — resolved for v0.2.0 (regenerated
  from permissively-licensed crates, not Argon40's assets), noted there
  rather than as its own line item
