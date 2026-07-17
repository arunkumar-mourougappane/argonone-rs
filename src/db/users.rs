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
    pub created_at: String,
    pub last_login_at: Option<String>,
}

pub async fn list_users(pool: &DbPool) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, first_name, last_name, role, must_change_pw, locked_until, created_at, last_login_at
         FROM users ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
}

/// How many admin accounts currently exist — callers use this to refuse
/// deleting/demoting the last one, since that would permanently lock
/// everyone out of user management (the CLI `admin reset-password`
/// fallback can't change a role, only a password).
pub async fn count_admins(pool: &DbPool) -> Result<i64, sqlx::Error> {
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

/// `Ok(true)` if a row was actually deleted, `Ok(false)` if `id` didn't
/// exist — callers turn that into a 404 rather than a false-success 200.
pub async fn delete_user(pool: &DbPool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM users WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_role(pool: &DbPool, id: i64, role: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE users SET role = ?1 WHERE id = ?2")
        .bind(role)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Role of a single user, for last-admin-guard checks before a
/// delete/role-change — `None` if `id` doesn't exist.
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
        assert!(delete_user(&pool, id).await.unwrap());
        assert!(!delete_user(&pool, id).await.unwrap());
    }

    #[tokio::test]
    async fn update_role_changes_stored_role() {
        let pool = test_pool().await;
        let id = create_user(&pool, "u1", None, None, "viewer", "hash")
            .await
            .unwrap();
        assert!(update_role(&pool, id, "operator").await.unwrap());
        assert_eq!(
            role_of(&pool, id).await.unwrap().as_deref(),
            Some("operator")
        );
    }

    #[tokio::test]
    async fn update_role_on_missing_user_reports_false() {
        let pool = test_pool().await;
        assert!(!update_role(&pool, 999, "admin").await.unwrap());
    }
}
