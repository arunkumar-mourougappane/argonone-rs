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

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Run the daemon: fan control loop, power-button monitor, EON
    /// OLED/RTC, and the web server. This is what the systemd unit
    /// invokes.
    Service,
    /// One-shot: signal the case MCU that the Pi is shutting down, then
    /// invoke the system shutdown. Matches `argononed.py SHUTDOWN`.
    Shutdown,
    /// One-shot: turn the fan off and exit. Matches `argononed.py FANOFF`.
    Fanoff,
    /// One-shot: print current stats (CPU%, RAM, temp, disks, RAID, IP)
    /// and exit. Parity with `argonstatus.py`'s pretty-printer.
    Status,
    /// User-account administration, run directly against the database.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AdminCommand {
    /// Reset a user's password directly, bypassing the web layer
    /// entirely — the "no admin can log in" fallback (A§1.2 Tier 2).
    /// Prints a generated temporary password to stdout; the user is
    /// forced to change it on next login.
    ResetPassword {
        #[arg(long)]
        username: String,
    },
}

/// The legacy Python daemon was invoked as bare uppercase tokens
/// (`argononed.py SHUTDOWN`), not a subcommand — existing shell scripts
/// and systemd units built around it use that exact spelling. Recognize
/// it before handing off to `clap`, so both `argonone-rs SHUTDOWN` and
/// `argonone-rs shutdown` work.
pub fn parse() -> Cli {
    parse_args(std::env::args().collect())
}

fn parse_args(mut args: Vec<String>) -> Cli {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(rest: &[&str]) -> Vec<String> {
        std::iter::once("argonone-rs")
            .chain(rest.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn legacy_uppercase_tokens_normalize_to_subcommands() {
        assert_eq!(parse_args(args(&["SERVICE"])).command, Command::Service);
        assert_eq!(parse_args(args(&["SHUTDOWN"])).command, Command::Shutdown);
        assert_eq!(parse_args(args(&["FANOFF"])).command, Command::Fanoff);
    }

    #[test]
    fn lowercase_subcommands_pass_through_unchanged() {
        assert_eq!(parse_args(args(&["service"])).command, Command::Service);
        assert_eq!(parse_args(args(&["status"])).command, Command::Status);
    }
}
