//! `argonone-rs admin` subcommands: direct database administration for
//! whoever already has shell access to the box, bypassing the web layer
//! entirely (A§1.2 Tier 2 — the "no admin can log in" fallback).

use crate::auth::hash_password;
use crate::db::DbPool;
use rand::distr::{Alphanumeric, SampleString};
use std::path::Path;

/// Opens `path` as an argonone-rs database, exiting with a diagnosable
/// message on failure — shared by every `admin` subcommand that needs a
/// live pool, so the read-only-rootfs diagnosis (A§3.5) applies uniformly.
async fn open_db(path: &Path) -> DbPool {
    match crate::db::connect(path).await {
        Ok(pool) => pool,
        Err(e) => {
            if let Some(diagnosis) = crate::db::diagnose_unwritable_state_dir(path) {
                eprintln!("error: {diagnosis}");
            } else {
                eprintln!("error: failed to open database at {}: {e}", path.display());
            }
            std::process::exit(1);
        }
    }
}

/// Resets `username`'s password to a freshly generated temporary one,
/// printed to stdout, and forces a change on next login. Exits non-zero
/// if the database can't be opened or the user doesn't exist — this is a
/// one-shot operator command, not a service that should limp along.
pub async fn reset_password(username: &str) {
    let db_path = crate::service::db_path();
    let pool = open_db(&db_path).await;

    let temp_password = Alphanumeric.sample_string(&mut rand::rng(), 16);
    let hash = hash_password(&temp_password);

    let result = sqlx::query(
        "UPDATE users SET password_hash = ?1, must_change_pw = 1, failed_attempts = 0, locked_until = NULL WHERE username = ?2",
    )
    .bind(&hash)
    .bind(username)
    .execute(&pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => {
            eprintln!("error: no user named {username:?}");
            std::process::exit(1);
        }
        Ok(_) => {
            println!("Password reset for {username:?}.");
            println!("Temporary password: {temp_password}");
            println!("This user will be required to set a new password on next login.");
        }
        Err(e) => {
            eprintln!("error: database update failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Writes a consistent live snapshot of the running database to `output`
/// (A§3.4) — `VACUUM INTO` is SQLite's own atomic online-backup mechanism,
/// safe to run against the live WAL-mode database without stopping the
/// service. Preferred here over shelling out to the `sqlite3` CLI (the
/// research doc's other suggested route) since it needs no external
/// binary and takes `output` as a bound parameter instead of a
/// dot-command string that would need shell-style quoting.
pub async fn backup(output: &Path) {
    let db_path = crate::service::db_path();
    let pool = open_db(&db_path).await;

    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("error: cannot create {}: {e}", parent.display());
        std::process::exit(1);
    }

    match sqlx::query("VACUUM INTO ?")
        .bind(output.to_string_lossy().as_ref())
        .execute(&pool)
        .await
    {
        Ok(_) => println!("Backup written to {}.", output.display()),
        Err(e) => {
            eprintln!("error: backup failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Restores `input` (a file produced by [`backup`], or any prior
/// `argonone.db`) as the live database (A§3.4). Runs embedded migrations
/// against `input` first, so restoring a backup taken on an older release
/// still comes up schema-current. The swap onto `db_path` is a same-
/// directory rename, so it's atomic against a concurrently *reading*
/// process — this still expects the service to be stopped first, since a
/// live writer could otherwise reopen the old file handle mid-swap.
pub async fn restore(input: &Path) {
    if !input.is_file() {
        eprintln!("error: no such file: {}", input.display());
        std::process::exit(1);
    }

    let pool = crate::db::connect(input).await.unwrap_or_else(|e| {
        eprintln!(
            "error: {} is not a usable argonone-rs database: {e}",
            input.display()
        );
        std::process::exit(1);
    });
    if let Err(e) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await
    {
        eprintln!("error: failed to checkpoint {}: {e}", input.display());
        std::process::exit(1);
    }
    pool.close().await;

    let db_path = crate::service::db_path();
    if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("error: cannot create {}: {e}", parent.display());
        std::process::exit(1);
    }

    let tmp_path = db_path.with_extension("restore-tmp");
    if let Err(e) = std::fs::copy(input, &tmp_path) {
        eprintln!(
            "error: failed to stage restore at {}: {e}",
            tmp_path.display()
        );
        std::process::exit(1);
    }
    if let Err(e) = std::fs::rename(&tmp_path, &db_path) {
        let _ = std::fs::remove_file(&tmp_path);
        eprintln!("error: failed to install {}: {e}", db_path.display());
        std::process::exit(1);
    }
    // Drop stale sidecars from the file just replaced — a leftover -wal
    // could otherwise get replayed on top of the just-restored data.
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", db_path.display()));
    }

    println!("Restored {} to {}.", input.display(), db_path.display());
    println!("Stop and restart the argonone-rs service for this to take effect.");
}
