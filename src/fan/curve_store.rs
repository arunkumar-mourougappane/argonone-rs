//! SQLite persistence for fan curves (A§2.3's `fan_curve_points` table).
//! Per the documented plan (`config::FanCurve`'s own doc comment, and
//! A§3.4's "the text-config-file source of truth is gone by design"),
//! the database is the live source of truth from here on — the plain-text
//! `/etc/argononed*.conf` files stay readable for a one-time import
//! later (v0.7.0), not as an ongoing dual-write target.

use crate::config::{CurvePoint, FanCurve};
use crate::db::DbPool;

/// `"cpu"` or `"hdd"` — matches the `fan_curve_points.curve` CHECK
/// constraint.
pub async fn load(pool: &DbPool, curve: &str) -> Result<FanCurve, sqlx::Error> {
    let rows: Vec<(f64, i64)> = sqlx::query_as(
        "SELECT temp_c, fan_pct FROM fan_curve_points WHERE curve = ?1 ORDER BY temp_c DESC",
    )
    .bind(curve)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(FanCurve::default_curve());
    }

    Ok(FanCurve(
        rows.into_iter()
            .map(|(temp_c, fan_pct)| CurvePoint {
                temp_c: temp_c as i32,
                speed_pct: fan_pct.clamp(0, 100) as u8,
            })
            .collect(),
    ))
}

/// Replaces every stored point for `curve` with `points`, atomically.
pub async fn save(pool: &DbPool, curve: &str, fan_curve: &FanCurve) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM fan_curve_points WHERE curve = ?1")
        .bind(curve)
        .execute(&mut *tx)
        .await?;
    for point in &fan_curve.0 {
        sqlx::query("INSERT INTO fan_curve_points (curve, temp_c, fan_pct) VALUES (?1, ?2, ?3)")
            .bind(curve)
            .bind(point.temp_c as f64)
            .bind(point.speed_pct as i64)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_falls_back_to_default_when_no_points_stored() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let curve = load(&pool, "cpu").await.unwrap();
        assert_eq!(curve, FanCurve::default_curve());
    }

    #[tokio::test]
    async fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let curve = FanCurve(vec![
            CurvePoint {
                temp_c: 70,
                speed_pct: 100,
            },
            CurvePoint {
                temp_c: 50,
                speed_pct: 20,
            },
        ]);
        save(&pool, "cpu", &curve).await.unwrap();
        let loaded = load(&pool, "cpu").await.unwrap();
        assert_eq!(loaded, curve);
    }

    #[tokio::test]
    async fn cpu_and_hdd_curves_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let cpu = FanCurve(vec![CurvePoint {
            temp_c: 60,
            speed_pct: 50,
        }]);
        save(&pool, "cpu", &cpu).await.unwrap();

        let hdd = load(&pool, "hdd").await.unwrap();
        assert_eq!(hdd, FanCurve::default_curve());
    }

    #[tokio::test]
    async fn save_replaces_prior_points_rather_than_appending() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let first = FanCurve(vec![CurvePoint {
            temp_c: 60,
            speed_pct: 50,
        }]);
        save(&pool, "cpu", &first).await.unwrap();

        let second = FanCurve(vec![CurvePoint {
            temp_c: 70,
            speed_pct: 80,
        }]);
        save(&pool, "cpu", &second).await.unwrap();

        let loaded = load(&pool, "cpu").await.unwrap();
        assert_eq!(loaded, second);
    }
}
