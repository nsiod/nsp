//! Listener setup for `axum-server`.
//!
//! Four modes, picked in order of priority:
//!
//! 1. **Plain HTTP** when `tls.enabled = false`.
//! 2. **ACME** via `rustls-acme` (`tls.acme.enabled = true`), in-memory cert
//!    rotation with an on-disk cache under `tls.acme.cache_dir`. Uses the
//!    TLS-ALPN-01 challenge so no plaintext `:80` is required.
//! 3. **Static PEM** when `tls.cert_path` / `tls.key_path` are both set.
//! 4. **Self-signed** fallback for dev runs where neither of the above is
//!    configured.

use anyhow::{anyhow, Context};
use axum_server::tls_rustls::RustlsConfig;
use nsp_core::config::{HttpConfig, TlsConfig};
use rustls_acme::axum::AxumAcceptor;
use tokio::task::JoinHandle;

pub mod acme;
pub mod static_pem;

/// Returned from [`build`]; server.rs dispatches based on the variant.
pub enum Acceptor {
    /// Plain HTTP listener. Useful for local development and reverse-proxy TLS
    /// termination.
    Plain,
    /// Static PEM or self-signed cert. `axum_server::bind_rustls` accepts this
    /// directly.
    Static(RustlsConfig),
    /// ACME mode. The `AxumAcceptor` is consumed by `axum_server::bind(..)
    /// .acceptor(acc)`; the `JoinHandle` keeps the renewal loop alive for as
    /// long as the server runs.
    Acme(AxumAcceptor, JoinHandle<()>),
}

impl Acceptor {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Static(_) => "static",
            Self::Acme(_, _) => "acme",
        }
    }

    #[must_use]
    pub fn protocol(&self) -> &'static str {
        match self {
            Self::Plain => "HTTP",
            Self::Static(_) | Self::Acme(_, _) => "HTTPS",
        }
    }
}

/// Install the default rustls `ring` crypto provider. Idempotent.
pub fn install_default_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Build the TLS acceptor from config.
pub async fn build(http: &HttpConfig, tls: &TlsConfig) -> anyhow::Result<Acceptor> {
    if !tls.enabled {
        if tls.acme.enabled || tls.cert_path.is_some() || tls.key_path.is_some() {
            return Err(anyhow!(
                "tls.enabled=false cannot be combined with ACME or static TLS certs"
            ));
        }
        return Ok(Acceptor::Plain);
    }

    install_default_crypto_provider();

    if tls.acme.enabled {
        let acceptor = acme::start(http, &tls.acme)
            .await
            .context("start ACME acceptor")?;
        return Ok(acceptor);
    }

    // Static PEM takes priority over self-signed when configured.
    match (&tls.cert_path, &tls.key_path) {
        (Some(cert), Some(key)) => {
            tracing::info!(
                cert = %cert.display(),
                key  = %key.display(),
                "using static TLS cert"
            );
            let cfg = static_pem::load(cert, key).await?;
            Ok(Acceptor::Static(cfg))
        }
        (Some(_), None) | (None, Some(_)) => Err(anyhow!(
            "tls.cert_path and tls.key_path must be set together"
        )),
        (None, None) => {
            let cfg = static_pem::self_signed(http).await?;
            Ok(Acceptor::Static(cfg))
        }
    }
}

/// Small convenience wrapper used by server.rs to bind the listener.
#[allow(clippy::missing_errors_doc)]
pub async fn serve(
    addr: std::net::SocketAddr,
    acceptor: Acceptor,
    handle: axum_server::Handle,
    router: axum::Router,
) -> anyhow::Result<()> {
    match acceptor {
        Acceptor::Plain => axum_server::bind(addr)
            .handle(handle)
            .serve(router.into_make_service())
            .await
            .context("http server (plain HTTP)"),
        Acceptor::Static(cfg) => axum_server::bind_rustls(addr, cfg)
            .handle(handle)
            .serve(router.into_make_service())
            .await
            .context("http server (static TLS)"),
        Acceptor::Acme(acc, _renewal) => axum_server::bind(addr)
            .handle(handle)
            .acceptor(acc)
            .serve(router.into_make_service())
            .await
            .context("http server (ACME)"),
    }
}
