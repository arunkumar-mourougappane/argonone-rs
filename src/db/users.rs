//! User-management queries backing the Users admin page (v0.5.0, A§2.3's
//! schema). Separate from `crate::auth`'s `User` (the auth-hot-path
//! struct, deliberately minimal) — `UserRow` selects the display-only
//! columns `auth::User` skips, since only this page's list view needs them.

use super::DbPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub role: String,
    pub must_change_pw: bool,
    pub locked_until: Option<String>,
    /// Computed in SQL (`locked_until` set and still in the future) —
    /// matches `auth::is_locked`'s own definition of "currently locked",
    /// rather than the page re-deriving it from a raw timestamp string.
    pub is_locked: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

pub async fn list_users(pool: &DbPool) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, first_name, last_name, role, must_change_pw, locked_until,
                (locked_until IS NOT NULL AND locked_until > datetime('now')) AS is_locked,
                created_at, last_login_at
         FROM users ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
}

/// Single-row lookup for the dashboard's "Signed in as" card (v0.6.0) —
/// `auth::User` (the session-hot-path struct) deliberately skips
/// `first_name`/`last_name`/`last_login_at`, so this reuses `UserRow`'s
/// fuller column set instead of adding those fields to the hot path.
pub async fn get_user(pool: &DbPool, id: i64) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, first_name, last_name, role, must_change_pw, locked_until,
                (locked_until IS NOT NULL AND locked_until > datetime('now')) AS is_locked,
                created_at, last_login_at
         FROM users WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// How many admin accounts currently exist. Production code folds this
/// into the same atomic statement as the guarded delete/role-change
/// below rather than checking it as a separate step first (a prior
/// version did that as a standalone query before the write — two
/// concurrent requests could both pass the check before either write
/// committed, leaving zero admins, a real TOCTOU race under SQLite's
/// WAL mode). Kept as a `cfg(test)` helper for asserting the
/// admin-count invariant directly in tests.
#[cfg(test)]
async fn count_admins(pool: &DbPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(pool)
        .await
}

pub async fn create_user(
    pool: &DbPool,
    username: &str,
    first_name: Option<&str>,
    last_name: Option<&str>,
    role: &str,
    password_hash: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (username, first_name, last_name, role, password_hash, must_change_pw)
         VALUES (?1, ?2, ?3, ?4, ?5, 1) RETURNING id",
    )
    .bind(username)
    .bind(first_name)
    .bind(last_name)
    .bind(role)
    .bind(password_hash)
    .fetch_one(pool)
    .await
}

/// Result of a delete/role-change that's guarded against removing the
/// last admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardedOutcome {
    Applied,
    NotFound,
    /// The target is the sole remaining admin — the write was refused.
    LastAdmin,
}

/// Deletes `id` unless it's the last remaining admin. The `COUNT(*)`
/// guard is part of the same SQL statement as the `DELETE`, not a
/// separate round-trip before it — SQLite holds the writer lock for a
/// statement's full execution, so this can't interleave with a
/// concurrent request the way a check-then-act sequence could.
pub async fn delete_user(pool: &DbPool, id: i64) -> Result<GuardedOutcome, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM users WHERE id = ?1
         AND (role != 'admin' OR (SELECT COUNT(*) FROM users WHERE role = 'admin') > 1)",
    )
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() > 0 {
        return Ok(GuardedOutcome::Applied);
    }
    // The guarded statement affected nothing — find out why for a
    // precise error. This follow-up read isn't itself race-free, but it
    // only picks the error *message*; the delete above already
    // atomically decided whether the row was actually removed.
    match role_of(pool, id).await? {
        None => Ok(GuardedOutcome::NotFound),
        Some(_) => Ok(GuardedOutcome::LastAdmin),
    }
}

/// Same atomicity guarantee as [`delete_user`]: refuses to demote the
/// last admin, guard and write in one statement.
pub async fn update_role(
    pool: &DbPool,
    id: i64,
    role: &str,
) -> Result<GuardedOutcome, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE users SET role = ?1 WHERE id = ?2
         AND (role != 'admin' OR ?1 = 'admin' OR (SELECT COUNT(*) FROM users WHERE role = 'admin') > 1)",
    )
    .bind(role)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() > 0 {
        return Ok(GuardedOutcome::Applied);
    }
    match role_of(pool, id).await? {
        None => Ok(GuardedOutcome::NotFound),
        Some(current) if current == role => Ok(GuardedOutcome::Applied), // already that role, no-op
        Some(_) => Ok(GuardedOutcome::LastAdmin),
    }
}

/// Clears a failed-login lockout without rotating the password — a
/// lighter-weight action than `reset_password` (`src/web/users.rs`) for
/// when an account just needs unsticking, not a fresh credential.
/// `Ok(true)` if `id` existed.
pub async fn unlock_user(pool: &DbPool, id: i64) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE id = ?1")
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Role of a single user, for last-admin-guard error messages and
/// self-delete checks — `None` if `id` doesn't exist.
pub async fn role_of(pool: &DbPool, id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT role FROM users WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> DbPool {
        let dir = tempfile::tempdir().unwrap();
        // Leak the tempdir so it outlives the pool (test-only, bounded by
        // process lifetime) — matches web::tests's test_router() pattern.
        let path = Box::leak(Box::new(dir)).path().join("t.db");
        crate::db::connect(&path).await.unwrap()
    }

    #[tokio::test]
    async fn create_then_list_round_trips() {
        let pool = test_pool().await;
        let id = create_user(&pool, "jdoe", Some("John"), Some("Doe"), "operator", "hash")
            .await
            .unwrap();

        let users = list_users(&pool).await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, id);
        assert_eq!(users[0].username, "jdoe");
        assert_eq!(users[0].first_name.as_deref(), Some("John"));
        assert_eq!(users[0].role, "operator");
        assert!(users[0].must_change_pw);
        assert!(!users[0].is_locked);
    }

    #[tokio::test]
    async fn get_user_finds_an_existing_row_and_none_for_a_missing_id() {
        let pool = test_pool().await;
        let id = create_user(&pool, "jdoe", Some("John"), Some("Doe"), "operator", "hash")
            .await
            .unwrap();

        let found = get_user(&pool, id).await.unwrap().unwrap();
        assert_eq!(found.username, "jdoe");
        assert_eq!(found.first_name.as_deref(), Some("John"));

        assert!(get_user(&pool, id + 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn count_admins_reflects_current_roles() {
        let pool = test_pool().await;
        assert_eq!(count_admins(&pool).await.unwrap(), 0);
        create_user(&pool, "a1", None, None, "admin", "hash")
            .await
            .unwrap();
        create_user(&pool, "v1", None, None, "viewer", "hash")
            .await
            .unwrap();
        assert_eq!(count_admins(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn delete_user_reports_whether_a_row_existed() {
        let pool = test_pool().await;
        let id = create_user(&pool, "temp", None, None, "viewer", "hash")
            .await
            .unwrap();
        assert_eq!(
            delete_user(&pool, id).await.unwrap(),
            GuardedOutcome::Applied
        );
        assert_eq!(
            delete_user(&pool, id).await.unwrap(),
            GuardedOutcome::NotFound
        );
    }

    #[tokio::test]
    async fn delete_user_refuses_to_remove_the_last_admin() {
        let pool = test_pool().await;
        let id = create_user(&pool, "solo-admin", None, None, "admin", "hash")
            .await
            .unwrap();
        assert_eq!(
            delete_user(&pool, id).await.unwrap(),
            GuardedOutcome::LastAdmin
        );
        assert_eq!(count_admins(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn delete_user_allows_removing_an_admin_when_another_remains() {
        let pool = test_pool().await;
        let id1 = create_user(&pool, "a1", None, None, "admin", "hash")
            .await
            .unwrap();
        create_user(&pool, "a2", None, None, "admin", "hash")
            .await
            .unwrap();
        assert_eq!(
            delete_user(&pool, id1).await.unwrap(),
            GuardedOutcome::Applied
        );
        assert_eq!(count_admins(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn update_role_changes_stored_role() {
        let pool = test_pool().await;
        let id = create_user(&pool, "u1", None, None, "viewer", "hash")
            .await
            .unwrap();
        assert_eq!(
            update_role(&pool, id, "operator").await.unwrap(),
            GuardedOutcome::Applied
        );
        assert_eq!(
            role_of(&pool, id).await.unwrap().as_deref(),
            Some("operator")
        );
    }

    #[tokio::test]
    async fn update_role_on_missing_user_reports_not_found() {
        let pool = test_pool().await;
        assert_eq!(
            update_role(&pool, 999, "admin").await.unwrap(),
            GuardedOutcome::NotFound
        );
    }

    #[tokio::test]
    async fn update_role_refuses_to_demote_the_last_admin() {
        let pool = test_pool().await;
        let id = create_user(&pool, "solo-admin", None, None, "admin", "hash")
            .await
            .unwrap();
        assert_eq!(
            update_role(&pool, id, "operator").await.unwrap(),
            GuardedOutcome::LastAdmin
        );
        assert_eq!(role_of(&pool, id).await.unwrap().as_deref(), Some("admin"));
    }

    #[tokio::test]
    async fn update_role_allows_demoting_an_admin_when_another_remains() {
        let pool = test_pool().await;
        let id1 = create_user(&pool, "a1", None, None, "admin", "hash")
            .await
            .unwrap();
        create_user(&pool, "a2", None, None, "admin", "hash")
            .await
            .unwrap();
        assert_eq!(
            update_role(&pool, id1, "viewer").await.unwrap(),
            GuardedOutcome::Applied
        );
        assert_eq!(count_admins(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn unlock_user_clears_lockout_fields() {
        let pool = test_pool().await;
        let id = create_user(&pool, "locked", None, None, "viewer", "hash")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE users SET failed_attempts = 5, locked_until = datetime('now', '+15 minutes') WHERE id = ?1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
        let users = list_users(&pool).await.unwrap();
        assert!(users[0].is_locked);

        assert!(unlock_user(&pool, id).await.unwrap());
        let users = list_users(&pool).await.unwrap();
        assert!(!users[0].is_locked);
        assert!(users[0].locked_until.is_none());
    }

    #[tokio::test]
    async fn unlock_user_on_missing_user_reports_false() {
        let pool = test_pool().await;
        assert!(!unlock_user(&pool, 999).await.unwrap());
    }
}
