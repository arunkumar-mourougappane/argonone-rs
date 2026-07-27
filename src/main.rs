mod admin;
mod auth;
mod cli;
mod config;
mod db;
mod fan;
mod hardware;
mod https;
mod logs;
mod notify;
mod oled;
mod rtc_schedule;
mod service;
mod sysinfo;
mod web;

use cli::{AdminCommand, Command};

#[tokio::main]
async fn main() {
    // Explicit, not feature-inferred: with `reqwest`'s `rustls-tls`
    // pulling in a second candidate backend (`ring`) alongside the
    // `aws-lc-rs` one `rustls-acme`/`axum-server` already use, rustls can
    // no longer auto-select a process-level `CryptoProvider` from crate
    // features alone — it needs exactly one candidate compiled in, or an
    // explicit install like this one. Must happen before anything touches
    // TLS (HTTPS mode dispatch, the webhook notifier's `reqwest` client).
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("no CryptoProvider should already be installed this early in main()");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = cli::parse();
    match cli.command {
        Command::Service => service::run().await,
        Command::Shutdown => service::shutdown_once(),
        Command::Fanoff => service::fanoff_once(),
        Command::Status => service::print_status().await,
        Command::Admin { command } => match command {
            AdminCommand::ResetPassword { username } => admin::reset_password(&username).await,
            AdminCommand::Backup { output } => admin::backup(&output).await,
            AdminCommand::Restore { input } => admin::restore(&input).await,
            AdminCommand::ImportLegacyConfig => admin::import_legacy_config().await,
        },
    }
}
