//! Authenticated landing page — the app shell (sidebar/status-strip) is
//! shared with the fan/storage/system pages via `app_shell.html`; this
//! page itself stays a light landing spot (deeper live widgets are
//! v0.5.0).

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
            active_page => "dashboard",
        },
    );
    html.into_response()
}
