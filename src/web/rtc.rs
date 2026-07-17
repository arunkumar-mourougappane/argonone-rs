//! Power & RTC schedule (v0.5.0, a card on `/system` — mirrors
//! `08-system-settings.html#power`, not a standalone page). `GET/PUT
//! /api/rtc/schedule` per the API contract (W§2.5): `viewer+` reads,
//! `operator+` writes, 404s (not an empty state) when there's no EON RTC
//! to schedule.

use super::AppState;
use crate::auth::{AuthSession, Role};
use crate::config::{RtcEventKind, RtcSchedule, RtcScheduleEntry};
use crate::hardware::board::Board;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn get_schedule(State(state): State<AppState>) -> Response {
    if state.board != Board::Eon {
        return StatusCode::NOT_FOUND.into_response();
    }
    let schedule = crate::db::settings::load_rtc_schedule(&state.pool).await;
    Json(schedule).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PutScheduleRequest {
    pub enabled: bool,
    pub entries: Vec<RtcScheduleEntry>,
}

fn validate(entries: &[RtcScheduleEntry]) -> Option<&'static str> {
    for e in entries {
        if e.hour > 23 {
            return Some("hour must be 0-23");
        }
        if e.minute > 59 {
            return Some("minute must be 0-59");
        }
        if e.days == 0 {
            return Some("at least one day must be selected");
        }
        if e.days & !0x7f != 0 {
            return Some("days must be a 7-bit mask (bit 0 = Sunday .. bit 6 = Saturday)");
        }
    }
    None
}

pub async fn put_schedule(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Json(body): Json<PutScheduleRequest>,
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
    if let Some(error) = validate(&body.entries) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }

    let schedule = RtcSchedule {
        enabled: body.enabled,
        entries: body.entries,
    };

    if let Err(e) = crate::db::settings::save_rtc_schedule(&state.pool, &schedule, user.id).await {
        tracing::error!(error = %e, "failed to save RTC schedule");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    state.rtc_schedule_tx.send_replace(schedule.clone());

    let wake_count = schedule
        .entries
        .iter()
        .filter(|e| e.kind == RtcEventKind::Wake)
        .count();
    let sleep_count = schedule.entries.len() - wake_count;
    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'rtc_schedule.update', ?2)",
    )
    .bind(user.id)
    .bind(format!(
        "{{\"enabled\":{},\"wake_entries\":{wake_count},\"sleep_entries\":{sleep_count}}}",
        schedule.enabled
    ))
    .execute(&state.pool)
    .await;

    Json(schedule).into_response()
}
