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

Nothing shipped as code yet — this project is in the research and design
phase (see [docs/ROADMAP.md](docs/ROADMAP.md), Phase 0). What exists so
far:

- Two research docs covering the full Rust rewrite: daemon architecture,
  hardware protocol, web UI/UX, auth, persistence, and systemd/HTTPS/
  packaging for Ubuntu 26.04 on Raspberry Pi.
- Nine interactive HTML mockups for every planned screen.
- A seven-milestone roadmap (`v0.1.0` → `v0.7.0`) sequencing that
  research into implementation order.
- CI and tag-triggered release workflows, ready for the first real binary.
- An original `argonone` OLED boot screen, replacing Argon40's splash
  entirely.

## What's next

`v0.1.0` is the first milestone with actual code — a CLI/systemd daemon
with I2C fan control, GPIO power-button handling, and system-info
collection, matching the existing Python daemon's core behavior for the
Argon ONE case. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full plan.
