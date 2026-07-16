//! SQLite persistence (A§2.3, §3): connection pool, embedded migrations,
//! and the WAL/`synchronous=NORMAL` pragma pairing that's the
//! documented-safe tradeoff between SD-card wear and torn-write resilience
//! on a device that controls its own power state (A§3.3).

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::path::Path;

pub mod settings;

pub type DbPool = SqlitePool;

/// Default path under systemd's `StateDirectory=` (A§3.2) — real disk, not
/// `/dev/shm`/`/tmp`, since users/settings/fan-curves must survive a
/// reboot unlike the Python daemon's ephemeral status files.
pub const DEFAULT_DB_PATH: &str = "/var/lib/argonone-rs/argonone.db";

/// Opens the pool (creating the file if missing) and runs embedded
/// migrations. `path` is a parameter rather than always
/// [`DEFAULT_DB_PATH`] so tests can point at a temp file.
pub async fn connect(path: &Path) -> Result<DbPool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_creates_db_and_applies_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let pool = connect(&db_path).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn connect_is_idempotent_on_existing_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        connect(&db_path).await.unwrap();
        // Reconnecting (e.g. a daemon restart) must not fail re-running
        // already-applied migrations.
        let pool = connect(&db_path).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
