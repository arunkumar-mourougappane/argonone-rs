//! `axum` web server: the v0.3.0 "foundation" milestone — persistence,
//! auth/sessions, and a bare authenticated shell, per
//! `docs/ROADMAP.md`'s v0.3.0 entry. No feature screens yet.

mod audit;
mod dashboard;
mod fan_curve;
mod login;
mod oled;
mod rtc;
mod setup;
mod status;
mod storage;
mod system;
pub mod templates;
#[cfg(test)]
mod tests;
mod users;
mod ws;

use crate::auth::Backend;
use crate::config::{FanCurve, OledConfig, RtcSchedule, TempUnit};
use crate::db::DbPool;
use crate::oled::Screen;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
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
    /// W§2.7's live-apply channels: PUT handlers send here after their DB
    /// write commits, the control loop (`service::run`) wakes on the
    /// change and applies it without restarting.
    pub cpu_curve_tx: tokio::sync::watch::Sender<FanCurve>,
    pub hdd_curve_tx: tokio::sync::watch::Sender<FanCurve>,
    pub units_tx: tokio::sync::watch::Sender<TempUnit>,
    pub rtc_schedule_tx: tokio::sync::watch::Sender<RtcSchedule>,
    pub oled_config_tx: tokio::sync::watch::Sender<OledConfig>,
    /// Published by the service loop's OLED render tick, not a web write
    /// path — the live preview (`src/web/oled.rs`) and `/api/ws`'s
    /// `oled_screen` message both read from this.
    pub oled_screen: tokio::sync::watch::Receiver<Option<Screen>>,
    /// Shared with the fan control loop (`service::run`) — the IR remote
    /// learn/program endpoints (v0.6.0, `src/web/system.rs`) call
    /// straight through to the same backend rather than round-tripping
    /// through a watch channel, since `FanBackend` methods take `&self`.
    pub fan: Arc<dyn crate::hardware::FanBackend>,
}

#[allow(clippy::too_many_arguments)]
pub async fn build_router(
    pool: DbPool,
    board: crate::hardware::board::Board,
    fan_speed: FanSpeedRx,
    cpu_curve_tx: tokio::sync::watch::Sender<FanCurve>,
    hdd_curve_tx: tokio::sync::watch::Sender<FanCurve>,
    units_tx: tokio::sync::watch::Sender<TempUnit>,
    rtc_schedule_tx: tokio::sync::watch::Sender<RtcSchedule>,
    oled_config_tx: tokio::sync::watch::Sender<OledConfig>,
    oled_screen: tokio::sync::watch::Receiver<Option<Screen>>,
    fan: Arc<dyn crate::hardware::FanBackend>,
    https_mode: crate::config::HttpsMode,
) -> Router {
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

    if user_count == 0 {
        // A§1.1's exposure-window recommendation: `/setup` is
        // unauthenticated by definition, so until the real admin claims
        // it, print a one-time token here (and to the journal) that
        // `/setup` requires as a query param — closes "first request on
        // the LAN wins" without adding a step to the common single-user
        // case.
        let token = crate::db::settings::generate_and_store_setup_token(&pool).await;
        tracing::warn!(
            "first-run setup is open on this device and unauthenticated until claimed — visit \
             /setup?token={token} to claim the admin account now (a fresh token replaces this one \
             on every restart until setup completes)"
        );
    }

    let state = AppState {
        pool: pool.clone(),
        env: Arc::new(templates::build_env()),
        setup_complete: Arc::new(AtomicBool::new(user_count > 0)),
        board,
        fan_speed,
        cpu_curve_tx,
        hdd_curve_tx,
        units_tx,
        rtc_schedule_tx,
        oled_config_tx,
        oled_screen,
        fan,
    };

    let session_store = SqliteStore::new(pool);
    session_store
        .migrate()
        .await
        .expect("tower-sessions-sqlx-store migration failed");
    let session_layer = SessionManagerLayer::new(session_store)
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        // Tied to the active HTTPS mode (A§4.4) — a `Secure` cookie would
        // never be sent back over plain HTTP, breaking login entirely, so
        // this can't just default to `true` once HTTPS exists as an option.
        .with_secure(https_mode != crate::config::HttpsMode::Off)
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
        .route("/api/users/{id}/unlock", post(users::unlock))
        .route("/users", get(users::page))
        .route("/audit", get(audit::page))
        .route("/api/users", get(users::list).post(users::create))
        .route("/api/users/{id}", delete(users::delete))
        .route("/api/users/{id}/role", put(users::update_role))
        .route("/fan", get(fan_curve::page))
        .route(
            "/api/fan/curve/{curve}",
            get(fan_curve::get_curve).put(fan_curve::put_curve),
        )
        .route("/storage", get(storage::page))
        .route("/system", get(system::page))
        .route(
            "/api/settings/units",
            get(system::get_units).put(system::put_units),
        )
        .route("/api/system/ir", get(system::get_ir))
        .route("/api/system/ir/learn", post(system::learn_ir))
        .route(
            "/api/system/https",
            get(system::get_https).put(system::put_https),
        )
        .route(
            "/api/system/https/reissue",
            post(system::reissue_https_cert),
        )
        .route(
            "/api/rtc/schedule",
            get(rtc::get_schedule).put(rtc::put_schedule),
        )
        .route("/display", get(oled::page))
        .route(
            "/api/oled/config",
            get(oled::get_config).put(oled::put_config),
        )
        .route("/api/oled/preview", get(oled::get_preview))
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
/// 1s (not the original 2s) so the dashboard's fan/network sparklines
/// (v0.6.0) read as a smooth trend rather than a handful of visibly
/// separate line segments — still far above the cost of the cheap
/// `/proc` reads each tick does, even with several clients connected.
pub const WS_TICK_INTERVAL: Duration = Duration::from_secs(1);
