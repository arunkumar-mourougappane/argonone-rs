//! System page: mirrors `08-system-settings.html`'s "Units" and
//! "Firmware & service" cards (v0.4.0), plus the "Power & RTC" schedule
//! (v0.5.0) and "IR remote"/"HTTPS & remote access" (v0.6.0) cards.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use crate::config::{HttpsConfig, HttpsMode, TempUnit};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };

    let unit = crate::db::settings::load_units(&state.pool).await;
    let is_eon = state.board == crate::hardware::board::Board::Eon;
    let board = match state.board {
        crate::hardware::board::Board::NoCase => "No case detected",
        crate::hardware::board::Board::One => "Argon ONE",
        crate::hardware::board::Board::Eon => "Argon EON",
    };
    let rtc_schedule = if is_eon {
        Some(crate::db::settings::load_rtc_schedule(&state.pool).await)
    } else {
        None
    };
    let has_case = state.board != crate::hardware::board::Board::NoCase;
    let ir_code = crate::db::settings::load_ir_code(&state.pool)
        .await
        .map(|c| format!("{c:08X}"));
    let https_config = crate::db::settings::load_https_config(&state.pool).await;
    let cert_status = if https_config.mode == HttpsMode::Tailscale {
        crate::https::read_cert_status(&crate::https::tls_cert_dir())
    } else {
        None
    };

    let html: Html<String> = render(
        &state.env,
        "system.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "system",
            can_edit => user.role() >= Role::Operator,
            is_admin => user.role() >= Role::Admin,
            unit => match unit { TempUnit::Celsius => "C", TempUnit::Fahrenheit => "F" },
            version => env!("CARGO_PKG_VERSION"),
            board => board,
            os => crate::sysinfo::os_release_pretty_name().unwrap_or_else(|| "unknown".to_string()),
            is_eon => is_eon,
            rtc_schedule => rtc_schedule,
            has_case => has_case,
            ir_code => ir_code,
            https_mode => https_config.mode.as_str(),
            https_domain => https_config.domain,
            https_email => https_config.email,
            cert_status => cert_status,
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

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'settings.update_units', ?2)",
    )
    .bind(user.id)
    .bind(format!("{{\"unit\":\"{}\"}}", body.unit))
    .execute(&state.pool)
    .await;

    Json(UnitsResponse { unit: body.unit }).into_response()
}

#[derive(Debug, Serialize)]
pub struct IrCodeResponse {
    /// Hex string (e.g. `"20DF10EF"`), or `null` if nothing's been
    /// learned yet.
    pub code: Option<String>,
}

fn ir_code_response(code: Option<u32>) -> IrCodeResponse {
    IrCodeResponse {
        code: code.map(|c| format!("{c:08X}")),
    }
}

pub async fn get_ir(State(state): State<AppState>) -> Json<IrCodeResponse> {
    let code = crate::db::settings::load_ir_code(&state.pool).await;
    Json(ir_code_response(code))
}

/// Triggers the case MCU's own IR-learn window (v0.6.0, W§3.2) and stores
/// whatever it captured. See [`crate::hardware::FanBackend::learn_ir_code`]
/// for the unverified-hardware caveat — this endpoint's behavior is only
/// as good as that reconstruction until confirmed on real hardware.
pub async fn learn_ir(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Operator {
        return StatusCode::FORBIDDEN.into_response();
    }

    let learned = match state.fan.learn_ir_code() {
        Ok(code) => code,
        Err(e) => {
            tracing::error!(error = %e, "IR learn failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(super::users::ErrorResponse {
                    error: "IR learn failed — is a case attached?".to_string(),
                }),
            )
                .into_response();
        }
    };

    let Some(code) = learned else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(super::users::ErrorResponse {
                error: "No code captured — press the remote's power button during the listen window and try again"
                    .to_string(),
            }),
        )
            .into_response();
    };

    if let Err(e) = crate::db::settings::save_ir_code(&state.pool, code, user.id).await {
        tracing::error!(error = %e, "failed to save learned IR code");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'system.ir_learn', ?2)",
    )
    .bind(user.id)
    .bind(format!("{{\"code\":\"{code:08X}\"}}"))
    .execute(&state.pool)
    .await;

    Json(ir_code_response(Some(code))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PutHttpsRequest {
    pub mode: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HttpsConfigResponse {
    pub mode: String,
    pub domain: Option<String>,
    pub email: Option<String>,
}

fn https_config_response(config: &HttpsConfig) -> HttpsConfigResponse {
    HttpsConfigResponse {
        mode: config.mode.as_str().to_string(),
        domain: config.domain.clone(),
        email: config.email.clone(),
    }
}

pub async fn get_https(State(state): State<AppState>) -> Json<HttpsConfigResponse> {
    let config = crate::db::settings::load_https_config(&state.pool).await;
    Json(https_config_response(&config))
}

/// Admin-only, unlike the operator-level fan/units/RTC writes — this
/// changes what's reachable on the network, not just a hardware setting,
/// and (per [`crate::https::spawn_server`]'s doc comment) only takes
/// effect after the next daemon restart, which the response doesn't hide.
pub async fn put_https(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Json(body): Json<PutHttpsRequest>,
) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let Ok(mode) = HttpsMode::from_str(&body.mode) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(super::users::ErrorResponse {
                error: "mode must be off, tailscale, or acme".to_string(),
            }),
        )
            .into_response();
    };

    let domain = body.domain.filter(|d| !d.trim().is_empty());
    if matches!(mode, HttpsMode::Tailscale | HttpsMode::Acme) && domain.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(super::users::ErrorResponse {
                error: "domain is required for this mode".to_string(),
            }),
        )
            .into_response();
    }

    let config = HttpsConfig {
        mode,
        domain,
        email: body.email.filter(|e| !e.trim().is_empty()),
    };

    if let Err(e) = crate::db::settings::save_https_config(&state.pool, &config, user.id).await {
        tracing::error!(error = %e, "failed to save HTTPS config");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'settings.update_https', ?2)",
    )
    .bind(user.id)
    .bind(format!("{{\"mode\":\"{}\"}}", mode.as_str()))
    .execute(&state.pool)
    .await;

    Json(https_config_response(&config)).into_response()
}

#[derive(Debug, Serialize)]
pub struct CertStatusResponse {
    pub issuer: String,
    pub expires_at: String,
    pub days_until_expiry: i64,
}

impl From<crate::https::CertStatus> for CertStatusResponse {
    fn from(status: crate::https::CertStatus) -> Self {
        CertStatusResponse {
            issuer: status.issuer,
            expires_at: status.expires_at,
            days_until_expiry: status.days_until_expiry,
        }
    }
}

/// Manually re-runs `tailscale cert` outside the daily renewal cadence
/// (`08-system-settings.html`'s "Re-issue now" button) — admin-only, and
/// only meaningful while the *currently saved* config is `tailscale` with
/// a domain, regardless of what's selected-but-unsaved in the form.
pub async fn reissue_https_cert(
    auth_session: AuthSession,
    State(state): State<AppState>,
) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let config = crate::db::settings::load_https_config(&state.pool).await;
    let (HttpsMode::Tailscale, Some(domain)) = (config.mode, config.domain) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(super::users::ErrorResponse {
                error: "HTTPS mode must be 'tailscale' with a domain saved to re-issue".to_string(),
            }),
        )
            .into_response();
    };

    let dir = crate::https::tls_cert_dir();
    let issued = {
        let dir = dir.clone();
        let domain = domain.clone();
        tokio::task::spawn_blocking(move || crate::https::run_tailscale_cert(&domain, &dir)).await
    };
    match issued {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::error!(error = %e, "manual tailscale cert re-issue failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(super::users::ErrorResponse {
                    error: format!("tailscale cert failed: {e}"),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "tailscale cert re-issue task panicked");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'settings.https_reissue', ?2)",
    )
    .bind(user.id)
    .bind(format!("{{\"domain\":\"{domain}\"}}"))
    .execute(&state.pool)
    .await;

    match crate::https::read_cert_status(&dir) {
        Some(status) => Json(CertStatusResponse::from(status)).into_response(),
        None => {
            tracing::error!(
                "tailscale cert re-issue succeeded but the written cert couldn't be parsed back"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
