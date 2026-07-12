//! CLI surface. Preserves the exact argv compat the Python daemon's shell
//! wrappers rely on (`argononed.py SERVICE|SHUTDOWN|FANOFF`, W§4 migration
//! notes) alongside a normal lowercase `clap` subcommand for anything new.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "argonone-rs", version, about = "Argon ONE/EON case daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the daemon: fan control loop + power-button monitor. This is
    /// what the systemd unit invokes.
    Service,
    /// One-shot: signal the case MCU that the Pi is shutting down, then
    /// invoke the system shutdown. Matches `argononed.py SHUTDOWN`.
    Shutdown,
    /// One-shot: turn the fan off and exit. Matches `argononed.py FANOFF`.
    Fanoff,
    /// One-shot: print current stats (CPU%, RAM, temp, disks, RAID, IP)
    /// and exit. Parity with `argonstatus.py`'s pretty-printer.
    Status,
}

/// The legacy Python daemon was invoked as bare uppercase tokens
/// (`argononed.py SHUTDOWN`), not a subcommand — existing shell scripts
/// and systemd units built around it use that exact spelling. Recognize
/// it before handing off to `clap`, so both `argonone-rs SHUTDOWN` and
/// `argonone-rs shutdown` work.
pub fn parse() -> Cli {
    let mut args: Vec<String> = std::env::args().collect();
    if let Some(first) = args.get(1) {
        let normalized = match first.to_ascii_uppercase().as_str() {
            "SERVICE" => Some("service"),
            "SHUTDOWN" => Some("shutdown"),
            "FANOFF" => Some("fanoff"),
            _ => None,
        };
        if let Some(normalized) = normalized {
            args[1] = normalized.to_string();
        }
    }
    Cli::parse_from(args)
}
