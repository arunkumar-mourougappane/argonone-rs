//! Notifications page + webhook settings (v0.7.0, mirrors
//! `13-notifications.html`) — admin-only throughout, including reads:
//! unlike `system.rs::get_https`'s mode/domain (not sensitive), a webhook
//! URL can itself be a bearer secret (Slack/Discord/ntfy topic URLs all
//! work this way), so this doesn't let any logged-in user read it back.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use crate::notify::{Alert, WebhookConfig};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::{Deserialize, Serialize};

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };
    if user.role() < Role::Admin {
        return Redirect::to("/").into_response();
    }

    let config = crate::db::settings::load_webhook_config(&state.pool).await;
    let last_fired_at = crate::db::settings::load_webhook_last_fired(&state.pool).await;

    let html: Html<String> = render(
        &state.env,
        "notifications.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "notifications",
            enabled => config.enabled,
            webhook_url => config.url,
            last_fired_at => last_fired_at,
        },
    );
    html.into_response()
}

#[derive(Debug, Serialize)]
pub struct WebhookConfigResponse {
    pub enabled: bool,
    pub url: Option<String>,
}

fn webhook_config_response(config: &WebhookConfig) -> WebhookConfigResponse {
    WebhookConfigResponse {
        enabled: config.enabled,
        url: config.url.clone(),
    }
}

pub async fn get_webhook(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    let config = crate::db::settings::load_webhook_config(&state.pool).await;
    Json(webhook_config_response(&config)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct PutWebhookRequest {
    pub enabled: bool,
    #[serde(default)]
    pub url: Option<String>,
}

pub async fn put_webhook(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Json(body): Json<PutWebhookRequest>,
) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let url = body.url.filter(|u| !u.trim().is_empty());
    if body.enabled && url.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(super::users::ErrorResponse {
                error: "url is required to enable webhook notifications".to_string(),
            }),
        )
            .into_response();
    }

    let config = WebhookConfig {
        enabled: body.enabled,
        url,
    };

    if let Err(e) = crate::db::settings::save_webhook_config(&state.pool, &config, user.id).await {
        tracing::error!(error = %e, "failed to save webhook config");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // Live-apply (W§2.7's pattern): the control loop picks this up on its
    // next poll without a restart, same as every other watch-channel
    // setting.
    state.webhook_config_tx.send_replace(config.clone());

    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, detail) VALUES (?1, 'settings.update_webhook', ?2)",
    )
    .bind(user.id)
    .bind(format!("{{\"enabled\":{}}}", config.enabled))
    .execute(&state.pool)
    .await;

    Json(webhook_config_response(&config)).into_response()
}

/// Sends a test notification against whatever's currently *saved*, not
/// an unsaved form value — the mockup's own JS only enables "Send test"
/// once the toggle is on and a URL is present, and this keeps the
/// backend's notion of "what would actually fire" single-sourced from
/// the DB rather than trusting a client-supplied URL to probe arbitrary
/// hosts from the server.
pub async fn test_webhook(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = &auth_session.user else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if user.role() < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let config = crate::db::settings::load_webhook_config(&state.pool).await;
    let Some(url) = config.url.filter(|_| config.enabled) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(super::users::ErrorResponse {
                error: "enable notifications and save a webhook URL first".to_string(),
            }),
        )
            .into_response();
    };

    let alert = Alert {
        source: "test",
        message: "Test notification from argonone-rs".to_string(),
    };
    match crate::notify::send(&url, &alert).await {
        Ok(()) => {
            let _ = crate::db::settings::record_webhook_fired(&state.pool).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(super::users::ErrorResponse { error: e }),
        )
            .into_response(),
    }
}
