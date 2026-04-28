//! `/api/protocol/wg/{status,start,stop}` — protocol-service routes for
//! the WireGuard driver.
//!
//! User-scoped operations (list, create, delete, rotate, detail) live
//! on `/api/users/:id/wg/*` and are served from `users.rs`. The server
//! never persists a client peer's private key; one-shot secrets flow
//! only through the enable/rotate responses there.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use nsp_wg_driver::{WgDriver, WgStatus};

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(status))
        .route("/start", post(start))
        .route("/stop", post(stop))
}

/// Router with the JWT auth middleware applied. Used by the top-level
/// `nest("/api/protocol/wg", ...)` call in `lib.rs`.
pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

fn driver(state: &Arc<AppState>) -> Result<&WgDriver, ApiError> {
    state
        .wg
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable("wireguard disabled".into()))
}

async fn status(State(state): State<Arc<AppState>>) -> Result<Json<WgStatus>, ApiError> {
    let Some(d) = state.wg.as_ref() else {
        return Ok(Json(WgStatus {
            running: false,
            interface: String::new(),
            listen_port: 0,
            subnet: String::new(),
            server_public_key: String::new(),
            total_peers: 0,
            endpoint_host: None,
            available: false,
            reason: Some("wireguard disabled in configuration".to_owned()),
        }));
    };
    let s = d.status_view().await?;
    Ok(Json(s))
}

/// Start the WireGuard data plane. Returns 204 on transition, 409 if
/// already running, 503 with a reason when preconditions are missing.
async fn start(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let d = driver(&state)?;
    if d.is_running().await {
        return Err(ApiError::Conflict("wireguard already running".to_owned()));
    }
    let avail = d.availability().await;
    if !avail.available {
        return Err(ApiError::Unavailable(
            avail
                .reason
                .unwrap_or_else(|| "wireguard preconditions unmet".to_owned()),
        ));
    }
    d.spawn_real().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop the WireGuard data plane. Idempotent at the driver layer, but
/// the API returns 409 if already stopped so clients see the transition.
async fn stop(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let d = driver(&state)?;
    if !d.is_running().await {
        return Err(ApiError::Conflict("wireguard not running".to_owned()));
    }
    d.stop().await?;
    Ok(StatusCode::NO_CONTENT)
}
