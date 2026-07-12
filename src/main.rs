mod cli;
mod config;
mod fan;
mod hardware;
mod service;
mod sysinfo;

use cli::Command;

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
        Command::Status => service::print_status(),
    }
}
