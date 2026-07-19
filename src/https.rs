//! HTTPS serving (v0.6.0, A§4.4): Tailscale-issued certs as the
//! recommended default for this audience, `rustls-acme` for a
//! custom-domain/Let's Encrypt path, or an explicit plain-HTTP opt-out.
//! Only one mode is active per boot — switching modes needs a daemon
//! restart to rebind the listener, this isn't hot-reloaded the way a
//! cert renewal is.

use crate::config::{HttpsConfig, HttpsMode};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Under systemd's `StateDirectory=` (A§3.2), same as the SQLite DB —
/// overridable for local dev/testing, matching `ARGONONE_DB_PATH`'s
/// pattern.
const DEFAULT_TLS_CERT_DIR: &str = "/var/lib/argonone-rs/tls";
const DEFAULT_ACME_CACHE_DIR: &str = "/var/lib/argonone-rs/acme-cache";

/// `pub` (not just crate-internal) — `src/web/system.rs`'s HTTPS card
/// reads the same on-disk cert this module itself issues/renews, and
/// triggers a manual re-issue through the same [`run_tailscale_cert`]
/// this module's own renewal loop uses.
pub fn tls_cert_dir() -> PathBuf {
    std::env::var("ARGONONE_TLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_TLS_CERT_DIR))
}

fn acme_cache_dir() -> PathBuf {
    std::env::var("ARGONONE_ACME_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_ACME_CACHE_DIR))
}

/// How often to re-run `tailscale cert`. The command is cheap and
/// idempotent — it only actually reissues a cert once it's near expiry —
/// so a daily cadence trades precision for simplicity rather than tracking
/// the current cert's own expiry timestamp.
const TAILSCALE_RENEW_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Shells out to `tailscale cert`, writing `fullchain.pem`/`privkey.pem`
/// under `dir`. Blocking (subprocess spawn + wait) — callers run it via
/// `spawn_blocking`.
pub fn run_tailscale_cert(domain: &str, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let status = std::process::Command::new("tailscale")
        .arg("cert")
        .arg("--cert-file")
        .arg(dir.join("fullchain.pem"))
        .arg("--key-file")
        .arg(dir.join("privkey.pem"))
        .arg(domain)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "tailscale cert exited with {status}"
        )));
    }
    Ok(())
}

/// Serves `router` under the Tailscale-cert path: issues (or renews) a
/// cert via `tailscale cert` up front, then hands off to `axum-server`
/// with a background task that periodically re-runs `tailscale cert` and
/// hot-swaps the loaded cert via [`RustlsConfig::reload_from_pem_file`] —
/// no listener restart needed for renewal, unlike a mode switch.
async fn serve_tailscale(
    listener: std::net::TcpListener,
    domain: String,
    router: Router,
) -> std::io::Result<()> {
    let dir = tls_cert_dir();

    {
        let dir = dir.clone();
        let domain = domain.clone();
        tokio::task::spawn_blocking(move || run_tailscale_cert(&domain, &dir))
            .await
            .expect("tailscale cert task panicked")?;
    }

    let config =
        RustlsConfig::from_pem_file(dir.join("fullchain.pem"), dir.join("privkey.pem")).await?;

    {
        let config = config.clone();
        let dir = dir.clone();
        let domain = domain.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TAILSCALE_RENEW_INTERVAL);
            interval.tick().await; // first tick fires immediately; already issued above.
            loop {
                interval.tick().await;
                let dir_for_cert = dir.clone();
                let domain_for_cert = domain.clone();
                let renewed = tokio::task::spawn_blocking(move || {
                    run_tailscale_cert(&domain_for_cert, &dir_for_cert)
                })
                .await;
                match renewed {
                    Ok(Ok(())) => {
                        let reload = config
                            .reload_from_pem_file(
                                dir.join("fullchain.pem"),
                                dir.join("privkey.pem"),
                            )
                            .await;
                        match reload {
                            Ok(()) => tracing::info!("Tailscale cert renewal check complete"),
                            Err(e) => {
                                tracing::error!(error = %e, "failed to reload renewed Tailscale cert")
                            }
                        }
                    }
                    Ok(Err(e)) => tracing::error!(error = %e, "tailscale cert renewal failed"),
                    Err(e) => tracing::error!(error = %e, "tailscale cert renewal task panicked"),
                }
            }
        });
    }

    tracing::info!(domain = %domain, "serving HTTPS via Tailscale-issued certificate");
    axum_server::from_tcp(listener)?
        .acceptor(axum_server::tls_rustls::RustlsAcceptor::new(config))
        .serve(router.into_make_service())
        .await
}

/// Serves `router` under the `rustls-acme` (Let's Encrypt) path — fully
/// automatic issuance/renewal, no daemon-owned cron logic needed the way
/// the Tailscale path requires, since the crate's own background task
/// handles it.
async fn serve_acme(
    listener: std::net::TcpListener,
    domain: String,
    email: Option<String>,
    router: Router,
) -> std::io::Result<()> {
    use rustls_acme::AcmeConfig;
    use rustls_acme::caches::DirCache;
    use tokio_stream::StreamExt;

    let mut acme_state = AcmeConfig::new([domain.clone()])
        .contact(email.iter().map(|e| format!("mailto:{e}")))
        .cache(DirCache::new(acme_cache_dir()))
        .directory_lets_encrypt(true)
        .state();
    let acceptor = acme_state.axum_acceptor(acme_state.default_rustls_config());

    tokio::spawn(async move {
        while let Some(event) = acme_state.next().await {
            match event {
                Ok(ok) => tracing::info!(event = ?ok, "ACME event"),
                Err(e) => tracing::error!(error = %e, "ACME error"),
            }
        }
    });

    tracing::info!(domain = %domain, "serving HTTPS via rustls-acme (Let's Encrypt)");
    axum_server::from_tcp(listener)?
        .acceptor(acceptor)
        .serve(router.into_make_service())
        .await
}

/// Binds and serves `router` on a background task, choosing the
/// transport per `https.mode`. A mode that's missing its required domain
/// falls back to plain HTTP with a logged error rather than failing to
/// bind at all — a misconfigured HTTPS setting shouldn't take the whole
/// web UI down.
pub fn spawn_server(listener: std::net::TcpListener, https: HttpsConfig, router: Router) {
    let effective = match https.mode {
        HttpsMode::Tailscale if https.domain.is_none() => {
            tracing::error!(
                "HTTPS mode is 'tailscale' but no domain/hostname is configured — falling back to plain HTTP"
            );
            HttpsConfig::disabled()
        }
        HttpsMode::Acme if https.domain.is_none() => {
            tracing::error!(
                "HTTPS mode is 'acme' but no domain is configured — falling back to plain HTTP"
            );
            HttpsConfig::disabled()
        }
        _ => https,
    };

    match effective.mode {
        HttpsMode::Off => {
            tokio::spawn(async move {
                listener
                    .set_nonblocking(true)
                    .expect("failed to set listener non-blocking");
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("failed to hand listener to tokio");
                tracing::info!("web server listening (plain HTTP)");
                if let Err(e) = axum::serve(listener, router).await {
                    tracing::error!(error = %e, "web server task ended unexpectedly");
                }
            });
        }
        HttpsMode::Tailscale => {
            let domain = effective.domain.expect("checked above");
            tokio::spawn(async move {
                if let Err(e) = serve_tailscale(listener, domain, router).await {
                    tracing::error!(error = %e, "Tailscale HTTPS server ended unexpectedly");
                }
            });
        }
        HttpsMode::Acme => {
            let domain = effective.domain.expect("checked above");
            let email = effective.email;
            tokio::spawn(async move {
                if let Err(e) = serve_acme(listener, domain, email, router).await {
                    tracing::error!(error = %e, "ACME HTTPS server ended unexpectedly");
                }
            });
        }
    }
}

/// The Tailscale-issued cert's actual on-disk status (`08-system-settings.html`'s
/// Tailscale detail table — issuer/expiry/auto-renew), read directly from
/// the PEM file this module's own issuance/renewal writes rather than
/// tracked separately, so it can never drift from what's really loaded.
#[derive(Debug, Clone, Serialize)]
pub struct CertStatus {
    pub issuer: String,
    /// Human-readable, not machine-parseable — e.g. `"Aug 15 12:00:00 2026 +00:00"`.
    pub expires_at: String,
    pub days_until_expiry: i64,
}

/// `None` when no cert has been issued yet at `dir` (mode just switched to
/// `tailscale` but the daemon hasn't restarted since, or the mode isn't
/// `tailscale` at all) or the file can't be parsed — either way, "nothing
/// to show" rather than an error the web layer has to handle specially.
pub fn read_cert_status(dir: &Path) -> Option<CertStatus> {
    let pem_bytes = std::fs::read(dir.join("fullchain.pem")).ok()?;
    let (_, pem) = x509_parser::pem::parse_x509_pem(&pem_bytes).ok()?;
    let cert = pem.parse_x509().ok()?;

    let issuer = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .unwrap_or("unknown issuer")
        .to_string();

    let not_after = cert.validity().not_after;
    let days_until_expiry =
        (not_after.timestamp() - x509_parser::time::ASN1Time::now().timestamp()) / 86400;

    Some(CertStatus {
        issuer,
        expires_at: not_after.to_string(),
        days_until_expiry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway self-signed cert (CN=test.example.ts.net, O=Test Org,
    // 10-year validity from issuance) — parsing doesn't check trust or
    // expiry itself, so a self-signed/long-expired cert is just as valid
    // a fixture as a real Let's Encrypt-issued one for exercising the
    // parser.
    const TEST_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIDQzCCAiugAwIBAgIUHqdtVTGd+Lad0/rQQnLCT1pMzqwwDQYJKoZIhvcNAQEL
BQAwMTEcMBoGA1UEAwwTdGVzdC5leGFtcGxlLnRzLm5ldDERMA8GA1UECgwIVGVz
dCBPcmcwHhcNMjYwNzE5MDQzMDQ1WhcNMzYwNzE2MDQzMDQ1WjAxMRwwGgYDVQQD
DBN0ZXN0LmV4YW1wbGUudHMubmV0MREwDwYDVQQKDAhUZXN0IE9yZzCCASIwDQYJ
KoZIhvcNAQEBBQADggEPADCCAQoCggEBAKR5QfiOk/X2HXvag1DituJnjD9DpcW/
lmhQ+bHn5qvC6/4IwTO1x8/WpJQt4sCLHlc9qdVasm6mT/SfdCZhTf8M8HiRfUq4
DPNLdeqoGAwnH7Gv+TyUl3zr5wRyn+Qz9eqGfOPJLyk+Uuxv1e3Nl6++ee1qWtvh
WiCwyWo2RonyFsY/d1AQNpinMskHJW0OXcny3RDmE3S8fdQfGVzQulPb4ZaO1O12
2rYrlgtDcubn8UtcJHNYMlrtdYOJoJc3bZcLvy8FKNUhMW6uRqRzvMXj20baGMkg
C6Lz0DuCz0glD85jfbXw69iFngM46FxdBunCVAMHNyECY78PM7zZjZECAwEAAaNT
MFEwHQYDVR0OBBYEFEKbFFoIPJSniiyTqxPszZYcDvriMB8GA1UdIwQYMBaAFEKb
FFoIPJSniiyTqxPszZYcDvriMA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZIhvcNAQEL
BQADggEBAIhBOg+DT5O7eAxFAwsHCTYbtWxtbbX9G7zR2enQnpBo2n+HdkeMjbwx
DEwN5zgAIZ0DNC5uTZrtWUnNC0vJjBR67DY5gZsZedbux7Bg0zUjzumDKG9jFvLh
Lf7MJ7ZRNhNZbnMPz4tRIKS5oOHih/9wdYHqhxcqMgY4CCkcacqpl/4rnWKXcWOU
wBX0QuKIwZg/IPBSjuAc8tCvQtKowcKIGnGT7489GnjJGO+OUspc7GEeWtzWcrwh
OkpJ2FJFWoDRI8XWpZC/2GTZqbmZkNy4vm1wEqp2Ppp43kp7+gfbOqW9122XVc+H
SuYMYlIZ3HBeXPdrJWG968lVSgM1F7w=
-----END CERTIFICATE-----
";

    #[test]
    fn read_cert_status_parses_issuer_and_expiry_from_a_real_pem() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fullchain.pem"), TEST_CERT_PEM).unwrap();

        let status = read_cert_status(dir.path()).expect("should parse the fixture cert");
        assert_eq!(status.issuer, "test.example.ts.net");
        // Issued 2026-07-19, 10-year validity -> ~10 years out from now
        // (2026), comfortably not expiring soon and not negative.
        assert!(status.days_until_expiry > 365 * 5);
    }

    #[test]
    fn read_cert_status_is_none_when_no_cert_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_cert_status(dir.path()).is_none());
    }

    #[test]
    fn read_cert_status_is_none_for_garbage_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fullchain.pem"), "not a cert").unwrap();
        assert!(read_cert_status(dir.path()).is_none());
    }
}
