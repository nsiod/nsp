//! `/api/protocol/ss/{status,start,stop}` — protocol-service routes for
//! the Shadowsocks driver.
//!
//! User-scoped operations (list, create, delete, rotate, config, QR)
//! live on `/api/users/:id/ss/*` and are served from `users.rs`. Keeping
//! this module protocol-only means there is exactly one way to reach
//! any given user resource.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use nsp_ss_driver::{SsDriver, SsSnapshot};
use serde::Serialize;

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(status))
        .route("/start", post(start))
        .route("/stop", post(stop))
}

/// Router with the JWT auth middleware applied. Used by the top-level
/// `nest("/api/protocol/ss", ...)` call in `lib.rs`.
pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

fn driver(state: &AppState) -> Result<&SsDriver, ApiError> {
    state
        .ss_driver
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable("shadowsocks driver not initialised".to_owned()))
}

// ---------- payloads ----------

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub running: bool,
    pub listen_port: u16,
    pub public_host: String,
    pub method: String,
    pub users: u64,
    pub reload_count: u64,
    pub last_swap_ms: u64,
    /// Preflight availability (cached briefly). `false` when the driver
    /// cannot be (re)started right now.
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl From<SsSnapshot> for StatusResponse {
    fn from(s: SsSnapshot) -> Self {
        Self {
            running: s.running,
            listen_port: s.listen_port,
            public_host: s.public_host,
            method: s.method,
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
    let Some(d) = state.ss_driver.as_ref() else {
        return Json(StatusResponse {
            running: false,
            listen_port: 0,
            public_host: String::new(),
            method: String::new(),
            users: 0,
            reload_count: 0,
            last_swap_ms: 0,
            available: false,
            reason: Some("shadowsocks disabled in configuration".to_owned()),
        });
    };
    let snap = d.status().await;
    let avail = d.availability().await;
    let mut resp: StatusResponse = snap.into();
    resp.available = avail.available;
    resp.reason = avail.reason;
    Json(resp)
}

/// Start the Shadowsocks data plane. Returns 204 on transition, 409 if
/// already running, 503 if preconditions fail.
async fn start(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let d = driver(&state)?;
    if d.is_running().await {
        return Err(ApiError::Conflict("shadowsocks already running".to_owned()));
    }
    let avail = d.availability().await;
    if !avail.available {
        return Err(ApiError::Unavailable(
            avail
                .reason
                .unwrap_or_else(|| "shadowsocks preconditions unmet".to_owned()),
        ));
    }
    d.start().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop the Shadowsocks data plane. Idempotent at the driver layer, but
/// the API returns 409 if already stopped so clients see the transition.
async fn stop(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let d = driver(&state)?;
    if !d.is_running().await {
        return Err(ApiError::Conflict("shadowsocks not running".to_owned()));
    }
    d.stop().await?;
    Ok(StatusCode::NO_CONTENT)
}
