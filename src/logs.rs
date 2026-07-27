//! System log viewer (v0.7.0, W§3.8) — admin-only, read-only tail of
//! `journalctl -u argonone-rs`, the only way to see daemon logs today
//! without SSH. Shells out the same way `sysinfo` already does for
//! `smartctl`/`df`/`mdadm`; no new persistence, no log data stored by
//! this app itself.

use std::process::Command;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct LogLine {
    pub ts: String,
    /// Best-effort, parsed from this project's own `tracing_subscriber`
    /// output (the token right after the timestamp) rather than
    /// `journalctl -p`'s own priority filter — everything this app
    /// writes lands at the same syslog priority regardless of the level
    /// its own formatter printed, since it logs over plain stdout under
    /// a `Type=notify` unit rather than through the journal's structured
    /// logging API.
    pub level: &'static str,
    pub message: String,
}

/// `tracing_subscriber`'s default formatter colors its output with ANSI
/// SGR escapes (`\x1b[2m`, `\x1b[33m`, `\x1b[0m`, …) when it thinks it's
/// writing to a color-capable terminal — journalctl captures a
/// service's stdout byte-for-byte, ANSI codes included, so without this
/// they'd show up as literal `[0m`-style noise in both the parsed view
/// and the raw download. Strips any general CSI sequence (`ESC [
/// params... final-byte`), not just SGR, since that's cheap and correct
/// for any other escape this formatter (or a future one) might emit.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            let mut lookahead = chars.clone();
            if lookahead.next() == Some('[') {
                chars.next(); // consume '['
                for c2 in chars.by_ref() {
                    if ('@'..='~').contains(&c2) {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn parse_line(line: &str) -> LogLine {
    let line = strip_ansi(line);
    let mut parts = line.splitn(2, char::is_whitespace);
    let ts = parts.next().unwrap_or("").to_string();
    let message = parts.next().unwrap_or("").trim_start().to_string();
    let level = match message.split_whitespace().next().unwrap_or("") {
        "ERROR" => "error",
        "WARN" => "warn",
        "TRACE" | "DEBUG" => "debug",
        _ => "info",
    };
    LogLine { ts, level, message }
}

/// `since` must be a journalctl-accepted relative time expression, e.g.
/// `"-15 minutes"`, `"-1 hour"`, `"-24 hours"` — validate/whitelist
/// this at the call site rather than accepting arbitrary strings, since
/// it's shelled out verbatim.
pub fn read_journal(since: &str, max_lines: u32) -> Result<Vec<LogLine>, String> {
    let output = Command::new("journalctl")
        .args([
            "-u",
            "argonone-rs",
            "--no-pager",
            "-o",
            "cat",
            "--since",
            since,
            "-n",
            &max_lines.to_string(),
        ])
        .output()
        .map_err(|e| format!("failed to run journalctl: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("journalctl exited with {}", output.status)
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(parse_line)
        .collect())
}

/// One raw block of text for the "Download .log" action — journalctl's
/// own output, not the parsed/re-joined `LogLine`s, so a downloaded file
/// matches exactly what `journalctl -u argonone-rs` itself would show.
pub fn read_journal_raw(since: &str, max_lines: u32) -> Result<String, String> {
    let output = Command::new("journalctl")
        .args([
            "-u",
            "argonone-rs",
            "--no-pager",
            "--since",
            since,
            "-n",
            &max_lines.to_string(),
        ])
        .output()
        .map_err(|e| format!("failed to run journalctl: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("journalctl exited with {}", output.status)
        } else {
            stderr
        });
    }

    Ok(strip_ansi(&String::from_utf8_lossy(&output.stdout)))
}

/// Maps the toolbar's `since` dropdown value to the journalctl-accepted
/// relative time expression — a fixed whitelist since this ends up on a
/// shelled-out command line.
pub fn since_expr(since: &str) -> Option<&'static str> {
    match since {
        "15m" => Some("-15 minutes"),
        "1h" => Some("-1 hour"),
        "24h" => Some("-24 hours"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_splits_timestamp_and_detects_level() {
        let line = "2026-07-26T23:52:18.492808Z  WARN argonone_rs::hardware: non-Linux platform";
        let parsed = parse_line(line);
        assert_eq!(parsed.ts, "2026-07-26T23:52:18.492808Z");
        assert_eq!(parsed.level, "warn");
        assert_eq!(
            parsed.message,
            "WARN argonone_rs::hardware: non-Linux platform"
        );
    }

    #[test]
    fn parse_line_defaults_to_info_for_unrecognized_level_token() {
        let line = "2026-07-26T23:52:18Z INFO argonone_rs::service: daemon starting";
        assert_eq!(parse_line(line).level, "info");
    }

    #[test]
    fn parse_line_detects_error_level() {
        let line = "2026-07-26T23:52:18Z ERROR argonone_rs::db: failed to open database";
        assert_eq!(parse_line(line).level, "error");
    }

    #[test]
    fn strip_ansi_removes_tracing_subscribers_sgr_escapes() {
        // A real line as tracing_subscriber's default formatter emits it
        // (and as journalctl captures it byte-for-byte, since it's just
        // this process's stdout) — colon/space runs each individually
        // wrapped in their own dim/color/reset escape.
        let line = "\u{1b}[2m2026-07-26T23:52:18.492808Z\u{1b}[0m \u{1b}[33m WARN\u{1b}[0m \u{1b}[2margonone_rs::hardware\u{1b}[0m\u{1b}[2m:\u{1b}[0m non-Linux platform";
        assert_eq!(
            strip_ansi(line),
            "2026-07-26T23:52:18.492808Z  WARN argonone_rs::hardware: non-Linux platform"
        );
    }

    #[test]
    fn strip_ansi_is_a_no_op_on_plain_text() {
        let line = "2026-07-26T23:52:18Z INFO argonone_rs::service: daemon starting";
        assert_eq!(strip_ansi(line), line);
    }

    #[test]
    fn parse_line_strips_ansi_before_splitting_timestamp_and_level() {
        let line = "\u{1b}[2m2026-07-26T23:52:18Z\u{1b}[0m \u{1b}[33mWARN\u{1b}[0m argonone_rs::hardware: msg";
        let parsed = parse_line(line);
        assert_eq!(parsed.ts, "2026-07-26T23:52:18Z");
        assert_eq!(parsed.level, "warn");
        assert_eq!(parsed.message, "WARN argonone_rs::hardware: msg");
    }

    #[test]
    fn since_expr_only_accepts_the_known_dropdown_values() {
        assert_eq!(since_expr("15m"), Some("-15 minutes"));
        assert_eq!(since_expr("1h"), Some("-1 hour"));
        assert_eq!(since_expr("24h"), Some("-24 hours"));
        assert_eq!(since_expr("; rm -rf /"), None);
    }
}
