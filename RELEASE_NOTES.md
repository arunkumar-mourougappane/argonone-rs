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

## Highlights

Work in progress toward **v0.2.0 — EON extras (OLED + RTC)**, completing
Python-parity for the Argon EON case. Still CLI/systemd only, no web
server. See [docs/ROADMAP.md](docs/ROADMAP.md#v020--eon-extras-oled--rtc)
for the full milestone scope.

- OLED dashboard: screen-rotation state machine (configurable switch
  duration, screensaver blank-after-idle, power-button force-advance),
  seven live screens (clock, IP, CPU, RAM, storage, temperature, RAID),
  and an original splash screen (`RPI` rotated 90°, detected Pi model,
  `argonone` signature) — none of it built from Argon40's original
  assets; fonts/backgrounds are regenerated from permissively-licensed
  crates instead.
- RTC (PCF8563): daily wake-alarm programming and a daily sleep
  (scheduled poweroff) check driven off the RTC's own clock, both
  config-file driven (`/etc/argoneonoled.conf`, `/etc/argonrtc.conf`).
- `status` command now reports RTC time and the configured wake/sleep
  schedule alongside the existing CPU/RAM/disk/RAID/IP output.

**Not yet done**: verified on real EON hardware — v0.1.0 didn't ship
until that happened, and v0.2.0 needs the same before it's considered
complete.

## What's next

Once EON hardware verification closes out v0.2.0, v0.3.0 starts the web
server: SQLite persistence, forced first-run setup, and Argon2id auth —
infrastructure only, no feature screens yet. See
[docs/ROADMAP.md](docs/ROADMAP.md#v030--web-foundation-persistence-auth-live-shell).
