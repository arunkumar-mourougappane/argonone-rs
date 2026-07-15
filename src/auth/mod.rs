//! App-level auth (A§2): Argon2id password hashing, the `axum-login`
//! `AuthUser`/`AuthnBackend` implementations, and a DB-backed
//! failed-attempt throttle. Separate from OS-level service identity (the
//! Linux account the daemon process runs as) — this is purely the web
//! UI's own `users` table.

use crate::db::DbPool;
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum_login::{AuthUser as AxumAuthUser, AuthnBackend};
use serde::Deserialize;

/// Failed logins allowed before the account locks (A§2.2).
const MAX_FAILED_ATTEMPTS: i64 = 5;
/// How long an account stays locked once [`MAX_FAILED_ATTEMPTS`] is hit.
const LOCKOUT_MINUTES: i64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Operator => "operator",
            Role::Viewer => "viewer",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "admin" => Ok(Role::Admin),
            "operator" => Ok(Role::Operator),
            "viewer" => Ok(Role::Viewer),
            _ => Err(()),
        }
    }
}

/// The subset of a `users` row (A§2.3's schema sketch) auth actually
/// needs. `first_name`/`last_name`/`created_at`/`last_login_at` are
/// display-only fields for the future Users admin page (v0.5.0), not
/// used here — a dedicated DTO can select those when that page exists,
/// rather than this struct carrying fields nothing reads yet.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub must_change_pw: bool,
    pub role: String,
}

impl User {
    /// Parsed role, defaulting to the least-privileged tier if the stored
    /// value somehow doesn't match the schema's `CHECK` constraint (never
    /// trust a DB value blindly, even a constrained one).
    pub fn role(&self) -> Role {
        self.role.parse().unwrap_or(Role::Viewer)
    }
}

impl AxumAuthUser for User {
    type Id = i64;

    fn id(&self) -> i64 {
        self.id
    }

    /// Ties the session to the current password hash — changing a
    /// password (including an admin-issued reset) invalidates any
    /// existing sessions for that account, matching A§2.2's rationale.
    fn session_auth_hash(&self) -> &[u8] {
        self.password_hash.as_bytes()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct Backend {
    pool: DbPool,
}

impl Backend {
    pub fn new(pool: DbPool) -> Self {
        Backend { pool }
    }
}

impl AuthnBackend for Backend {
    type User = User;
    type Credentials = Credentials;
    type Error = AuthError;

    async fn authenticate(&self, creds: Credentials) -> Result<Option<User>, AuthError> {
        let Some(user) = sqlx::query_as::<_, User>(
            "SELECT id, username, password_hash, must_change_pw, role FROM users WHERE username = ?1",
        )
        .bind(&creds.username)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        if is_locked(&self.pool, user.id).await? {
            return Ok(None);
        }

        if verify_password(&creds.password, &user.password_hash) {
            record_success(&self.pool, user.id).await?;
            Ok(Some(user))
        } else {
            record_failure(&self.pool, user.id).await?;
            Ok(None)
        }
    }

    async fn get_user(&self, user_id: &i64) -> Result<Option<User>, AuthError> {
        Ok(sqlx::query_as::<_, User>(
            "SELECT id, username, password_hash, must_change_pw, role FROM users WHERE id = ?1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}

async fn is_locked(pool: &DbPool, user_id: i64) -> Result<bool, sqlx::Error> {
    let locked: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM users WHERE id = ?1 AND locked_until IS NOT NULL AND locked_until > datetime('now')",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(locked.is_some())
}

async fn record_success(pool: &DbPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET failed_attempts = 0, locked_until = NULL, last_login_at = datetime('now') WHERE id = ?1",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn record_failure(pool: &DbPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET failed_attempts = failed_attempts + 1,
         locked_until = CASE WHEN failed_attempts + 1 >= ?1 THEN datetime('now', ?2) ELSE locked_until END
         WHERE id = ?3",
    )
    .bind(MAX_FAILED_ATTEMPTS)
    .bind(format!("+{LOCKOUT_MINUTES} minutes"))
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Hashes a plaintext password to an Argon2id PHC string, for storing in
/// `users.password_hash`.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 hashing with a freshly generated salt cannot fail")
        .to_string()
}

fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub type AuthSession = axum_login::AuthSession<Backend>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_round_trip() {
        let hash = hash_password("correct horse battery staple");
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn verify_rejects_malformed_hash() {
        assert!(!verify_password("anything", "not-a-phc-string"));
    }

    #[test]
    fn role_round_trips_through_str() {
        for role in [Role::Admin, Role::Operator, Role::Viewer] {
            assert_eq!(role.as_str().parse::<Role>().unwrap(), role);
        }
    }

    #[test]
    fn role_ordering_reflects_privilege() {
        assert!(Role::Admin > Role::Operator);
        assert!(Role::Operator > Role::Viewer);
    }

    #[test]
    fn unknown_role_string_defaults_to_viewer() {
        let user = User {
            id: 1,
            username: "x".into(),
            password_hash: String::new(),
            must_change_pw: false,
            role: "bogus".into(),
        };
        assert_eq!(user.role(), Role::Viewer);
    }

    async fn seed_user(pool: &DbPool, username: &str, password: &str) -> i64 {
        let hash = hash_password(password);
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, 'admin') RETURNING id",
        )
        .bind(username)
        .bind(hash)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn authenticate_succeeds_with_correct_password() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        seed_user(&pool, "alice", "hunter2").await;

        let backend = Backend::new(pool);
        let user = backend
            .authenticate(Credentials {
                username: "alice".into(),
                password: "hunter2".into(),
            })
            .await
            .unwrap();
        assert!(user.is_some());
    }

    #[tokio::test]
    async fn authenticate_fails_with_wrong_password_and_increments_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let id = seed_user(&pool, "bob", "correct-password").await;

        let backend = Backend::new(pool.clone());
        let user = backend
            .authenticate(Credentials {
                username: "bob".into(),
                password: "wrong".into(),
            })
            .await
            .unwrap();
        assert!(user.is_none());

        let attempts: i64 = sqlx::query_scalar("SELECT failed_attempts FROM users WHERE id = ?1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(attempts, 1);
    }

    #[tokio::test]
    async fn account_locks_after_max_failed_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        seed_user(&pool, "carol", "correct-password").await;
        let backend = Backend::new(pool.clone());

        for _ in 0..MAX_FAILED_ATTEMPTS {
            backend
                .authenticate(Credentials {
                    username: "carol".into(),
                    password: "wrong".into(),
                })
                .await
                .unwrap();
        }

        // Even the correct password is now rejected — account is locked.
        let user = backend
            .authenticate(Credentials {
                username: "carol".into(),
                password: "correct-password".into(),
            })
            .await
            .unwrap();
        assert!(user.is_none());
    }

    #[tokio::test]
    async fn authenticate_returns_none_for_unknown_username() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect(&dir.path().join("t.db")).await.unwrap();
        let backend = Backend::new(pool);
        let user = backend
            .authenticate(Credentials {
                username: "nobody".into(),
                password: "whatever".into(),
            })
            .await
            .unwrap();
        assert!(user.is_none());
    }
}
