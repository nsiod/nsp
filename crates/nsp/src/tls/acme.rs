//! ACME (Let's Encrypt) TLS via `rustls-acme`.
//!
//! We bind on `:443` only; the TLS-ALPN-01 challenge reuses the same socket,
//! so no plaintext `:80` handler is required. The renewal loop stores
//! materialized certs in memory (via rustls-acme's resolver) and mirrors them
//! to `tls.acme.cache_dir` so a restart reuses existing material instead of
//! hitting the ACME rate limit.

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use futures::StreamExt;
use nsp_core::config::{AcmeConfig, HttpConfig};
use rustls_acme::{caches::DirCache, AcmeConfig as AcmeCfg};
use tokio::task::JoinHandle;

use super::Acceptor;

/// Start the ACME client; return an [`Acceptor::Acme`].
pub async fn start(http: &HttpConfig, cfg: &AcmeConfig) -> anyhow::Result<Acceptor> {
    let domains = resolve_domains(http, cfg)?;
    let email = cfg
        .email
        .clone()
        .ok_or_else(|| anyhow!("tls.acme.email is required when acme is enabled"))?;

    let cache_dir = ensure_cache_dir(&cfg.cache_dir).await?;

    tracing::info!(
        domains = ?domains,
        email = %email,
        production = cfg.production,
        cache = %cache_dir.display(),
        "starting rustls-acme"
    );

    let mut state = AcmeCfg::new(domains.clone())
        .contact(vec![format!("mailto:{email}")])
        .cache(DirCache::new(cache_dir))
        .directory_lets_encrypt(cfg.production)
        .state();

    let rustls_server_cfg = state.default_rustls_config();
    let acceptor = state.axum_acceptor(rustls_server_cfg);

    // Renewal task: rustls-acme drives itself when polled. We spawn a task
    // that forever drains the event stream and surfaces events as tracing.
    let renewal: JoinHandle<()> = tokio::spawn(async move {
        while let Some(ev) = state.next().await {
            match ev {
                Ok(ok) => tracing::info!(target: "nsp::acme", ?ok, "acme event"),
                Err(err) => tracing::error!(target: "nsp::acme", %err, "acme error"),
            }
        }
    });

    Ok(Acceptor::Acme(acceptor, renewal))
}

fn resolve_domains(http: &HttpConfig, cfg: &AcmeConfig) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = cfg.domains.clone();
    if out.is_empty() {
        if let Some(domain) = http.domain.as_ref() {
            out.push(domain.clone());
        }
    }
    if out.is_empty() {
        return Err(anyhow!(
            "tls.acme is enabled but no domain is configured \
             (set tls.acme.domains or http.domain)"
        ));
    }
    Ok(out)
}

async fn ensure_cache_dir(dir: &PathBuf) -> anyhow::Result<PathBuf> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("create acme cache dir {}", dir.display()))?;
    Ok(dir.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsp_core::config::{AcmeConfig as A, HttpConfig as H};
    use std::net::SocketAddr;

    #[test]
    fn resolve_uses_acme_domains_when_set() {
        let http = H {
            listen: SocketAddr::from(([0, 0, 0, 0], 443)),
            domain: Some("fallback.example".into()),
        };
        let cfg = A {
            enabled: true,
            email: Some("a@b".into()),
            domains: vec!["prio.example".into()],
            production: false,
            cache_dir: PathBuf::from("/tmp"),
        };
        let got = resolve_domains(&http, &cfg).unwrap();
        assert_eq!(got, vec!["prio.example".to_owned()]);
    }

    #[test]
    fn resolve_falls_back_to_domain() {
        let http = H {
            listen: SocketAddr::from(([0, 0, 0, 0], 443)),
            domain: Some("fallback.example".into()),
        };
        let cfg = A {
            enabled: true,
            email: Some("a@b".into()),
            domains: vec![],
            production: false,
            cache_dir: PathBuf::from("/tmp"),
        };
        let got = resolve_domains(&http, &cfg).unwrap();
        assert_eq!(got, vec!["fallback.example".to_owned()]);
    }

    #[test]
    fn resolve_errs_without_any_domain() {
        let http = H {
            listen: SocketAddr::from(([0, 0, 0, 0], 443)),
            domain: None,
        };
        let cfg = A {
            enabled: true,
            email: Some("a@b".into()),
            domains: vec![],
            production: false,
            cache_dir: PathBuf::from("/tmp"),
        };
        assert!(resolve_domains(&http, &cfg).is_err());
    }
}
