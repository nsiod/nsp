//! `/api/protocol/proxy/{status,start,stop}` — protocol-service routes
//! for the SOCKS5 + HTTP CONNECT proxy driver.
//!
//! User-scoped operations (enable / disable / rotate) live on
//! `/api/users/:id/proxy[...]` and are served from `users.rs`; this
//! module owns the protocol lifecycle exclusively.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use nsp_proxy_driver::{ProxyDriver, ProxySnapshot};
use serde::Serialize;

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(status))
        .route("/start", post(start))
        .route("/stop", post(stop))
}

/// Router with the JWT auth middleware applied. Used by the top-level
/// `nest("/api/protocol/proxy", ...)` call in `lib.rs`.
pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

fn driver(state: &AppState) -> Result<&ProxyDriver, ApiError> {
    state
        .proxy
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable("proxy driver not initialised".to_owned()))
}

// ---------- payloads ----------

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub running: bool,
    pub socks5_port: u16,
    pub http_port: u16,
    pub public_host: String,
    pub users: u64,
    pub reload_count: u64,
    pub last_swap_ms: u64,
    /// Preflight availability (cached briefly). `false` when the driver
    /// cannot be (re)started right now.
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl From<ProxySnapshot> for StatusResponse {
    fn from(s: ProxySnapshot) -> Self {
        Self {
            running: s.running,
            socks5_port: s.socks5_port,
            http_port: s.http_port,
            public_host: s.public_host,
            users: s.users,
            reload_count: s.reload_count,
            last_swap_ms: s.last_swap_ms,
            available: true,
            reason: None,
        }
    }
}

// ---------- handlers ----------

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let Some(d) = state.proxy.as_ref() else {
        return Json(StatusResponse {
            running: false,
            socks5_port: 0,
            http_port: 0,
            public_host: String::new(),
            users: 0,
            reload_count: 0,
            last_swap_ms: 0,
            available: false,
            reason: Some("proxy disabled in configuration".to_owned()),
        });
    };
    let snap = d.status().await;
    let avail = d.availability().await;
    let mut resp: StatusResponse = snap.into();
    resp.available = avail.available;
    resp.reason = avail.reason;
    Json(resp)
}

/// Start the proxy data plane. Returns 204 on transition, 409 if
/// already running, 503 if preconditions fail.
async fn start(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let d = driver(&state)?;
    if d.is_running().await {
        return Err(ApiError::Conflict("proxy already running".to_owned()));
    }
    let avail = d.availability().await;
    if !avail.available {
        return Err(ApiError::Unavailable(
            avail
                .reason
                .unwrap_or_else(|| "proxy preconditions unmet".to_owned()),
        ));
    }
    d.start().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop the proxy data plane. Idempotent at the driver layer, but the
/// API returns 409 if already stopped so clients see the transition.
async fn stop(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let d = driver(&state)?;
    if !d.is_running().await {
        return Err(ApiError::Conflict("proxy not running".to_owned()));
    }
    d.stop().await?;
    Ok(StatusCode::NO_CONTENT)
}
