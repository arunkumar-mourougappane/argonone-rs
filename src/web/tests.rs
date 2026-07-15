//! Route-level smoke tests, driving the real router (`build_router`)
//! in-process via `tower::ServiceExt::oneshot` — no TCP listener needed.
//! Complements the manual end-to-end pass (setup/login/status/ws/
//! reset-password/lockout, all confirmed against a real running server)
//! with something that runs in CI on every change.

use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn test_router() -> (Router, crate::db::DbPool) {
    let dir = tempfile::tempdir().unwrap();
    // Leak the tempdir so it outlives the router (test-only, bounded by
    // process lifetime) — the pool holds an open handle to the file.
    let path = Box::leak(Box::new(dir)).path().join("t.db");
    let pool = crate::db::connect(&path).await.unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(0u8);
    let router = build_router(pool.clone(), crate::hardware::board::Board::NoCase, rx).await;
    (router, pool)
}

fn extract_set_cookie(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .expect("response should set a session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

fn form_request(method: &str, uri: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn root_redirects_to_setup_before_first_admin_exists() {
    let (router, _pool) = test_router().await;
    let resp = router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/setup");
}

#[tokio::test]
async fn full_setup_login_dashboard_flow() {
    let (router, _pool) = test_router().await;

    let setup_resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(setup_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(setup_resp.headers().get("location").unwrap(), "/login");

    let login_resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/login",
            "username=admin&password=correcthorsebatterystaple",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(login_resp.headers().get("location").unwrap(), "/");
    let cookie = extract_set_cookie(&login_resp);

    let dashboard_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dashboard_resp.status(), StatusCode::OK);
    let body = dashboard_resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Welcome, admin"));
}

#[tokio::test]
async fn second_setup_submission_does_not_create_a_second_admin() {
    let (router, _pool) = test_router().await;
    let body = "username=first&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple";

    let first = router
        .clone()
        .oneshot(form_request("POST", "/setup", body, None))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::SEE_OTHER);

    let second_body = "username=second&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple";
    let second = router
        .clone()
        .oneshot(form_request("POST", "/setup", second_body, None))
        .await
        .unwrap();
    // Loses the race: redirected straight to login, no second admin
    // created (A§1.1 step 5's singleton guard).
    assert_eq!(second.status(), StatusCode::SEE_OTHER);
    assert_eq!(second.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn api_status_requires_login() {
    let (router, _pool) = test_router().await;
    // Setup completed so the setup-gate doesn't mask the auth check.
    router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple",
            None,
        ))
        .await
        .unwrap();

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn non_admin_cannot_reset_passwords() {
    let (router, pool) = test_router().await;
    router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple",
            None,
        ))
        .await
        .unwrap();

    // Seed a viewer directly — no admin UI to create one yet (v0.5.0).
    let viewer_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, role) VALUES ('viewer1', ?1, 'viewer') RETURNING id",
    )
    .bind(crate::auth::hash_password("viewerpassword1"))
    .fetch_one(&pool)
    .await
    .unwrap();

    let login_resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/login",
            "username=viewer1&password=viewerpassword1",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::SEE_OTHER);
    let cookie = extract_set_cookie(&login_resp);

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/users/{viewer_id}/reset-password"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn static_assets_are_served() {
    let (router, _pool) = test_router().await;
    for path in ["/static/htmx.min.js", "/static/htmx-ext-ws.js"] {
        let resp = router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should be served");
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/javascript"
        );
    }
}
