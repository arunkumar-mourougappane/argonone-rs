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

v0.7.0 (in progress) — packaging and operability. So far: `.deb` packaging via `cargo-deb` (systemd unit, I2C udev rule, `argonone` system-user creation, and the daemon's own privilege drop off `root`), `admin backup`/`admin restore`, one-time legacy config-file import, a diagnosable read-only-rootfs startup error, critical-state webhook notifications, dashboard card drag-reorder, and a system log viewer. See [docs/ROADMAP.md](docs/ROADMAP.md#v070--packaging--operability) for the full item-by-item status.

**Not ready to tag yet** — two v0.7.0 items are still outstanding: install docs verified against real Ubuntu 26.04-on-Pi hardware, and the `.deb` package itself installed and confirmed on that hardware (everything above has only been verified in-process/locally, not on the actual Pi target).

## What's next

Closing the two hardware-verification gaps above before this milestone can ship as v0.7.0.
