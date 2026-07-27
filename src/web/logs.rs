//! System log viewer (v0.7.0, mirrors `12-logs.html`) — admin-only,
//! read-only. Shells out to `journalctl` on a blocking-pool thread
//! (`tokio::task::spawn_blocking`), the same pattern `sysinfo` already
//! uses for `smartctl`/`lsblk`, so a slow journal read can't stall a
//! tokio worker.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::Deserialize;

const DEFAULT_SINCE: &str = "1h";
const MAX_LINES: u32 = 1000;

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    since: Option<String>,
}

/// Maps the (untrusted) query param to a known dropdown key plus the
/// journalctl expression it corresponds to — anything unrecognized
/// falls back to the default rather than erroring, since this only
/// controls how far back a read-only log view looks.
fn resolve_since(raw: Option<&str>) -> (&'static str, &'static str) {
    let key = raw.unwrap_or(DEFAULT_SINCE);
    match crate::logs::since_expr(key) {
        Some(expr) => (key_or_default(key), expr),
        None => (
            DEFAULT_SINCE,
            crate::logs::since_expr(DEFAULT_SINCE).unwrap(),
        ),
    }
}

fn key_or_default(key: &str) -> &'static str {
    match key {
        "15m" => "15m",
        "24h" => "24h",
        _ => DEFAULT_SINCE,
    }
}

async fn read_journal(since_expr: &'static str) -> Result<Vec<crate::logs::LogLine>, String> {
    tokio::task::spawn_blocking(move || crate::logs::read_journal(since_expr, MAX_LINES))
        .await
        .unwrap_or_else(|e| Err(format!("internal error: {e}")))
}

async fn read_journal_raw(since_expr: &'static str) -> Result<String, String> {
    tokio::task::spawn_blocking(move || crate::logs::read_journal_raw(since_expr, MAX_LINES))
        .await
        .unwrap_or_else(|e| Err(format!("internal error: {e}")))
}

pub async fn page(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };
    if user.role() < Role::Admin {
        return Redirect::to("/").into_response();
    }

    let (since_key, since_expr) = resolve_since(query.since.as_deref());
    let (lines, error) = match read_journal(since_expr).await {
        Ok(lines) => (lines, None),
        Err(e) => (Vec::new(), Some(e)),
    };

    let html: Html<String> = render(
        &state.env,
        "logs.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "logs",
            since => since_key,
            lines => lines,
            error => error,
        },
    );
    html.into_response()
}

#[derive(Debug, serde::Serialize)]
pub struct LogsResponse {
    pub lines: Vec<crate::logs::LogLine>,
}

pub async fn get_logs(auth_session: AuthSession, Query(query): Query<LogsQuery>) -> Response {
    let Some(user) = auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let (_, since_expr) = resolve_since(query.since.as_deref());
    match read_journal(since_expr).await {
        Ok(lines) => axum::Json(LogsResponse { lines }).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(super::users::ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

pub async fn download(auth_session: AuthSession, Query(query): Query<LogsQuery>) -> Response {
    let Some(user) = auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let (_, since_expr) = resolve_since(query.since.as_deref());
    match read_journal_raw(since_expr).await {
        Ok(text) => (
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"argonone-rs.log\"",
                ),
            ],
            text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(super::users::ErrorResponse { error: e }),
        )
            .into_response(),
    }
}
