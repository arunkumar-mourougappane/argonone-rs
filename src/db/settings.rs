//! DB-backed generic settings (A§2.3's `settings` key/value table).
//! `units` is the only key v0.4.0 actually reads/writes; the table is
//! generic on purpose so later settings (OLED/RTC config, once those
//! move off their config files too) reuse the same store rather than
//! each growing a dedicated table.

use super::DbPool;
use crate::config::TempUnit;

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
}
