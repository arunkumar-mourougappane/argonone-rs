//! Audit log queries backing the v0.6.0 viewer (W§3.6). The table itself
//! (and every write into it) has existed since v0.5.0 — this is the first
//! reader.

use super::DbPool;
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Serialize, FromRow)]
pub struct AuditRow {
    pub id: i64,
    /// `None` when the acting user has since been deleted — `user_id` is a
    /// nullable FK with no `ON DELETE` action, so the row (and its history)
    /// outlives the account.
    pub username: Option<String>,
    pub action: String,
    pub detail: Option<String>,
    pub created_at: String,
}

pub const PAGE_SIZE: i64 = 50;

/// Distinct usernames that have ever written an audit entry, for the
/// actor filter dropdown — not just `list_users`' current roster, since a
/// deleted user's past actions should still be filterable.
pub async fn distinct_actors(pool: &DbPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT u.username FROM audit_log a
         JOIN users u ON u.id = a.user_id
         ORDER BY u.username ASC",
    )
    .fetch_all(pool)
    .await
}

/// Paginated, newest-first, optionally filtered by exact actor username
/// and/or an action prefix (e.g. `"user"` matches `user.create`,
/// `user.delete`, ...). Returns the matching page plus the total row count
/// for the pager.
pub async fn list(
    pool: &DbPool,
    actor: Option<&str>,
    action_prefix: Option<&str>,
    page: i64,
) -> Result<(Vec<AuditRow>, i64), sqlx::Error> {
    let offset = page.max(0) * PAGE_SIZE;
    let action_like = action_prefix.map(|p| format!("{p}.%"));

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log a
         LEFT JOIN users u ON u.id = a.user_id
         WHERE (?1 IS NULL OR u.username = ?1)
           AND (?2 IS NULL OR a.action LIKE ?2)",
    )
    .bind(actor)
    .bind(&action_like)
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query_as::<_, AuditRow>(
        "SELECT a.id, u.username, a.action, a.detail, a.created_at
         FROM audit_log a
         LEFT JOIN users u ON u.id = a.user_id
         WHERE (?1 IS NULL OR u.username = ?1)
           AND (?2 IS NULL OR a.action LIKE ?2)
         ORDER BY a.created_at DESC, a.id DESC
         LIMIT ?3 OFFSET ?4",
    )
    .bind(actor)
    .bind(&action_like)
    .bind(PAGE_SIZE)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((rows, total))
}
