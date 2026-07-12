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
  them (subject to §1.5's licensing caveat); write a small blitter
  compatible with the existing byte layout. **Revising the blanket claim
  made here originally**: "row-major, 1bpp, page-based like SSD1306's
  native format" is accurate for the *backgrounds* only — the fonts use a
  meaningfully different, non-obvious packing (per-character-plane layout,
  inverted bit order relative to backgrounds). Decoded in full in §1.7;
  don't build a blitter against this one-line description alone.

### 1.5 Licensing: the OLED assets are not cleared for redistribution

§1.4 says to ship the downloaded `.bin` fonts/backgrounds/logo as-is — that
claim was never actually checked against whether Argon40 permits
redistribution, and it should be revised. Checked directly: **none of the
downloaded scripts or assets carry a license header, `LICENSE` file, or any
copyright/redistribution statement** — `download.argon40.com` ships them as
bare files with no metadata at all. Absence of a license is not permission;
under default copyright, Argon40 (or whoever they licensed the artwork
from) retains all rights to the fonts, background bitmaps, and the
`logo1v5.bin` logo specifically — the logo in particular is a trademark/brand
asset, a different and clearer problem than the fonts.

This matters differently for the two asset classes:

- **Fonts/backgrounds (`font*.bin`, `bg*.bin`)**: functional bitmap data
  (glyph shapes, simple line-art dashboard backgrounds). Low creative
  threshold, but still not obviously fair game to vendor into a public
  Rust repo without asking.
- **`logo1v5.bin` (Argon40 logo)**: don't ship this one regardless of the
  fonts/backgrounds decision — redistributing someone else's brand mark in
  a third-party rewrite is the clearer problem of the two, license question
  aside.

Recommended path, in order of preference:

1. **Ask Argon40 directly** whether the asset files (not just the scripts)
   can be redistributed by a community Rust rewrite — cheap to do, and the
   scripts themselves being freely downloadable suggests they may be fine
   with it, but that's an assumption, not a confirmed answer.
2. **If no answer / declined**: regenerate equivalent assets from scratch —
   the fonts are standard bitmap font sizes (6x8, 8x16, etc.) that can be
   redrawn or sourced from a permissively-licensed bitmap font, and the
   dashboard backgrounds are simple enough (per the blitting code in
   `argononeoled.py`) to redraw as plain rectangles/labels rather than
   pixel-identical recreations of Argon40's originals.
3. **Ship no default logo screen** — trivial to drop as a screen option
   entirely; it's one of seven rotation screens in the mockup and the least
   functionally important one.
4. Whatever the outcome, **don't commit the original `.bin` files to the
   `argonone-rs` git history** while this is unresolved — treat them the
   way the existing `argonone` research-scratch repo treats them (fetched
   at install/build time from `download.argon40.com`, not vendored) so
   there's no redistribution act to walk back later.

### 1.6 Is reimplementing the protocol/software allowed at all?

Separate question from §1.5's asset-redistribution issue: is building a
compatible Rust rewrite — reading Argon40's published scripts to
understand the I2C register map, GPIO pulse semantics, and (per §1.7)
the OLED asset formats, then writing new code that does the same thing —
itself permissible? Checked, not assumed:

- **Protocol facts aren't copyrightable, regardless of source.** "Fan duty
  cycle is I2C register `0x80`, write 0–100" is functional/interoperability
  information — the idea/expression dichotomy that underlies *Sega v.
  Accolade* (US) and Article 6 of the EU Software Directive's explicit
  reverse-engineering-for-interoperability carve-out. This is also just
  how the embedded/driver-reimplementation world normally operates.
- **This isn't classic black-box reverse engineering anyway.** Argon40
  publishes the Python source in the clear (`curl | bash`), unobfuscated —
  reading published source to understand a protocol and re-expressing
  equivalent behavior in new code is a much easier case than decompiling
  a binary. What stays off-limits regardless: copying their code's literal
  *expression* (comments, structure, specific phrasing) — write the Rust
  independently, don't transliterate line-by-line.
- **Checked Argon40's actual Terms of Service**
  (`argon40.com/policies/terms-of-service`) directly: no reverse-engineering,
  decompilation, or reimplementation clause exists. The closest provision
  (§2, restricting reproduction of "the Service") governs their
  website/store, not a software EULA restricting protocol reimplementation
  — there isn't one.
- **Real community precedent, on Argon40's own forum.** A thread titled
  *"Much Better Argon One Fan Linux Software Alternative"*
  (forum.argon40.com) is exactly this — a third-party fan-daemon
  reimplementation, still live, unmoderated, with a community maintainer
  (not Argon40 staff) actively participating. No staff objection anywhere
  in it.
- **Not checked**: patents. No evidence found either way; low apparent risk
  for a fan-controller register interface, but genuinely unresearched, not
  cleared. This isn't legal advice — if `argonone-rs` gets wide
  distribution, a real IP attorney's sanity check is cheap insurance, even
  though nothing found here suggests a real problem for a protocol-compatible
  rewrite.

### 1.7 OLED asset binary format, decoded

The hardware itself is unremarkable and well-supported: the EON's OLED is
a 128×64 **SSD1306**-controller I2C panel — the exact same controller/
resolution combination as, e.g., Adafruit's 1.3" 128x64 OLED breakout
(product #938). Any existing Rust `ssd1306`-crate driver targets this
chip already; nothing custom about the display silicon. What *is*
undocumented outside Argon40's own code is the `.bin` **asset format**,
decoded here by direct inspection of the downloaded files plus tracing
`argononeoled.py`'s actual read/write code (not just eyeballing rendered
output — see the bit-order note below for why that distinction mattered).

**Backgrounds (`bg*.bin`, `logo1v5.bin`) — simple, byte-exact match:**
Each is exactly 1024 bytes = 128×64÷8, the SSD1306's native page-addressed
framebuffer size. `oled_loadbg()` copies them verbatim into the live
display buffer, no transformation. Within a byte, bit 0 (LSB) is the top
pixel of that 8-row band, bit 7 (MSB) the bottom — standard SSD1306 page
format, confirmed against `oled_writebuffer`'s `ybit = 1 << (y & 7)`.

**Fonts (`font*.bin`) — same 1bpp idea, non-obvious packing:**

- **256-glyph table**, indexed by raw byte value (0–255). Most fonts only
  populate the glyphs actually used by the dashboard screens — confirmed
  `font16x8.bin` has real (non-zero) data for only 112 of 256 slots
  (digits, punctuation, a handful of specific letters); the rest are
  zero-filled, not present-with-different-index.
- **Not per-character-contiguous.** Layout is per-page-*plane*: for a
  `charwd`-wide, `charht`-tall glyph (`numfontrow = charht/8` planes), the
  file is `numfontrow` blocks of `256 × charwd` bytes each — all 256
  characters' plane-0 columns first, then all 256 characters' plane-1
  columns, etc. A glyph's column `c` in plane `p` is at
  `p×256×charwd + charcode×charwd + c`. A naive "char N at offset
  N×bytes_per_char" read (the obvious first guess) is wrong for any font
  taller than 8px — verified by cross-checking this exact indexing formula
  from `oled_writetext` against every downloaded font file's byte size,
  which matches for 6 of 7 files exactly.
- **`charht` is derived from `charwd`**, not independent:
  `charht = round_up_to_8(charwd × 8 / 6)`. This is why the width/height
  pairings in the filenames look arbitrary (`font24x16`, `font48x32`) —
  they're not; they fall out of this formula.
- **Bit order is inverted relative to backgrounds**: in font bytes, bit 7
  (MSB) is the glyph's *top* row, bit 0 (LSB) the bottom — backwards from
  the background/framebuffer convention above. Confirmed by tracing
  `oled_writetext`'s read loop (`curbit` starts at `0x80`, shifts right as
  `row` increases) against `oled_writebuffer`'s write logic (`ybit = 1 <<
  (y & 7)`, i.e. row 0 → bit 0) — `oled_writetext` is doing a genuine
  bit-order flip while blitting a font glyph into the framebuffer, not a
  straight copy. **Caught this by tracing the code, not by rendering and
  eyeballing the result** — a rendered `0` or `A` looks *plausible* under
  either bit-order assumption at 6×8 resolution; only checking the actual
  read/write source distinguished the correct one from a coincidentally-
  readable wrong one. Worth internalizing as a method note as much as a
  format detail: don't trust "it looks right" for reverse-engineered
  binary formats when the ground-truth source is available to check
  instead.
- **One asset anomaly**: `font48x32.bin` is 65536 bytes on disk, but the
  code's own formula (`charht=48` → 6 planes) only ever reads the first
  49152 of them — 16KB of trailing data this specific file ships that the
  current Python code never touches. Every other font file's size matches
  the formula exactly; flagging as a one-off inconsistency in Argon40's
  own asset set, not a misunderstanding of the format.

Implementation note for the Rust blitter (§2.1's `oled.rs`): this means
two distinct reader paths, not one shared "1bpp blit" routine — a
straight background loader (§ above), and a font-plane reader that
(a) locates the right plane/character/column via the formula above and
(b) flips the bit order on the way into the framebuffer. Getting either
detail wrong produces a *plausible-looking but subtly corrupted* render
(mirrored or shifted glyphs) rather than an obvious crash — worth a unit
test that renders a known character (e.g. `'0'` or `'A'`) against the
downloaded fixture and asserts the exact expected bit pattern, not just
"did it run without panicking."

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

### 2.5 API and WebSocket contract

Never actually specified past "push live stats over WebSocket" — worth
pinning down before implementation starts, since the daemon, the web
server, and the frontend all need to agree on it independently.

**REST, for state that changes on user action (not every few seconds):**

| Method/path | Auth (per doc 2 RBAC) | Purpose |
|---|---|---|
| `POST /api/setup` | none (setup mode only) | create the first admin account |
| `POST /api/login` / `POST /api/logout` | none / any | session cookie issuance |
| `GET /api/status` | viewer+ | one-shot snapshot of everything the status strip shows — also doubles as a `/healthz` (see below) |
| `GET/PUT /api/fan/curve/{cpu,hdd}` | viewer+ / operator+ | fan curve points |
| `GET/PUT /api/oled/config` | viewer+ / operator+ | screen rotation, timings — 404s (not empty-state) if no OLED detected, see §2.6 |
| `GET/PUT /api/rtc/schedule` | viewer+ / operator+ | EON wake/sleep schedule — 404s on non-EON hardware |
| `GET/PUT /api/settings/units` | viewer+ / operator+ | C/F |
| `GET/POST/DELETE /api/users` | admin only | user management |
| `POST /api/users/{id}/reset-password` | admin only | Tier-1 recovery, per doc 2 §1.2 |

**WebSocket, for the status-strip/dashboard values that tick every few
seconds — one shared connection, not one socket per widget** (per doc 1
§3.5's "avoid one-socket-per-widget" note, now made concrete):

- `GET /api/ws` (upgrade). Auth rides the session cookie already on the
  upgrade request — no separate WS auth handshake needed, `axum`'s
  WebSocket upgrade sees the same cookie jar as any other route under
  `axum-login`'s session layer.
- Server → client messages are a tagged JSON enum, e.g.
  `{"type":"stats","cpu_temp_c":46.2,"fan_pct":38,"ram_used_pct":47,"net_rx_bps":..}`,
  `{"type":"oled_screen","name":"clock"}` (drives the live OLED preview
  from §3.2 item 4), `{"type":"fan_state","curve":"cpu","current_pct":38}`.
  One message type per logical update, not one giant "everything" blob on
  every tick — lets the frontend subscribe to only what's on-screen.
- No client → server messages over this socket — writes go through REST
  (`PUT /api/fan/curve/...` etc.), keeping the WS connection strictly
  server-push and the write path's auth/validation in one place (REST
  handlers) instead of duplicated into a WS message handler too.
- Reconnect policy: client-side exponential backoff, resubscribe on
  reconnect; server holds no per-connection state that needs replaying (all
  pushed values are current-snapshot, not deltas), so a dropped/reconnected
  socket just resumes getting fresh ticks — no message-replay/sequence-number
  design needed.

**Health check**: `GET /api/status` doubles as the health endpoint (per the
Tier-3-adjacent "no metrics endpoint" call in §3.3 — a full Prometheus
`/metrics` exporter is out of scope, but a plain "is this thing up and is
the hardware backend actually responding" check is not the same ask and is
worth having). Return `200` with a `hardware: "ok" | "degraded" | "absent"`
field reflecting the `HardwareBackend` state from §1.4, not just "process is
running" — a daemon that's up but silently lost its I2C bus should read as
degraded, not healthy.

### 2.6 Board and hardware auto-detection at runtime

The mockups gate OLED/RTC/UPS screens behind "EON only," and the API table
above 404s EON-only routes on non-EON hardware — neither doc actually said
how the daemon knows which board it's on. The Python install-time approach
(`argononed.py` checks `os.path.exists("/etc/argon/argoneonoled.py")` —
i.e. "which optional module did the installer drop on disk") doesn't
translate to a single self-contained Rust binary that ships all
capabilities and detects at runtime instead of install time. Runtime probe
strategy, mirroring what `argonregister_checksupport` already does for the
fan-register capability:

1. **I2C bus + fan controller presence** (`0x1a`): probe on startup exactly
   like the Python does — write then read back the duty-cycle register
   (`0x80`); if the bus or device isn't there, the read/write errors and
   the daemon falls back to the `HardwareBackend::None` no-op impl from
   §1.4. This distinguishes "no Argon case attached at all" from "case
   attached."
2. **OLED presence** (`0x3c`, SSD1306): a separate I2C probe at that
   address. This is the actual EON-vs-ONE signal — the OLED panel is EON's
   distinguishing hardware, not a firmware flag. If it responds, enable
   OLED routes/screens; if not, 404 them rather than showing an empty
   state (per §3.2 item 4's existing "hide the whole section" precedent
   for RAID).
3. **RTC presence** (`0x51`, PCF8563): third independent I2C probe — EON
   ships the OLED and RTC together in practice, but probing them
   separately (rather than inferring RTC-from-OLED) means the daemon
   doesn't silently misbehave if a future Argon board mixes capabilities
   differently than today's two-model lineup.
4. **UPS presence**: the Python version has no real UPS *detection* — it
   just tails `/dev/shm/upslog.txt` if present, written by a separate
   unresearched UPS monitoring process this project doesn't own. Flagging
   as a genuine open question rather than guessing: is UPS status still
   sourced from some external process's log file, or does `argonone-rs`
   need to own UPS polling itself? Not resolved by either doc; needs an
   answer before the Power/RTC card's "Power source: UPS · plugged" field
   in the dashboard mockup can be backed by real data.

All three I2C probes run once at startup (matching the fan-register
capability check's existing pattern), cached for the process lifetime —
not re-probed per-request. A case doesn't change hardware while running;
re-probing on every dashboard load would just be latency for no benefit.

### 2.7 Config writes vs. the live control loop

Doc 1's architecture (§2.1) puts the fan-control loop, GPIO monitor, and
web server in one process communicating over `tokio` channels instead of
the Python version's inter-process `Queue` — but never actually named the
mechanism for the one case that matters most: an operator saves a new fan
curve from the web UI while the temperature-poll loop is mid-cycle.

Concretely: `PUT /api/fan/curve/cpu` writes the new points to
`fan_curve_points` in SQLite (doc 2 §2.3), then must get that change into
the *running* `temp_check` loop without restarting it (a restart would
lose the loop's in-memory hysteresis state — the "don't ramp down for 30s"
timer from §1.4 — right as the operator is trying to test their new
curve). Mechanism: a `tokio::sync::watch::channel<FanCurve>` — the REST
handler sends the new curve on the channel after the DB write commits; the
control loop's `select!` wakes on either its poll-interval timer or a
channel update, re-reading `watch::Receiver::borrow()` for the current
curve on each iteration. `watch` (not `mpsc`) specifically because the
control loop only ever cares about the *latest* curve, never a queued
history of edits — exactly the "hold most recent value" semantics `watch`
is for.

### 2.8 Fan curve safety validation

Neither the fan-curve editor mockup nor either research doc stops an
operator from saving a curve that's unsafe at the hardware level — e.g.
0% fan at 90°C, or CPU curve with no point above 40% anywhere. The Python
version has no such validation either (`get_fanspeed` just returns 0 if
no configured bucket matches), so this isn't a regression, but it's worth
closing given the web UI makes "delete all the points and save" a two-click
mistake in a way the old sequential CLI prompts didn't as easily allow.

Recommend the daemon enforce a floor **independent of stored
configuration**, not just client-side form validation (which an API caller
bypasses trivially): reject a `PUT` whose curve implies less than some
minimum fan percentage (e.g. 25%, matching the Python code's own existing
"fancfg < 25 → treat as 25%" floor in `get_fanspeed`) at any temperature at
or above a hardcoded safety ceiling (e.g. 75°C) regardless of what points
the operator configured. This mirrors a floor the Python code already has
implicitly (`get_fanspeed`'s `fancfg < 25` clamp) — making it an explicit,
documented, server-enforced invariant rather than a side effect of one
function's rounding behavior is the actual fix, not new behavior.

### 2.9 CI: closing the test and supply-chain gaps

The CI workflow (`.github/workflows/ci.yml`) as originally written has two
real gaps, found by re-reading it against what it's supposed to guarantee:

- **Tests only ran in a non-blocking job.** `cargo test` ran in
  `build-macos`, which is `continue-on-error: true` (deliberately, since
  macOS is a secondary dev-convenience target, not the deploy target). Net
  effect: a broken test suite could not fail CI, on any target — the
  primary `build-raspberry-pi` job cross-compiles and can't execute
  aarch64 test binaries on an x86_64 runner. Fixed by adding a `test` job
  on `ubuntu-latest` at the host target: business-logic tests (fan-curve
  math, config parsing, RBAC role checks) don't need Pi silicon to
  validate, only the actual hardware I/O paths do (and those are behind
  the `HardwareBackend` trait from §1.4 specifically so they're mockable
  in tests without real I2C/GPIO).
- **No dependency vulnerability scanning**, despite doc 2 making real
  security-relevant crate choices (`argon2`, `tower-sessions`,
  `axum-login`). Added a `cargo-audit` job (RustSec advisory database,
  the canonical vulnerability check) gating the build jobs the same way
  `fmt`/`clippy` do. `cargo-deny` (license compliance + dependency-source
  policy, broader than `cargo-audit`'s CVE-only scope) is worth adding
  later once the dependency list stabilizes past the current empty
  `Cargo.toml` — introducing a license allowlist before there's anything
  to allowlist is premature.

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
- **Superseded: authentication.** This section originally argued for no
  login-wall by default. That's been reversed —
  [`research-auth-persistence-service.md`](./research-auth-persistence-service.md)
  designs a forced first-run admin setup and multi-user RBAC instead, and is
  the current source of truth for auth. Left the reversal noted here rather
  than silently deleting the old bullet, since the *reasoning* for reversal
  (a multi-user, privileged system needs real accounts, not a shared
  network-perimeter trust model) is useful context for anyone wondering why
  the direction changed.
- **Mobile-usable, not mobile-first** — primary use is a laptop/desktop
  browser on the LAN, but a phone pulled out to check "why is the fan loud"
  should work without horizontal scrolling. Responsive breakpoints, not a
  separate mobile design.

### 3.5 Frontend stack — decided: htmx + minijinja, with one vanilla-JS island

This was left as an open question in an earlier draft of this doc. Resolved
now, since "closing gaps" should include closing gaps in the research
itself, not just the product surface:

**Recommendation: `axum` + `minijinja` (server-rendered HTML) + `htmx` +
`axum-htmx`, not Svelte.** This is an established pattern (sometimes called
the "MASH stack": minijinja/axum/sqlite/htmx), not a novel combination —
and it directly resolves the concern that was holding the decision open.
The stated tiebreaker for Svelte was "richer live-updating charts" — but
htmx ships a native WebSocket extension (`hx-ws`) that does exactly the
"tick a value in place over the shared WS connection" job from §2.5's
protocol design via out-of-band HTML-fragment swaps, no hand-rolled
client-side JS needed for the status strip, sparkline updates, or the OLED
live-preview crossfade. That was the one real capability gap Svelte had
over htmx for this project; it isn't a gap.

What tips it decisively, beyond matching htmx's capabilities:

- **No separate JS build/toolchain** to keep in sync with the Rust daemon
  across releases — `minijinja` templates and `axum` handlers live in the
  same crate, same `cargo build`, same CI job from §2.9. A Svelte build
  step is one more thing the aarch64 cross-compile CI job (§2.9) would
  need to reproduce for a release.
- **The daemon already holds all the state in-process** (per §2.1) — server-side
  rendering isn't fetching from a separate API it doesn't otherwise need;
  it's just formatting data the daemon already has. htmx's server-renders-
  fragments model fits that shape directly; a client-side SPA fetching its
  own JSON from the same process it's embedded in is the extra layer.
- **The appliance-UI precedent** (Portainer, Proxmox, TrueNAS — cited in
  §3.1) skews server-rendered-with-light-JS, not SPA, reinforcing this isn't
  an unusual choice for this class of tool.

**The one carve-out: the fan curve editor.** Direct-manipulation dragging
of SVG points (§3.4) is genuinely awkward to express in pure htmx —
continuous pointer-drag isn't a request/response interaction. Recommend a
**single small vanilla-JS island** (no framework, no build step — a
`<script>` block or one static `.js` file, same pattern the interactive
mockups in `docs/mockups/` already use) that owns just that one widget,
talking to `GET/PUT /api/fan/curve/{cpu,hdd}` (§2.5) directly. Everything
else on the page stays htmx/server-rendered. This is "mostly htmx, one
JS island for the one truly interactive widget," not a hybrid architecture
— the rest of the app never needs client-side state management because
the fan curve editor doesn't either (it POSTs the final curve on save, same
as any htmx form).

Practical notes:

- Bake templates + static assets into the binary (`rust-embed` or
  `include_dir`), matching §2.1's single-binary preference.
- `axum-htmx` (typed `HX-*` header extractors/responses) over hand-parsing
  htmx's request headers — small, focused crate, not a framework
  commitment.
- No charting library needed anywhere — the sparklines and the fan curve
  editor are both small enough for hand-authored inline SVG (per the
  mockups), same call as before.

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

## 5. Sources consulted (gap-closing research pass)

`htmx`/`axum`/`minijinja` server-rendered stack, resolving §3.5's
previously-open frontend decision ([axum-htmx on
crates.io](https://crates.io/crates/axum-htmx),
[rust-minijinja-htmx example](https://github.com/thiagovarela/rust-minijinja-htmx),
[MASH stack writeup](https://emschwartz.me/building-a-fast-website-with-the-mash-stack-in-rust/)),
and `cargo-audit`/`cargo-deny` for §2.9's CI supply-chain gap ([Rust supply
chain security tools compared](https://blog.logrocket.com/comparing-rust-supply-chain-safety-tools/),
[cargo-audit and cargo-deny recipe](https://pocketcmds.com/recipes/rust/rust-dependency-audit)).
The OLED-asset licensing finding in §1.5 is a direct inspection of
`~/projects/argonone/downloaded_files/` (no license headers found in any
downloaded script or asset), not a web source. §1.6's reimplementation-
legality check is a direct read of [Argon40's Terms of
Service](https://argon40.com/policies/terms-of-service) and the
["Much Better Argon One Fan Linux Software
Alternative"](https://forum.argon40.com/t/much-better-argon-one-fan-linux-software-alternative/891)
forum thread, plus confirming `Argon40Tech/Argon40case` — the official
GitHub mirror of these scripts — carries no declared license (checked via
`GET /repos/Argon40Tech/Argon40case`, `license: null`) and doesn't include
the `.bin` assets at all. §1.7's binary-format decode is direct analysis
of the downloaded `.bin` files and `argononeoled.py`'s source, cross-checked
against [Adafruit's SSD1306 128×64 OLED
(#938)](https://www.adafruit.com/product/938) for the display hardware —
no other external sources.
