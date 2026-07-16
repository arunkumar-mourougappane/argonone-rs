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

# Release Notes — v0.3.0

## Overview

v0.3.0 is **Web foundation: persistence, auth, live shell** — the first release with a web server. It's deliberately scoped to infrastructure, not feature screens: no page in this milestone does more than prove auth, sessions, and the live status pipe work end to end. See [docs/ROADMAP.md](docs/ROADMAP.md) for the full v0.1.0 → v0.7.0 plan.

## What's New

- **SQLite persistence** — `users`, `settings`, `fan_curve_points`, and `audit_log` tables, WAL mode + `synchronous=NORMAL`, embedded `sqlx::migrate!()` migrations that self-apply on startup.
- **Forced first-run admin setup** — the first visit to a fresh install redirects to a setup wizard; a singleton-INSERT guard stops two browsers from racing to claim the first admin account.
- **Argon2id auth with three-role RBAC** (`admin`/`operator`/`viewer`) via `axum-login` and SQLite-backed `tower-sessions`.
- **Account safety**: a `must_change_pw` forced-change flow, and a DB-backed failed-login throttle (locks after 5 attempts for 15 minutes).
- **Password recovery** — an admin can reset any user's password from the API (`POST /api/users/{id}/reset-password`); if there's no admin left who can log in, `argonone-rs admin reset-password --username <u>` resets it directly against the database from the shell.
- **A bare authenticated shell** (`axum` + `minijinja` + `htmx`) with a live `stats`/`fan_state` WebSocket ticking over `htmx-ext-ws` — proves the real-time pipe works, even though there's no dashboard content yet.
- **`GET /api/status`** — an auth-gated snapshot/health endpoint reporting hardware presence alongside CPU/RAM/temp/fan stats.
- `htmx` and `htmx-ext-ws` are vendored from upstream GitHub releases (not CDN-loaded) and embedded into the binary, keeping the single-binary deploy story intact.

## Verified on Hardware

v0.3.0 has been run end-to-end on a real Argon ONE case: board auto-detection, the full setup → login → session flow, and `GET /api/status` returning live sysinfo were all confirmed over the network from a browser on the LAN.

## Deploying

The systemd unit now needs `StateDirectory=argonone-rs` for the SQLite database (already in [packaging/systemd/argonone-rs.service](packaging/systemd/argonone-rs.service) — re-copy it if upgrading from v0.2.0). The service still runs as `root` for I2C/GPIO access; dropping to a dedicated system user is `.deb`-packaging scope in v0.7.0.

```sh
cargo install argonone-rs
# or cross-compile / copy the binary to the Pi — see README.md
sudo cp packaging/systemd/argonone-rs.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now argonone-rs
```

Then visit `http://<pi-ip>:8080/` to complete setup. See [README.md](README.md) for full deployment instructions.

## What's Next

v0.4.0 is the highest-value milestone: the fan curve editor, storage/RAID page, and system settings — what actually replaces `argonone-fanconfig.sh` and friends with the web UI. See [docs/ROADMAP.md](docs/ROADMAP.md#v040--core-dashboard-fan-control-storage-system).
