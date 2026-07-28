# Research: Active Session Management

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet. Motivated as a
natural pairing with [docs/research-totp-2fa.md](research-totp-2fa.md): once
a second factor protects login, "what if a session cookie leaks anyway" is
the next real question, and today there's no visibility into what sessions
even exist.

## 1. What exists today — and its real limitation

Sessions are `tower-sessions` 0.14 + `tower-sessions-sqlx-store` 0.15
(`Cargo.toml`), backed by the same SQLite pool as everything else
([`src/web/mod.rs`](../src/web/mod.rs)`::build_router`). Checked the store's
actual schema (`tower-sessions-sqlx-store`'s `SqliteStore::migrate()`):

```sql
create table if not exists tower_sessions (
  id text primary key not null,
  data blob not null,
  expiry_date integer not null
)
```

**The `data` column is a single `rmp_serde` (MessagePack) blob** — the whole
session record, opaque to SQL. Only `id` and `expiry_date` are queryable
columns; there is no `user_id`, `ip`, `user_agent`, or `last_seen_at` column
to query by. Building "show me my active sessions" against this table alone
means deserializing every row in Rust to find out whose session it even is
— workable, but a straight `SELECT ... WHERE user_id = ?` isn't possible
without adding our own side table (§3).

## 2. A coarse version of this already exists — and is worth keeping distinct

[`src/auth/mod.rs`](../src/auth/mod.rs)`::User::session_auth_hash` ties the
session to the current `password_hash`:

```rust
fn session_auth_hash(&self) -> &[u8] {
    self.password_hash.as_bytes()
}
```

`axum-login` re-checks this hash against the stored session on each
request, so **changing a password already invalidates every session for
that account** — including the one making the change (comment: "changing a
password (including an admin-issued reset) invalidates any existing
sessions for that account"). This is the "kick everyone out everywhere"
hammer, already shipped, for free. What's missing is the scalpel: *list*
what's active, and revoke *one* of them without nuking your own current
session and forcing a re-login on every other device too.

## 3. What a real implementation needs — a side table, populated by middleware

Propose `migrations/000N_session_metadata.sql`:

```sql
CREATE TABLE session_metadata (
  session_id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id),
  user_agent TEXT,
  ip_address TEXT,
  created_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);
```

Populated by:

- **On login** ([`src/web/login.rs::submit`](../src/web/login.rs), right
  after `auth_session.login(&user)` succeeds): insert a row keyed by the
  new session's id, capturing `User-Agent` and the request's peer address
  (or `X-Forwarded-For`, if this deployment is behind Tailscale/a reverse
  proxy — needs the same care `src/https.rs`'s mode-awareness already
  applies elsewhere, not a naive trust-the-header read).
- **On every authenticated request**: a small `axum::middleware::from_fn`,
  installed alongside the existing `require_login` layer
  ([`src/web/mod.rs`](../src/web/mod.rs)'s `protected` router), upserting
  `last_seen_at`. Same shape as `require_login` itself — this app already
  has the "small per-request middleware" pattern established, not a new
  architecture.
- **Cleanup**: rows whose `session_id` no longer exists in `tower_sessions`
  (expired or deleted) are orphaned metadata — needs a periodic sweep
  (`DELETE FROM session_metadata WHERE session_id NOT IN (SELECT id FROM
  tower_sessions)`), most naturally run from the same poll tick the fan
  control loop already uses in `service::run`, or a coarser once-an-hour
  interval given this is metadata cleanup, not anything latency-sensitive.

**Adjacent gap worth flagging, not fixing here**: nothing in this app today
runs `tower-sessions`'s continuous-deletion task for genuinely expired
session rows either — `SqliteStore::migrate()` is called, but not a
deletion-task spawn. Expired sessions may currently just accumulate in
`tower_sessions` forever (harmless — `expiry_date > ?` in the load query
already excludes them functionally — just an unbounded-growth housekeeping
gap). Out of scope for this feature specifically, but the same cleanup pass
this feature needs anyway is a natural place to also start pruning
`tower_sessions` itself, not just `session_metadata`.

## 4. Revocation — needs the store handle kept around

`SqliteStore` isn't stored anywhere today — it's constructed once in
`build_router`, then moved into `SessionManagerLayer::new(session_store)`
and never referenced again. `tower_sessions::SessionStore`'s trait exposes
`delete(&self, session_id: &Id)`. Since the store is cheaply `Clone` (backed
by the same pool clone every other DB access already uses), the fix is
small: clone it once before it's moved into the layer, and stash that clone
on `AppState` so a revoke handler can call `store.delete(&id)` directly —
no new architecture, just not discarding a handle this app already
constructs.

Revoking **the current session** (the row the request itself came in on)
needs to also actually log the browser out — same as the existing `/logout`
route — rather than leaving a stale cookie pointed at now-nonexistent
session data.

## 5. Scope: self-service only, not an admin cross-user view

Only ever list/revoke **the requesting user's own** sessions, filtered by
`user_id` in the side table — never expose another user's raw session ids
or metadata, even to an admin. Admins already have the equivalent-but-
coarser tool for this (force a password reset, which invalidates
everything for that account per §2) — a fine-grained "admin revokes one of
someone else's sessions" view isn't needed on top of that and would be a
meaningfully bigger privacy/scope question (should an admin be able to see
what IPs a viewer account logs in from?) not worth opening here.

## 6. Mockup

One new screen: [`docs/mockups/18-account-sessions.html`](mockups/18-account-sessions.html)
— an "Active sessions" card added to the existing Account page
(`10-account.html`'s route/shell), listing:

- The current session, labeled "This device," not revocable from itself
  (use `/logout` for that — no point offering a confusing "revoke this one"
  button that just does what logout already does).
- Every other session: a best-effort device/browser label parsed from
  `User-Agent` (simple substring matching — "Mobile", "Chrome", "Firefox",
  "Safari" — not a full UA-parsing dependency; labeled as approximate,
  never asserted as exact), IP address, and relative "last active" time.
- A per-row "Revoke" button, plus a "Revoke all other sessions" bulk action
  for the "I think something's wrong" case.

## 7. Open questions

- Should `ip_address` be shown at all to a `viewer`-role account, or is that
  over-sharing for the least-privileged tier? Leaning toward showing it to
  everyone for their *own* sessions only (§5's boundary) — it's your own
  login history, not someone else's.
- User-agent parsing: worth a tiny dependency (e.g. `woothee` or similar) or
  is hand-rolled substring matching suffficient? Given this app's existing
  aversion to dependencies for small jobs (rolling its own ANSI stripper
  rather than pulling in a crate, per the v0.7.0 log viewer), leaning toward
  hand-rolled and explicitly labeled "approximate" in the UI copy, same
  spirit as `docs/research-ups-battery.md`'s "detect presence, don't
  fabricate precision you don't have."
