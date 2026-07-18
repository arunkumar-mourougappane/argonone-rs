# Research: Auth, RBAC, SQLite Persistence, and Service Install (Ubuntu 26.04 / Raspberry Pi)

Follow-up to [`research-rust-backend-webui.md`](./research-rust-backend-webui.md). That
doc assumed no login wall by default; this one designs the opposite —
forced first-run admin setup, multi-user RBAC, and SQLite-backed persistence
— for the concrete deploy target **Ubuntu Server 26.04 LTS on Raspberry Pi**.

Two auth domains get conflated easily; keep them separate throughout this doc:

- **App-level auth** — the web UI's own users (admin/operator/viewer),
  stored in SQLite, unrelated to Linux accounts. This is what "forced admin
  setup on first run" means.
- **OS-level service identity** — the Linux account the `argonone-rs`
  *process* runs as, and the group membership it needs for `/dev/i2c-1` and
  the GPIO chardev. Ubuntu's default `ubuntu` login user is irrelevant to
  either of these — don't run the daemon as it, and don't let it satisfy the
  "admin user" requirement.

## 1. First-run forced admin setup

### 1.1 Flow

1. On startup, before binding the "real" router, check `SELECT COUNT(*) FROM
   users`. If zero, the app enters **setup mode**.
2. In setup mode, every route except `/setup/*` returns a redirect to
   `/setup` (or `503` for API clients) — enforced by a small `axum`
   middleware that checks a `setup_complete: Arc<AtomicBool>` computed once
   at boot and flipped after the wizard completes, not re-queried per
   request.
3. `/setup` wizard collects: first/last name (display only, optional —
   `username` remains the sole login identifier and the only thing that
   must be unique), username, password (+ confirmation, strength check),
   and implicitly assigns `role = 'admin'` — the first account is always
   admin, no role picker on setup.
4. On submit: hash the password (Argon2id, see §2.1), insert the row,
   flip `setup_complete`, redirect to `/login`.
5. Guard against a race where two browsers hit `/setup` concurrently before
   either submits: enforce with a `UNIQUE` constraint on a singleton
   `INSERT OR IGNORE INTO settings(key,value) VALUES ('setup_complete','1')`
   done in the same transaction as the user insert — second submitter's
   transaction fails cleanly instead of creating two "first" admins.

That guard covers two browsers racing *each other*, but not the broader
exposure it was extracted from: **`/setup` is unauthenticated by
definition, so it's a "first request wins" admin claim, open to anyone who
can reach the box on the LAN** during the window between first boot and
whenever the real installer actually completes setup. On a single-user
home network this is a non-issue (you're the only one who can reach a
freshly-flashed Pi). It stops being a non-issue the moment the device sits
on a shared network — a household with other technical users, a shared
apartment LAN, a dorm — where "first to visit the IP" isn't necessarily
the person who racked the case. Two ways to close this, neither
implemented by default since both are extra steps for what's normally a
single-user device, but worth documenting as options:

- **Bind `/setup` to `localhost` only** until the first boot completes,
  requiring an SSH port-forward (`ssh -L 8080:localhost:80 ...`) to reach
  it remotely — closest to zero-trust, but adds a step to the common case
  (installer sitting on the same LAN, just wants to open a browser).
- **Print a one-time setup token to the console/journal on first boot**
  (`journalctl -u argonone-rs` or the physical console if one's attached),
  required as a query param on `/setup` — cheap, doesn't block the common
  LAN case, and matches the pattern other self-hosted tools (Jellyfin,
  ArgoCD) use for exactly this "who gets to claim admin first" problem.
  This is the better default of the two if one is going to be built at
  all: recommend implementing it, gated behind nothing so it doesn't
  complicate the LAN-only common case, but present in the docs telling
  installers to complete setup immediately after first boot rather than
  leaving the box sitting unconfigured on the network.

### 1.2 Password recovery — two tiers, no hints

An earlier draft of this doc proposed an optional password hint. Dropped:
storing a hint is a standing leak (DB file access, a backup, or any bug that
over-exposes the `users` table hands out a clue toward the password for
free) in exchange for a convenience that a proper admin-mediated reset
covers just as well without the tradeoff. Recovery instead has two tiers,
matching who's locked out:

**Tier 1 — a non-admin user forgets their password.** They ask an admin,
who resets it from the **Users** page — a plain "Reset password" action
against any account, admin included, that generates a new temporary
password (or a one-time setup link) and forces a change on next login. This
is the common case and it never needs shell access; it's just another
authenticated admin action (role model in §2.1), logged in `audit_log` like
any other.

**Tier 2 — there is no admin left who can log in** (sole admin forgot their
own password, or their account got locked by the failed-attempt throttle).
This is the headless-box fallback, same trust model as most self-hosted
appliances (Jellyfin, Portainer, etc.) — whoever already has shell access to
the Pi:

- Ship a CLI subcommand, e.g. `argonone-rs admin reset-password --username
  <u>`, runnable only by someone who can already run commands as the
  service's Linux user or root (`sudo -u argonone argonone-rs admin
  reset-password ...` or a root-only wrapper). It writes directly to the
  SQLite file, bypassing the web layer entirely.
- Alternatively/additionally, a one-time reset token file at
  `/var/lib/argonone-rs/reset-token` (mode `0600`, owned by the service
  user), generated on demand by the CLI subcommand and consumed by a
  `/setup/recover?token=...` route — same class of pattern as Django's
  `createsuperuser` / Jellyfin's recovery key file.
- Do **not** implement email-based recovery — there's no mail infrastructure
  on a home Pi and it'd be a false sense of security (whoever configures SMTP
  creds effectively owns recovery anyway).

The login page should surface both tiers in one line — "Forgot your
password? Ask an administrator, or with shell access run
`argonone-rs admin reset-password`" — so a non-admin user doesn't reach for
the CLI instruction that isn't meant for them.

## 2. Multi-user RBAC

### 2.1 Roles

Three tiers cover "modify vs. view hardware" cleanly without over-engineering
a generic permissions matrix the UI doesn't need yet:

| Role | Can view stats/dashboard | Can modify hardware settings (fan curve, OLED, RTC, units) | Can manage users |
|---|---|---|---|
| `viewer` | ✅ | ❌ | ❌ |
| `operator` | ✅ | ✅ | ❌ |
| `admin` | ✅ | ✅ | ✅ |

Model as a single `role TEXT CHECK(role IN ('admin','operator','viewer'))`
column rather than a separate permissions table — a full RBAC engine
(Casbin-style policies, per-resource ACLs) is overkill for three fixed tiers
over a handful of resources. Revisit only if a real need for
per-resource-instance permissions shows up (it won't for a fan
controller).

### 2.2 Libraries

- **Password hashing**: `argon2` (RustCrypto) + the `password-hash` crate
  traits, `SaltString::generate(&mut OsRng)`. Argon2id is the current
  general-purpose recommendation (OWASP), and the RustCrypto crate is
  maintained and dependency-light — no need for `bcrypt`/`scrypt` unless
  there's a specific reason.
- **Sessions**: `tower-sessions` + `tower-sessions-sqlx-store` (SQLite
  backend). Cookie-based, `HttpOnly` + `Secure` (set `Secure` conditionally —
  see §4.4 on TLS) + `SameSite=Lax`. Sessions live in the *same* SQLite file
  as everything else, so a service restart doesn't silently log everyone
  out — sessions persist until their expiry, not just until process
  restart, which matters for a service that may restart on `journalctl`
  driven crashes or updates.
- **Auth/authz middleware**: `axum-login` sits on top of `tower-sessions`
  and gives an `AuthSession<Backend>` extractor plus `login_required!` /
  `permission_required!` macros (or the newer `require` builder). Implement
  `AuthUser` (wraps the `users` row) and `AuthnBackend` (verifies
  username+password against the Argon2 hash) once; role checks can be a
  thin custom extractor (`Extension`-style) that reads `AuthSession::user`'s
  role and rejects with `403` — simpler than wiring axum-login's generic
  `AuthzBackend` for just three fixed roles, but that trait is there if the
  permission model grows.
- **Rate limiting login attempts**: worth a simple in-memory
  (`governor` crate) or DB-backed (`failed_attempts` + `locked_until`
  columns on `users`) throttle — this box may be reachable from outside the
  LAN via VPN/port-forward, don't skip this because it "seems like a home
  toy."

### 2.3 Schema sketch

```sql
CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    username        TEXT UNIQUE NOT NULL,
    first_name      TEXT,
    last_name       TEXT,
    password_hash   TEXT NOT NULL,        -- PHC string, argon2id
    must_change_pw  INTEGER NOT NULL DEFAULT 0,  -- set on admin-issued reset
    role            TEXT NOT NULL CHECK (role IN ('admin','operator','viewer')),
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until    TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    last_login_at   TEXT
);

-- tower-sessions-sqlx-store owns its own table (`tower_sessions`), created
-- by its own migration — don't hand-roll this one.

CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,             -- JSON for structured values
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_by  INTEGER REFERENCES users(id)
);

CREATE TABLE fan_curve_points (
    id       INTEGER PRIMARY KEY,
    curve    TEXT NOT NULL CHECK (curve IN ('cpu','hdd')),
    temp_c   REAL NOT NULL,
    fan_pct  INTEGER NOT NULL CHECK (fan_pct BETWEEN 0 AND 100)
);

CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER REFERENCES users(id),
    action      TEXT NOT NULL,             -- e.g. "fan_curve.update", "user.create"
    detail      TEXT,                      -- JSON
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
```

`audit_log` isn't strictly requested but is cheap to add now and valuable
later given this is a multi-user privileged system (who changed the fan
curve / added a user, and when) — flagging it as a recommendation, not
assuming it's wanted.

## 3. SQLite persistence details

### 3.1 sqlx vs. rusqlite

Given the daemon is already `tokio`+`axum` (per the prior doc) and
`tower-sessions-sqlx-store` is the natural session backend, **`sqlx`
(SQLite driver, async)** is the better fit here — it avoids bridging a sync
`rusqlite` connection across `spawn_blocking` for every query, and its
compile-time-checked queries (`sqlx::query!`) catch schema drift at build
time. `rusqlite` remains the simpler choice for a *sync-only* CLI-style
tool; it's not that here.

Use `sqlx::migrate!()` (embedded migrations, compiled into the binary) so
the binary self-migrates its own schema on startup — critical for a
curl-pipe-bash-style single-binary update story where there's no separate
migration-runner step.

### 3.2 Where the DB file lives, and why it must be real disk

`/var/lib/argonone-rs/argonone.db` (systemd `StateDirectory=`, see §4.2) —
**not** `/dev/shm` or `/tmp`, which some of the existing Python scripts use
for ephemeral status files (`upslog.txt`, the shutdown flag) — those don't
need to survive a reboot, but users/settings/fan-curves absolutely do. This
is the direct answer to "persist across boots": a tmpfs-backed path is
wiped at boot by definition, so the DB has to be under a real,
`StateDirectory`-managed path on the SD card / SSD.

### 3.3 Durability vs. SD-card wear

Two competing concerns on a Pi: (a) don't corrupt the DB on a power-loss
event (no clean shutdown — this device controls its *own* power state, so
this genuinely happens), (b) don't wear out the SD card with excessive
writes.

- Enable **WAL mode** (`PRAGMA journal_mode=WAL`) — better concurrent
  read/write behavior for a web app with concurrent requests, and more
  resilient to torn writes than rollback-journal mode.
- `PRAGMA synchronous=NORMAL` (the documented-safe pairing with WAL: safe
  against app/process crashes, small risk of losing the most recent
  transaction on an actual power loss, but not database corruption) — full
  `synchronous=FULL` is more paranoid but meaningfully slower on SD card
  storage for negligible real benefit here, since writes are infrequent
  human-driven config changes, not a transaction log.
- **Do not persist high-frequency telemetry to SQLite** — temperature/fan
  RPM/CPU% readouts pushed over the WebSocket (per the previous doc) should
  stay in-memory only. Writing every few-second sensor sample to disk would
  both wear the SD card and bloat the DB for data nobody queries after a
  few minutes. If historical graphs are wanted later, that's a
  purpose-built ring buffer or a separate opt-in time-series table with
  aggressive rotation — not a default.
- Recommend the install docs mention (not necessarily *implement*) that a
  UPS-less Pi doing frequent writes benefits from a decent SD card /
  moving to SSD/USB boot; this is an operational note, not something the
  Rust app can fix.

### 3.4 Backup and restore

Never addressed: this SQLite file is now the **sole** source of truth for
users, roles, fan curves, and settings — the text-config-file fallback
that made the Python version trivially backup-able (just `tar` up
`/etc/argon*.conf`) is gone by design (§1.3 of the previous doc explicitly
drops the config-file source-of-truth model). Losing the SD card now means
losing accounts and tuning, not just re-running a setup script. Worth
closing since it's a direct consequence of a design decision made
elsewhere in this doc, not a hypothetical:

- **Backup is mechanically trivial** — WAL mode (§3.3) means the live DB
  file plus its `-wal`/`-shm` sidecars are a valid backup if copied
  together, or (cleaner) use SQLite's own **online backup API**
  (`sqlx` can shell out to `sqlite3 /path/to/db ".backup '/path/to/out'"`,
  or use the `rusqlite`-style `backup` module if that dependency ends up
  in the tree anyway for the CLI reset tool from the prior doc's §1.2) to
  get a consistent snapshot without stopping the service. No custom
  export format needed — it's just SQLite.
- **What's missing is the *documented, easy* path**, not the mechanism:
  add a CLI subcommand, `argonone-rs admin backup --output <path>`, doing
  exactly the online-backup call above, and `argonone-rs admin restore
  --input <path>` that stops accepting writes, swaps the file, and
  restarts (or just documents "stop the service, replace the file, start
  the service" if a dedicated subcommand feels like scope creep for a
  first pass). Either way, this needs to exist and be documented in the
  install docs, not left as "well, it's just a SQLite file, you could
  copy it" tribal knowledge.
- **Doesn't need to be automatic.** No cron-backup-to-cloud story is being
  proposed here — this is a home device, and building a backup-scheduling
  system is real scope for a fan controller. The bar is "a documented
  single command an installer can run before an SD card swap or OS
  upgrade," not managed backup infrastructure.
- Cross-reference: this is the same tension as §3.3's SD-card-wear
  discussion, opposite direction — durability of *existing* data
  (frequent small writes) vs. recoverability of *all* data (infrequent
  full copies). Both are real, neither substitutes for the other.

### 3.5 Read-only rootfs: detect and fail loudly, not silently

Gap found comparing against RPi-Monitor (the established prior art in this
exact niche — Perl daemon + RRD, `github.com/XavierBerger/RPi-Monitor`),
which explicitly supports running against a **read-only root filesystem**
— a real, increasingly common Pi hardening/longevity setup (overlayfs root,
SD card mounted read-only, writes redirected to tmpfs or nowhere).
argonone-rs's entire persistence model (§3.2 above) assumes a writable
`/var/lib/argonone-rs` — reasonable as the *primary* supported mode, this
project's "SQLite is the sole source of truth" design (§1–§3) is a
deliberate choice RPi-Monitor's RRD-plus-config-file model didn't have to
make. Full parity (monitoring continues, all writes degrade gracefully) is
real scope that cuts against that design and isn't being proposed here.

What's missing isn't the capability, it's the *failure mode*: today, if
`/var/lib/argonone-rs` isn't writable, `crate::db::connect` returns
whatever raw `sqlx::Error` SQLite produces (`service.rs`'s `run()`, which
logs it via `tracing::error!` and exits) — accurate, but not actionable.
Someone hitting this on a hardened image sees a generic I/O error, not
"this daemon needs a writable state directory." Cheap, worth doing
regardless of whether broader read-only support ever happens: detect this
specific failure at startup (the DB path's parent directory isn't
writable, or the open fails with a permissions-shaped error) and log a
message that names the actual constraint and points at the fix (mount
`/var/lib/argonone-rs` read-write, or don't run this daemon on a
read-only-root image). No new capability, no schema/architecture change —
just turning a silent/confusing failure into a diagnosable one.

## 4. Running as a systemd service with the right privileges (Ubuntu 26.04 / Raspberry Pi)

### 4.1 Does a new Linux account need to be created?

**Yes — a dedicated *system* service account, not the interactive `ubuntu`
user and not root.** Two ways to get there:

**Option A — static system user (simplest to reason about, recommended):**
```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin \
    --home-dir /var/lib/argonone-rs argonone
sudo usermod -aG dialout argonone     # see §4.3 for why dialout
```
Stable UID across restarts/upgrades, straightforward to `chown` the state
directory and debug ("who owns this file").

**Option B — `DynamicUser=yes` in the unit** (systemd allocates an ephemeral
UID per run, no `useradd` step needed at install time). systemd's
`StateDirectory=` still gives a *persistent* directory across the changing
UIDs (it manages ownership via the directory itself, not a fixed UID you
have to remember), and `SupplementaryGroups=` is honored even with
`DynamicUser=yes`, so hardware group access still works. This is less to
document/package (no explicit `useradd` in the install script) but slightly
more "magic" if someone's debugging file ownership by hand later.

Recommendation: **Option A** for this project — the install story here is
closer to "curl-pipe-bash / single deb" than "twelve-factor cloud app," and
a stable, greppable UID matches the debugging expectations of the
self-hoster audience identified in the UI/UX doc. Either is fine
technically.

Either way: this account is **separate from** the web UI's own `admin`
SQLite user created in the setup wizard (§1) — don't conflate "the Linux
account the process runs as" with "the first web login."

### 4.2 systemd unit

```ini
[Unit]
Description=Argon40 case monitor/controller (fan, OLED, RTC, web UI)
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/bin/argonone-rs --service
User=argonone
Group=argonone
SupplementaryGroups=dialout
StateDirectory=argonone-rs
StateDirectoryMode=0750
Restart=on-failure
RestartSec=5

# Hardening — none of this needs to be root
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/argonone-rs
DeviceAllow=/dev/i2c-1 rw
DeviceAllow=char-gpio rw

[Install]
WantedBy=multi-user.target
```

Notes:
- `Type=notify` + `sd_notify(READY=1)` (via the `sd-notify` crate, called
  once the web server and hardware backends are actually up) gives systemd
  an accurate "is it really ready" signal — an improvement over the
  existing Python units' `Type=simple` + `RemainAfterExit=true` guesswork
  flagged in the prior doc.
- `StateDirectory=argonone-rs` makes systemd create
  `/var/lib/argonone-rs` owned by the service user before start, every
  boot — this is what makes Option A/B both work without a manual `mkdir`
  in an install script.
- `DeviceAllow=` lines are effective under `ProtectSystem=strict` (which
  otherwise makes `/dev` read-only-ish for the unit) — needed in addition
  to group membership when hardening flags are used; drop them if
  `ProtectSystem`/device cgroup restrictions turn out to fight with runtime
  GPIO chardev access in testing (device cgroup filtering by device *name*
  glob like `char-gpio` needs verifying on the actual target kernel —
  flagged as something to confirm on real hardware, not assumed).

### 4.3 Group access to `/dev/i2c-1` and GPIO on Ubuntu (not Raspberry Pi OS)

This is genuinely different from Raspberry Pi OS, which ships
`raspberrypi-sys-mods` to pre-create `i2c`/`gpio`/`spi` groups and udev
rules out of the box. **Ubuntu Server for Raspberry Pi does not
necessarily ship the same setup**, so this needs to be handled by the
package/install script, not assumed:

- Ubuntu's stock udev rules already put **GPIO chardev access under the
  `dialout` group**, and the default first-boot user is a member of
  `dialout` — confirmed current practice for Ubuntu Server on Pi.
- **`/dev/i2c-1` is not guaranteed to be group-`dialout` (or even to have a
  dedicated group) out of the box** — ship a udev rule as part of the
  install (`/etc/udev/rules.d/60-argonone-i2c.rules`):
  ```
  SUBSYSTEM=="i2c-dev", KERNEL=="i2c-1", GROUP="dialout", MODE="0660"
  ```
  reusing the existing `dialout` group rather than inventing a new `i2c`
  group — one group to add the service account to, matching the group GPIO
  chardev access already uses, less to document.
- Enabling the I2C interface itself (the `dtparam=i2c_arm=on` /
  `i2c-dev` kernel module) is a `config.txt`/`/boot/firmware/config.txt`
  step, independent of Rust or systemd — the install docs need a step for
  this regardless of language, same as the existing Python installer
  handles it via `argon1.sh`'s config.txt editing.
- Verify all of the above **on actual Ubuntu 26.04 Raspberry Pi hardware**
  before shipping — group-name/default-membership specifics are the kind
  of thing that drifts between Ubuntu releases and is worth a real check
  rather than trusting this doc a version later.

### 4.4 Automatic, browser-trusted HTTPS

Revises an earlier, thinner version of this section that suggested a
self-signed cert "generated on first run." That doesn't actually satisfy
"trusted by browsers" — a self-signed cert (or a private CA the browser
doesn't already trust) produces the standard "Your connection is not
private" interstitial regardless of how it's generated, so it was never a
real answer to this requirement. Researched properly below.

**The hard constraint first, because it shapes every option**: a publicly
trusted CA (Let's Encrypt included) will not issue a certificate for a
bare private IP address or for a hostname that doesn't publicly resolve —
this is a CA/Browser Forum baseline requirement, not a Let's Encrypt
policy choice. A device that's *only* reachable as `192.168.1.42` or
`argonone.local` cannot get a browser-trusted cert, full stop, no matter
what tooling runs on it. "The web server does the work" is achievable —
"works with zero network prerequisites" is not. The options below differ
in *what* prerequisite they require, tiered by how well each fits this
project's already-established audience (self-hosters, several already on
Tailscale per doc 1 §3.1):

**Option A — Tailscale-issued certs (recommended primary path).** If the
device is already on a tailnet (a documented common case for this
audience), Tailscale operates its own integration with Let's Encrypt for
each node's `*.ts.net` MagicDNS name — a real, publicly resolvable name
even though the IP behind it is only reachable over the tailnet, so it
clears the constraint above without exposing anything to the public
internet or requiring the user to own a domain. Mechanically: the daemon
shells out to `tailscale cert --cert-file=<path> --key-file=<path>
<hostname>.ts.net` — consistent with this project's existing pattern of
shelling out to well-established external tools (`smartctl`, `mdadm`)
rather than reimplementing them, and Tailscale doesn't expose a stable
public Rust API for this, the CLI is the sanctioned interface. Two things
this needs that a naive "call it once on first run" wouldn't cover:

- **Detecting Tailscale is actually present and running** before
  offering this as an option — `tailscale status --json` (or checking
  for the `tailscaled` socket) rather than assuming; this is a UI-gating
  decision (§2.6-style runtime detection, same pattern as
  ONE-vs-EON hardware detection in the other doc) as much as a TLS one.
- **Renewal is the daemon's job, not Tailscale's**, for file-based certs.
  `tailscaled` auto-renews certs it serves directly, but a cert dumped to
  files via `--cert-file`/`--key-file` is explicitly the caller's
  responsibility to renew (confirmed from Tailscale's own docs) — Let's
  Encrypt certs are 90-day. The daemon needs a background task checking
  the cert's expiry (parse the X.509 `notAfter`, `x509-parser` crate or
  similar) and re-running `tailscale cert` when under ~30 days remain,
  same cadence discipline as the fan-curve hysteresis timer elsewhere in
  this project, just for a different resource.

**Option B — Let's Encrypt via `rustls-acme`, for a real domain
(secondary/optional path).** For installers who own a domain (or a free
dynamic-DNS name like a `duckdns.org` subdomain) pointed at their home IP,
with port 443 forwarded to the Pi. This is the one path where "the web
server does the work" is literal, not shelled-out: `rustls-acme` is an
async ACME client built for exactly this, with an `axum`-compatible TLS
acceptor — cert acquisition and renewal both happen in-process, no
external CLI, no cron job. Two implementation notes worth having decided
up front rather than discovered mid-build:

- **Use TLS-ALPN-01, not HTTP-01**, as the challenge type —
  `rustls-acme` defaults to and recommends TLS-ALPN-01 specifically
  because it only needs port 443 open, not 80+443. One forwarded port
  instead of two is a meaningfully smaller attack surface for a device
  that also controls case power/fans over I2C.
- **Certificate/account caching is mandatory, not an optimization** —
  `rustls-acme`'s own docs warn that a production server must cache
  issued certs and the ACME account (its `caches::DirCache` file-based
  cache covers this) or risk hitting Let's Encrypt's rate limits on
  every restart. Cache path: alongside the SQLite DB under
  `StateDirectory=` (§3.2), not somewhere that gets wiped on upgrade.
- This path means opening port 443 to the public internet on a home
  router — a real, explicit trade-off to surface in the UI (§4.4's mockup
  update below), not a silent consequence of picking "Let's Encrypt" from
  a dropdown.

**Option C — HTTP only (explicit opt-out, not a silent default).** For
installers on a pure LAN with no Tailscale and no domain, there is no
browser-trusted option — say so plainly in the UI rather than offering a
self-signed cert that *looks* like it solved the problem but produces a
browser warning anyway. If a cert is wanted for pure-LAN use despite the
warning (e.g., a private CA installed on managed devices), that's a
reverse-proxy/`step-ca` concern outside this project's scope, not
something `argonone-rs` should build a private-CA-plus-trust-distribution
system to solve.

**`Secure` cookie flag**: ties directly to which of the three modes is
active, not a separate decision — `Secure` on whenever HTTPS is actually
terminated (Options A or B), off for Option C. No `--tls` flag needed
distinct from the mode selection above; the mode *is* the flag.

| Concern | Crate/tool | Why |
|---|---|---|
| Tailscale cert acquisition | shell out to `tailscale cert` (CLI) | no stable public Rust API for this; matches the project's existing external-tool pattern |
| Cert expiry parsing (Option A renewal check) | `x509-parser` | small, dependency-light, enough to read one field (`notAfter`) |
| Let's Encrypt ACME client (Option B) | `rustls-acme` | async, axum-compatible acceptor, TLS-ALPN-01 by default, handles acquisition + renewal + must-have caching in one crate |
| TLS termination | `rustls` (via `rustls-acme`'s acceptor, or `axum-server`'s `tls-rustls` feature for Option A's file-based certs) | already the TLS stack implied by `tower-sessions`/`axum-login`'s dependency tree, no reason to add `native-tls`/OpenSSL alongside it |

Sources: `rustls-acme` ([docs.rs](https://docs.rs/rustls-acme/latest/rustls_acme/),
[axum ACME discussion](https://github.com/tokio-rs/axum/discussions/495)),
Tailscale's own HTTPS docs on `tailscale cert` and its renewal caveat
([Enabling HTTPS](https://tailscale.com/docs/how-to/set-up-https-certificates),
[Secure Tailscale services with TLS](https://tailscale.com/blog/tls-certs)),
`instant-acme` as the maintained lower-level ACME client `rustls-acme`
builds on (mentioned for completeness, not needed directly unless a
custom DNS-01 flow is built later for non-Tailscale, no-port-forward
installs — that's real additional scope, not assumed here).

### 4.5 Packaging as a `.deb`

Both this doc and the previous one repeatedly assume a "curl-pipe-bash /
single deb" install experience (§4.1's reasoning for picking a static
system user over `DynamicUser=`, §1 of the prior doc's "matches this
project's install story") without ever actually researching the packaging
step itself. Closing that: **`cargo-deb`** is the standard tool for this —
generates a `.deb` directly from `Cargo.toml` metadata plus a
`[package.metadata.deb]` table, no separate packaging-repo/spec file to
maintain in parallel with the Rust project.

What the package needs to carry, mapping directly onto artifacts this pair
of docs already designed:

- **The binary itself**, built for `aarch64` (matches the CI release
  workflow's Pi target).
- **The systemd unit** from §4.2 — `cargo-deb` has first-class support for
  this via `[package.metadata.deb.systemd-units]`, which auto-adds the unit
  file as a package asset *and* generates the `postinst`/`prerm`/`postrm`
  script fragments that enable+start the service on install and
  stop+disable it on removal — the exact "enable at boot" requirement from
  the original ask, without hand-writing maintainer scripts for it.
- **The udev rule** from §4.3
  (`/etc/udev/rules.d/60-argonone-i2c.rules`) — a plain asset entry in
  `[package.metadata.deb]`'s `assets` list, installed to
  `/etc/udev/rules.d/`. Needs a `udevadm control --reload-rules` in a
  small custom `postinst` snippet (`cargo-deb` supports supplying your own
  maintainer-script fragments alongside its generated systemd ones) so the
  rule takes effect without a reboot.
- **The `argonone` system user** from §4.1 Option A — also belongs in a
  `postinst` snippet (`useradd --system ...`, idempotent — check
  `id argonone` first so a package *upgrade* doesn't error trying to
  recreate an existing user), run before the systemd-units script fragment
  tries to start a service as a user that doesn't exist yet.
- **`/boot/firmware/config.txt` I2C enablement** (`dtparam=i2c_arm=on`) —
  deliberately **not** automated in `postinst`. Editing the bootloader
  config unattended and possibly requiring a reboot to take effect is a
  bigger blast-radius action than a package install should silently take;
  keep this a documented manual step (same as `argon1.sh` already asks the
  installer to do), with a clear message if the daemon can't reach the I2C
  bus post-install pointing at the missing `config.txt` step.

**Upgrade path**: `sqlx::migrate!()` (§3.1) already self-migrates the
schema on the next service start, so a `.deb` upgrade is just "replace the
binary, restart the service" — `cargo-deb`'s generated `postinst` already
does the restart-on-upgrade as part of its systemd-units support. No
special upgrade migration tooling needed beyond what §3.1 already
specified; flagging that the packaging layer doesn't need to duplicate it.

**Not building yet**: an actual `[package.metadata.deb]` table, since
there's no daemon code, systemd unit file, or udev rule file checked into
the repo to reference — this section documents the packaging *plan*, the
config itself is real work for once those artifacts exist, not before.

## 5. Summary of concrete recommendations

- Argon2id via the `argon2` crate for password hashing.
- `axum-login` + `tower-sessions` + `tower-sessions-sqlx-store` for
  auth/session, SQLite-backed so sessions survive process restarts.
- `sqlx` (not `rusqlite`) for the DB layer, `sqlx::migrate!()` embedded
  migrations, WAL mode, `synchronous=NORMAL`.
- Three fixed roles (`admin`/`operator`/`viewer`) as a column, not a
  generic permissions engine.
- Forced setup wizard gated on `users` table being empty; no password
  hints. Recovery is two-tiered — admin resets any user from the Users
  page, CLI-based reset only for the headless "no admin can log in" case,
  no email-based recovery.
- DB file under systemd `StateDirectory=` on real disk, never `/tmp`/`/dev/shm`.
- No time-series telemetry written to SQLite — live stats stay in-memory/WebSocket-only.
- Dedicated `argonone` system account (static `useradd --system`,
  recommended over `DynamicUser=` for this project's debugging/ops style),
  member of `dialout`, plus a shipped udev rule putting `/dev/i2c-1` under
  `dialout` too (Ubuntu doesn't do this by default the way Raspberry Pi OS does).
- Confirm the Ubuntu-26.04-on-Pi group/udev specifics against real hardware
  before shipping — this doc's §4.3 claims are current best understanding,
  not hardware-verified for this exact release.
- Setup-wizard exposure window: recommend a one-time console-printed setup
  token (§1.1) rather than trusting "first request wins" on shared
  networks; document "complete setup immediately after first boot"
  regardless.
- Backup/restore isn't automatic, but needs a documented `admin backup`/
  `admin restore` CLI path (§3.4) using SQLite's online-backup API — the
  DB is now sole source of truth, unlike the old text-config-file model.
- Package as a `.deb` via `cargo-deb` (§4.5), bundling the systemd unit,
  udev rule, and `argonone` user creation into generated maintainer
  scripts — `config.txt` I2C enablement stays a documented manual step,
  not automated in `postinst`.
- HTTPS (§4.4, revised): no self-signed-cert option presented as "secure"
  — it isn't, browsers still warn. Tiered instead: Tailscale-issued certs
  via `tailscale cert` as the recommended default for this audience (real
  browser trust, zero public exposure, daemon owns renewal since
  file-based certs aren't auto-renewed by `tailscaled`); `rustls-acme`
  (TLS-ALPN-01) for installers with a real domain and port 443 forwarded,
  fully in-process; plain HTTP as an explicit, clearly-labeled opt-out for
  everyone else — a publicly-trusted cert for a private-IP-only device is
  not achievable by any tooling, CA/Browser Forum baseline requirement,
  not a gap in this research.

Sources consulted: Ubuntu/Raspberry Pi GPIO/I2C permission conventions
([Dr. Gutow — Ubuntu 20.04 GPIO/I2C](https://cms.gutow.uwosh.edu/Gutow/useful-chemistry-links/software-tools-and-coding/computer-and-coding-how-tos/allowing-access-to-gpio-i2c-and-spi-on-pi-under-ubuntu-20.04),
[Robotics Back-End — Pi hardware permissions](https://roboticsbackend.com/raspberry-pi-hardware-permissions/)),
`axum-login`/`tower-sessions` ecosystem ([axum-login on GitHub](https://github.com/maxcountryman/axum-login),
[docs.rs/axum-login](https://docs.rs/axum-login)), `sqlx`/`rusqlite`
tradeoffs for an embedded systemd service ([Diesel vs SQLx vs SeaORM vs
Rusqlite, 2026](https://aarambhdevhub.medium.com/rust-orms-in-2026-diesel-vs-sqlx-vs-seaorm-vs-rusqlite-which-one-should-you-actually-use-706d0fe912f3)),
`cargo-deb`'s systemd-unit packaging support ([cargo-deb on
GitHub](https://github.com/kornelski/cargo-deb),
[cargo-deb systemd.md](https://github.com/kornelski/cargo-deb/blob/main/systemd.md)),
and automatic HTTPS provisioning ([rustls-acme on
docs.rs](https://docs.rs/rustls-acme/latest/rustls_acme/), [axum ACME
discussion](https://github.com/tokio-rs/axum/discussions/495), [Tailscale
— Enabling HTTPS](https://tailscale.com/docs/how-to/set-up-https-certificates),
[Tailscale — Secure services with TLS](https://tailscale.com/blog/tls-certs)).
Ubuntu-26.04-specific group/udev behavior should still be confirmed on
real hardware — the sourced material above is Ubuntu-on-Pi generally, not
26.04-specific.
