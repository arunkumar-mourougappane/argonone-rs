//! DB-backed generic settings (A§2.3's `settings` key/value table).
//! `units` is the only key v0.4.0 actually reads/writes; the table is
//! generic on purpose so later settings (OLED/RTC config, once those
//! move off their config files too) reuse the same store rather than
//! each growing a dedicated table.

use super::DbPool;
use crate::config::{ConfigPaths, RtcSchedule, TempUnit};
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
}
