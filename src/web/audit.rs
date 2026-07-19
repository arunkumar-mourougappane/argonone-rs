//! Audit log viewer (v0.6.0, W§3.6): admin-only, read-only, paginated.
//! `audit_log` has been populated since v0.5.0 by every privileged
//! mutation — this is the first handler that reads it back.

use super::AppState;
use super::templates::render;
use crate::auth::{AuthSession, Role};
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    actor: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    page: Option<i64>,
}

/// [`crate::db::audit::AuditRow`] plus a badge category derived from the
/// action's prefix (`"user.create"` -> `"user"`), matching
/// `09-audit-log.html`'s per-category badge coloring — kept out of the DB
/// row itself since it's a presentation detail, not stored data.
#[derive(Debug, Serialize)]
struct AuditRowView {
    username: Option<String>,
    action: String,
    category: String,
    detail: Option<String>,
    created_at: String,
}

impl From<crate::db::audit::AuditRow> for AuditRowView {
    fn from(row: crate::db::audit::AuditRow) -> Self {
        let category = row
            .action
            .split_once('.')
            .map(|(prefix, _)| prefix.to_string())
            .unwrap_or_else(|| row.action.clone());
        AuditRowView {
            username: row.username,
            action: row.action,
            category,
            detail: row.detail,
            created_at: row.created_at,
        }
    }
}

pub async fn page(
    auth_session: AuthSession,
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };
    if user.role() < Role::Admin {
        return Redirect::to("/").into_response();
    }

    let page_num = query.page.unwrap_or(0).max(0);
    let actor = query.actor.as_deref().filter(|s| !s.is_empty());
    let action = query.action.as_deref().filter(|s| !s.is_empty());

    let (rows, total) = crate::db::audit::list(&state.pool, actor, action, page_num)
        .await
        .unwrap_or_default();
    let rows: Vec<AuditRowView> = rows.into_iter().map(AuditRowView::from).collect();
    let actors = crate::db::audit::distinct_actors(&state.pool)
        .await
        .unwrap_or_default();

    let page_size = crate::db::audit::PAGE_SIZE;
    let showing_from = if total == 0 {
        0
    } else {
        page_num * page_size + 1
    };
    let showing_to = (page_num * page_size + rows.len() as i64).min(total);

    let html: Html<String> = render(
        &state.env,
        "audit.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "audit",
            is_eon => state.board == crate::hardware::board::Board::Eon,
            rows => rows,
            actors => actors,
            selected_actor => actor,
            selected_action => action,
            page => page_num,
            showing_from => showing_from,
            showing_to => showing_to,
            total => total,
            has_next => showing_to < total,
            has_prev => page_num > 0,
        },
    );
    html.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(action: &str) -> crate::db::audit::AuditRow {
        crate::db::audit::AuditRow {
            id: 1,
            username: Some("admin1".to_string()),
            action: action.to_string(),
            detail: None,
            created_at: "2026-07-18 00:00:00".to_string(),
        }
    }

    #[test]
    fn category_is_the_action_prefix_before_the_dot() {
        let view = AuditRowView::from(row("user.create"));
        assert_eq!(view.category, "user");
        assert_eq!(view.action, "user.create");
    }

    #[test]
    fn category_falls_back_to_the_whole_action_when_no_dot() {
        let view = AuditRowView::from(row("no_dot_action"));
        assert_eq!(view.category, "no_dot_action");
    }
}
