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
        std::sync::Arc::new(crate::hardware::noop::NoopFan),
        crate::config::HttpsMode::Off,
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

/// Fetches the one-time setup token `build_router` generated (v0.6.0's
/// exposure-window guard, A§1.1) so tests can drive `/setup` the same way
/// a real installer would, rather than bypassing the check.
async fn setup_token(pool: &crate::db::DbPool) -> String {
    crate::db::settings::current_setup_token(pool)
        .await
        .expect("build_router should have generated a setup token")
}

/// Seeds an admin (so setup is already complete) plus a second user with
/// `role`, and returns that second user's session cookie. Callable more
/// than once against the same router/pool (several tests do, to add
/// multiple non-admin users) — the token is only present pre-setup, so a
/// later call with none left just skips straight to seeding the user.
async fn seed_and_login(
    router: &Router,
    pool: &crate::db::DbPool,
    username: &str,
    role: &str,
) -> String {
    if let Some(token) = crate::db::settings::current_setup_token(pool).await {
        router
            .clone()
            .oneshot(form_request(
                "POST",
                "/setup",
                &format!(
                    "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
                ),
                None,
            ))
            .await
            .unwrap();
    }

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
    let (router, pool) = test_router().await;
    let token = setup_token(&pool).await;

    let setup_resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            &format!(
                "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
            ),
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
    let dashboard_html = String::from_utf8_lossy(&body);
    assert!(dashboard_html.contains("@admin"));
    assert!(
        !dashboard_html.contains("internal error rendering"),
        "dashboard.html failed to render: {dashboard_html}"
    );
    assert!(dashboard_html.contains("sysLoadAvg"));
    assert!(dashboard_html.contains("Storage"));
    assert!(dashboard_html.contains("Signed in as"));
}

/// Sidebar structure (nav icons, brand mark, account dropdown) matching
/// the mockups — `accountToggle`/`accountMenu` is the collapsible
/// avatar+dropdown (`03-dashboard.html`'s `.account-row`/`.account-menu`
/// pattern), not the plain always-visible link this replaced.
#[tokio::test]
async fn sidebar_matches_mockup_structure() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains(">Ar</div>"), "brand mark should be 'Ar'");
    assert!(html.contains("accountToggle"));
    assert!(html.contains("accountMenu"));
    assert!(html.contains("<svg"), "nav links should carry icons");
    assert!(html.contains("class=\"divider\""));
    assert!(
        html.contains(r#"id="fan-trend""#),
        "Fan ribbon stat should reserve a fixed-width trend slot instead of variable-width ramping text"
    );
}

#[tokio::test]
async fn second_setup_submission_does_not_create_a_second_admin() {
    let (router, pool) = test_router().await;
    // Both racers present the same valid token — two browsers that both
    // loaded `/setup?token=...` before either submitted, the scenario the
    // singleton DB guard (not the token check) is meant to resolve.
    let token = setup_token(&pool).await;
    let body = format!(
        "username=first&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
    );

    let first = router
        .clone()
        .oneshot(form_request("POST", "/setup", &body, None))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::SEE_OTHER);

    let second_body = format!(
        "username=second&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
    );
    let second = router
        .clone()
        .oneshot(form_request("POST", "/setup", &second_body, None))
        .await
        .unwrap();
    // Loses the race: redirected straight to login, no second admin
    // created (A§1.1 step 5's singleton guard).
    assert_eq!(second.status(), StatusCode::SEE_OTHER);
    assert_eq!(second.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn setup_form_rejects_missing_or_wrong_token() {
    let (router, pool) = test_router().await;
    assert!(
        crate::db::settings::current_setup_token(&pool)
            .await
            .is_some()
    );

    let no_token = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_token.status(), StatusCode::OK);
    let body = no_token.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Missing or incorrect setup token"));

    let wrong_token = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/setup?token=not-the-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = wrong_token.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Missing or incorrect setup token"));
}

#[tokio::test]
async fn setup_submit_rejects_wrong_token_without_creating_admin() {
    let (router, pool) = test_router().await;

    let resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token=wrong",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Missing or incorrect setup token"));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn setup_with_correct_token_clears_it_after_completion() {
    let (router, pool) = test_router().await;
    let token = setup_token(&pool).await;

    let resp = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            &format!(
                "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
            ),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    assert!(
        crate::db::settings::current_setup_token(&pool)
            .await
            .is_none(),
        "token should be consumed once the admin account is claimed"
    );
}

#[tokio::test]
async fn api_status_requires_login() {
    let (router, pool) = test_router().await;
    // Setup completed so the setup-gate doesn't mask the auth check.
    let token = setup_token(&pool).await;
    router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            &format!(
                "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
            ),
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
    let token = setup_token(&pool).await;
    router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            &format!(
                "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
            ),
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

/// Regression test: `FanCurve::speed_for` (and so `violates_safety_floor`)
/// assumes descending-sorted points, same as `FanCurve::parse` and
/// `curve_store::load`'s `ORDER BY temp_c DESC` both guarantee — but a
/// client-submitted point order isn't sorted by construction. Submitted
/// in ascending order, `[60%->100%, 95C->0%]` would (if evaluated
/// unsorted) match `95 >= 60` first and read as "100% at 95C", passing
/// the safety check even though the curve actually commands 0% fan at
/// 95C. `put_curve` must sort before validating, not just before saving.
#[tokio::test]
async fn unsafe_curve_is_rejected_even_when_submitted_out_of_order() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let put_resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/fan/curve/cpu",
            r#"{"points":[{"temp_c":60,"fan_pct":100},{"temp_c":95,"fan_pct":0}]}"#,
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

#[tokio::test]
async fn ir_code_defaults_to_null_and_viewer_can_read_it() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/api/system/ir", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], serde_json::Value::Null);
}

#[tokio::test]
async fn viewer_cannot_trigger_ir_learn_but_operator_can() {
    let (router, pool) = test_router().await;
    let viewer_cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let forbidden = router
        .clone()
        .oneshot(empty_request(
            "POST",
            "/api/system/ir/learn",
            &viewer_cookie,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let op_cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    // The test router's fan backend is the no-op stub (no case attached),
    // so `learn_ir_code` always returns `Ok(None)` — this exercises the
    // permission gate and the "nothing captured" response path, not a
    // real hardware capture (that needs the real I2C backend, untestable
    // without hardware).
    let resp = router
        .clone()
        .oneshot(empty_request("POST", "/api/system/ir/learn", &op_cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Nothing captured -> nothing persisted.
    let after = router
        .clone()
        .oneshot(empty_request("GET", "/api/system/ir", &op_cookie))
        .await
        .unwrap();
    let body = after.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], serde_json::Value::Null);
}

#[tokio::test]
async fn https_config_defaults_to_off_and_operator_cannot_change_it() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let get = router
        .clone()
        .oneshot(empty_request("GET", "/api/system/https", &cookie))
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = get.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["mode"], "off");
    assert_eq!(json["domain"], serde_json::Value::Null);

    let put = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/system/https",
            r#"{"mode":"tailscale","domain":"rpi01.example.ts.net"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_set_and_read_back_https_config() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    let put = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/system/https",
            r#"{"mode":"tailscale","domain":"rpi01.example.ts.net"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    assert_eq!(
        crate::db::settings::load_https_config(&pool).await,
        crate::config::HttpsConfig {
            mode: crate::config::HttpsMode::Tailscale,
            domain: Some("rpi01.example.ts.net".to_string()),
            email: None,
        }
    );
}

#[tokio::test]
async fn reissue_cert_requires_admin() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let resp = router
        .clone()
        .oneshot(empty_request("POST", "/api/system/https/reissue", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reissue_cert_rejects_when_mode_is_not_tailscale() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    // Default mode is "off" — re-issue only makes sense once "tailscale"
    // with a domain is actually saved (not just selected-but-unsaved in
    // the form).
    let resp = router
        .clone()
        .oneshot(empty_request("POST", "/api/system/https/reissue", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn system_page_https_card_starts_with_save_disabled_and_no_cert_status() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/system", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains(r#"id="saveHttpsBtn" type="button" disabled"#));
    assert!(!html.contains("Certificate active"));
    assert!(html.contains("Active</span>")); // HTTP-only is the default active mode
}

#[tokio::test]
async fn https_config_rejects_tailscale_or_acme_mode_without_a_domain() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    for mode in ["tailscale", "acme"] {
        let resp = router
            .clone()
            .oneshot(json_request(
                "PUT",
                "/api/system/https",
                &format!(r#"{{"mode":"{mode}"}}"#),
                &cookie,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn https_config_rejects_unknown_mode() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    let resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/system/https",
            r#"{"mode":"bogus"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
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

/// v0.6.0's dashboard data-surface gaps (W§3.3 Tier 1) — load average and
/// swap are cheap single-snapshot reads, so `/api/status` carries them
/// too, not just the WebSocket `stats` message.
#[tokio::test]
async fn api_status_includes_load_avg_and_swap_fields() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;

    let resp = router
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
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Present in the shape either way — `null` on a box without a
    // readable `/proc` is a valid value, a missing key is not.
    assert!(json.get("load_avg_1").is_some());
    assert!(json.get("load_avg_5").is_some());
    assert!(json.get("load_avg_15").is_some());
    assert!(json.get("swap_used_pct").is_some());
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

/// Mockup-fidelity pass: fan/storage/users all gained the shared
/// `fadeUp` entrance animation (gated behind `prefers-reduced-motion`,
/// matching every mockup) — this locks in that it doesn't silently
/// regress on a future edit.
#[tokio::test]
async fn fan_storage_and_users_pages_include_entrance_animation() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    // `@keyframes fadeUp` alone isn't a strong enough check — base.html
    // defines it on every page regardless of whether a given page
    // actually uses it. Check for `animation:fadeUp` (the usage), not
    // just the shared definition every page inherits either way.
    for path in ["/fan", "/storage", "/users", "/audit", "/system"] {
        let resp = router
            .clone()
            .oneshot(empty_request("GET", path, &cookie))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should render");
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("animation:fadeUp"),
            "{path} should use the shared entrance animation, not just inherit its definition"
        );
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
        assert!(
            html.contains("IR remote"),
            "{role} should see the IR remote card on a real case"
        );
    }
}

/// Same rationale as `system_page_renders_rtc_card_on_eon_board` — the
/// dashboard's Power & RTC and Display cards are `{% if is_eon %}`-gated
/// and never exercised by the default `Board::NoCase` test router.
#[tokio::test]
async fn dashboard_renders_eon_only_cards_on_eon_board() {
    let (router, pool) = test_router_with_board(crate::hardware::board::Board::Eon).await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        !html.contains("internal error rendering"),
        "dashboard.html failed to render: {html}"
    );
    assert!(html.contains("Power &amp; RTC"));
    assert!(html.contains("Not scheduled")); // no schedule saved yet
    assert!(html.contains("oledThumbCanvas"));
}

#[tokio::test]
async fn dashboard_hides_eon_only_cards_with_no_case_attached() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(!html.contains("Power &amp; RTC"));
    assert!(!html.contains("oledThumbCanvas"));
}

#[tokio::test]
async fn system_page_hides_ir_card_with_no_case_attached() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "op1", "operator").await;
    let resp = router
        .clone()
        .oneshot(empty_request("GET", "/system", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(!html.contains("IR remote"));
}

#[tokio::test]
async fn system_page_shows_https_card_to_admin_only() {
    let (router, pool) = test_router().await;

    let admin_cookie = seed_and_login(&router, &pool, "admin1", "admin").await;
    let admin_resp = router
        .clone()
        .oneshot(empty_request("GET", "/system", &admin_cookie))
        .await
        .unwrap();
    let admin_body = admin_resp.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&admin_body).contains("HTTPS &amp; remote access"));

    let op_cookie = seed_and_login(&router, &pool, "op1", "operator").await;
    let op_resp = router
        .clone()
        .oneshot(empty_request("GET", "/system", &op_cookie))
        .await
        .unwrap();
    let op_body = op_resp.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&op_body).contains("HTTPS &amp; remote access"));
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
    let token = setup_token(&pool).await;
    router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            &format!(
                "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
            ),
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
        std::sync::Arc::new(crate::hardware::noop::NoopFan),
        crate::config::HttpsMode::Off,
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

#[tokio::test]
async fn non_admin_cannot_view_audit_log() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "operator1", "operator").await;

    let page = router
        .clone()
        .oneshot(empty_request("GET", "/audit", &cookie))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::SEE_OTHER);
    assert_eq!(page.headers().get("location").unwrap(), "/");
}

#[tokio::test]
async fn admin_can_view_and_filter_audit_log() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "admin1", "admin").await;

    // `seed_and_login`'s own user-creation isn't audited (it's a direct
    // SQL insert, not the API handler) — create one through the real
    // handler so there's a `user.create` row to find.
    let create = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/users",
            r#"{"username":"newop","role":"operator"}"#,
            &cookie,
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    let page = router
        .clone()
        .oneshot(empty_request("GET", "/audit", &cookie))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let body = page.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(
        !html.contains("internal error rendering"),
        "audit.html failed to render: {html}"
    );
    assert!(html.contains("user.create"));
    assert!(html.contains("admin1"));
    assert!(
        html.contains("action-badge user"),
        "user.create should get the 'user' badge category: {html}"
    );

    // Filtered to a prefix with no matching rows -> empty result, not an error.
    let filtered = router
        .clone()
        .oneshot(empty_request("GET", "/audit?action=fan_curve", &cookie))
        .await
        .unwrap();
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered_body = filtered.into_body().collect().await.unwrap().to_bytes();
    let filtered_html = String::from_utf8_lossy(&filtered_body);
    assert!(!filtered_html.contains("user.create"));
    assert!(filtered_html.contains("No matching audit entries"));
}

#[tokio::test]
async fn voluntary_change_password_shows_voluntary_copy_and_redirects_with_notice() {
    let (router, pool) = test_router().await;
    let cookie = seed_and_login(&router, &pool, "viewer1", "viewer").await;

    let form_resp = router
        .clone()
        .oneshot(empty_request("GET", "/account/change-password", &cookie))
        .await
        .unwrap();
    assert_eq!(form_resp.status(), StatusCode::OK);
    let body = form_resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Choose a new password for your account."));
    assert!(!html.contains("An administrator reset your password"));
    assert!(html.contains("Cancel"));

    let submit = router
        .clone()
        .oneshot(form_request(
            "POST",
            "/account/change-password",
            "password=brandnewpassword1&password_confirm=brandnewpassword1",
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(submit.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        submit.headers().get("location").unwrap(),
        "/login?notice=password_updated"
    );
}

#[tokio::test]
async fn forced_change_password_shows_forced_copy_and_no_cancel_button() {
    let (router, pool) = test_router().await;
    // Seed the user directly with `must_change_pw` already set, rather than
    // driving the real admin-reset flow — that flow is covered elsewhere
    // (`non_admin_cannot_reset_passwords`); this test only needs a logged-in
    // session that's still in the forced-change state.
    let token = setup_token(&pool).await;
    router
        .clone()
        .oneshot(form_request(
            "POST",
            "/setup",
            &format!(
                "username=admin&password=correcthorsebatterystaple&password_confirm=correcthorsebatterystaple&token={token}"
            ),
            None,
        ))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO users (username, password_hash, role, must_change_pw) VALUES ('viewer1', ?1, 'viewer', 1)",
    )
    .bind(crate::auth::hash_password("viewerpassword1"))
    .execute(&pool)
    .await
    .unwrap();

    let cookie = {
        let resp = router
            .clone()
            .oneshot(form_request(
                "POST",
                "/login",
                "username=viewer1&password=viewerpassword1",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            resp.headers().get("location").unwrap(),
            "/account/change-password"
        );
        extract_set_cookie(&resp)
    };

    let form_resp = router
        .clone()
        .oneshot(empty_request("GET", "/account/change-password", &cookie))
        .await
        .unwrap();
    assert_eq!(form_resp.status(), StatusCode::OK);
    let body = form_resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("An administrator reset your password"));
    assert!(!html.contains("Cancel"));
}

#[tokio::test]
async fn login_page_shows_password_updated_notice() {
    let (router, pool) = test_router().await;
    let _ = seed_and_login(&router, &pool, "viewer1", "viewer").await;
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login?notice=password_updated")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Password updated"));
}
