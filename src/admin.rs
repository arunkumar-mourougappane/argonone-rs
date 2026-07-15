//! `argonone-rs admin` subcommands: direct database administration for
//! whoever already has shell access to the box, bypassing the web layer
//! entirely (A§1.2 Tier 2 — the "no admin can log in" fallback).

use crate::auth::hash_password;
use rand::distr::{Alphanumeric, SampleString};

/// Resets `username`'s password to a freshly generated temporary one,
/// printed to stdout, and forces a change on next login. Exits non-zero
/// if the database can't be opened or the user doesn't exist — this is a
/// one-shot operator command, not a service that should limp along.
pub async fn reset_password(username: &str) {
    let db_path = crate::service::db_path();
    let pool = match crate::db::connect(&db_path).await {
        Ok(pool) => pool,
        Err(e) => {
            eprintln!(
                "error: failed to open database at {}: {e}",
                db_path.display()
            );
            std::process::exit(1);
        }
    };

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
