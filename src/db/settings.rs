//! DB-backed generic settings (A§2.3's `settings` key/value table).
//! `units` is the only key v0.4.0 actually reads/writes; the table is
//! generic on purpose so later settings (OLED/RTC config, once those
//! move off their config files too) reuse the same store rather than
//! each growing a dedicated table.

use super::DbPool;
use crate::config::{ConfigPaths, HttpsConfig, OledConfig, RtcSchedule, TempUnit};
use rand::distr::{Alphanumeric, SampleString};
use std::path::Path;

pub async fn load_units(pool: &DbPool) -> TempUnit {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'units'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match value.as_deref() {
        Some("F") => TempUnit::Fahrenheit,
        _ => TempUnit::Celsius,
    }
}

pub async fn save_units(pool: &DbPool, unit: TempUnit, updated_by: i64) -> Result<(), sqlx::Error> {
    let value = match unit {
        TempUnit::Celsius => "C",
        TempUnit::Fahrenheit => "F",
    };
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES ('units', ?1, datetime('now'), ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
    )
    .bind(value)
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Falls back to `/etc/argonrtc.conf` (via [`RtcSchedule::load_or_default`])
/// when nothing's been saved to the DB yet, or the stored JSON is somehow
/// corrupt — same "config file is the pre-DB default" relationship fan
/// curves have.
pub async fn load_rtc_schedule(pool: &DbPool) -> RtcSchedule {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'rtc_schedule'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match value.and_then(|json| serde_json::from_str(&json).ok()) {
        Some(schedule) => schedule,
        None => RtcSchedule::load_or_default(Path::new(ConfigPaths::RTC_SCHEDULE)),
    }
}

pub async fn save_rtc_schedule(
    pool: &DbPool,
    schedule: &RtcSchedule,
    updated_by: i64,
) -> Result<(), sqlx::Error> {
    let value = serde_json::to_string(schedule).expect("RtcSchedule always serializes");
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES ('rtc_schedule', ?1, datetime('now'), ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
    )
    .bind(value)
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Falls back to `/etc/argoneonoled.conf` (via [`OledConfig::load_or_default`])
/// when nothing's been saved to the DB yet, same relationship as the RTC
/// schedule/fan curves have to their old config files.
pub async fn load_oled_config(pool: &DbPool) -> OledConfig {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'oled_config'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match value.and_then(|json| serde_json::from_str(&json).ok()) {
        Some(cfg) => cfg,
        None => OledConfig::load_or_default(Path::new(ConfigPaths::OLED)),
    }
}

pub async fn save_oled_config(
    pool: &DbPool,
    cfg: &OledConfig,
    updated_by: i64,
) -> Result<(), sqlx::Error> {
    let value = serde_json::to_string(cfg).expect("OledConfig always serializes");
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES ('oled_config', ?1, datetime('now'), ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
    )
    .bind(value)
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Falls back to [`HttpsConfig::disabled`] (plain HTTP) when nothing's
/// been configured yet — matches every other DB-backed setting's
/// fallback-to-default pattern.
pub async fn load_https_config(pool: &DbPool) -> HttpsConfig {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'https_config'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    value
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_else(HttpsConfig::disabled)
}

pub async fn save_https_config(
    pool: &DbPool,
    config: &HttpsConfig,
    updated_by: i64,
) -> Result<(), sqlx::Error> {
    let value = serde_json::to_string(config).expect("HttpsConfig always serializes");
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES ('https_config', ?1, datetime('now'), ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
    )
    .bind(value)
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// The learned IR remote code (v0.6.0, W§3.2), stored as its hex-string
/// form. `None` when nothing's been learned yet.
pub async fn load_ir_code(pool: &DbPool) -> Option<u32> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'ir_code'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    value.and_then(|hex| u32::from_str_radix(hex.trim_start_matches("0x"), 16).ok())
}

pub async fn save_ir_code(pool: &DbPool, code: u32, updated_by: i64) -> Result<(), sqlx::Error> {
    let value = format!("{code:08X}");
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES ('ir_code', ?1, datetime('now'), ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
    )
    .bind(value)
    .bind(updated_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Generates a fresh setup-exposure token and stores it, overwriting any
/// prior value — called once at boot while `users` is still empty (A§1.1's
/// "print a one-time setup token" recommendation, closing the "first
/// request wins" window on shared networks). A new token every restart
/// means a token printed to a since-rotated log is simply stale, not a
/// standing credential.
///
/// Propagates a persistence failure rather than swallowing it — the
/// caller needs to know the token didn't actually get stored, since
/// `/setup`'s own check treats "no stored token" as "fail closed, deny
/// everything" precisely so this can't silently degrade into "no token
/// required."
pub async fn generate_and_store_setup_token(pool: &DbPool) -> Result<String, sqlx::Error> {
    let token = Alphanumeric.sample_string(&mut rand::rng(), 24);
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES ('setup_token', ?1, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(&token)
    .execute(pool)
    .await?;
    Ok(token)
}

pub async fn current_setup_token(pool: &DbPool) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = 'setup_token'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Consumes the token once the real admin account is claimed, in the same
/// transaction as that insert — a losing racer's request can no longer
/// present a still-valid token after the winner commits.
pub async fn clear_setup_token(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM settings WHERE key = 'setup_token'")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_user(pool: &DbPool) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, role) VALUES ('admin', 'x', 'admin') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn load_units_defaults_to_celsius_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        assert_eq!(load_units(&pool).await, TempUnit::Celsius);
    }

    #[tokio::test]
    async fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        save_units(&pool, TempUnit::Fahrenheit, user_id)
            .await
            .unwrap();
        assert_eq!(load_units(&pool).await, TempUnit::Fahrenheit);
    }

    #[tokio::test]
    async fn save_twice_overwrites_rather_than_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        save_units(&pool, TempUnit::Fahrenheit, user_id)
            .await
            .unwrap();
        save_units(&pool, TempUnit::Celsius, user_id).await.unwrap();
        assert_eq!(load_units(&pool).await, TempUnit::Celsius);
    }

    #[tokio::test]
    async fn load_rtc_schedule_falls_back_to_disabled_when_unset_and_no_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        assert_eq!(load_rtc_schedule(&pool).await, RtcSchedule::disabled());
    }

    #[tokio::test]
    async fn save_then_load_rtc_schedule_round_trips() {
        use crate::config::{RtcEventKind, RtcScheduleEntry};

        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        let schedule = RtcSchedule {
            enabled: true,
            entries: vec![RtcScheduleEntry {
                kind: RtcEventKind::Wake,
                days: 0b0111110,
                hour: 7,
                minute: 30,
            }],
        };
        save_rtc_schedule(&pool, &schedule, user_id).await.unwrap();
        assert_eq!(load_rtc_schedule(&pool).await, schedule);
    }

    #[tokio::test]
    async fn load_oled_config_falls_back_to_default_when_unset_and_no_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        assert_eq!(load_oled_config(&pool).await, OledConfig::default_config());
    }

    #[tokio::test]
    async fn save_then_load_oled_config_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        let cfg = OledConfig {
            switch_duration_secs: 15,
            screensaver_secs: 300,
            screenlist: "clock cpu raid".to_string(),
            enabled: false,
        };
        save_oled_config(&pool, &cfg, user_id).await.unwrap();
        assert_eq!(load_oled_config(&pool).await, cfg);
    }

    #[tokio::test]
    async fn load_ir_code_defaults_to_none_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        assert_eq!(load_ir_code(&pool).await, None);
    }

    #[tokio::test]
    async fn save_then_load_ir_code_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        save_ir_code(&pool, 0x20DF10EF, user_id).await.unwrap();
        assert_eq!(load_ir_code(&pool).await, Some(0x20DF10EF));
    }

    #[tokio::test]
    async fn save_ir_code_twice_overwrites_rather_than_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        save_ir_code(&pool, 0x1111_1111, user_id).await.unwrap();
        save_ir_code(&pool, 0x2222_2222, user_id).await.unwrap();
        assert_eq!(load_ir_code(&pool).await, Some(0x2222_2222));
    }

    #[tokio::test]
    async fn load_https_config_defaults_to_disabled_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        assert_eq!(load_https_config(&pool).await, HttpsConfig::disabled());
    }

    #[tokio::test]
    async fn save_then_load_https_config_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let user_id = seed_user(&pool).await;
        let config = HttpsConfig {
            mode: crate::config::HttpsMode::Tailscale,
            domain: Some("myhost.tailnet-name.ts.net".to_string()),
            email: None,
        };
        save_https_config(&pool, &config, user_id).await.unwrap();
        assert_eq!(load_https_config(&pool).await, config);
    }
}
