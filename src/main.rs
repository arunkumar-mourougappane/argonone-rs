mod admin;
mod auth;
mod cli;
mod config;
mod db;
mod fan;
mod hardware;
mod https;
mod oled;
mod rtc_schedule;
mod service;
mod sysinfo;
mod web;

use cli::{AdminCommand, Command};

#[tokio::main]
async fn main() {
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
        },
    }
}
