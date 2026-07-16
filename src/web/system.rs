//! System page (v0.4.0, mirrors the `08-system-settings.html` "Units"
//! and "Firmware & service" cards only — power schedule/IR/HTTPS/danger
//! zone are later milestones per `docs/ROADMAP.md`'s v0.4.0 scope note).

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use crate::config::TempUnit;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::{Deserialize, Serialize};

fn os_release_pretty_name() -> Option<String> {
    let contents = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };

    let unit = crate::db::settings::load_units(&state.pool).await;
    let board = match state.board {
        crate::hardware::board::Board::NoCase => "No case detected",
        crate::hardware::board::Board::One => "Argon ONE",
        crate::hardware::board::Board::Eon => "Argon EON",
    };

    let html: Html<String> = render(
        &state.env,
        "system.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "system",
            can_edit => user.role() >= Role::Operator,
            unit => match unit { TempUnit::Celsius => "C", TempUnit::Fahrenheit => "F" },
            version => env!("CARGO_PKG_VERSION"),
            board => board,
            os => os_release_pretty_name().unwrap_or_else(|| "unknown".to_string()),
        },
    );
    html.into_response()
}

#[derive(Debug, Deserialize)]
pub struct PutUnitsRequest {
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct UnitsResponse {
    pub unit: String,
}

pub async fn get_units(State(state): State<AppState>) -> Json<UnitsResponse> {
    let unit = match crate::db::settings::load_units(&state.pool).await {
        TempUnit::Celsius => "C",
        TempUnit::Fahrenheit => "F",
    };
    Json(UnitsResponse {
        unit: unit.to_string(),
    })
}

pub async fn put_units(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Json(body): Json<PutUnitsRequest>,
) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Operator {
        return StatusCode::FORBIDDEN.into_response();
    }
    let unit = match body.unit.as_str() {
        "C" => TempUnit::Celsius,
        "F" => TempUnit::Fahrenheit,
        _ => return StatusCode::UNPROCESSABLE_ENTITY.into_response(),
    };

    if let Err(e) = crate::db::settings::save_units(&state.pool, unit, user.id).await {
        tracing::error!(error = %e, "failed to save units setting");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    state.units_tx.send_replace(unit);

    Json(UnitsResponse { unit: body.unit }).into_response()
}
