//! SQLite persistence (A§2.3, §3): connection pool, embedded migrations,
//! and the WAL/`synchronous=NORMAL` pragma pairing that's the
//! documented-safe tradeoff between SD-card wear and torn-write resilience
//! on a device that controls its own power state (A§3.3).

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::path::Path;

pub mod audit;
pub mod settings;
pub mod users;

pub type DbPool = SqlitePool;

/// Default path under systemd's `StateDirectory=` (A§3.2) — real disk, not
/// `/dev/shm`/`/tmp`, since users/settings/fan-curves must survive a
/// reboot unlike the Python daemon's ephemeral status files.
pub const DEFAULT_DB_PATH: &str = "/var/lib/argonone-rs/argonone.db";

/// Returns a human-actionable message if `path`'s parent directory isn't
/// writable — most commonly a read-only root filesystem under a failed or
/// misconfigured `StateDirectory=` (A§3.5). `None` means the directory
/// looks fine, so a `connect` failure has some other cause and should
/// surface its own `sqlx::Error` unchanged rather than this diagnosis.
pub fn diagnose_unwritable_state_dir(path: &Path) -> Option<String> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty())?;

    if let Err(e) = std::fs::create_dir_all(dir) {
        return Some(format!(
            "cannot create state directory {}: {e} — if the root filesystem is \
             mounted read-only, remount it read-write (or fix systemd's \
             StateDirectory=argonone-rs provisioning) before starting the service",
            dir.display()
        ));
    }

    let probe = dir.join(".argonone-rs-write-test");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        // EROFS (30): read-only filesystem, the case this diagnostic exists
        // for; PermissionDenied is close enough in practice to give the
        // same advice rather than a bare "permission denied".
        Err(e)
            if e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(30) =>
        {
            Some(format!(
                "state directory {} is not writable ({e}) — this usually means the \
                 root filesystem is mounted read-only. argonone-rs's SQLite database \
                 is its sole source of truth (A§1-§3) and requires a writable \
                 StateDirectory; remount the filesystem read-write, or check that \
                 systemd actually provisioned StateDirectory=argonone-rs",
                dir.display()
            ))
        }
        Err(e) => Some(format!(
            "cannot write to state directory {}: {e}",
            dir.display()
        )),
    }
}

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

    #[test]
    fn diagnose_unwritable_state_dir_passes_for_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        assert!(diagnose_unwritable_state_dir(&db_path).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn diagnose_unwritable_state_dir_flags_read_only_dir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("argonone-rs");
        std::fs::create_dir(&state_dir).unwrap();
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

        let db_path = state_dir.join("argonone.db");
        let diagnosis = diagnose_unwritable_state_dir(&db_path);

        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let message = diagnosis.expect("read-only directory should be flagged");
        assert!(message.contains("not writable"));
    }
}
