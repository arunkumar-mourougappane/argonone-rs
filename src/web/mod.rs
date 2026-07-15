//! `axum` web server: the v0.3.0 "foundation" milestone — persistence,
//! auth/sessions, and a bare authenticated shell, per
//! `docs/ROADMAP.md`'s v0.3.0 entry. No feature screens yet.

mod dashboard;
mod login;
mod setup;
mod status;
pub mod templates;
#[cfg(test)]
mod tests;
mod users;
mod ws;

use crate::auth::Backend;
use crate::db::DbPool;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum_login::AuthManagerLayerBuilder;
use axum_login::tower_sessions::cookie::SameSite;
use axum_login::tower_sessions::{Expiry, SessionManagerLayer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower_sessions_sqlx_store::SqliteStore;

/// Latest fan speed, published by the control loop (`service::run`) so the
/// web layer can report it without owning the hardware backend itself.
pub type FanSpeedRx = tokio::sync::watch::Receiver<u8>;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub env: Arc<minijinja::Environment<'static>>,
    /// Computed once at boot from `SELECT COUNT(*) FROM users`, then
    /// flipped after the setup wizard completes — not re-queried per
    /// request (A§1.1).
    pub setup_complete: Arc<AtomicBool>,
    pub board: crate::hardware::board::Board,
    pub fan_speed: FanSpeedRx,
}

pub async fn build_router(
    pool: DbPool,
    board: crate::hardware::board::Board,
    fan_speed: FanSpeedRx,
) -> Router {
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    let state = AppState {
        pool: pool.clone(),
        env: Arc::new(templates::build_env()),
        setup_complete: Arc::new(AtomicBool::new(user_count > 0)),
        board,
        fan_speed,
    };

    let session_store = SqliteStore::new(pool);
    session_store
        .migrate()
        .await
        .expect("tower-sessions-sqlx-store migration failed");
    let session_layer = SessionManagerLayer::new(session_store)
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        // No HTTPS until v0.6.0 (A§4.4) — a `Secure` cookie would never be
        // sent back over plain HTTP, breaking login entirely.
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(14)));

    let backend = Backend::new(state.pool.clone());
    let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

    let protected = Router::new()
        .route("/", get(dashboard::show))
        .route("/api/ws", get(ws::handler))
        .route("/api/status", get(status::status))
        .route(
            "/account/change-password",
            get(login::change_password_form).post(login::change_password_submit),
        )
        .route(
            "/api/users/{id}/reset-password",
            post(users::reset_password),
        )
        .route_layer(middleware::from_fn(login::require_login));

    let public = Router::new()
        .route("/setup", get(setup::form).post(setup::submit))
        .route("/login", get(login::form).post(login::submit))
        .route("/logout", post(login::logout))
        .route("/static/htmx.min.js", get(htmx_js))
        .route("/static/htmx-ext-ws.js", get(htmx_ws_js));

    Router::new()
        .merge(protected)
        .merge(public)
        .layer(auth_layer)
        .layer(middleware::from_fn_with_state(state.clone(), setup_gate))
        .with_state(state)
}

/// Redirects every request except `/setup*`/`/static/*` to `/setup` until
/// the first admin account exists (A§1.1 step 2).
async fn setup_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.setup_complete.load(Ordering::Relaxed) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path.starts_with("/setup") || path.starts_with("/static") {
        return next.run(req).await;
    }
    Redirect::to("/setup").into_response()
}

async fn htmx_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../../assets/htmx.min.js"),
    )
}

async fn htmx_ws_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../../assets/htmx-ext-ws.js"),
    )
}

/// How often the status strip / WebSocket ticks, independent of the fan
/// control loop's own 30s poll (W§2.5 — "a status strip that ticks").
pub const WS_TICK_INTERVAL: Duration = Duration::from_secs(2);
