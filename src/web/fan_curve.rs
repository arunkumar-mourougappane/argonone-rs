//! Fan curve editor (v0.4.0, W§2.5/§2.7/§2.8): `GET /fan` serves the
//! draggable-SVG editor page; `GET`/`PUT /api/fan/curve/{cpu,hdd}` are
//! the REST contract it (and any other client) talks to. Writes are
//! operator+ (RBAC per A§2.1 — viewers can look, not touch hardware
//! settings), validated against the server-enforced safety floor
//! independent of what the client sent, and pushed onto the live-apply
//! watch channel after the DB write commits.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use crate::config::{CurvePoint, FanCurve};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct PointDto {
    pub temp_c: i32,
    pub fan_pct: u8,
}

#[derive(Debug, Serialize)]
pub struct CurveDto {
    pub curve: &'static str,
    pub points: Vec<PointDto>,
}

fn to_dto_ascending(curve_name: &'static str, curve: &FanCurve) -> CurveDto {
    let mut points: Vec<PointDto> = curve
        .0
        .iter()
        .map(|p| PointDto {
            temp_c: p.temp_c,
            fan_pct: p.speed_pct,
        })
        .collect();
    points.sort_by_key(|p| p.temp_c);
    CurveDto {
        curve: curve_name,
        points,
    }
}

fn curve_name(curve: &str) -> Option<&'static str> {
    match curve {
        "cpu" => Some("cpu"),
        "hdd" => Some("hdd"),
        _ => None,
    }
}

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };

    let cpu = crate::fan::curve_store::load(&state.pool, "cpu")
        .await
        .unwrap_or_else(|_| FanCurve::default_curve());
    let hdd = crate::fan::curve_store::load(&state.pool, "hdd")
        .await
        .unwrap_or_else(|_| FanCurve::default_curve());
    let unit = crate::db::settings::load_units(&state.pool).await;

    let html: Html<String> = render(
        &state.env,
        "fan_curve.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "fan",
            can_edit => user.role() >= Role::Operator,
            cpu_points => to_dto_ascending("cpu", &cpu).points,
            hdd_points => to_dto_ascending("hdd", &hdd).points,
            is_eon => state.board == crate::hardware::board::Board::Eon,
            unit => match unit { crate::config::TempUnit::Celsius => "C", crate::config::TempUnit::Fahrenheit => "F" },
        },
    );
    html.into_response()
}

pub async fn get_curve(State(state): State<AppState>, Path(curve): Path<String>) -> Response {
    let Some(name) = curve_name(&curve) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let fan_curve = crate::fan::curve_store::load(&state.pool, name)
        .await
        .unwrap_or_else(|_| FanCurve::default_curve());
    Json(to_dto_ascending(name, &fan_curve)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PutCurveRequest {
    pub points: Vec<PointDto>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn put_curve(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Path(curve): Path<String>,
    Json(body): Json<PutCurveRequest>,
) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Operator {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(name) = curve_name(&curve) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if body.points.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "at least one point is required".to_string(),
            }),
        )
            .into_response();
    }

    let mut points: Vec<CurvePoint> = body
        .points
        .iter()
        .map(|p| CurvePoint {
            temp_c: p.temp_c,
            speed_pct: p.fan_pct.min(100),
        })
        .collect();
    // FanCurve::speed_for (and so violates_safety_floor) assumes
    // descending-sorted points, the same invariant FanCurve::parse
    // enforces before ever constructing one — a client-submitted point
    // order isn't guaranteed to already be sorted, and curve_store::load
    // reloads via `ORDER BY temp_c DESC`, so an unsorted curve that
    // happened to validate as safe in this order could evaluate
    // differently (and unsafely) after the next restart if this weren't
    // sorted before validating.
    points.sort_by_key(|p| std::cmp::Reverse(p.temp_c));
    let fan_curve = FanCurve(points);

    if fan_curve.violates_safety_floor() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "curve must give at least 25% fan speed at or above 75\u{b0}C".to_string(),
            }),
        )
            .into_response();
    }

    if let Err(e) = crate::fan::curve_store::save(&state.pool, name, &fan_curve).await {
        tracing::error!(error = %e, curve = name, "failed to save fan curve");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let sender = if name == "cpu" {
        &state.cpu_curve_tx
    } else {
        &state.hdd_curve_tx
    };
    sender.send_replace(fan_curve.clone());

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'fan_curve.update', ?2)",
    )
    .bind(user.id)
    .bind(format!("{{\"curve\":\"{name}\"}}"))
    .execute(&state.pool)
    .await;

    Json(to_dto_ascending(name, &fan_curve)).into_response()
}
