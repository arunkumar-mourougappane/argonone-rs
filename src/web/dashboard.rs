//! Bare authenticated shell (W§3.2) — the v0.3.0 milestone deliberately
//! stops here: no feature screens, just proof that auth/session routing
//! and the live WebSocket both work end to end.

use super::AppState;
use super::templates::render;
use crate::auth::AuthSession;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;

pub async fn show(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        // require_login should already have caught this, but a handler
        // shouldn't assume its middleware always ran.
        return Redirect::to("/login").into_response();
    };
    let html: Html<String> = render(
        &state.env,
        "dashboard.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
        },
    );
    html.into_response()
}
