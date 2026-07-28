# Research: UPS / Battery Status

Speculative — added to [docs/ROADMAP.md](ROADMAP.md) as a **low-priority**,
unscheduled item, not a committed milestone. Follow-up to a gap
[docs/research-rust-backend-webui.md](research-rust-backend-webui.md) already
flagged (§2.6 item 4, §3 "No UPS/battery data exists anywhere in this
codebase") and that `docs/ROADMAP.md`'s v0.6.0 entry calls out explicitly: the
dashboard mockup shows "Power source"/"Battery" rows that
[`docs/mockups/03-dashboard.html`](mockups/03-dashboard.html) currently
hardcodes (`UPS · plugged`, `100%`) rather than backs with real data.

## 1. This is an optional add-on, not stock EON/ONE hardware

Traced through the actual upstream source this time
(`~/projects/argonone/downloaded_files/scripts/argonone-upsconfig.sh`, outside
this repo — the Argon40 installer scripts), not just the summary in the first
research doc. Key finding: **UPS support is a separate, optional product**
("Argon Industria UPS," a distinct HAT), not something every Argon ONE/EON
owner has. `argonone-upsconfig.sh` is itself an *installer* — on first run it
`wget`s two extra daemons straight from `download.argon40.com`
(`argononeupsd.py`, `argonupsrtcd.py`) and registers them as their own
systemd services, separate from `argononed`. Their source was never part of
the downloaded snapshot this project's earlier research drew from — it's
fetched live, on demand, only by people who bought the add-on. This project
genuinely does not have that daemon's implementation to port.

## 2. The actual protocol: a text file, not I2C

Resolved the open question §2.6 item 4 of the first research doc left
unanswered ("is UPS status sourced from some external process's log file, or
does `argonone-rs` need to own UPS polling itself?"). Traced
`argondashboard.py`'s actual parser
(`~/projects/argonone/downloaded_files/scripts/argondashboard.py`,
`displayipbattery()`):

- The UPS daemon writes `/dev/shm/upslog.txt` — a shared-memory tmpfs path,
  plain text, `key: value` lines (colon-separated, key lowercased).
- The relevant line is `power`, e.g. `Power: Battery 45%` or `Power: AC 100%`.
  Its value is two space-separated tokens:
  - First token: `battery` → running on battery; anything else (`ac`) →
    charging/plugged.
  - Second token: the percentage (`"45%"`, `%` stripped, parsed as int).
- Display rule, straight from the Python source: while charging, `>99%`
  shows as **"Plugged"** rather than "Charging". While on battery, `<=20%` is
  alert/crit-colored, `<=50%` is warning-colored, else good — the same
  three-tier color semantics this app already uses for CPU/disk temp
  (`config::SAFETY_CEILING_C` et al.), so no new color convention needed.

So there is **no I2C register involved** — nothing to probe on the fan
controller or RTC chip. Whatever writes the UPS status is a wholly separate
process this app doesn't own and has never had the source for.

## 3. What this means for `argonone-rs` — two real options

1. **Tail the same file, if present.** If a user's Argon Industria UPS
   daemon is running independently (its own Python process, installed via
   `argonone-upsconfig.sh`, entirely outside `argonone-rs`), this app could
   just read `/dev/shm/upslog.txt` itself, same as the old curses dashboard
   did — cheap, matches this project's established "shell out / read a
   well-known path" pattern (`sysinfo`'s `smartctl`/`df` calls). Caveat: this
   depends on a third-party Python daemon nobody but Industria-UPS owners
   will ever have running, and that daemon's own reliability/format stability
   is outside this project's control.
2. **Own the polling.** Would require reverse-engineering the actual UPS
   HAT's hardware protocol (I2C address, register map) from scratch, since
   `argononeupsd.py`'s source was never in reach to read. Hits the same
   "can't verify without the physical hardware" wall this project already
   ran into with IR remote learn/program and ACME HTTPS
   (`docs/ROADMAP.md`'s v0.6.0/v0.7.0 entries) — except here there isn't even
   a one-line register hint to reconstruct from, unlike the IR case's I2C
   `0x82` documentation.

**Recommendation if this ever gets built:** option 1 (tail the existing file)
is the only version of this that doesn't require owning brand-new,
undocumented hardware protocol work. Detect presence the same way this app
already prefers — check whether `/dev/shm/upslog.txt` exists at startup,
cached for the process lifetime (matching the existing I2C-presence-probe
pattern in §2.6 of the first research doc) — and hide the Power &amp; RTC
card's UPS rows entirely when it's absent, the same "hide the section, don't
fabricate placeholder values" precedent already used for the Storage card's
RAID row when no array exists. See
[docs/mockups/14-ups-battery.html](mockups/14-ups-battery.html) for what each
state (plugged / charging / on-battery-warn / on-battery-crit / absent)
actually looks like against the real card design.

## 4. Why this stays low priority

- It's an optional, separately-purchased add-on — most Argon ONE/EON owners
  this app already serves have no UPS hardware to report on at all.
- The one buildable path (tailing a third-party daemon's log file) has zero
  verification story without a real Argon Industria UPS unit in hand — same
  category of "can't confirm without hardware" as IR remote and ACME, which
  is exactly why those two are still flagged unverified in the README despite
  being implemented.
- Nobody has asked for it yet. This doc and
  [docs/mockups/14-ups-battery.html](mockups/14-ups-battery.html) exist so
  the design question is answered *before* anyone needs it, not because it's
  imminent work.
