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

v0.6.2 is a small, unplanned fix on top of v0.6.1: HTTPS over Tailscale-issued certs is now verified end-to-end against a real Tailscale-joined device, and a bug found in the process — the HTTPS listener hanging indefinitely on every connection — is fixed.

## What's Fixed

- **HTTPS listener hang (Tailscale/ACME modes)** — `spawn_server` only put the TCP listener in non-blocking mode for plain-HTTP mode, not `tailscale`/`acme`. Passing a blocking listener into tokio's async accept loop (via `axum_server::from_tcp`) is erroneous per tokio's own docs — every TLS connection hung indefinitely right after the `Client Hello`, on both loopback and Tailscale-routed traffic. Now fixed for all three HTTPS modes.

## What's New

- [docs/tailscale-setup.md](docs/tailscale-setup.md) — a full Tailscale HTTPS setup and troubleshooting guide, written from an actual end-to-end setup: prerequisites, enabling HTTPS Certificates on your tailnet, validating cert issuance and client-side network reachability, and troubleshooting the issues above.

## Deploying

No new migrations since v0.4.0.

```sh
cargo install argonone-rs
# or cross-compile / copy the binary to the Pi — see README.md, or use scripts/deploy.sh
sudo systemctl restart argonone-rs
```

See [README.md](README.md) for full deployment instructions and [docs/tailscale-setup.md](docs/tailscale-setup.md) if you're setting up Tailscale HTTPS for the first time.

## What's Next

v0.7.0 turns to packaging and operability. See [docs/ROADMAP.md](docs/ROADMAP.md#v070--packaging--operability).
