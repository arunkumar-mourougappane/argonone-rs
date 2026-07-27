//! One-time import of the legacy plain-text config files into the
//! DB-backed `settings`/`fan_curve_points` tables (A§3.4) — deferred from
//! the v0.4.0/v0.5.0 migrations that made the DB the live source of truth,
//! for whoever is upgrading from a v0.1.0/v0.2.0 install or the original
//! Python daemon. Safe to run more than once: anything the DB already has
//! a value for is left alone rather than clobbered by a stale file, since
//! an operator may have already changed a setting through the web UI.

use super::{ConfigPaths, FanCurve, OledConfig, RtcSchedule, TempUnit};
use crate::db::DbPool;
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

impl ImportSummary {
    pub fn is_empty(&self) -> bool {
        self.imported.is_empty() && self.skipped.is_empty() && self.errors.is_empty()
    }
}

/// The five well-known legacy config paths this import covers, bundled
/// into one call so [`crate::admin`] doesn't need to know the mapping
/// between [`ConfigPaths`] constants and the DB keys/tables they land in.
pub async fn import_all(pool: &DbPool) -> ImportSummary {
    import_legacy_config(
        pool,
        Path::new(ConfigPaths::CPU_CURVE),
        Path::new(ConfigPaths::HDD_CURVE),
        Path::new(ConfigPaths::UNITS),
        Path::new(ConfigPaths::OLED),
        Path::new(ConfigPaths::RTC_SCHEDULE),
    )
    .await
}

/// Same as [`import_all`] but with explicit paths, so tests can point at
/// a temp directory instead of the real `/etc/argon*.conf` locations.
pub async fn import_legacy_config(
    pool: &DbPool,
    cpu_curve_path: &Path,
    hdd_curve_path: &Path,
    units_path: &Path,
    oled_path: &Path,
    rtc_path: &Path,
) -> ImportSummary {
    let mut summary = ImportSummary::default();
    import_curve(pool, "cpu", cpu_curve_path, &mut summary).await;
    import_curve(pool, "hdd", hdd_curve_path, &mut summary).await;
    import_units(pool, units_path, &mut summary).await;
    import_oled(pool, oled_path, &mut summary).await;
    import_rtc(pool, rtc_path, &mut summary).await;
    summary
}

async fn curve_has_points(pool: &DbPool, curve: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fan_curve_points WHERE curve = ?1")
        .bind(curve)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

async fn setting_exists(pool: &DbPool, key: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

/// Imported settings have no acting web-session user, unlike every other
/// `settings` writer — `updated_by` stays `NULL`, which the column already
/// allows (it's an FK with no `NOT NULL`, matching a deleted-user row's
/// existing nullability in `audit_log`).
async fn set_setting_raw(pool: &DbPool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at, updated_by) VALUES (?1, ?2, datetime('now'), NULL)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

async fn import_curve(
    pool: &DbPool,
    curve: &'static str,
    path: &Path,
    summary: &mut ImportSummary,
) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            summary.errors.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    match curve_has_points(pool, curve).await {
        Ok(true) => {
            summary
                .skipped
                .push(format!("{curve} fan curve (DB already has values)"));
            return;
        }
        Ok(false) => {}
        Err(e) => {
            summary
                .errors
                .push(format!("checking existing {curve} curve: {e}"));
            return;
        }
    }
    let parsed = match FanCurve::parse(&contents) {
        Ok(c) => c,
        Err(e) => {
            summary.errors.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    match crate::fan::curve_store::save(pool, curve, &parsed).await {
        Ok(()) => summary
            .imported
            .push(format!("{curve} fan curve ({})", path.display())),
        Err(e) => summary.errors.push(format!("saving {curve} curve: {e}")),
    }
}

async fn import_units(pool: &DbPool, path: &Path, summary: &mut ImportSummary) {
    if !path.is_file() {
        return;
    }
    match setting_exists(pool, "units").await {
        Ok(true) => {
            summary
                .skipped
                .push("units (DB already has a value)".to_string());
            return;
        }
        Ok(false) => {}
        Err(e) => {
            summary
                .errors
                .push(format!("checking existing units setting: {e}"));
            return;
        }
    }
    let value = match TempUnit::load_or_default(path) {
        TempUnit::Celsius => "C",
        TempUnit::Fahrenheit => "F",
    };
    match set_setting_raw(pool, "units", value).await {
        Ok(()) => summary.imported.push(format!("units ({})", path.display())),
        Err(e) => summary.errors.push(format!("saving units: {e}")),
    }
}

async fn import_oled(pool: &DbPool, path: &Path, summary: &mut ImportSummary) {
    if !path.is_file() {
        return;
    }
    match setting_exists(pool, "oled_config").await {
        Ok(true) => {
            summary
                .skipped
                .push("OLED config (DB already has a value)".to_string());
            return;
        }
        Ok(false) => {}
        Err(e) => {
            summary
                .errors
                .push(format!("checking existing oled_config setting: {e}"));
            return;
        }
    }
    let cfg = OledConfig::load_or_default(path);
    let value = serde_json::to_string(&cfg).expect("OledConfig always serializes");
    match set_setting_raw(pool, "oled_config", &value).await {
        Ok(()) => summary
            .imported
            .push(format!("OLED config ({})", path.display())),
        Err(e) => summary.errors.push(format!("saving oled_config: {e}")),
    }
}

async fn import_rtc(pool: &DbPool, path: &Path, summary: &mut ImportSummary) {
    if !path.is_file() {
        return;
    }
    match setting_exists(pool, "rtc_schedule").await {
        Ok(true) => {
            summary
                .skipped
                .push("RTC schedule (DB already has a value)".to_string());
            return;
        }
        Ok(false) => {}
        Err(e) => {
            summary
                .errors
                .push(format!("checking existing rtc_schedule setting: {e}"));
            return;
        }
    }
    let schedule = RtcSchedule::load_or_default(path);
    let value = serde_json::to_string(&schedule).expect("RtcSchedule always serializes");
    match set_setting_raw(pool, "rtc_schedule", &value).await {
        Ok(()) => summary
            .imported
            .push(format!("RTC schedule ({})", path.display())),
        Err(e) => summary.errors.push(format!("saving rtc_schedule: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CurvePoint;

    fn missing(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        dir.join(name)
    }

    #[tokio::test]
    async fn imports_all_five_files_when_present_and_db_empty() {
        let db_dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&db_dir.path().join("t.db"))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cpu.conf"), "65=100\n55=30\n").unwrap();
        std::fs::write(dir.path().join("hdd.conf"), "50=60\n").unwrap();
        std::fs::write(dir.path().join("units.conf"), "temperature=F\n").unwrap();
        std::fs::write(
            dir.path().join("oled.conf"),
            "switchduration=15\nscreensaver=60\nscreenlist=\"clock cpu\"\nenabled=Y\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("rtc.conf"), "enabled=Y\nwake=07:00\n").unwrap();

        let summary = import_legacy_config(
            &pool,
            &dir.path().join("cpu.conf"),
            &dir.path().join("hdd.conf"),
            &dir.path().join("units.conf"),
            &dir.path().join("oled.conf"),
            &dir.path().join("rtc.conf"),
        )
        .await;

        assert_eq!(summary.imported.len(), 5, "{summary:?}");
        assert!(summary.skipped.is_empty());
        assert!(summary.errors.is_empty());

        let cpu = crate::fan::curve_store::load(&pool, "cpu").await.unwrap();
        assert_eq!(
            cpu.0,
            vec![
                CurvePoint {
                    temp_c: 65,
                    speed_pct: 100
                },
                CurvePoint {
                    temp_c: 55,
                    speed_pct: 30
                },
            ]
        );
        assert_eq!(
            crate::db::settings::load_units(&pool).await,
            TempUnit::Fahrenheit
        );
        assert!(crate::db::settings::load_oled_config(&pool).await.enabled);
        assert!(crate::db::settings::load_rtc_schedule(&pool).await.enabled);
    }

    #[tokio::test]
    async fn missing_files_are_silently_skipped() {
        let db_dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&db_dir.path().join("t.db"))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();

        let summary = import_legacy_config(
            &pool,
            &missing(dir.path(), "cpu.conf"),
            &missing(dir.path(), "hdd.conf"),
            &missing(dir.path(), "units.conf"),
            &missing(dir.path(), "oled.conf"),
            &missing(dir.path(), "rtc.conf"),
        )
        .await;

        assert!(summary.is_empty());
    }

    #[tokio::test]
    async fn does_not_overwrite_a_value_already_saved_via_the_db() {
        let db_dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&db_dir.path().join("t.db"))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cpu.conf"), "65=100\n").unwrap();

        let existing = FanCurve(vec![CurvePoint {
            temp_c: 80,
            speed_pct: 100,
        }]);
        crate::fan::curve_store::save(&pool, "cpu", &existing)
            .await
            .unwrap();

        let summary = import_legacy_config(
            &pool,
            &dir.path().join("cpu.conf"),
            &missing(dir.path(), "hdd.conf"),
            &missing(dir.path(), "units.conf"),
            &missing(dir.path(), "oled.conf"),
            &missing(dir.path(), "rtc.conf"),
        )
        .await;

        assert!(summary.imported.is_empty());
        assert_eq!(summary.skipped.len(), 1);
        let cpu = crate::fan::curve_store::load(&pool, "cpu").await.unwrap();
        assert_eq!(cpu, existing);
    }

    #[tokio::test]
    async fn malformed_curve_file_reports_error_without_blocking_other_imports() {
        let db_dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&db_dir.path().join("t.db"))
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cpu.conf"), "not-a-line\n").unwrap();
        std::fs::write(dir.path().join("units.conf"), "temperature=F\n").unwrap();

        let summary = import_legacy_config(
            &pool,
            &dir.path().join("cpu.conf"),
            &missing(dir.path(), "hdd.conf"),
            &dir.path().join("units.conf"),
            &missing(dir.path(), "oled.conf"),
            &missing(dir.path(), "rtc.conf"),
        )
        .await;

        assert_eq!(summary.errors.len(), 1, "{summary:?}");
        assert_eq!(
            summary.imported,
            vec![format!(
                "units ({})",
                dir.path().join("units.conf").display()
            )]
        );
    }
}
