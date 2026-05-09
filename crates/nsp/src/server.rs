//! Axum server with graceful-shutdown on SIGINT/SIGTERM.

use std::net::SocketAddr;

use axum::Router;
use axum_server::Handle;
use nsp_core::driver::Driver;
use nsp_db::Pool;
use nsp_ss_driver::SsDriver;
use nsp_wg_driver::WgDriver;
use tokio::signal;

use crate::tls::Acceptor;

pub async fn serve(
    addr: SocketAddr,
    acceptor: Acceptor,
    router: Router,
    pool: Pool,
    ss: Option<SsDriver>,
    wg: Option<WgDriver>,
) -> anyhow::Result<()> {
    let handle = Handle::new();

    // Schedule shutdown on signal.
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        if let Err(err) = wait_for_signal().await {
            tracing::error!(?err, "signal listener failed");
            return;
        }
        tracing::info!("shutdown signal received; graceful stop in progress");
        shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
    });

    tracing::info!(%addr, kind = acceptor.kind(), protocol = acceptor.protocol(), "listening");
    let result = crate::tls::serve(addr, acceptor, handle, router).await;

    shutdown_runtime(pool, ss, wg).await;
    result
}

/// Headless variant: no HTTP listener. Block until SIGINT/SIGTERM
/// and then run the same driver/pool teardown as [`serve`] so the
/// process exits cleanly. Used when `security.api = "disabled"`
/// — the operator administers the node exclusively through the
/// reverse-API control center, so the local admin port isn't even
/// bound (one less attack surface).
pub async fn wait_for_shutdown_signal(
    pool: Pool,
    ss: Option<SsDriver>,
    wg: Option<WgDriver>,
) -> anyhow::Result<()> {
    if let Err(err) = wait_for_signal().await {
        tracing::error!(?err, "signal listener failed");
    } else {
        tracing::info!("shutdown signal received; tearing down headless node");
    }
    shutdown_runtime(pool, ss, wg).await;
    Ok(())
}

async fn shutdown_runtime(pool: Pool, ss: Option<SsDriver>, wg: Option<WgDriver>) {
    tracing::info!("shutting down drivers");
    if let Some(ss) = ss {
        if let Err(err) = ss.shutdown().await {
            tracing::warn!(?err, "ss shutdown");
        }
    }
    if let Some(wg) = wg {
        if let Err(err) = wg.shutdown().await {
            tracing::warn!(?err, "wg shutdown");
        }
    }

    tracing::info!("closing db pool");
    pool.close().await;

    tracing::info!("bye");
}

#[cfg(unix)]
async fn wait_for_signal() -> std::io::Result<()> {
    use signal::unix::{signal as unix_signal, SignalKind};

    let mut sigterm = unix_signal(SignalKind::terminate())?;
    let mut sigint = unix_signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = sigterm.recv() => {},
        _ = sigint.recv() => {},
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_signal() -> std::io::Result<()> {
    signal::ctrl_c().await
}
