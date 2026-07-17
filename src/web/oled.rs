//! OLED config page + live preview (v0.5.0, mirrors `06-oled-display.html`,
//! EON-only). `GET/PUT /api/oled/config` follows the same viewer+/
//! operator+/404-on-non-EON shape as `/api/rtc/schedule`; `GET
//! /api/oled/preview` renders the currently-selected screen into an
//! in-memory framebuffer (`crate::oled::framebuffer::Framebuffer`) using
//! the same `draw_screen` function the real panel uses, so "what's on the
//! OLED right now" is an actual render, not a simulation.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use crate::config::OledConfig;
use crate::hardware::board::Board;
use crate::oled::Screen;
use crate::oled::framebuffer::Framebuffer;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };
    if state.board != Board::Eon {
        return StatusCode::NOT_FOUND.into_response();
    }

    let cfg = crate::db::settings::load_oled_config(&state.pool).await;

    let html: Html<String> = render(
        &state.env,
        "oled.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "oled",
            can_edit => user.role() >= Role::Operator,
            cfg => cfg,
            is_eon => true,
        },
    );
    html.into_response()
}

pub async fn get_config(State(state): State<AppState>) -> Response {
    if state.board != Board::Eon {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(crate::db::settings::load_oled_config(&state.pool).await).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PutConfigRequest {
    pub switch_duration_secs: u32,
    pub screensaver_secs: u32,
    pub screenlist: String,
    pub enabled: bool,
}

fn validate_config(body: &PutConfigRequest) -> Option<&'static str> {
    if body.switch_duration_secs > 60 {
        return Some("switch duration must be 0-60 seconds");
    }
    if body.screensaver_secs > 600 {
        return Some("screensaver timeout must be 0-600 seconds");
    }
    None
}

pub async fn put_config(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Json(body): Json<PutConfigRequest>,
) -> Response {
    if state.board != Board::Eon {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Operator {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(error) = validate_config(&body) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }

    let cfg = OledConfig {
        switch_duration_secs: body.switch_duration_secs,
        screensaver_secs: body.screensaver_secs,
        screenlist: body.screenlist,
        enabled: body.enabled,
    };

    if let Err(e) = crate::db::settings::save_oled_config(&state.pool, &cfg, user.id).await {
        tracing::error!(error = %e, "failed to save OLED config");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    state.oled_config_tx.send_replace(cfg.clone());

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'oled_config.update', ?2)",
    )
    .bind(user.id)
    .bind(format!(
        "{{\"enabled\":{},\"screenlist\":\"{}\"}}",
        cfg.enabled, cfg.screenlist
    ))
    .execute(&state.pool)
    .await;

    Json(cfg).into_response()
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    screen: Option<&'static str>,
    width: usize,
    height: usize,
    /// Row-major, 8 pixels/byte, MSB first — `None` when nothing's
    /// currently shown (screensaver blanked, or no screens configured).
    /// See [`Framebuffer::packed_bits`]. Plain JSON byte array rather than
    /// base64 — 1024 bytes for the whole 128×64 panel, small enough that
    /// pulling in an encoding crate for it isn't worth it.
    bits: Option<Vec<u8>>,
}

pub async fn get_preview(State(state): State<AppState>) -> Response {
    if state.board != Board::Eon {
        return StatusCode::NOT_FOUND.into_response();
    }
    let current: Option<Screen> = *state.oled_screen.borrow();
    let Some(screen) = current else {
        return Json(PreviewResponse {
            screen: None,
            width: crate::oled::framebuffer::WIDTH,
            height: crate::oled::framebuffer::HEIGHT,
            bits: None,
        })
        .into_response();
    };

    let unit = crate::db::settings::load_units(&state.pool).await;
    // Two samples needed for a real CPU% delta (crate::sysinfo::CpuUsage's
    // own contract) — a short, bounded, non-blocking (async) wait, same
    // trick `service::print_status` already uses for the same reason.
    let mut cpu = crate::sysinfo::CpuUsage::new();
    cpu.sample_percent();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let data = crate::service::build_oled_data(&mut cpu, unit);

    let mut fb = Framebuffer::new();
    if let Err(e) = crate::oled::render::draw_screen(&mut fb, screen, &data) {
        tracing::warn!(error = ?e, ?screen, "failed to render OLED preview");
    }

    Json(PreviewResponse {
        screen: Some(screen.name()),
        width: crate::oled::framebuffer::WIDTH,
        height: crate::oled::framebuffer::HEIGHT,
        bits: Some(fb.packed_bits()),
    })
    .into_response()
}
