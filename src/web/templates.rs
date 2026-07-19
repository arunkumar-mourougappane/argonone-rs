//! `minijinja` environment setup: templates are embedded via
//! `include_str!` (single-binary deploy story, W§2.2) rather than read
//! from disk at runtime.

use minijinja::Environment;

pub fn build_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template("base.html", include_str!("../../templates/base.html"))
        .expect("base.html is valid minijinja syntax");
    env.add_template(
        "app_shell.html",
        include_str!("../../templates/app_shell.html"),
    )
    .expect("app_shell.html is valid minijinja syntax");
    env.add_template("setup.html", include_str!("../../templates/setup.html"))
        .expect("setup.html is valid minijinja syntax");
    env.add_template("login.html", include_str!("../../templates/login.html"))
        .expect("login.html is valid minijinja syntax");
    env.add_template(
        "change_password.html",
        include_str!("../../templates/change_password.html"),
    )
    .expect("change_password.html is valid minijinja syntax");
    env.add_template(
        "dashboard.html",
        include_str!("../../templates/dashboard.html"),
    )
    .expect("dashboard.html is valid minijinja syntax");
    env.add_template(
        "fan_curve.html",
        include_str!("../../templates/fan_curve.html"),
    )
    .expect("fan_curve.html is valid minijinja syntax");
    env.add_template("storage.html", include_str!("../../templates/storage.html"))
        .expect("storage.html is valid minijinja syntax");
    env.add_template("system.html", include_str!("../../templates/system.html"))
        .expect("system.html is valid minijinja syntax");
    env.add_template("users.html", include_str!("../../templates/users.html"))
        .expect("users.html is valid minijinja syntax");
    env.add_template("oled.html", include_str!("../../templates/oled.html"))
        .expect("oled.html is valid minijinja syntax");
    env.add_template("audit.html", include_str!("../../templates/audit.html"))
        .expect("audit.html is valid minijinja syntax");
    env
}

/// Renders `name` with `ctx` to an HTML response. Falls back to a plain
/// `500`-style error string on a render failure (a template bug, not a
/// user-facing condition) rather than panicking the request task.
pub fn render(
    env: &Environment<'static>,
    name: &str,
    ctx: minijinja::Value,
) -> axum::response::Html<String> {
    let body = match env.get_template(name).and_then(|t| t.render(ctx)) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!(template = name, error = %e, "template render failed");
            format!("internal error rendering {name}")
        }
    };
    axum::response::Html(body)
}
