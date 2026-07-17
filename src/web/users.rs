//! Users admin page (v0.5.0, mirrors `07-users-rbac.html`): full CRUD +
//! role assignment, wired to the admin-issued password reset mechanism
//! that landed in v0.3.0 (auth depended on it existing before this page
//! did). All handlers here are admin-only.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role, hash_password};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

fn generate_temp_password() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), 16)
}

async fn audit(pool: &crate::db::DbPool, actor_id: i64, action: &str, detail: String) {
    let _ = sqlx::query("INSERT INTO audit_log (user_id, action, detail) VALUES (?1, ?2, ?3)")
        .bind(actor_id)
        .bind(action)
        .bind(detail)
        .execute(pool)
        .await;
}

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };
    if user.role() < Role::Admin {
        return Redirect::to("/").into_response();
    }

    let users = crate::db::users::list_users(&state.pool)
        .await
        .unwrap_or_default();

    let html: Html<String> = render(
        &state.env,
        "users.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "users",
            current_user_id => user.id,
            users => users,
        },
    );
    html.into_response()
}

pub async fn list(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(actor) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if actor.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    match crate::db::users::list_users(&state.pool).await {
        Ok(users) => Json(users).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "users.list: db query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct CreateUserResponse {
    pub id: i64,
    pub temporary_password: String,
}

pub async fn create(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Json(body): Json<CreateUserRequest>,
) -> Response {
    let Some(actor) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if actor.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    if body.username.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "username is required".to_string(),
            }),
        )
            .into_response();
    }
    if Role::from_str(&body.role).is_err() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "role must be admin, operator, or viewer".to_string(),
            }),
        )
            .into_response();
    }

    let temp_password = generate_temp_password();
    let hash = hash_password(&temp_password);

    let result = crate::db::users::create_user(
        &state.pool,
        body.username.trim(),
        body.first_name.as_deref(),
        body.last_name.as_deref(),
        &body.role,
        &hash,
    )
    .await;

    match result {
        Ok(id) => {
            audit(
                &state.pool,
                actor.id,
                "user.create",
                format!("{{\"target_user_id\":{id},\"role\":\"{}\"}}", body.role),
            )
            .await;
            Json(CreateUserResponse {
                id,
                temporary_password: temp_password,
            })
            .into_response()
        }
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "username already exists".to_string(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "users.create: db insert failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn delete(
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
    if actor.id == id {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "cannot remove your own account".to_string(),
            }),
        )
            .into_response();
    }

    match crate::db::users::role_of(&state.pool, id).await {
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Ok(Some(role)) if role == "admin" => {
            match crate::db::users::count_admins(&state.pool).await {
                Ok(n) if n <= 1 => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(ErrorResponse {
                            error: "cannot remove the last admin account".to_string(),
                        }),
                    )
                        .into_response();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(error = %e, "users.delete: admin-count check failed");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
        Ok(Some(_)) => {}
        Err(e) => {
            tracing::error!(error = %e, "users.delete: role lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    match crate::db::users::delete_user(&state.pool, id).await {
        Ok(true) => {
            audit(
                &state.pool,
                actor.id,
                "user.delete",
                format!("{{\"target_user_id\":{id}}}"),
            )
            .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "users.delete: db delete failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct RoleResponse {
    pub role: String,
}

pub async fn update_role(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateRoleRequest>,
) -> Response {
    let Some(actor) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if actor.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    if Role::from_str(&body.role).is_err() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "role must be admin, operator, or viewer".to_string(),
            }),
        )
            .into_response();
    }

    match crate::db::users::role_of(&state.pool, id).await {
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Ok(Some(current)) if current == "admin" && body.role != "admin" => {
            match crate::db::users::count_admins(&state.pool).await {
                Ok(n) if n <= 1 => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(ErrorResponse {
                            error: "cannot demote the last admin account".to_string(),
                        }),
                    )
                        .into_response();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(error = %e, "users.update_role: admin-count check failed");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
        Ok(Some(_)) => {}
        Err(e) => {
            tracing::error!(error = %e, "users.update_role: role lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    match crate::db::users::update_role(&state.pool, id, &body.role).await {
        Ok(true) => {
            audit(
                &state.pool,
                actor.id,
                "user.update_role",
                format!("{{\"target_user_id\":{id},\"role\":\"{}\"}}", body.role),
            )
            .await;
            Json(RoleResponse {
                role: body.role.clone(),
            })
            .into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "users.update_role: db update failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

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

    let temp_password = generate_temp_password();
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
            audit(
                &state.pool,
                actor.id,
                "user.reset_password",
                format!("{{\"target_user_id\":{id}}}"),
            )
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
