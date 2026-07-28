# Research: TOTP Two-Factor Authentication

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet. Motivated by this
app's own HTTPS/ACME work (`src/https.rs`) already anticipating
internet-exposed installs, not just LAN-only ones — single-factor auth is a
proportionate gap to close once a login form can be reached from the public
internet.

## 1. Crate choice

Checked current versions on crates.io (2026-07-27):

| Crate | Version | What it is |
|---|---|---|
| `totp-rs` | 5.7.2 | RFC 6238-compliant TOTP: secret generation, `otpauth://` URI, code verification with time-skew tolerance, and (via the optional `qrcodegen-image` feature) direct base64 QR-code PNG generation — no separate `qrcode`/`image` crate needed |

`totp-rs`'s `get_qr_base64()` returns a ready-to-embed `data:image/png;base64,...`
image, which fits this app's existing "bake everything into one binary"
philosophy (`rust-embed`/`include_dir` for templates/static, per
`docs/research-rust-backend-webui.md` §3.5) better than pulling in `qrcode` +
`image` and hand-rolling PNG encoding — one dependency, no new asset pipeline.

## 2. Schema — new migration, not an edit to `0001_init.sql`

This app's migrations run via `sqlx::migrate!("./migrations")`
([`src/db/mod.rs`](../src/db/mod.rs)), which checksums each applied migration
file. `migrations/0001_init.sql` has already been applied on real hardware
(the v0.7.0 verification pass) — editing it in place would break that
checksum on any existing install's next upgrade. This needs a new
`migrations/0002_totp.sql`:

```sql
ALTER TABLE users ADD COLUMN totp_secret TEXT;         -- base32, NULL until enrolled
ALTER TABLE users ADD COLUMN totp_enabled INTEGER NOT NULL DEFAULT 0;

CREATE TABLE totp_recovery_codes (
  id INTEGER PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id),
  code_hash TEXT NOT NULL,   -- Argon2id, same as password_hash — these are
                             -- one-time passwords, hash them like one
  used_at TEXT
);
```

`totp_secret` is stored as plain base32 text, not hashed — unlike a
password, the server needs the raw secret back to *generate* the expected
code for comparison, not just verify a hash. This is consistent with this
app's existing threat model: the webhook URL
([`src/notify/mod.rs`](../src/notify/mod.rs)) and IR code
(`db::settings::save_ir_code`) are already stored in plaintext in the same
DB file, trusted to filesystem permissions (the `argonone` system account,
v0.7.0's packaging work) rather than app-level encryption-at-rest. Recovery
codes are different: they're used exactly like a password (single-use
bearer credential a user might reuse a variant of elsewhere), so they get
the same Argon2id treatment `hash_password`/`verify_password`
([`src/auth/mod.rs`](../src/auth/mod.rs)) already gives real passwords.

## 3. The login-flow change — reuse `must_change_pw`'s existing gate pattern

This is the actual design problem, not the TOTP math. Today's flow
([`src/web/login.rs::submit`](../src/web/login.rs)):

```rust
match auth_session.authenticate(creds).await {
    Ok(Some(user)) => {
        auth_session.login(&user).await; // full session established here
        ...
    }
}
```

`axum-login`'s model conflates "credentials checked" and "session
established" into adjacent calls. A second factor needs a real gap between
those two — the session must not count as fully trusted between password-
verified and code-verified.

**This app already has a working precedent for exactly this shape**:
`must_change_pw`. A user with a pending forced password change *is* logged
in (`auth_session.login()` already ran) but
[`login::require_login`](../src/web/login.rs)'s middleware redirects every
route except `/account/change-password` until they clear it. Recommend the
identical pattern for 2FA rather than inventing a new pre-session "pending"
state: call `auth_session.login(&user)` as today, but if `totp_enabled`, set
a session flag (`pending_2fa`, stored via the same `tower_sessions::Session`
the app already holds) and extend `require_login`'s existing check —
already gating on `must_change_pw` — to also gate on `pending_2fa`,
redirecting everything except `/login/2fa` until the code is verified. One
gate mechanism, two reasons to hold it closed, not two different
auth-state machines.

Consequence worth being explicit about: the session cookie is issued before
the second factor passes. That's fine — nothing behind the gate is
reachable, so a stolen mid-flow cookie is exactly as useless as no cookie at
all; the pattern's whole point is the middleware, not delaying cookie
issuance.

## 4. Failed-code rate limiting — reuse the existing lockout counter

`src/auth/mod.rs` already has `record_failure`/`is_locked`
(`failed_attempts`, `locked_until`, `MAX_FAILED_ATTEMPTS`) for password
attempts. A brute-forceable 6-digit code (1,000,000 possibilities, RFC 6238's
own default window ±1 step = 3 valid codes at any instant) needs the same
protection — recommend routing failed `/login/2fa` attempts through the same
counter/lockout columns rather than adding a parallel rate-limit mechanism,
since the two failure modes (wrong password, wrong code) should count
against the same "someone's guessing at this account" signal, not be
tracked independently.

## 5. Enrollment and recovery flow

- **Enable**: Account page generates a secret, shows the QR
  (`get_qr_base64()`) + a manual-entry fallback string, and — critically —
  requires the user to submit one *currently valid* code before
  `totp_enabled` flips to true. Never enable on the strength of "I scanned
  it," only on "I proved I can generate a matching code" — otherwise a
  botched scan (wrong secret transcribed, wrong device clock) locks the user
  out of their own account on the next login.
- **Recovery codes**: generated once, at enable time, shown exactly once
  (same "shown once" UX as most TOTP implementations — Argon2id-hashed
  server-side per §2, so they can't be redisplayed later even to a
  legitimate re-request). A "Regenerate codes" action invalidates all
  previous ones.
- **Disable**: requires re-entering the current password (not just clicking
  a toggle) — disabling 2FA is a meaningful security downgrade, same
  "re-auth for a sensitive action" bar this app doesn't currently have
  anywhere else but should for this one.
- **Admin-side reset**: an admin needs a way to force-disable a user's 2FA
  (lost device, no recovery codes) — same shape as the existing
  `POST /api/users/{id}/reset-password` action on the Users page
  ([`src/web/users.rs`](../src/web/users.rs)), add a sibling
  `POST /api/users/{id}/reset-2fa`.

## 6. Mockups

Two new screens needed — this genuinely adds new user-facing flows, unlike
config export/import (CLI-only) or the metrics/UPS work (small additions to
existing cards):

- [`docs/mockups/15-login-2fa.html`](mockups/15-login-2fa.html) — the
  post-password challenge screen (code entry + "use a recovery code
  instead" link), extending the `02-login.html` flow as its own step, the
  same way the setup wizard and login already get their own full-screen
  mockups for each distinct unauthenticated step.
- [`docs/mockups/16-account-2fa.html`](mockups/16-account-2fa.html) — the
  self-service enable/manage flow on the existing Account page
  (`10-account.html`'s shell and route), covering: disabled → QR
  setup+confirm → recovery codes shown once → already-enabled (regenerate/
  disable). Reuses `10-account.html`'s exact card styling rather than
  inventing a new visual language, since this lives on the same page as the
  password-change card, not a new sidebar entry.

No change needed to `07-users-rbac.html`'s mockup file for the admin
reset-2FA action — that's a single new button next to the existing
"Reset password"/"Unlock" row actions already shown there, small enough to
describe here rather than needing its own supplementary states mockup.

## 7. Open questions

- WebAuthn/passkeys would be the more modern alternative to TOTP for this
  exact problem (no shared secret to steal, phishing-resistant) — not
  investigated here. TOTP is proposed first because it needs no client
  hardware capability beyond "a phone with an authenticator app," which
  matches this app's low-assumption target audience better; WebAuthn is
  worth its own research pass if TOTP ships and phishing-resistance becomes
  a stated goal later.
- Should 2FA be enforceable *per role* (e.g. admin-required, optional for
  viewer) rather than purely self-service opt-in? Leaning toward self-service
  only for a first version — an admin-mandated policy needs its own
  enrollment-grace-period UX (what happens on the login right after an admin
  flips that switch for someone who hasn't set it up yet) that's a second
  research pass, not part of the base feature.
