//! HTTP route definitions.

use std::sync::Arc;

use axum::extract::Request;
use axum::{
    extract::State,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use nsp_core::auth;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

pub fn public_router() -> Router<Arc<AppState>> {
    Router::new().route("/api/healthz", get(healthz))
}

pub fn api_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let public = Router::new().route("/api/auth/login", post(login));

    let protected = Router::new()
        .route("/api/me", get(whoami))
        .route("/api/status", get(status))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new().merge(public).merge(protected)
}

// ------- handlers -------

async fn healthz() -> Response {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

#[derive(Serialize)]
struct StatusResponse {
    version: &'static str,
    ss_enabled: bool,
    wg_enabled: bool,
    proxy_enabled: bool,
}

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    // `enabled` reports the live running state, not config intent: once the
    // API has driven a driver up or down via /api/{ss,wg,proxy}/{start,stop},
    // the config `enabled` flag is no longer the source of truth.
    let ss_enabled = if let Some(ss) = state.ss_driver.as_ref() {
        ss.is_running().await
    } else {
        false
    };
    let wg_enabled = if let Some(wg) = state.wg.as_ref() {
        wg.is_running().await
    } else {
        false
    };
    let proxy_enabled = if let Some(p) = state.proxy.as_ref() {
        p.is_running().await
    } else {
        false
    };
    Json(StatusResponse {
        version: state.version,
        ss_enabled,
        wg_enabled,
        proxy_enabled,
    })
}

#[derive(Deserialize)]
struct LoginRequest {
    password: SecretString,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    expires_at: i64,
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    use nsp_db::SettingsRepo;

    let repo = SettingsRepo::new(&state.db);
    let row = repo.get().await?;
    let phc = row.admin_password_hash.ok_or(ApiError::Unauthorized)?;
    let ok = auth::verify_password(&req.password, &phc)?;
    if !ok {
        return Err(ApiError::Unauthorized);
    }
    let (token, expires_at) = auth::issue_jwt(
        "admin",
        row.token_generation,
        state.jwt_ttl_secs,
        &state.jwt_key,
    )?;
    Ok(Json(LoginResponse { token, expires_at }))
}

#[derive(Serialize)]
struct Me {
    sub: String,
}

async fn whoami(req: Request) -> Result<Json<Me>, ApiError> {
    let claims = req
        .extensions()
        .get::<auth::Claims>()
        .cloned()
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(Me { sub: claims.sub }))
}

// ------- middleware -------

pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    use nsp_db::SettingsRepo;

    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or(ApiError::Unauthorized)?;
    let decoded = auth::decode_jwt(token, &state.jwt_key).map_err(|_| ApiError::Unauthorized)?;
    let current = SettingsRepo::new(&state.db)
        .token_generation()
        .await
        .map_err(|_| ApiError::Unauthorized)?;
    if decoded.claims.tgen < current {
        return Err(ApiError::Unauthorized);
    }
    req.extensions_mut().insert(decoded.claims);
    Ok(next.run(req).await)
}
