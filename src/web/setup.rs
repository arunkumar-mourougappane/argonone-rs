//! First-run forced admin setup (A§1.1). Gated by [`super::setup_gate`]
//! for every other route; this module only needs to handle its own form.

use super::AppState;
use super::templates::render;
use crate::auth::hash_password;
use crate::db::settings::{clear_setup_token, current_setup_token};
use axum::Form;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Redirect};
use minijinja::context;
use serde::Deserialize;
use std::sync::atomic::Ordering;

const BAD_TOKEN_ERROR: &str = "Missing or incorrect setup token. Check `journalctl -u argonone-rs` \
    (or the console) for the token printed at boot, then open /setup?token=<token>.";

#[derive(Debug, Deserialize)]
pub struct SetupQuery {
    #[serde(default)]
    token: Option<String>,
}

pub async fn form(State(state): State<AppState>, Query(query): Query<SetupQuery>) -> Html<String> {
    let expected = current_setup_token(&state.pool).await;
    if expected.is_some() && query.token != expected {
        return render(
            &state.env,
            "setup.html",
            context! { error => BAD_TOKEN_ERROR },
        );
    }
    render(&state.env, "setup.html", context! { token => query.token })
}

#[derive(Debug, Deserialize)]
pub struct SetupForm {
    username: String,
    password: String,
    password_confirm: String,
    #[serde(default)]
    token: Option<String>,
}

pub async fn submit(
    State(state): State<AppState>,
    Form(form): Form<SetupForm>,
) -> impl IntoResponse {
    let expected = current_setup_token(&state.pool).await;
    if expected.is_some() && form.token != expected {
        return render(
            &state.env,
            "setup.html",
            context! { error => BAD_TOKEN_ERROR },
        )
        .into_response();
    }
    if form.username.trim().is_empty() {
        return render(
            &state.env,
            "setup.html",
            context! { error => "Username is required" },
        )
        .into_response();
    }
    if form.password.len() < 8 {
        return render(
            &state.env,
            "setup.html",
            context! { error => "Password must be at least 8 characters" },
        )
        .into_response();
    }
    if form.password != form.password_confirm {
        return render(
            &state.env,
            "setup.html",
            context! { error => "Passwords do not match" },
        )
        .into_response();
    }

    let hash = hash_password(&form.password);

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "setup: failed to start transaction");
            return render(
                &state.env,
                "setup.html",
                context! { error => "Internal error, try again" },
            )
            .into_response();
        }
    };

    // Singleton guard against two browsers racing to claim the first admin
    // account (A§1.1 step 5): only the first submitter's INSERT actually
    // inserts a row.
    let claim =
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('setup_complete', '1')")
            .execute(&mut *tx)
            .await;
    match claim {
        Ok(result) if result.rows_affected() == 0 => {
            // Someone else already finished setup while we were filling out
            // the form.
            let _ = tx.rollback().await;
            state.setup_complete.store(true, Ordering::Relaxed);
            return Redirect::to("/login").into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "setup: failed to claim setup_complete");
            let _ = tx.rollback().await;
            return render(
                &state.env,
                "setup.html",
                context! { error => "Internal error, try again" },
            )
            .into_response();
        }
        Ok(_) => {}
    }

    let insert_user =
        sqlx::query("INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, 'admin')")
            .bind(&form.username)
            .bind(&hash)
            .execute(&mut *tx)
            .await;

    if let Err(e) = insert_user {
        let _ = tx.rollback().await;
        tracing::warn!(error = %e, "setup: failed to insert first admin user");
        return render(
            &state.env,
            "setup.html",
            context! { error => "Could not create that account (username may already be taken)" },
        )
        .into_response();
    }

    // Consumed in the same transaction as the admin insert — a losing
    // racer's request can't present a still-valid token after this commits.
    if let Err(e) = clear_setup_token(&mut tx).await {
        tracing::warn!(error = %e, "setup: failed to clear setup token");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "setup: failed to commit");
        return render(
            &state.env,
            "setup.html",
            context! { error => "Internal error, try again" },
        )
        .into_response();
    }

    state.setup_complete.store(true, Ordering::Relaxed);
    Redirect::to("/login").into_response()
}
