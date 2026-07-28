# Research: Self-Update Check

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet. Closes a real gap:
there's currently no way to know a new release exists short of manually
checking GitHub — a headless Pi running this daemon has no other signal.

## 1. What's actually being proposed — a check, not an updater

Explicitly scoped as **read-only version comparison + a UI notice**, not an
in-place self-update mechanism. Auto-applying an update to a systemd-managed
service that also controls fan/power hardware is a much bigger blast-radius
feature (what happens to a live fan-control loop mid-restart, who owns
`.deb` upgrade vs. binary replacement, does it need the same care `admin
restore` gives migrations) — genuinely a separate, larger research topic if
ever wanted. This doc only covers "tell the admin a newer version exists."

## 2. The data source, verified live

`GET https://api.github.com/repos/arunkumar-mourougappane/argonone-rs/releases/latest`
— checked against the real repo just now:

```json
{
  "tag_name": "v0.7.0",
  "html_url": "https://github.com/arunkumar-mourougappane/argonone-rs/releases/tag/v0.7.0",
  "assets": ["argonone-rs-v0.7.0-linux-aarch64", "argonone-rs-v0.7.0-linux-aarch64.deb", "argonone-rs-v0.7.0-macos-arm64"]
}
```

Unauthenticated, public, no token needed for a public repo at this request
volume (one check per day per install, nowhere near GitHub's unauthenticated
rate limit of 60 req/hr/IP). Compare `tag_name` (strip the leading `v`)
against `env!("CARGO_PKG_VERSION")` — a simple `semver::Version` comparison
(crates.io: `semver` 1.0.28, already a transitive dependency of several
crates in `Cargo.lock` today, so likely zero new dependency weight) rather
than a naive string `!=`, so this doesn't misfire on a differently-formatted
but equal version string.

## 3. This app's own stated precedent argues for opt-in, not on-by-default

`src/notify/mod.rs`'s doc comment is explicit: the webhook notifier is
"**this app's first outbound network call**," scoped deliberately rather
than bolted on. An unconditional GitHub API call on every boot/timer would
make that comment simply wrong — and more importantly, contradicts a
posture this project has already committed to: outbound network activity
from a self-hosted appliance is something an operator opts into, not
something that happens by default before they've decided they want it.
This matters more here than for the webhook, since the webhook needs a URL
an operator explicitly typed in; a hardcoded GitHub endpoint requires no
configuration at all to start silently calling home.

**Recommendation**: off by default, one settings toggle (`db::settings`
pattern, same `load_x`/`save_x` idiom every other setting already uses) —
"Check for updates" — on the System page, next to the existing "Firmware &
service" card. When enabled, check once per boot plus once every 24h
thereafter (`tokio::spawn`ed interval task, same shape as the webhook's own
polling/notify task in `service::run`), not on every dashboard page load.

## 4. Failure handling

A failed check (no internet, GitHub down, rate-limited) should be silent —
log at `debug`, not `warn`/`error`, and simply not show the "update
available" banner. This isn't a monitored critical-state trigger like the
webhook's temp/RAID alerts; a Pi with no internet access (a common,
legitimate LAN-only deployment this app already supports) should never see
a spurious "can't check for updates" nag on every page load.

## 5. Mockup

One supplementary states-reference mockup, following the same pattern as
`docs/mockups/14-ups-battery.html` — not a new sidebar page, since this is a
small addition to the existing "Firmware & service" card on
`08-system-settings.html`, already showing the current `argonone-rs`
version:

- [`docs/mockups/17-system-update-banner.html`](mockups/17-system-update-banner.html)
  — the Firmware & service card in three states: up-to-date, update
  available (with a link out to the GitHub release), and checking-disabled
  (the default, opt-in-off state, with the toggle visible).

## 6. Open questions

- Should the update notice be admin-only-visible, or any logged-in role? The
  Firmware & service card today is visible to all roles (it's on the
  Dashboard's System card too) — leaning toward keeping the update notice
  equally visible, since "a newer version exists" isn't sensitive
  information the way a webhook URL or TOTP secret is. Only the *toggle* to
  enable/disable checking should be operator+/admin, matching this app's
  existing "viewers can look, not touch" convention.
- `.deb` users vs. manual-install users get the same notice today with no
  way to distinguish "you can just re-run `dpkg -i`" from "you built from
  source, here's the cross-compile command" — the banner's copy would need
  to stay generic (link to the release page, let the user pick their own
  install path) rather than trying to detect which install method is in use.
