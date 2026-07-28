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
- Verified on real Argon ONE/EON hardware — fan curve editor and safety
  floor applying live to the real fan, Storage & RAID against actual
  attached disks and a real RAID array (`lsblk`/`smartctl`/`mdstat`
  against real data, not just synthetic test fixtures), and the units
  toggle propagating across every display, all confirmed on-device,
  matching the bar v0.1.0-v0.3.0 shipped with.

---

## v0.5.0 — EON web screens + Users/RBAC admin

Completes web-UI parity with every mockup screen.

- OLED config page: screen rotation drag-to-reorder, timing, **live
  preview rendered server-side from the actual framebuffer** (W§3.2 item
  4 — the "what's on the panel right now" feature flagged as
  genuinely hard to get today). Done — `templates/oled.html`,
  `src/web/oled.rs`, `src/oled/framebuffer.rs` (an in-memory
  `DrawTarget` so the same `draw_screen` function the physical panel
  uses renders into memory instead — no framebuffer-readback API or new
  dependency needed). `/api/ws`'s `oled_screen` message drives the
  preview's refresh, matching the API contract exactly.
- RTC/Power schedule page. Done — as a card on `/system`
  (`templates/system.html`), not a standalone route, matching
  `08-system-settings.html#power`'s actual structure. Went further than
  a simple wake/sleep pair: a full multi-entry, day-of-week schedule
  table (`src/rtc_schedule.rs`), since the PCF8563's one alarm slot can
  only ever hold a single next occurrence — resolving *which* one is
  the real content of this feature, not just the web form around it.
- Users admin page: full CRUD, role assignment, the "Reset password"
  action wired to the mechanism built in v0.3.0 (`07-users-rbac.html`).
  Done — `templates/users.html`, `src/web/users.rs`, `src/db/users.rs`.
  Guards the last-admin case (can't delete or demote the sole
  remaining admin) since nothing else in this milestone would recover
  from that.
- `audit_log` actually populated and worth having by this point — enough
  privileged multi-user actions exist to make it meaningful (A§2.3).
  Done — every user-management action plus the previously-unlogged
  `settings.update_units` now write an entry.
- **Not yet done**: verified on real EON hardware — this dev pass ran
  without a case attached (`Board::NoCase`), so the RTC/OLED 404-on-
  non-EON paths and the general web-layer plumbing are what's actually
  been exercised locally (confirmed the OLED render loop and
  `oled_screen` WS message do tick correctly even against the no-op
  backend). The *happy* paths — real wake-alarm behavior, a real
  drag-reordered screen rotation on the physical panel — need the same
  hardware pass every prior release got before this milestone is
  considered complete.

---

## v0.6.0 — HTTPS, dashboard data-surface gaps, hardening

Everything that makes the web UI safe to expose beyond localhost and
closes the researched-but-not-yet-built dashboard gaps.

- HTTPS: Tailscale-issued certs (`tailscale cert` + daemon-owned renewal)
  as the primary path, `rustls-acme`/TLS-ALPN-01 for the custom-domain
  path, `Secure` cookie tied to active mode (A§4.4) — mockup already
  built (`08-system-settings.html`'s HTTPS card). Done —
  `src/https.rs` (mode dispatch, Tailscale cert issuance/renewal via
  `axum_server::tls_rustls::RustlsConfig`, `rustls-acme` integration for
  the custom-domain path), `src/config/mod.rs` (`HttpsMode`/`HttpsConfig`),
  `src/db/settings.rs` (`https_config` setting), `src/web/system.rs`
  (`GET`/`PUT /api/system/https`, admin-only), `templates/system.html`.
  `SessionManagerLayer::with_secure` now follows the active mode
  (`src/web/mod.rs`) rather than the previous hardcoded `false`. A mode
  change takes effect on the next daemon restart — the listener isn't
  hot-swapped the way a cert *renewal* is. Beyond the original bullet:
  the Tailscale option's detail panel also surfaces the real on-disk
  cert's issuer/expiry/auto-renew status (`https::read_cert_status`,
  parsed via `x509-parser` from the actual `fullchain.pem` this module
  writes — not tracked separately, so it can't drift from what's really
  loaded) plus a "Re-issue now" action (`POST
  /api/system/https/reissue`), matching `08-system-settings.html`'s
  cert-status table exactly rather than a text-input-only stand-in for
  it. **Not yet verified on real hardware/infra**: needs an actual
  Tailscale-joined device and a real domain+ACME run to confirm the
  `tailscale cert` invocation and the `rustls-acme` directory flow both
  work end-to-end, not just that the code compiles and the
  no-op/off-mode paths (and the cert parser, against a locally-generated
  test cert) pass in-process tests.
- Network throughput, load average, swap — the Tier 1 dashboard gaps
  from W§3.3, already scoped and mocked on the dashboard. Done —
  `src/sysinfo/mod.rs` (`read_load_avg`, `MemInfo::swap_used_percent`,
  `NetUsage::sample_rates` diffing `/proc/net/dev` against the
  default-routed interface from `/proc/net/route`), surfaced on both
  `GET /api/status` and `/api/ws`'s `stats` message. **Beyond the
  original bullet list**: rather than three standalone stat cards, these
  landed as part of a full rebuild of the dashboard into the card grid
  `03-dashboard.html` always specified — Fan control, Power & RTC (EON),
  Network (with a client-side rolling sparkline), Storage, Display (EON,
  a live OLED thumbnail reusing the same framebuffer-to-canvas approach
  `/display`'s own preview uses), System, and Signed-in-as, each card
  reusing its dedicated page's existing data rather than a second copy
  of that logic (`src/web/dashboard.rs`, `templates/dashboard.html`).
  This pulls v0.7.0's card-grid *contents* forward — see that entry's
  note. `rtc_schedule::next_wake`'s matching logic was generalized into
  `next_occurrence`/`next_sleep` for the Power & RTC card's "next sleep"
  row, resolved against local wall-clock time via the `date` command
  (this project's established "shell out" convention, and a deliberate
  way to avoid `time`'s `local-offset` feature, which is unsound
  cross-thread and gated off by default) — display-only; the real RTC
  wake-alarm programming in `service::run` still reads the actual
  hardware RTC. No UPS/battery data exists anywhere in this codebase, so
  the mockup's "Power source"/"Battery" rows aren't shown — fabricating
  those would be a real correctness bug, not an honest gap.
- IR remote config page (learn/store code). Done —
  `src/hardware/mod.rs` (`FanBackend::learn_ir_code`/`program_ir_code`),
  `src/hardware/i2c.rs`, `src/db/settings.rs` (`ir_code` setting),
  `src/web/system.rs`, `templates/system.html`. **Unverified against real
  hardware**: the research doc documents I2C register `0x82` only as "IR
  code (block write)" — no confirmed trigger sequence or code width. The
  implementation here (a zeroed block write to start a 2s listen window,
  then a 4-byte block read for the result) is a best-effort
  reconstruction from that one line, flagged in code and needing the
  same on-hardware confirmation pass every other hardware claim in this
  project gets before being called settled.
- Setup-wizard exposure window: one-time console-printed setup token
  (A§1.1). Done — `src/db/settings.rs` (`generate_and_store_setup_token`/
  `current_setup_token`/`clear_setup_token`), `src/web/setup.rs`,
  `src/web/mod.rs`. A fresh token is generated (and logged via
  `tracing::warn!`, visible in `journalctl -u argonone-rs`) on every boot
  while `users` is still empty; `/setup` (both `GET` and `POST`) rejects
  anything but the current token, and the token is consumed in the same
  transaction as the winning admin insert so a losing racer's request
  can't replay it afterward.
- Audit log viewer (W§3.6): `audit_log` has been populated since v0.5.0
  by every privileged mutation but nothing reads it back — admin-only
  `GET /audit`, paginated, actor/action filters, no new schema. Mocked
  at `docs/mockups/09-audit-log.html`. Done — `src/db/audit.rs`,
  `src/web/audit.rs`, `templates/audit.html`.
- Self-service "change my password" (W§3.6): `/account/change-password`
  (`src/web/login.rs`) already works but is only reachable via the
  forced `must_change_pw` redirect — add a sidebar account-menu link to
  it for voluntary use, no new backend route. Mocked at
  `docs/mockups/10-account.html` (which also documents the destination
  screen, never mocked before now). Done — `templates/app_shell.html`.

**Deferred, not planned**: per-core CPU, disk I/O throughput, OS-update
count (W§3.3 Tier 2 — real, just not scheduled yet); container
management, network topology (W§3.3 Tier 3 — explicitly out of scope,
not just unscheduled). Historical time-series was also Tier 3
"explicitly out of scope" as a *stored, server-held* concept — that
call still stands (no time-series DB/buffer exists) — but the Network
card's sparkline above is a client-side-only rolling window over the
current browser tab's own WebSocket ticks, reset on every reconnect,
which doesn't actually implement what that Tier 3 line was ruling out.

---

## v0.7.0 — Packaging & operability

Turns "a binary that works" into "a thing someone installs."

- `.deb` via `cargo-deb`: bundles the systemd unit, the `/dev/i2c-1` udev
  rule, and `argonone` system-user creation into generated maintainer
  scripts (A§4.5) — `config.txt` I2C enablement stays a documented manual
  step, deliberately not automated. Done — `Cargo.toml`'s
  `[package.metadata.deb]`, `packaging/udev/60-argonone-i2c.rules`,
  `packaging/deb/postinst`/`postrm`. `postinst` creates the `argonone`
  system account (idempotent across upgrades) and joins it to `dialout`
  (I2C/GPIO) and `systemd-journal` (the v0.7.0 log viewer needs to read
  its own unit's journal as a non-root account) before cargo-deb's own
  generated systemd-units fragment tries to start a service running as
  that account. The systemd unit itself moved from `User=root` to
  `User=argonone` as part of this — `scripts/deploy-local.sh` (the
  manual, non-.deb install path) does the same account/group setup so
  both paths land on the same permission model, and both now install
  the binary to `/usr/bin` (not `/usr/local/bin`) so one unit file works
  for either. `.github/workflows/release.yml` gained a `build-deb` job
  (required, like the Pi binary build) so every tagged release actually
  publishes a `.deb` asset, not just raw binaries. Verified on real
  Ubuntu 26.04-on-Pi hardware (2026-07-26), in addition to the local
  build-and-inspect pass: `dpkg -i` installed cleanly, `postinst`
  created the `argonone` account and group memberships correctly, and
  the systemd unit came up enabled and running under it with no
  further manual steps beyond the documented `config.txt` I2C step.
- `argonone-rs admin backup` / `admin restore` using SQLite's online-backup
  API (A§3.4). Done — `src/admin.rs::backup`/`restore`, `argonone-rs admin
  backup --output <path>` / `admin restore --input <path>`. Uses a
  parameterized `VACUUM INTO` rather than shelling out to the `sqlite3`
  CLI (the research doc's other suggested route): no external binary
  dependency, and no dot-command shell-quoting to get right. `restore`
  runs embedded migrations against `--input` first (so an older backup
  comes up schema-current), checkpoints its WAL, then stages+renames it
  onto the live DB path — atomic against a concurrent reader, though the
  service still needs to be stopped first since a live writer could
  otherwise reopen the old handle mid-swap.
- Legacy config-file → SQLite one-time import, for anyone upgrading from
  a v0.1.0/v0.2.0 install or the original Python daemon (ties off the
  "config files are the only source of truth until v0.3.0" note above —
  this is where that migration actually has to land). Done —
  `src/config/import.rs`, `argonone-rs admin import-legacy-config`. Safe
  to run more than once: anything the DB already has a value for
  (checked directly, not via each type's own load-or-default fallback) is
  left alone rather than clobbered by a stale file.
- Install docs verified against real Ubuntu 26.04-on-Pi hardware — several
  claims in A§4.3 are flagged "needs hardware verification, not assumed";
  this is where that verification actually has to happen before shipping.
  Verified on real Ubuntu 26.04-on-Pi hardware (2026-07-26) — both install
  paths confirmed clean, no issues found: the `.deb` package (`dpkg -i`,
  exercising `cargo-deb`'s generated `postinst`'s `argonone` user/group
  creation, the I2C udev rule, and the systemd-units enable/start
  integration) and the manual path (`scripts/deploy-local.sh`, exercising
  its own equivalent user/group setup). A§4.3's Ubuntu-on-Pi group/udev
  behavior claims (`dialout` covering I2C + GPIO chardev access,
  `systemd-journal` for the log viewer) hold as documented.
- Read-only-rootfs startup failure gets a diagnosable error (A§3.5, found
  comparing against RPi-Monitor's read-only-root support): if
  `/var/lib/argonone-rs` isn't writable, detect that specific failure and
  log what's actually wrong and how to fix it, instead of surfacing a raw
  `sqlx::Error`. Not full read-only-rootfs support (degrading writes
  gracefully) — that cuts against the SQLite-is-sole-source-of-truth
  design (A§1–§3) and isn't scoped here, just a clearer failure mode for
  the one that already happens today. Done — `src::db::mod::diagnose_unwritable_state_dir`
  (a pre-flight write-probe against the state directory, distinguishing
  EROFS/permission-denied from other failures), wired into both
  `service::run`'s and every `admin` subcommand's DB-open path.
- Customizable dashboard card layout (W§3.7, also found comparing against
  RPi-Monitor): drag-and-drop reorder of the dashboard's card grid (Fan,
  Power & RTC, Network, Storage, Display, System, Signed in as), same
  interaction the OLED screen-rotation list already shipped in v0.5.0.
  **The card grid itself (all seven cards, with live data) shipped early
  in v0.6.0** — pulled forward so the dashboard matched
  `03-dashboard.html` rather than staying the deliberately-light landing
  page it started as; what's left here is purely the reorder
  interaction, not building the cards. New `settings` key
  (`dashboard_layout`), `GET/PUT` following the existing
  DB-backed-with-fallback-default pattern — no new architecture. Mocked
  at `docs/mockups/11-dashboard-reorder.html`. Done —
  `db::settings::load`/`save_dashboard_layout` (global, one layout for
  everyone with write access — the mockup's own "saved per account"
  copy doesn't match this; per-account would need new schema, which the
  roadmap's own "no new architecture" call already ruled out), wired
  into `src/web/dashboard.rs`/`templates/dashboard.html`. Reorders the
  already server-rendered cards client-side (same native HTML5
  drag-and-drop pattern `oled.html`'s screen list uses) rather than
  re-templating their content, since the cards embed live data the
  server already computed; a bad/unknown key in a PUT is sanitized
  (dropped/de-duped/backfilled) rather than rejected, since a bad
  layout has zero safety impact unlike the fan curve or RTC schedule.
  Verified live in a real browser: dragging a card actually moves it
  and the new order survives a page reload.
- System log viewer (W§3.8, found comparing against Cockpit's Logs
  module): admin-only, read-only page tailing `journalctl -u
  argonone-rs` — the only way to see daemon logs today is SSH. Shells
  out the same way `sysinfo` already does for `smartctl`/`df`, no new
  persistence. Mocked at `docs/mockups/12-logs.html`. Done — `src/logs.rs`
  (shells out via `tokio::task::spawn_blocking`, same as `sysinfo`),
  `src/web/logs.rs`/`templates/logs.html`. Level filtering doesn't use
  `journalctl -p` — everything this app writes over plain stdout under
  a `Type=notify` unit lands at the same syslog priority regardless of
  the level its own `tracing_subscriber` formatter printed, so the
  level shown/colored per line is parsed from that formatter's own
  output instead. "Follow (live)" is a 5s poll that re-fetches and
  re-renders the window, not a genuine streaming tail — deliberately,
  to avoid a new WS message type/architecture for this. Depends on the
  `.deb`/deploy-local.sh privilege-drop work above: a non-root
  `argonone` account needs `systemd-journal` group membership to read
  its own unit's journal at all, which this milestone's own package
  work now grants.
- Critical-state webhook notification (W§3.8, found comparing against
  Netdata's alert model): one configurable webhook URL, POSTed once on
  a severity transition into `crit` (CPU/disk temp, RAID degraded) —
  reuses `web/storage.rs`'s existing `temp_severity`/`degraded`
  computation, doesn't act on it today beyond a badge color. Not a
  multi-channel notification engine — one URL, opt-in, fires on
  transitions only. This app's first outbound network call, so treat it
  deliberately rather than bolt it on. Mocked at
  `docs/mockups/13-notifications.html`. Done — `src/notify/mod.rs`
  (`AlertTracker`, tracking each source's last-seen crit/non-crit state
  in memory so a value that stays critical doesn't re-fire; `send`, a
  plain-JSON `reqwest` POST), `src/web/notifications.rs`/
  `templates/notifications.html` (admin-only throughout, including
  reads — unlike `system.rs::get_https`'s mode/domain, a webhook URL can
  itself be a bearer secret), `db/settings.rs`
  (`load`/`save_webhook_config`, `record`/`load_webhook_last_fired`).
  The CPU-critical threshold (75°C) is the same constant as the fan
  curve's own safety ceiling (`config::SAFETY_CEILING_C`), not a
  separately-maintained number. Checked every 30s poll alongside the
  existing disk-temp/RAID read, alert delivery `tokio::spawn`ed so a
  slow/unreachable webhook can't stall the fan control loop.

---

## v0.8.0 — Observability

Turns "you can watch the live dashboard" into "you can watch trends over time,
from outside a browser tab." Scoped from
[docs/research-prometheus-metrics.md](research-prometheus-metrics.md); not
started yet. First of eight single-feature minor releases (v0.8.0–v0.15.0)
covering everything researched after v0.7.0 shipped — one feature per
milestone this time, rather than bundling several into one version the way
v0.6.0/v0.7.0 did, since none of these share a common theme the way that
packaging-and-operability work did.

- Prometheus `/metrics` endpoint: exports the same data `/api/status` and the
  dashboard's WebSocket feed already read — CPU/disk temp, fan %, RAM/swap,
  load average, network throughput, RAID state, HTTPS cert expiry — no new
  sampling code, purely a new export surface. `metrics` 0.24 +
  `metrics-exporter-prometheus` 0.18, mounted as a route via
  `PrometheusHandle::render()` rather than that crate's own bundled HTTP
  server, so this app still owns exactly one listener. The real design
  question isn't the metrics, it's auth: every route today requires a login
  session, which a Prometheus scrape target can't do. Planned: an
  admin-configured bearer token checked ahead of the session middleware
  (same "a secret is itself sensitive" precedent as the webhook URL), not a
  fully public endpoint. No new mockup for the scrape endpoint itself (not a
  browsable page) — planned as a new card on the existing `/system` page,
  small enough not to cross the threshold that got Notifications/Logs their
  own sidebar page + mockup.

---

## v0.9.0 — Two-factor authentication

Not started. Scoped from [docs/research-totp-2fa.md](research-totp-2fa.md).

- TOTP two-factor authentication: `totp-rs` 5.7.2 for secret/QR/verification,
  a new migration (`users.totp_secret`/`totp_enabled` plus a
  `totp_recovery_codes` table — never edit `migrations/0001_init.sql` in
  place, it's already checksummed on real installs), and the key design
  call: gate the pending-second-factor state using the exact same
  `require_login`/`must_change_pw` middleware pattern already shipped,
  rather than inventing a new pre-session auth state machine. Recovery
  codes Argon2id-hashed like passwords, shown once at enrollment. Mocked at
  `docs/mockups/15-login-2fa.html` (the post-password challenge screen) and
  `docs/mockups/16-account-2fa.html` (enable/manage, on the existing
  Account page).

---

## v0.10.0 — Active session management

Not started. Scoped from
[docs/research-active-sessions.md](research-active-sessions.md); pairs
naturally with v0.9.0 — once a second factor protects login, "what if a
session cookie leaks anyway" is the next real question.

- List/revoke your own logged-in sessions. The session store's `data`
  column is an opaque MessagePack blob (no queryable `user_id`/`ip`/
  `last_seen`), so this needs a new `session_metadata` side table populated
  by a per-request middleware, not a query against the existing
  `tower_sessions` table alone. A coarser version of this already ships for
  free: changing a password already invalidates every session for that
  account via `session_auth_hash`, so this only adds the missing scalpel
  (revoke one session) next to that existing hammer. Self-service only —
  never another user's sessions, even for an admin. Mocked at
  `docs/mockups/18-account-sessions.html`.

---

## v0.11.0 — Notification target formats

Not started. Scoped from
[docs/research-notification-target-formats.md](research-notification-target-formats.md).

- Per-target webhook body formats (Discord/ntfy/Home Assistant): checked
  the v0.7.0 webhook notifier's own advertised compatibility against each
  target's real API. Home Assistant works (schema-agnostic), ntfy delivers
  but shows the raw JSON as the message body instead of clean text, and
  Discord fails outright (400 — its webhook endpoint requires a
  `content`/`embeds`/etc field none of today's fixed body has). Fix: a
  `format: Generic|Discord|Ntfy` enum on `WebhookConfig`, branching
  `notify::send`'s body construction — one new `<select>` on the existing
  Notifications page, no new mockup needed.

---

## v0.12.0 — Portable settings export/import

Not started. Scoped from
[docs/research-config-export-import.md](research-config-export-import.md).

- `argonone-rs admin export-settings`/`import-settings`: a JSON export of
  just the DB-backed settings and fan curves, distinct from `admin
  backup`/`restore`'s raw SQLite snapshot (which brings every user account
  and password hash with it, the wrong tool for moving tuning between two
  installs). CLI-only, no web UI and no mockup, matching the existing
  `admin backup`/`restore`/`import-legacy-config` precedent.

---

## v0.13.0 — Self-update check

Not started. Scoped from
[docs/research-self-update-check.md](research-self-update-check.md).

- Read-only "newer version available" check against GitHub's public
  releases API, explicitly not an auto-updater. Off by default:
  `src/notify/mod.rs`'s own doc comment calls the webhook notifier "this
  app's first outbound network call," and an always-on update check would
  make that simply false while contradicting this project's existing
  opt-in-outbound-calls posture. Component reference at
  `docs/mockups/17-system-update-banner.html` — a new state on the existing
  "Firmware & service" card, not a new page.

---

## v0.14.0 — Quiet-hours fan profile

Not started. Scoped from
[docs/research-quiet-hours-fan-profile.md](research-quiet-hours-fan-profile.md).

- Caps fan speed on a nightly day-of-week window; works on an Argon ONE
  too, since it only needs the system wall clock (the same `date`-shellout
  convention `src/web/dashboard.rs` already uses), not the EON's RTC chip.
  The one hard rule, caught and fixed during review: the cap must apply to
  the CPU curve's own output *before* the HDD curve's demand is folded in
  via `.max()`, never after — otherwise quiet hours could silently
  suppress either the 75°C CPU safety floor or a legitimate HDD-driven
  demand, exactly the two guarantees this app's fan control already makes
  today. Mocked at `docs/mockups/19-quiet-hours.html`, as a new card on the
  existing Fan control page (deliberately not scoped to the CPU/HDD curve
  tabs — it caps the one physical fan output, downstream of both).

---

## v0.15.0 — UPS/battery status

Not started, **low priority**. Scoped from
[docs/research-ups-battery.md](research-ups-battery.md); last of this
sequence deliberately — see why below.

- UPS/battery status for the optional Argon Industria UPS add-on (EON).
  The dashboard mockup's "Power source"/"Battery" rows are currently
  hardcoded placeholders (`docs/mockups/03-dashboard.html`), since "No
  UPS/battery data exists anywhere in this codebase" (this doc's v0.6.0
  entry above). Not I2C: the UPS daemon is a separate, optionally-installed
  third-party Python process (`argononeupsd.py`, fetched on demand by
  `argonone-upsconfig.sh`, never part of this project's own source) that
  writes plain-text status to `/dev/shm/upslog.txt` — `argonone-rs` could
  tail that file if present, the same way the original curses dashboard
  did. Staying last/low priority: it's an optional, separately-purchased
  add-on most owners don't have, the only buildable path depends on a
  third-party daemon this project doesn't control, and nobody's asked for
  it yet. Component reference at `docs/mockups/14-ups-battery.html` — not a
  new sidebar page, just the Power & RTC card's real states
  (plugged/charging/on-battery-warn/on-battery-crit/absent).

---

## Ongoing, not milestone-gated

- `cargo-audit` / CI already blocks on every push, not scheduled per
  version
- Fonts/backgrounds licensing (W§1.5) — resolved for v0.2.0 (regenerated
  from permissively-licensed crates, not Argon40's assets), noted there
  rather than as its own line item
