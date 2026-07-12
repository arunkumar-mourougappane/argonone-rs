# argonone-rs

[![CI](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/arunkumar-mourougappane/argonone-rs/actions/workflows/ci.yml)

A rust based monitoring and management system for argon one cases of raspberry pi

## Status

Pre-implementation: research and UI design are done, code isn't started
yet ([Phase 0](docs/ROADMAP.md#phase-0--research--design-done) of the
roadmap). What exists today is the research docs, the interactive HTML
mockups, and CI/release workflows ready for when there's a binary to
build. See [docs/ROADMAP.md](docs/ROADMAP.md) for the v0.1.0 → v0.7.0
implementation plan.

## Docs

- [docs/ROADMAP.md](docs/ROADMAP.md) — milestone plan (v0.1.0 → v0.7.0) sequencing the research below into implementation order.
- [docs/research-rust-backend-webui.md](docs/research-rust-backend-webui.md) — what the existing Argon40 Python stack does, proposed Rust daemon architecture, and web UI/UX research (target: homelab/NAS self-hosters).
- [docs/research-auth-persistence-service.md](docs/research-auth-persistence-service.md) — forced first-run admin setup, multi-user RBAC, SQLite persistence, and systemd service install for Ubuntu 26.04 on Raspberry Pi.
- [docs/mockups/](docs/mockups/00-index.html) — interactive HTML mockups of the web UI (setup, login, dashboard, fan curve editor, storage/RAID, OLED display, users, system settings). Open `00-index.html` in a browser to start.
- [CHANGELOG.md](CHANGELOG.md) — cumulative log of every change, by version. [RELEASE_NOTES.md](RELEASE_NOTES.md) covers just the current unreleased cycle; past releases are archived under [docs/releases/](docs/releases/README.md).
