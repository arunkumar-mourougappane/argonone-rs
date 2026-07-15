//! Admin-issued password reset (A§1.2 Tier 1) — the *mechanism*, landing
//! in v0.3.0 since auth depends on it existing; the Users admin page
//! itself is v0.5.0. Callable today via `curl`/an HTTP client with an
//! admin session; a UI button comes later.

use super::AppState;
use crate::auth::{AuthSession, Role, hash_password};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rand::distr::{Alphanumeric, SampleString};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ResetPasswordResponse {
    /// The generated temporary password — shown once, here, since there's
    /// no email delivery to fall back on (A§1.2). The admin hands this to
    /// the affected user out of band.
    temporary_password: String,
}

pub async fn reset_password(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Some(actor) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if actor.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let temp_password = Alphanumeric.sample_string(&mut rand::rng(), 16);
    let hash = hash_password(&temp_password);

    let result = sqlx::query(
        "UPDATE users SET password_hash = ?1, must_change_pw = 1, failed_attempts = 0, locked_until = NULL WHERE id = ?2",
    )
    .bind(&hash)
    .bind(id)
    .execute(&state.pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            let _ = sqlx::query(
                "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'user.reset_password', ?2)",
            )
            .bind(actor.id)
            .bind(format!("{{\"target_user_id\":{id}}}"))
            .execute(&state.pool)
            .await;
            Json(ResetPasswordResponse {
                temporary_password: temp_password,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "reset-password: db update failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
