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
    test_router_with_board(crate::hardware::board::Board::NoCase).await
}

async fn test_router_with_board(
    board: crate::hardware::board::Board,
) -> (Router, crate::db::DbPool) {
    let dir = tempfile::tempdir().unwrap();
    // Leak the tempdir so it outlives the router (test-only, bounded by
    // process lifetime) — the pool holds an open handle to the file.
    let path = Box::leak(Box::new(dir)).path().join("t.db");
    let pool = crate::db::connect(&path).await.unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(0u8);
    let (cpu_tx, _) = tokio::sync::watch::channel(crate::config::FanCurve::default_curve());
    let (hdd_tx, _) = tokio::sync::watch::channel(crate::config::FanCurve::default_curve());
    let (units_tx, _) = tokio::sync::watch::channel(crate::config::TempUnit::Celsius);
    let (rtc_schedule_tx, _) = tokio::sync::watch::channel(crate::config::RtcSchedule::disabled());
    let (oled_config_tx, _) =
        tokio::sync::watch::channel(crate::config::OledConfig::default_config());
    let (_oled_screen_tx, oled_screen_rx) =
        tokio::sync::watch::channel(None::<crate::oled::Screen>);
    let router = build_router(
        pool.clone(),
        board,
        rx,
        cpu_tx,
        hdd_tx,
        units_tx,
        rtc_schedule_tx,
        oled_config_tx,
        oled_screen_rx,
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

    // Seed a viewer directly, bypassing the (now real, v0.5.0) admin UI —
    // this test is specifically about the API being unreachable to a
    // non-admin, not about the creation flow itself.
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

/// `GET /api/settings/units` was missing entirely (only `PUT` was wired)
/// despite being documented viewer+ in the API contract (W§2.5) — a
/// viewer had no way to read the setting without hitting the broader
/// `/api/status` payload.
#[tokio::test]
async fn viewer_can_read_units_and_sees_it_change_after_a_put() {
    let (router, pool) = test_router().await;
    let viewer_cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let get_body = |resp: axum::response::Response| async {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };

    let before = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/settings/units")
                .header("cookie", &viewer_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    assert_eq!(get_body(before).await["unit"], "C");

    sqlx::query("INSERT INTO users (username, password_hash, role) VALUES ('op1', ?1, 'operator')")
        .bind(crate::auth::hash_password("password12345"))
        .execute(&pool)
        .await
        .unwrap();
    let op_login = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/login",
            "username=op1&password=password12345",
            None,
        ))
        .await
        .unwrap();
    let op_cookie = extract_set_cookie(&op_login);
    router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/settings/units",
            r#"{"unit":"F"}"#,
            &op_cookie,
        ))
        .await
        .unwrap();

    let after = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/settings/units")
                .header("cookie", &viewer_cookie)
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

/// `fan_storage_and_system_pages_render_for_logged_in_users` runs against
/// `Board::NoCase`, which skips the whole `{% if is_eon %}` Power & RTC
/// card — its Jinja never gets exercised there. Cover it explicitly, for
/// both a viewer (read-only) and an operator (with the add-schedule
/// controls), since `render()` swallows template errors into a 200 with
/// an error-string body rather than a non-200 status.
#[tokio::test]
async fn system_page_renders_rtc_card_on_eon_board() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;

    for (username, role) in [("viewer1", "viewer"), ("op1", "operator")] {
        let cookie = seed_and_login(&router, &pool, username, role).await;
        let resp = router
            .clone()
            .oneshot(empty_request("GET", "/system", &cookie))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            !html.contains("internal error rendering"),
            "system.html failed to render for {role}: {html}"
        );
        assert!(
            html.contains("Power &amp; RTC"),
            "{role} should see the RTC card"
        );
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

fn empty_request(method: &str, uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn non_admin_cannot_manage_users() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "operator1", "operator").await;

    let list = router
        .clone()
        .oneshot(empty_request("GET", "/api/users", &cookie))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::FORBIDDEN);

    let create = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/users",
            r#"{"username":"x","role":"viewer"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::FORBIDDEN);

    let update_role = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/users/1/role",
            r#"{"role":"admin"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(update_role.status(), StatusCode::FORBIDDEN);

    let delete = router
        .clone()
        .oneshot(empty_request("DELETE", "/api/users/1", &cookie))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::FORBIDDEN);

    let page = router
        .clone()
        .oneshot(empty_request("GET", "/users", &cookie))
        .await
        .unwrap();
    // Not admin -> redirected away from the page entirely, not a bare 403.
    assert_eq!(page.status(), StatusCode::SEE_OTHER);
    assert_eq!(page.headers().get("location").unwrap(), "/");
}

#[tokio::test]
async fn admin_can_create_list_and_delete_a_user() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    let page = router
        .clone()
        .oneshot(empty_request("GET", "/users", &cookie))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page_body = page.into_body().collect().await.unwrap().to_bytes();
    let page_html = String::from_utf8_lossy(&page_body);
    assert!(
        !page_html.contains("internal error rendering"),
        "users.html failed to render: {page_html}"
    );
    assert!(page_html.contains("admin1"));

    let create = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/users",
            r#"{"username":"newop","first_name":"New","last_name":"Op","role":"operator"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let body = create.into_body().collect().await.unwrap().to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let new_id = created["id"].as_i64().unwrap();
    assert!(created["temporary_password"].as_str().unwrap().len() >= 8);

    let list = router
        .clone()
        .oneshot(empty_request("GET", "/api/users", &cookie))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = list.into_body().collect().await.unwrap().to_bytes();
    let users: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(users.as_array().unwrap().iter().any(|u| u["id"] == new_id));

    let update_role = router
        .clone()
        .oneshot(json_request(
            "PUT",
            &format!("/api/users/{new_id}/role"),
            r#"{"role":"admin"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(update_role.status(), StatusCode::OK);

    let delete = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/users/{new_id}"),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn creating_a_duplicate_username_is_rejected_with_422() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/users",
            r#"{"username":"dupe","role":"viewer"}"#,
            &cookie,
        ))
        .await
        .unwrap();

    let second = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/users",
            r#"{"username":"dupe","role":"viewer"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn cannot_delete_or_demote_the_last_admin() {
    let (router, pool) = test_router().await;
    // seed_and_login's own /setup call creates the sole admin ("admin").
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;
    let admin_cookie = {
        let login = router
            .clone()
            .oneshot(form_request(
                "POST",
                "/login",
                "username=admin&password=correcthorsebatterystaple",
                None,
            ))
            .await
            .unwrap();
        extract_set_cookie(&login)
    };
    let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let _ = cookie; // only used to seed the viewer above

    let demote = router
        .clone()
        .oneshot(json_request(
            "PUT",
            &format!("/api/users/{admin_id}/role"),
            r#"{"role":"operator"}"#,
            &admin_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(demote.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let delete = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/users/{admin_id}"),
            &admin_cookie,
        ))
        .await
        .unwrap();
    // Also blocked by the separate "can't remove your own account" guard
    // since this admin is deleting itself, but either guard firing is
    // correct here — the point is the last admin survives.
    assert_eq!(delete.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ?1")
        .bind(admin_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(still_there, 1);
}

#[tokio::test]
async fn admin_cannot_remove_own_account() {
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
    // A second admin so the "last admin" guard doesn't also fire here —
    // isolates this test to the self-delete guard specifically.
    sqlx::query("INSERT INTO users (username, password_hash, role) VALUES ('admin2', ?1, 'admin')")
        .bind(crate::auth::hash_password("password12345"))
        .execute(&pool)
        .await
        .unwrap();
    let login = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/login",
            "username=admin&password=correcthorsebatterystaple",
            None,
        ))
        .await
        .unwrap();
    let cookie = extract_set_cookie(&login);
    let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let delete = router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/users/{admin_id}"),
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn rtc_endpoints_404_on_non_eon_board() {
    let (router, pool) = test_router().await; // defaults to Board::NoCase
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let get = router
        .clone()
        .oneshot(empty_request("GET", "/api/rtc/schedule", &cookie))
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::NOT_FOUND);

    let put = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/rtc/schedule",
            r#"{"enabled":true,"entries":[]}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn viewer_cannot_write_rtc_schedule_but_can_read_it() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;
    let viewer_cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let get = router
        .clone()
        .oneshot(empty_request("GET", "/api/rtc/schedule", &viewer_cookie))
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);

    let forbidden = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/rtc/schedule",
            r#"{"enabled":true,"entries":[{"kind":"Wake","days":127,"hour":7,"minute":30}]}"#,
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
            "/api/rtc/schedule",
            r#"{"enabled":true,"entries":[{"kind":"Wake","days":127,"hour":7,"minute":30}]}"#,
            &op_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let after = router
        .clone()
        .oneshot(empty_request("GET", "/api/rtc/schedule", &viewer_cookie))
        .await
        .unwrap();
    let body = after.into_body().collect().await.unwrap().to_bytes();
    let schedule: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(schedule["enabled"], true);
    assert_eq!(schedule["entries"][0]["hour"], 7);
}

#[tokio::test]
async fn rtc_schedule_rejects_invalid_entries() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let bad_hour = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/rtc/schedule",
            r#"{"enabled":true,"entries":[{"kind":"Wake","days":127,"hour":24,"minute":0}]}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(bad_hour.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let no_days = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/rtc/schedule",
            r#"{"enabled":true,"entries":[{"kind":"Wake","days":0,"hour":7,"minute":0}]}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(no_days.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn oled_endpoints_404_on_non_eon_board() {
    let (router, pool) = test_router().await; // defaults to Board::NoCase
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    for (method, uri) in [
        ("GET", "/display"),
        ("GET", "/api/oled/config"),
        ("GET", "/api/oled/preview"),
    ] {
        let resp = router
            .clone()
            .oneshot(empty_request(method, uri, &cookie))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{method} {uri}");
    }

    let put = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/oled/config",
            r#"{"switch_duration_secs":10,"screensaver_secs":120,"screenlist":"clock ip","enabled":true}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oled_page_and_config_render_and_round_trip_on_eon_board() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;

    for (username, role) in [("viewer1", "viewer"), ("op1", "operator")] {
        let cookie = seed_and_login(&router, &pool, username, role).await;
        let resp = router
            .clone()
            .oneshot(empty_request("GET", "/display", &cookie))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            !html.contains("internal error rendering"),
            "oled.html failed to render for {role}: {html}"
        );
    }

    let viewer_cookie = seed_and_login(&router, &pool, "viewer2", "viewer").await;
    let get = router
        .clone()
        .oneshot(empty_request("GET", "/api/oled/config", &viewer_cookie))
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);

    let forbidden = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/oled/config",
            r#"{"switch_duration_secs":15,"screensaver_secs":60,"screenlist":"clock ip cpu","enabled":true}"#,
            &viewer_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let op_cookie = {
        sqlx::query(
            "INSERT INTO users (username, password_hash, role) VALUES ('op2', ?1, 'operator')",
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
                "username=op2&password=password12345",
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
            "/api/oled/config",
            r#"{"switch_duration_secs":15,"screensaver_secs":60,"screenlist":"clock ip cpu","enabled":true}"#,
            &op_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let after = router
        .clone()
        .oneshot(empty_request("GET", "/api/oled/config", &viewer_cookie))
        .await
        .unwrap();
    let body = after.into_body().collect().await.unwrap().to_bytes();
    let cfg: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(cfg["switch_duration_secs"], 15);
    assert_eq!(cfg["screenlist"], "clock ip cpu");
}

#[tokio::test]
async fn oled_config_rejects_out_of_range_timing() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let bad = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/oled/config",
            r#"{"switch_duration_secs":9999,"screensaver_secs":60,"screenlist":"clock","enabled":true}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// The test router's `oled_screen` channel starts at `None` (nothing
/// rendered yet, matching a freshly-started daemon before its first OLED
/// tick) — the preview endpoint should report that as a valid "blanked"
/// state, not an error.
#[tokio::test]
async fn oled_preview_reports_blanked_when_nothing_shown_yet() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/api/oled/preview", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let preview: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(preview["screen"], serde_json::Value::Null);
    assert_eq!(preview["bits"], serde_json::Value::Null);
    assert_eq!(preview["width"], 128);
    assert_eq!(preview["height"], 64);
}

/// Exercises the actual render path (not just the blanked-state branch):
/// a real 128x64-pixel-array response for a genuinely selected screen.
#[tokio::test]
async fn oled_preview_renders_the_currently_selected_screen() {
    let dir = tempfile::tempdir().unwrap();
    let path = Box::leak(Box::new(dir)).path().join("t.db");
    let pool = crate::db::connect(&path).await.unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(0u8);
    let (cpu_tx, _) = tokio::sync::watch::channel(crate::config::FanCurve::default_curve());
    let (hdd_tx, _) = tokio::sync::watch::channel(crate::config::FanCurve::default_curve());
    let (units_tx, _) = tokio::sync::watch::channel(crate::config::TempUnit::Celsius);
    let (rtc_schedule_tx, _) = tokio::sync::watch::channel(crate::config::RtcSchedule::disabled());
    let (oled_config_tx, _) =
        tokio::sync::watch::channel(crate::config::OledConfig::default_config());
    let (_oled_screen_tx, oled_screen_rx) =
        tokio::sync::watch::channel(Some(crate::oled::Screen::Clock));
    let router = build_router(
        pool.clone(),
        crate::hardware::board::Board::Eon,
        rx,
        cpu_tx,
        hdd_tx,
        units_tx,
        rtc_schedule_tx,
        oled_config_tx,
        oled_screen_rx,
    )
    .await;

    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;
    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/api/oled/preview", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let preview: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(preview["screen"], "clock");
    let bits = preview["bits"].as_array().unwrap();
    assert_eq!(bits.len(), 1024); // 128*64/8
    assert!(
        bits.iter().any(|b| b.as_u64().unwrap() != 0),
        "clock screen should light at least one pixel"
    );
}

#[tokio::test]
async fn locked_user_shows_locked_badge_and_admin_can_unlock() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "victim", "viewer").await;
    let victim_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'victim'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE users SET failed_attempts = 5, locked_until = datetime('now', '+15 minutes') WHERE id = ?1",
    )
    .bind(victim_id)
    .execute(&pool)
    .await
    .unwrap();

    let admin_cookie = {
        let login = router
            .clone()
            .oneshot(form_request(
                "POST",
                "/login",
                "username=admin&password=correcthorsebatterystaple",
                None,
            ))
            .await
            .unwrap();
        extract_set_cookie(&login)
    };
    let _ = cookie;

    let page = router
        .clone()
        .oneshot(empty_request("GET", "/users", &admin_cookie))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let body = page.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("badge locked"),
        "locked user should show a Locked badge"
    );
    assert!(
        html.contains("unlock-user"),
        "locked user should show an Unlock action"
    );

    let unlock = router
        .clone()
        .oneshot(empty_request(
            "POST",
            &format!("/api/users/{victim_id}/unlock"),
            &admin_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(unlock.status(), StatusCode::NO_CONTENT);

    let still_locked: i64 = sqlx::query_scalar(
        "SELECT locked_until IS NOT NULL AND locked_until > datetime('now') FROM users WHERE id = ?1",
    )
    .bind(victim_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(still_locked, 0);
}

#[tokio::test]
async fn non_admin_cannot_unlock_users() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let resp = router
        .clone()
        .oneshot(empty_request("POST", "/api/users/1/unlock", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unlock_on_missing_user_is_404() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    let resp = router
        .clone()
        .oneshot(empty_request("POST", "/api/users/999/unlock", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
