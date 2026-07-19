//! Login/logout and the `must_change_pw` forced-change flow (A§2.2).

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Credentials, hash_password};
use axum::Form;
use axum::extract::{Query, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use minijinja::context;
use serde::Deserialize;

/// Rejects unauthenticated requests, and — while logged in with a
/// pending forced password change — rejects everything except the
/// change-password route itself, so a `must_change_pw` account can't
/// wander the rest of the authenticated shell first.
pub async fn require_login(auth_session: AuthSession, req: Request, next: Next) -> Response {
    let Some(user) = &auth_session.user else {
        return Redirect::to("/login").into_response();
    };
    if user.must_change_pw && req.uri().path() != "/account/change-password" {
        return Redirect::to("/account/change-password").into_response();
    }
    next.run(req).await
}

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    notice: Option<String>,
}

pub async fn form(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Query(query): Query<LoginQuery>,
) -> Response {
    if auth_session.user.is_some() {
        return Redirect::to("/").into_response();
    }
    render(
        &state.env,
        "login.html",
        context! { notice => query.notice },
    )
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

pub async fn submit(
    mut auth_session: AuthSession,
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let creds = Credentials {
        username: form.username,
        password: form.password,
    };
    match auth_session.authenticate(creds).await {
        Ok(Some(user)) => {
            let must_change = user.must_change_pw;
            if auth_session.login(&user).await.is_err() {
                return render(
                    &state.env,
                    "login.html",
                    context! { error => "Internal error, try again" },
                )
                .into_response();
            }
            if must_change {
                Redirect::to("/account/change-password").into_response()
            } else {
                Redirect::to("/").into_response()
            }
        }
        Ok(None) => render(
            &state.env,
            "login.html",
            context! { error => "Invalid username or password" },
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "login: backend error");
            render(
                &state.env,
                "login.html",
                context! { error => "Internal error, try again" },
            )
            .into_response()
        }
    }
}

pub async fn logout(mut auth_session: AuthSession) -> impl IntoResponse {
    let _ = auth_session.logout().await;
    Redirect::to("/login")
}

pub async fn change_password_form(
    auth_session: AuthSession,
    State(state): State<AppState>,
) -> Response {
    let forced = auth_session.user.as_ref().is_some_and(|u| u.must_change_pw);
    render(
        &state.env,
        "change_password.html",
        context! { forced => forced },
    )
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordForm {
    password: String,
    password_confirm: String,
}

pub async fn change_password_submit(
    mut auth_session: AuthSession,
    State(state): State<AppState>,
    Form(form): Form<ChangePasswordForm>,
) -> Response {
    let Some(user) = auth_session.user.clone() else {
        return Redirect::to("/login").into_response();
    };
    let forced = user.must_change_pw;
    if form.password.len() < 8 {
        return render(
            &state.env,
            "change_password.html",
            context! { error => "Password must be at least 8 characters", forced => forced },
        )
        .into_response();
    }
    if form.password != form.password_confirm {
        return render(
            &state.env,
            "change_password.html",
            context! { error => "Passwords do not match", forced => forced },
        )
        .into_response();
    }

    let hash = hash_password(&form.password);
    let update =
        sqlx::query("UPDATE users SET password_hash = ?1, must_change_pw = 0 WHERE id = ?2")
            .bind(&hash)
            .bind(user.id)
            .execute(&state.pool)
            .await;
    if let Err(e) = update {
        tracing::error!(error = %e, "change-password: db update failed");
        return render(
            &state.env,
            "change_password.html",
            context! { error => "Internal error, try again", forced => forced },
        )
        .into_response();
    }

    // The stored hash just changed, which invalidates `session_auth_hash`
    // for this session anyway (A§2.2) — log out explicitly rather than
    // leaving the browser holding a session about to be treated as stale.
    // The login page's `notice` banner (not an `error`) explains why,
    // since otherwise this looks like an unprompted, unexplained logout.
    let _ = auth_session.logout().await;
    Redirect::to("/login?notice=password_updated").into_response()
}
