-- Core web-foundation schema (A§2.3). `tower-sessions-sqlx-store` owns its
-- own `tower_sessions` table via its own migration, applied separately at
-- store-construction time — not hand-rolled here.

CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    username        TEXT UNIQUE NOT NULL,
    first_name      TEXT,
    last_name       TEXT,
    password_hash   TEXT NOT NULL,
    must_change_pw  INTEGER NOT NULL DEFAULT 0,
    role            TEXT NOT NULL CHECK (role IN ('admin', 'operator', 'viewer')),
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until    TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    last_login_at   TEXT
);

CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_by  INTEGER REFERENCES users (id)
);

CREATE TABLE fan_curve_points (
    id       INTEGER PRIMARY KEY,
    curve    TEXT NOT NULL CHECK (curve IN ('cpu', 'hdd')),
    temp_c   REAL NOT NULL,
    fan_pct  INTEGER NOT NULL CHECK (fan_pct BETWEEN 0 AND 100)
);

CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER REFERENCES users (id),
    action      TEXT NOT NULL,
    detail      TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
