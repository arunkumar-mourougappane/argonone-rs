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

## Overview

v0.6.1 is a small, unplanned fix on top of v0.6.0: the fan curve editor now honors the Celsius/Fahrenheit unit setting.

## What's Fixed

- **Fan curve editor unit awareness** — `/fan`'s axis labels, point table, and "now" badge were hardcoded to °C regardless of the System page's unit toggle, unlike every other temperature reading in the app. They now display in whichever unit is configured; all internal curve math and the save API still speak Celsius, so saved curves are unaffected.
- No fan-speed computation was ever unit-aware-broken — the control loop and every server-side target calculation always evaluate against raw Celsius. This was a display-only gap.

## Deploying

No new migrations since v0.4.0.

```sh
cargo install argonone-rs
# or cross-compile / copy the binary to the Pi — see README.md, or use scripts/deploy.sh
sudo systemctl restart argonone-rs
```

See [README.md](README.md) for full deployment instructions.

## What's Next

v0.7.0 turns to packaging and operability. See [docs/ROADMAP.md](docs/ROADMAP.md#v070--packaging--operability).
