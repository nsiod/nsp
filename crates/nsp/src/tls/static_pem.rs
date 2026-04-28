//! Static-PEM and self-signed TLS loaders.

use std::path::Path;

use anyhow::Context;
use axum_server::tls_rustls::RustlsConfig;
use nsp_core::config::HttpConfig;

pub async fn load(cert: &Path, key: &Path) -> anyhow::Result<RustlsConfig> {
    RustlsConfig::from_pem_file(cert, key)
        .await
        .context("load pem cert/key")
}

pub async fn self_signed(cfg: &HttpConfig) -> anyhow::Result<RustlsConfig> {
    let domain = cfg.domain.clone().unwrap_or_else(|| "localhost".to_owned());
    tracing::warn!(domain = %domain, "no TLS cert configured; generating self-signed (dev only)");

    let cert = rcgen::generate_simple_self_signed(vec![domain.clone(), "localhost".to_owned()])
        .context("generate self-signed cert")?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();

    RustlsConfig::from_pem(cert_pem.into_bytes(), key_pem.into_bytes())
        .await
        .context("load self-signed cert into rustls")
}
