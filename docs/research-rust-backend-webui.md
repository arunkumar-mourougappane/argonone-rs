# Research: Rust Backend Daemon + Web UI for Argon40 Cases

Source material: `~/projects/argonone/downloaded_files/` (official Argon40 Python/shell
scripts pulled from `download.argon40.com`) and `~/projects/argonone/argon1.sh`
(the upstream installer). This doc summarizes what the existing software does,
then proposes a Rust architecture and a web UI/UX approach to replace it.

## 1. What the existing (Python) stack actually does

Argon40 cases (Argon ONE, Argon EON) ship a fleet of loosely-coupled scripts,
installed to `/etc/argon/`, driven by two systemd services running as root.

### 1.1 Hardware surface

| Function | Interface | Detail |
|---|---|---|
| Fan control | I2C, addr `0x1a` | reg `0x80` = duty cycle (0-100), `0x81` = FW version, `0x82` = IR code (block write), `0x86` = ctrl (write `1` = signal poweroff). Older FW: no registers, just `write_byte(0x1a, speed)`, `write_byte(0x1a, 0xFF)` = poweroff signal. Capability is auto-detected by writing a probe value to `0x80` and reading it back (`argonregister_checksupport`). |
| Power button | GPIO (libgpiod), BCM pin 4 (`/dev/gpiochip4` on Pi5, `/dev/gpiochip0` older) | Pulse width on rising edge, measured in 10ms polling ticks: 20-30ms = reboot, 40-50ms = shutdown, 60-70ms = OLED screen switch. Two code paths exist for old vs new libgpiod Python bindings (`gpiod.LINE_REQ_*` vs `gpiod.request_lines`), i.e. real API churn to design around. |
| Lid switch (EON) | GPIO pin 27, pull-up | Falling edge = lid closed, timed shutdown after configurable seconds if held closed. |
| OLED (EON) | I2C addr `0x3c`, SSD1306 via `luma.oled` | 128x64 mono framebuffer. Custom binary format for fonts (`font8x6.bin` ... `font64x48.bin`) and backgrounds (`bgcpu.bin`, `bgram.bin`, `bgtemp.bin`, etc.) stored under `/etc/argon/oled/`. App draws by loading a background buffer, then blitting text/rects into it, then flushing to the panel. |
| RTC (EON) | I2C addr `0x51`, PCF8563 | BCD-encoded time/alarm/timer registers; supports scheduled wake, periodic alarms. |
| UPS (EON) | polls status into `/dev/shm/upslog.txt` | battery %, charging state, consumed by dashboard. |

### 1.2 Software components (what a Rust rewrite needs to replace 1:1 for parity)

- **`argononed.py`** — the core daemon (`argononed.service`). Three threads:
  a temperature-to-fan-speed control loop (30s poll, hysteresis on speed
  decrease to avoid fan flapping), a GPIO power-button monitor, and (EON only)
  an OLED render loop cycling through screens (clock/date, IP, CPU%, RAM,
  storage, temps, RAID) on a timer, with a screensaver blank-after-idle.
  Screens are driven by an in-process `Queue` used as a crude IPC channel
  between the button thread and the display thread (`OLEDSWITCH`/`OLEDSTOP`).
- **`argoneond.py`** — EON RTC scheduling daemon (separate service).
- **`argonsysinfo.py`** — stats collection library: CPU% (from `/proc/stat`
  deltas), RAM (`/proc/meminfo`), CPU temp (`/sys/class/thermal/thermal_zone0/temp`),
  disk temps (shells out to `smartctl`/`hddtemp`), disk usage (`df` +
  `/proc/partitions`, with RAID-aware de-duplication), RAID status (parses
  `/proc/mdstat` + `mdadm -D`), IP address (UDP-connect trick, no packets sent).
- **`argonregister.py` / `argonregister-v1.py`** — I2C register helpers described above.
- **`argonpowerbutton-libgpiod.py` / `-rpigpio.py`** — two GPIO backends
  (modern libgpiod vs RPi.GPIO), selected at install time based on OS.
- **`argonrtc.py`** — PCF8563 RTC register math.
- **`argondashboard.py`** — a **curses TUI**, not a web UI. This is the
  closest existing thing to a "dashboard" and it's terminal-only — there is
  **no existing web frontend to draw UX conventions from**; the web UI has to
  be designed fresh (see §3).
- **`argonstatus.py`** — CLI pretty-printer for one-off queries, driven by
  `argon-status.sh`'s numbered menu (temps / CPU / storage / RAM / IP / fan
  speed / RTC schedule / RAID / dashboard).
- **Shell config tools** — `argonone-fanconfig.sh` (interactive fan-curve
  editor, writes `/etc/argononed.conf` and `/etc/argononed-hdd.conf` as
  `temp=speed` line pairs), `argonone-upsconfig.sh`, `argonone-irconfig.sh`,
  `argonone-oledconfig.sh` / `argoneon-oledconfig.sh` (writes
  `/etc/argoneonoled.conf`: `switchduration`, `screensaver`, `screenlist`,
  `enabled`), `argon-unitconfig.sh` (C/F, `/etc/argonunits.conf`),
  `argon-blstrdac.sh`, `argon-rpi-eeprom-config-psu.py`.
- **`argon1.sh`** — the top-level installer/menu (curl-pipe-bash), handles
  detecting board revision, UART config, downloading + installing all of the
  above, registering systemd units.

### 1.3 Config file formats (plain text, trivial to parse, worth keeping as-is for migration compatibility)

```
# /etc/argononed.conf  (temp°C=fan% pairs, highest temp wins, sorted desc)
65=100
60=55
55=30

# /etc/argoneonoled.conf
switchduration=10
screensaver=120
screenlist="clock ip cpu storage temp"
enabled=Y

# /etc/argonunits.conf
temperature=C
```

### 1.4 Key constraints for a daemon rewrite

- Must run as **root** (raw I2C + GPIO), or use `CAP_SYS_RAWIO`/udev rules to
  drop privileges — worth doing properly in Rust since this is a good place
  to actually fix the "everything runs as root forever" problem the Python
  version has.
- Must tolerate **missing hardware gracefully** — every Python function
  wraps I2C/GPIO calls in try/except and no-ops; a Pi without the Argon board
  attached must not crash the daemon. A Rust version should model this as
  `Option<Bus>` / a `HardwareBackend` trait with a no-op impl, decided once at
  startup, not scattered `Result` handling everywhere.
- Must run on **old and new libgpiod / kernel GPIO character device ABI**
  (Pi 4 vs Pi 5 use different `/dev/gpiochipN`), and both **RPi.GPIO-style
  sysfs and libgpiod line-request styles** existed in the Python code —
  in Rust, the `gpiod` crate (character-device v2 uAPI) or `rppal` cover this;
  no need to replicate the old sysfs path.
- Fan speed control has **intentional hysteresis** (don't ramp down for 30s)
  to avoid audible fan flapping — must be preserved, it's a real UX detail
  users notice.
- OLED binary asset format (fonts/backgrounds) is undocumented outside these
  scripts — reuse the downloaded `.bin` files as-is rather than regenerating
  them; write a small blitter compatible with the existing byte layout
  (row-major, 1bpp, page-based like SSD1306's native format).

## 2. Proposed Rust architecture

### 2.1 High-level shape

```
argonone-rs/
├── argonone-core/     # hardware + sysinfo, no web deps, reusable by a CLI too
│   ├── i2c.rs          # register bus, mirrors argonregister.py
│   ├── fan.rs           # fan curve + control loop, hysteresis
│   ├── gpio.rs           # power button / lid pulse-width state machine
│   ├── oled.rs            # SSD1306 framebuffer + existing .bin asset loader
│   ├── rtc.rs               # PCF8563 register math
│   └── sysinfo.rs             # cpu/ram/temp/disk/raid, /proc + smartctl + mdadm
├── argonone-daemon/    # the systemd service binary (was argononed.py + argoneond.py)
│   └── main.rs          # tokio runtime, spawns fan/button/oled/rtc tasks
├── argonone-web/       # HTTP/WebSocket API + serves the SPA (was: nothing, new)
│   ├── api.rs
│   └── ws.rs
└── frontend/            # the web UI (separate build, embedded via rust-embed)
```

Whether this ends up as one binary or a workspace of crates is a call to make
once you know if the daemon needs to run without the web server (e.g. on a
headless box someone doesn't want exposing a port). A single binary with
`--no-web` is simplest operationally; a workspace makes the split explicit.
Given it's one daemon on one box, **recommend single binary, modular internally**
— avoids the multi-process IPC problem the Python version has (the `Queue`
between button-thread and OLED-thread) by keeping everything in one process
with `tokio::sync::mpsc`/`watch` channels instead.

### 2.2 Crate choices

| Concern | Crate | Why |
|---|---|---|
| Async runtime | `tokio` | needed for the web server anyway; also drives the fan/button/oled loops as tasks instead of OS threads + a hand-rolled queue |
| I2C | `linux-embedded-hal` + `embedded-hal` `I2c` trait, or raw `i2cdev` | `i2cdev` is the more direct analogue of `smbus`; `linux-embedded-hal` if you want to stay on the `embedded-hal` trait ecosystem (useful if an SSD1306 driver crate is used for OLED) |
| GPIO | `gpiod` (character-device v2, async-friendly) or `rppal` (simpler API, very popular for Pi projects, sync) | `gpiod` matches what the Python code does under the hood (`/dev/gpiochipN` + line requests with edge detection) and works cleanly with tokio via `AsyncLineRequest`/polling in a blocking task |
| OLED | `ssd1306` + `embedded-graphics` crates, or hand-rolled blitter against the existing `.bin` assets | Hand-rolled is less "idiomatic Rust embedded" but preserves exact visual parity with the existing background/font assets without a conversion step. Reasonable to start hand-rolled, revisit if `embedded-graphics` ends up less code. |
| Web framework | `axum` | tokio-native, good WebSocket support (`axum::extract::ws`) for pushing live stats to the browser, tower middleware for auth/logging, currently the most idiomatic pick in the tokio ecosystem |
| Serve the frontend | `rust-embed` or `include_dir` | bake the built SPA into the binary — one binary to deploy, no separate static-file directory to keep in sync, matches "curl-pipe-bash install" simplicity users of this project expect |
| Serialization | `serde` + `serde_json` | API payloads, WebSocket messages |
| Config files | keep the existing plain `key=value` formats, parse by hand (few lines) or with a tiny custom parser — no need for `toml`/`config` crate churn since **migration compatibility with existing `/etc/argononed.conf` etc. matters** if this is meant to be a drop-in replacement |
| Logging | `tracing` + `tracing-subscriber`, journald layer (`tracing-journald`) | this runs under systemd; journald integration matters for `journalctl -u argonone-rs` to work like the Python version's stdout-to-journal today |
| Privilege drop | `caps` crate or a udev rule granting group access to `/dev/i2c-1` and the gpiochip, running as a dedicated `argonone` system user instead of root | genuine improvement over the Python version, worth doing |
| CLI/arg parsing | `clap` | for `argonone-rs --service`, `argonone-rs fanoff`, `argonone-rs shutdown` etc., mirroring the existing `SERVICE`/`SHUTDOWN`/`FANOFF` argv modes |
| System stats | mostly hand-rolled `/proc` parsing (small, no crate really needed) or `sysinfo` crate for the generic CPU/RAM/disk parts, hand-rolled for the Argon-specific bits (RAID via `mdadm`, disk temp via `smartctl` subprocess, exactly like the Python does — no safe Rust crate reads S.M.A.R.T. registers directly for arbitrary drives, shelling out is the pragmatic choice here too) |

### 2.3 systemd integration

- Ship one `.service` unit (or two if daemon/web are split processes),
  `Type=notify` if using `sd_notify` (via the `sd-notify` crate) so systemd
  actually knows when the daemon is ready, instead of the Python version's
  `Type=simple` + `RemainAfterExit=true` guesswork.
- `ExecStartPre` can drop the udev rule / capability check with a clear
  failure message instead of the Python version's silent "Unable to detect
  i2c" print-and-continue.

### 2.4 What changes vs. Python, deliberately

- Single process instead of 2 services (`argononed` + `argoneond`) + N config
  shell scripts — the web UI becomes the config tool, replacing
  `argonone-fanconfig.sh` / `argonone-upsconfig.sh` / etc. entirely. Keep
  reading/writing the same config file paths for compatibility with existing
  installs and any external tooling that pokes those files.
- Live stats pushed over WebSocket instead of only polled — enables the "real
  dashboard" the curses TUI gestured at but couldn't do remotely.

## 3. Web UI/UX research

### 3.1 Who uses this and on what

This runs on a headless Raspberry Pi in a case, on a home/homelab network.
Users are self-hosters/NAS-builders (RAID and disk-temp support in the source
scripts confirms this — this is squarely a home-NAS audience). Access
patterns:
- Occasional check-ins from a phone or laptop on the LAN, not a
  permanently-open tab.
- Sometimes accessed over Tailscale/VPN from outside the LAN.
- Low-end device serving the page — a Pi 3/4/5 — so the frontend must be
  **light**: no heavy SPA framework bundle churning on the same box it's
  monitoring, no polling that hammers `/proc` every 200ms.

This argues strongly against a build-heavy SPA framework story and for
something that server-renders or ships a small JS payload. Comparable
prior art worth matching the tone of: **Portainer**, **Proxmox's web UI**,
**TrueNAS SCALE**, **Grafana single-panel views**, **Homepage/Homarr**
dashboard tiles — all "small appliance web UI" precedent, not consumer SaaS.

### 3.2 Information architecture

Mirror what `argonstatus.py`'s menu and `argondashboard.py`'s curses layout
already established as the important facts, since that's the maintainers'
own prioritization of what matters:

1. **At-a-glance status strip** (always visible, top of page): CPU temp,
   fan speed/state, RAM used, IP address — literally what the curses
   dashboard puts in the header row.
2. **Fan** — current speed (live), current temp driving it, and the active
   fan curve as an editable graph (temp→speed points), for both CPU and HDD
   curves. This is the single highest-value interaction: replacing
   `argonone-fanconfig.sh`'s "type temp, type speed, repeat" prompt flow
   with a draggable curve editor is a real UX upgrade, not just a reskin.
3. **Storage** — per-disk usage + temperature, RAID array status
   (state/active/working/failed device counts) when `/proc/mdstat` is
   non-empty — hide the whole RAID section when there's no array, don't show
   an empty state for a feature most single-disk users don't have.
4. **OLED / display** (EON only, and only shown if hardware detected)
   — screen rotation list (drag-to-reorder the same `screenlist` the config
   file stores), switch duration, screensaver timeout, live preview of what's
   currently on the physical panel (render the same framebuffer server-side
   to a tiny canvas/image — cheap, and satisfying: "what's on the OLED right
   now" is genuinely hard to know remotely today).
5. **Power/RTC** (EON only) — schedule wake/sleep, same as
   `argoneond.py GETSCHEDULELIST`.
6. **System** — units (C/F), IR config, EEPROM/PSU config — the
   lower-frequency settings, fine as a simple form page, doesn't need design
   investment.
7. **Network** — live throughput (rx/tx) on the primary interface. Not part
   of the inherited Python surface (see §3.3), but a NAS-in-a-case is
   exactly the kind of box where "is it actually being hit right now" is a
   natural glance-worthy question, on par with fan/storage.

### 3.3 Beyond the inherited Python surface: what else belongs on the dashboard

Everything in §3.2 items 1–6 came from what `argonsysinfo.py` and
`argondashboard.py` already exposed — that was a reasonable starting scope,
but it means it was never actually evaluated against "what does a homelab
dashboard need," only against "what did the Argon40 scripts happen to
collect." Checked the downloaded scripts directly: **none of them touch
networking beyond a single outbound-UDP-trick IP lookup** (`argonsysinfo_getip`)
— no interface stats, no throughput, no per-core CPU exposed anywhere in the
UI (only internally, for the aggregate CPU% figure). Worth closing that gap
deliberately rather than by omission. Tiered by value for this specific
audience (self-hosters running a NAS/RAID box in this case, per §3.1) and by
how cheap the data is to obtain from Linux without adding dependencies:

**Tier 1 — add now, cheap and clearly in-scope:**

- **Network throughput** (rx/tx bytes/sec, primary interface). Source:
  `/proc/net/dev` — same hand-parsed-`/proc` pattern already used for
  CPU/RAM/temp, no new crate. Compute a rate by diffing two samples on the
  same 30s-ish poll cadence the fan-control loop already runs; keep the
  interface selection simple (whichever non-loopback interface has the
  default route, from `/proc/net/route`) rather than exposing every
  interface by default — a Pi in a case almost always has exactly one that
  matters.
- **Load average** (1/5/15 min). Source: `/proc/loadavg`, a single line,
  trivially cheap. Classic Linux health signal this audience reads fluently;
  costs nothing to add.
- **Swap usage**. Source: same `/proc/meminfo` parse `argonsysinfo_getram()`
  already does — `SwapTotal`/`SwapFree` are sitting right next to the
  `MemTotal`/`MemFree` keys it already reads and discards. Matters
  specifically on a Pi (small, sometimes-swap-on-SD-card) where swapping
  is a real performance cliff worth surfacing, not a rounding error.

**Tier 2 — worth adding, but not in this pass (documented, not mocked):**

- **Per-core CPU breakdown.** The data already exists —
  `argonsysinfo_getcpuusagesnapshot()` parses per-`cpuN` lines from
  `/proc/stat` today, it's just never surfaced past the aggregate figure.
  A small per-core bar row would fit the "System" or a new "CPU" card
  without new data-collection work, just new UI for data already collected.
- **Disk I/O throughput** (read/write bytes/sec per device), from
  `/proc/diskstats` — belongs on the Storage & RAID page next to
  usage/temperature (not the dashboard) since it's a storage-specific
  detail, not a glance-level fact.
- **Available OS updates count**, for the System card, Proxmox/TrueNAS-style
  ("14 updates available"). On Ubuntu, read
  `/var/lib/update-notifier/updates-available` (a cache APT's own hooks
  refresh periodically) rather than shelling out to `apt list --upgradable`
  on every poll — that command takes over a second and touches the package
  index, wrong thing to run on a fan-control daemon's poll loop.

**Tier 3 — explicitly out of scope, flagging so it isn't quietly scope-crept
in later:**

- **Container/service management** (Docker, systemd unit list beyond
  argonone-rs's own status) — that's Portainer's/Cockpit's job; this is a
  case controller, not a generic homelab dashboard, and trying to be both
  dilutes what it's actually for.
- **Connected client / network topology** (ARP table, DHCP leases) — real
  complexity and a privacy-adjacent surface (who's on the network) for a
  fan-and-OLED controller to be taking on.
- **Historical time-series charts** for any of the above — per the
  persistence research doc, live stats stay in-memory/WebSocket-only, never
  written to SQLite. A live sparkline fed by an in-memory ring buffer (last
  N samples, lost on restart) is fine and consistent with how the fan-speed
  sparkline already works; a queryable history table is not, same SD-card-wear
  reasoning as before.

### 3.4 Interaction/visual approach

- **Dark-first**, not just dark-mode-supported — this is a box people check
  at night next to a rack/shelf; matches every comparable homelab tool's
  default and avoids a jarring white flash on a device that's otherwise all
  dark dashboards.
- **Live values via WebSocket, not polling-with-spinners** — temp/fan/CPU
  should visibly tick in place (a small numeric transition or sparkline),
  no full-panel reloads. This is the biggest tangible improvement over the
  curses dashboard's 5-second full-redraw.
- **Status color semantics carried over from `argondashboard.py`'s existing
  color pairs** (it already encodes battery/alert/warning/good as
  red/yellow/green) — reuse that exact semantic mapping for temperature and
  RAID health so returning users' mental model transfers: green = nominal,
  yellow = warning threshold, red = critical/failed.
- **Fan curve editor**: an SVG/canvas line chart, temp on X (0-100°C),
  speed on Y (0-100%), draggable points, snapping to whole numbers — this
  directly replaces a clunky sequential CLI prompt with a direct-manipulation
  equivalent of the same `temp=speed` pairs already in `argononed.conf`.
- **No login-wall by default**, but support optional basic auth /
  reverse-proxy-friendly headers — matches how most homelab tools ship
  (network-perimeter trust model), and don't force a Postgres/user-table
  just to gate a Pi fan controller. If auth is wanted later, an API token in
  a header is enough.
- **Mobile-usable, not mobile-first** — primary use is a laptop/desktop
  browser on the LAN, but a phone pulled out to check "why is the fan loud"
  should work without horizontal scrolling. Responsive breakpoints, not a
  separate mobile design.

### 3.5 Frontend stack recommendation

Given the "light payload, appliance-grade, one binary to deploy" constraints:

- **Svelte** (or SvelteKit in static-adapter mode) is the strongest fit —
  compiles to small vanilla JS with no runtime framework overhead, which
  matters when the server serving it is the same Pi it's monitoring.
  Alternative: **htmx + a Rust template engine (askama/minijinja)** server-side,
  which avoids a separate frontend build step entirely and lets `axum`
  own rendering — genuinely worth considering here since the daemon already
  has all the state in-process and the UI is mostly "show live data + small
  forms," which is htmx's sweet spot. This tradeoff (SPA-lite vs
  server-rendered+htmx) is the one open architectural question worth
  deciding explicitly before building — recommend htmx + minijinja if you
  want fewer moving parts and no separate JS build/toolchain to maintain
  alongside the Rust daemon; recommend Svelte if a richer draggable
  fan-curve editor and live-updating charts are worth a small JS build step.
- Either way: bake the built assets into the binary with `rust-embed`, serve
  from `axum`, push live values over one shared WebSocket topic that the
  frontend subscribes to (avoids one-socket-per-widget).
- Chart/curve editing: if going the Svelte route, a small dependency-free
  canvas/SVG component is enough — no need for a full charting library for
  one draggable line.

## 4. Migration/compat notes

- Keep config file paths and formats identical (`/etc/argononed.conf`,
  `/etc/argononed-hdd.conf`, `/etc/argoneonoled.conf`, `/etc/argonunits.conf`)
  so an existing install's tuning survives a swap to the Rust daemon.
- Keep the same I2C addresses/registers and GPIO pin numbers — this is fixed
  hardware, not a design choice.
- Keep CLI argv compatibility (`SERVICE`, `SHUTDOWN`, `FANOFF`, `OLEDSWITCH`)
  if anything else on the system (systemd unit `ExecStop`, the shutdown
  hook script) still shells out to the old argv contract during a
  transition period.
