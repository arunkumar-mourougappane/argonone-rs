# Research: Portable Config Export/Import

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet.

## 1. This is not a replacement for `admin backup`/`restore`

v0.7.0 already shipped `argonone-rs admin backup --output <path>` / `admin
restore --input <path>` ([`src/admin.rs`](../src/admin.rs)) — a raw SQLite
`VACUUM INTO` snapshot. That's the right tool for disaster recovery (exact
byte-for-byte state, including users/audit log/session store) but the wrong
tool for two things it can't do:

- **Move settings between two different installs** (a spare Pi, a fresh
  reinstall) — a raw DB snapshot brings every user account, password hash,
  and session cookie secret with it, not just "the fan curve I tuned."
- **Survive a schema change across versions** — `restore` already runs
  migrations against the input file first specifically to cope with this for
  whole-DB restores, but a human reading/diffing/version-controlling a raw
  SQLite file is a non-starter; a JSON export of just the settings values
  is something you can actually put in a git repo or read in a diff.

So this is a second, complementary tool: export/import of *just* the
DB-backed settings (`src/db/settings.rs`) and fan curves
(`src/fan/curve_store.rs`), not a competing backup mechanism.

## 2. What would actually be in it

Everything `src/db/settings.rs` already has a `load_*`/`save_*` pair for,
plus the fan curve points:

```
units, rtc_schedule, oled_config, ir_code, dashboard_layout,
fan curve points (cpu + hdd), https_config (mode/domain — NOT the cert
files themselves, those are regenerated), webhook_config (url — flagged
sensitive below)
```

**Explicitly excluded**: `users` table (accounts/password hashes — this is a
settings export, not an account migration tool; the receiving install
already went through its own first-run setup), `audit_log` (a historical
record tied to the install it happened on, not portable), session store
data, and the setup-token/webhook-last-fired bookkeeping fields (internal
state, not configuration).

**Sensitive field flagged, not silently included**: `webhook_config.url` can
itself be a bearer secret — same "a webhook URL can itself be a bearer
secret" precedent [`src/web/notifications.rs`](../src/web/notifications.rs)
already established for why that page is admin-only-including-reads.
Exporting it by default into a file a user might commit to version control
or paste into a support request would undercut that same precedent. Proposed:
export it by default (so a same-machine round-trip via export→reimport just
works) but print a one-line warning to stderr when the export contains a
non-empty webhook URL, same spirit as `admin backup`'s existing "this file
contains your database" framing — not blocking, just not silent.

## 3. Interface: CLI-only, matching the existing precedent

`admin backup`/`restore`/`import-legacy-config` are **all** CLI-only — none
of them has a web UI equivalent, per README's "Admin CLI subcommands, all
direct-to-database and bypassing the web layer entirely." Config
export/import should follow the same shape:

```sh
argonone-rs admin export-settings --output <path>   # JSON, human-readable
argonone-rs admin import-settings --input <path>    # same fallback-if-empty
                                                       # idempotency as
                                                       # import-legacy-config
```

**No new mockup needed** — same reasoning as `admin backup`/`restore`
themselves, which have no mockup either despite being real, documented,
hardware-verified v0.7.0 features. A file-export/import CLI action isn't a
browsable screen.

`import-settings` reuses `import-legacy-config`'s existing idempotency model
(`src/config/import.rs`): anything the DB already has a value for is left
alone, not clobbered — same "safe to run more than once" guarantee, just
against this app's own JSON format instead of the legacy `.conf` files.

## 4. Open questions

- Should `export-settings` include a schema-version field so a future
  `import-settings` can detect and reject/warn on an incompatible export
  from a much older version, rather than silently importing stale field
  names? Given the fallback-if-empty import model already tolerates missing
  keys gracefully (same as legacy import does for absent `.conf` files),
  probably yes for the version tag, but rejecting old exports outright is
  likely unnecessary — leaning toward "warn, don't refuse."
- Should the fan curve points be namespaced separately from the flat
  `settings` key-value table's values in the JSON output, given they come
  from a different table (`fan_curve_points`, not `settings`)? Yes — the
  export format should mirror the two real source tables, not flatten them
  into one ambiguous blob.
