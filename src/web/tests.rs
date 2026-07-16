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
    let (cpu_tx, _) = tokio::sync::watch::channel(crate::config::FanCurve::default_curve());
    let (hdd_tx, _) = tokio::sync::watch::channel(crate::config::FanCurve::default_curve());
    let (units_tx, _) = tokio::sync::watch::channel(crate::config::TempUnit::Celsius);
    let router = build_router(
        pool.clone(),
        crate::hardware::board::Board::NoCase,
        rx,
        cpu_tx,
        hdd_tx,
        units_tx,
    )
    .await;
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

fn json_request(method: &str, uri: &str, body: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Seeds an admin (so setup is already complete) plus a second user with
/// `role`, and returns that second user's session cookie.
async fn seed_and_login(
    router: &Router,
    pool: &crate::db::DbPool,
    username: &str,
    role: &str,
) -> String {
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

    sqlx::query("INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, ?3)")
        .bind(username)
        .bind(crate::auth::hash_password("password12345"))
        .bind(role)
        .execute(pool)
        .await
        .unwrap();

    let login_resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/login",
            &format!("username={username}&password=password12345"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::SEE_OTHER);
    extract_set_cookie(&login_resp)
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
async fn viewer_cannot_write_fan_curve_but_can_read_it() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let get_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/fan/curve/cpu")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    let put_resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/fan/curve/cpu",
            r#"{"points":[{"temp_c":60,"fan_pct":50}]}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn operator_can_save_a_safe_fan_curve_and_it_persists() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let put_resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/fan/curve/cpu",
            r#"{"points":[{"temp_c":50,"fan_pct":30},{"temp_c":80,"fan_pct":80}]}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::OK);

    let get_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/fan/curve/cpu")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = get_resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["points"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn saving_an_unsafe_curve_is_rejected_with_422() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    // 0% fan at 90C — well past the 75C/25% safety floor.
    let put_resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/fan/curve/cpu",
            r#"{"points":[{"temp_c":90,"fan_pct":0}]}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn unknown_curve_name_is_404() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/fan/curve/bogus")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn viewer_cannot_change_units_but_operator_can() {
    let (router, pool) = test_router().await;
    let viewer_cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let forbidden = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/settings/units",
            r#"{"unit":"F"}"#,
            &viewer_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let op_cookie = {
        sqlx::query(
            "INSERT INTO users (username, password_hash, role) VALUES ('op1', ?1, 'operator')",
        )
        .bind(crate::auth::hash_password("password12345"))
        .execute(&pool)
        .await
        .unwrap();
        let login_resp = router
            .clone()
            .oneshot(form_request(
                "POST",
                "/login",
                "username=op1&password=password12345",
                None,
            ))
            .await
            .unwrap();
        extract_set_cookie(&login_resp)
    };

    let ok = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/settings/units",
            r#"{"unit":"F"}"#,
            &op_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(
        crate::db::settings::load_units(&pool).await,
        crate::config::TempUnit::Fahrenheit
    );
}

/// Regression test: switching units on the System page used to only
/// reach the OLED display — `GET /api/status` (and the WebSocket `stats`
/// message it shares a payload shape with) kept reporting `unit: "C"`
/// regardless, because the PUT handler wrote the DB but the read paths
/// never looked at the live `units_tx` watch channel.
#[tokio::test]
async fn api_status_reflects_units_after_a_put() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let get_body = |resp: axum::response::Response| async {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };

    let before = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_body(before).await["unit"], "C");

    let put = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/settings/units",
            r#"{"unit":"F"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    let after = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_body(after).await["unit"], "F");
}

#[tokio::test]
async fn fan_storage_and_system_pages_render_for_logged_in_users() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    for path in ["/fan", "/storage", "/system"] {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should render");
    }
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
