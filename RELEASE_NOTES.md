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

v0.6.0 is **HTTPS, dashboard data-surface gaps, hardening** — every v0.6.0 roadmap item, a full-fidelity pass against every `docs/mockups/` reference design, and a comprehensive post-implementation bug sweep across all eight feature areas. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan.

## What's New

- **HTTPS** — mode dispatch between plain HTTP, Tailscale-issued certs (`tailscale cert` + daemon-owned renewal), and `rustls-acme` for a custom domain. The session cookie's `Secure` flag now follows the active mode. Tailscale mode's System-page card also shows the real cert's issuer/expiry/auto-renew status and a manual "Re-issue now" action.
- **Audit log viewer** (`/audit`, admin-only) — paginated, actor/action-filtered, reading back everything `audit_log` has recorded since v0.5.0.
- **IR remote learn/program** — the System page's IR card can now learn a code from a physical remote and re-program it after a restart.
- **Setup-wizard exposure window closed** — first-run `/setup` now requires a one-time, console-printed token, regenerated every boot until an admin account is claimed.
- **Full dashboard rebuild** — the card grid every mockup specified (Fan control, Power & RTC, Network with a live sparkline, Storage, Display, System, Signed-in-as), each card reusing its own dedicated page's data. Network throughput, load average, and swap — the last Tier 1 dashboard gaps — are wired in throughout.
- **Self-service password change** — a sidebar account-menu link, no longer only reachable via a forced first-login redirect.
- A full mockup-fidelity and cross-page CSS consistency pass across every screen (nav icons, entrance/hover motion, shared design tokens, a fixed-width ramping-trend indicator on the status ribbon, and several fan/HDD curve chart rendering bugs).
- **Eleven bugs fixed** in a dedicated post-implementation review, spanning correctness (a login-lockout race, a fan-curve validation ordering bug, a setup-token fail-open path, a blocked tokio runtime during IR learn, RAID failed-member misclassification, board misdetection on a boot-time bus glitch, a nondeterministic dashboard label, an inaccurate "ramping" indicator, no cross-process I2C locking, an unanchored device-name match, and an ambiguous all-zero IR code) — see [CHANGELOG.md](CHANGELOG.md#v060---2026-07-19) for the full list.

## Known Limitations

- **HTTPS is not yet verified against real Tailscale/ACME infrastructure.** The mode-dispatch logic, the no-op/off-mode paths, and cert parsing against a locally-generated test certificate all pass in-process tests, but confirming the actual `tailscale cert` invocation and a real ACME directory flow needs an actual Tailscale-joined device and a real domain.
- **IR remote learn/program is unverified against real hardware.** The only documentation for I2C register `0x82` is one line ("IR code (block write)") — the implementation here is a best-effort reconstruction pending confirmation on a real Argon ONE/EON case.
- No UPS/battery, NIC link-speed, or MCU-firmware-register data exists anywhere in this codebase, so the corresponding mockup rows are intentionally omitted rather than fabricated.

## Deploying

No new migrations since v0.4.0 — `users`, `settings`, and `audit_log` already had every column this release needed.

```sh
cargo install argonone-rs
# or cross-compile / copy the binary to the Pi — see README.md, or use scripts/deploy.sh
sudo systemctl restart argonone-rs
```

See [README.md](README.md) for full deployment instructions.

## What's Next

v0.7.0 turns to packaging and operability. See [docs/ROADMAP.md](docs/ROADMAP.md#v070--packaging--operability).
