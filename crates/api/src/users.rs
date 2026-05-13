//! `/api/users[...]` — the single user-scoped surface.
//!
//! This module owns every per-user read, write, rotate, and one-shot client
//! material. Protocol-service lifecycle lives on `/api/protocol/{ss,wg}/*`;
//! nothing touches a specific user row there.
//!
//! DB-as-truth: the API updates the database atomically, then wakes the
//! background reconciler. If the corresponding protocol driver is currently
//! running the handler also schedules an immediate apply; otherwise the
//! write still succeeds with `{pending: true}` and the next driver start —
//! or the next reconciler tick — converges the in-memory state.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use nsp_db::{UserRow, UsersRepo};
use nsp_proxy_driver::ProxyDriver;
use nsp_ss_driver::SsDriver;
use nsp_wg_driver::{PeerSecrets, PeerView, WgDriver};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_users).post(create_user))
        .route("/:id", get(get_user).patch(update_user).delete(delete_user))
        .route(
            "/:id/ss",
            get(get_ss_detail).post(enable_ss).delete(disable_ss),
        )
        .route("/:id/ss/rotate", post(rotate_ss))
        .route("/:id/ss/qr", get(ss_qr))
        .route(
            "/:id/wg",
            get(get_wg_detail).post(enable_wg).delete(disable_wg),
        )
        .route("/:id/wg/rotate", post(rotate_wg))
        .route(
            "/:id/proxy",
            get(get_proxy_detail)
                .post(enable_proxy)
                .delete(disable_proxy),
        )
        .route("/:id/proxy/rotate", post(rotate_proxy))
}

pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

fn ss_driver(state: &AppState) -> Result<&SsDriver, ApiError> {
    state
        .ss_driver
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable("shadowsocks driver not initialised".to_owned()))
}

fn wg_driver(state: &AppState) -> Result<&WgDriver, ApiError> {
    state
        .wg
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable("wireguard disabled".to_owned()))
}

fn proxy_driver(state: &AppState) -> Result<&ProxyDriver, ApiError> {
    state
        .proxy
        .as_ref()
        .ok_or_else(|| ApiError::Unavailable("proxy driver not initialised".to_owned()))
}

// ---------- DTOs ----------

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub ss_enabled: bool,
    pub wg_enabled: bool,
    pub proxy_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl From<UserRow> for UserDto {
    fn from(r: UserRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            created_at: r.created_at,
            ss_enabled: r.ss_enabled,
            wg_enabled: r.wg_enabled,
            proxy_enabled: r.proxy_enabled,
            note: r.note,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub name: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// `Some(Some(..))` replaces the note, `Some(None)` clears it,
    /// `None` leaves it untouched. Callers that want to clear the note
    /// must send an explicit JSON `null`.
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub note: Option<Option<String>>,
}

fn deserialize_optional_field<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Debug, Serialize)]
pub struct SsEnableResponse {
    pub user_id: String,
    pub name: String,
    /// Hex of the fresh per-user iPSK. Shown once.
    pub psk: String,
    /// Hex of the shared server uPSK. Same for every user of this server.
    pub server_psk: String,
    /// SS URL for QR / manual import. Embeds both PSKs as base64.
    pub url: String,
    /// `true` when the SS driver was not running at write time — the
    /// reconciler will converge on next start.
    pub pending: bool,
}

#[derive(Debug, Serialize)]
pub struct AckResponse {
    pub pending: bool,
}

#[derive(Debug, Serialize)]
pub struct WgEnableResponse {
    pub user_id: String,
    pub peer: WgPeerDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secrets: Option<WgPeerSecretsDto>,
    pub pending: bool,
}

#[derive(Debug, Serialize)]
pub struct WgPeerDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub public_key: String,
    pub allowed_ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keepalive: Option<u16>,
    pub has_psk: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_handshake_secs: Option<u64>,
}

impl From<PeerView> for WgPeerDto {
    fn from(p: PeerView) -> Self {
        Self {
            id: p.id,
            user_id: p.user_id,
            name: p.name,
            public_key: B64.encode(p.public_key.to_bytes()),
            allowed_ip: p.allowed_ip.to_string(),
            endpoint: p.endpoint.map(|a| a.to_string()),
            keepalive: p.keepalive,
            has_psk: p.has_psk,
            created_at: p.created_at,
            updated_at: p.updated_at,
            rx_bytes: p.rx_bytes,
            tx_bytes: p.tx_bytes,
            last_handshake_secs: p.last_handshake_secs,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WgPeerSecretsDto {
    /// Present only when the server generated the keypair because the
    /// caller did not supply a public key. Shown exactly once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preshared_key: Option<String>,
}

impl From<PeerSecrets> for WgPeerSecretsDto {
    fn from(s: PeerSecrets) -> Self {
        Self {
            private_key: s.private_key.map(|k| B64.encode(k)),
            preshared_key: s.preshared_key.map(|k| B64.encode(k)),
        }
    }
}

/// Optional request body for `POST /api/users/:id/wg[/rotate]`. The
/// caller may supply an already-generated client public key so the
/// server never sees the private half. When the body is absent or
/// `public_key` is null the server generates a fresh keypair and
/// returns the private key exactly once.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WgEnableRequest {
    #[serde(default)]
    pub public_key: Option<String>,
}

fn parse_wg_public_key(raw: Option<String>) -> Result<Option<[u8; 32]>, ApiError> {
    let Some(s) = raw else { return Ok(None) };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let bytes = B64
        .decode(trimmed)
        .map_err(|e| ApiError::BadRequest(format!("public_key base64: {e}")))?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map(Some)
        .map_err(|_| ApiError::BadRequest("public_key must decode to exactly 32 bytes".into()))
}

// ---------- handlers ----------

fn validate_name(name: &str) -> Result<(), ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest("name is required".to_owned()));
    }
    if trimmed.len() > 128 {
        return Err(ApiError::BadRequest(
            "name must be 128 characters or fewer".to_owned(),
        ));
    }
    Ok(())
}

async fn list_users(State(state): State<Arc<AppState>>) -> Result<Json<Vec<UserDto>>, ApiError> {
    let repo = UsersRepo::new(&state.db);
    let rows = repo.list().await?;
    Ok(Json(rows.into_iter().map(UserDto::from).collect()))
}

async fn get_user(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<UserDto>, ApiError> {
    let repo = UsersRepo::new(&state.db);
    let row = repo.get(&id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(UserDto::from(row)))
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    validate_name(&req.name)?;
    let name = req.name.trim().to_owned();
    let id = Uuid::now_v7().to_string();
    let repo = UsersRepo::new(&state.db);
    repo.create(&id, &name, req.note.as_deref()).await?;
    let row = repo.get(&id).await?.ok_or(ApiError::Internal)?;
    Ok((StatusCode::CREATED, Json(UserDto::from(row))))
}

async fn update_user(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserDto>, ApiError> {
    let repo = UsersRepo::new(&state.db);
    if repo.get(&id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    if let Some(new_name) = req.name.as_deref() {
        validate_name(new_name)?;
        let trimmed = new_name.trim();
        if !repo.rename(&id, trimmed).await? {
            return Err(ApiError::NotFound);
        }
    }
    if let Some(note) = req.note.as_ref() {
        if !repo.update_note(&id, note.as_deref()).await? {
            return Err(ApiError::NotFound);
        }
    }
    let row = repo.get(&id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(UserDto::from(row)))
}

/// Delete a user. The `ON DELETE CASCADE` foreign keys on
/// `ss_credentials` and `wg_peers` remove protocol state in the same
/// transaction, so the reconciler is notified to pull live WG peers
/// on its next cycle.
async fn delete_user(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let repo = UsersRepo::new(&state.db);
    if !repo.delete(&id).await? {
        return Err(ApiError::NotFound);
    }
    state.notify_reconciler();
    Ok(StatusCode::NO_CONTENT)
}

// ---------- SS per-user ----------

/// Public SS detail for a user. Excludes the PSK; callers must call
/// `POST /ss` or `POST /ss/rotate` to obtain fresh secret material.
#[derive(Debug, Serialize)]
pub struct SsDetailResponse {
    pub user_id: String,
    pub name: String,
    pub created_at: i64,
    pub url: String,
}

async fn get_ss_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SsDetailResponse>, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.ss_enabled {
        return Err(ApiError::NotFound);
    }
    let d = ss_driver(&state)?;
    let material = d.user_client_material(&id).await?;
    Ok(Json(SsDetailResponse {
        user_id: material.id,
        name: material.name,
        created_at: user.created_at,
        url: material.url,
    }))
}

async fn enable_ss(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<SsEnableResponse>), ApiError> {
    let d = ss_driver(&state)?;
    let material = d.enable_user(&id).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok((
        StatusCode::CREATED,
        Json(SsEnableResponse {
            user_id: material.id,
            name: material.name,
            psk: material.psk_hex,
            server_psk: material.server_psk_hex,
            url: material.url,
            pending,
        }),
    ))
}

async fn disable_ss(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AckResponse>, ApiError> {
    let d = ss_driver(&state)?;
    let removed = d.disable_user(&id).await?;
    state.notify_reconciler();
    if !removed {
        return Err(ApiError::NotFound);
    }
    let pending = !d.is_running().await;
    Ok(Json(AckResponse { pending }))
}

async fn rotate_ss(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SsEnableResponse>, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.ss_enabled {
        return Err(ApiError::NotFound);
    }
    let d = ss_driver(&state)?;
    let material = d.rotate_user(&id).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok(Json(SsEnableResponse {
        user_id: material.id,
        name: material.name,
        psk: material.psk_hex,
        server_psk: material.server_psk_hex,
        url: material.url,
        pending,
    }))
}

async fn ss_qr(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.ss_enabled {
        return Err(ApiError::NotFound);
    }
    let d = ss_driver(&state)?;
    let png = d.user_qr_png(&id).await?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        png,
    )
        .into_response())
}

// ---------- WG per-user ----------

async fn get_wg_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WgPeerDto>, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.wg_enabled {
        return Err(ApiError::NotFound);
    }
    let d = wg_driver(&state)?;
    let peers = d.list_peers().await?;
    let peer = peers
        .into_iter()
        .find(|p| p.user_id.as_deref() == Some(id.as_str()))
        .ok_or(ApiError::NotFound)?;
    Ok(Json(WgPeerDto::from(peer)))
}

async fn enable_wg(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<WgEnableRequest>>,
) -> Result<(StatusCode, Json<WgEnableResponse>), ApiError> {
    // Verify the user exists up-front so 404 beats any driver-level state.
    let users = UsersRepo::new(&state.db);
    if users.get(&id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    let req = body.map(|Json(r)| r).unwrap_or_default();
    let public_key = parse_wg_public_key(req.public_key)?;

    let d = wg_driver(&state)?;
    let (view, secrets) = d.enable_user_wg(&id, public_key).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok((
        StatusCode::CREATED,
        Json(WgEnableResponse {
            user_id: id,
            peer: WgPeerDto::from(view),
            secrets: secrets.map(WgPeerSecretsDto::from),
            pending,
        }),
    ))
}

async fn disable_wg(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AckResponse>, ApiError> {
    let users = UsersRepo::new(&state.db);
    if users.get(&id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let d = wg_driver(&state)?;
    d.disable_user_wg(&id).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok(Json(AckResponse { pending }))
}

async fn rotate_wg(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<WgEnableRequest>>,
) -> Result<Json<WgEnableResponse>, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.wg_enabled {
        return Err(ApiError::NotFound);
    }
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let public_key = parse_wg_public_key(req.public_key)?;

    let d = wg_driver(&state)?;
    let (view, secrets) = d.rotate_user_wg(&id, public_key).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok(Json(WgEnableResponse {
        user_id: id,
        peer: WgPeerDto::from(view),
        secrets: Some(WgPeerSecretsDto::from(secrets)),
        pending,
    }))
}

// ---------- Proxy per-user ----------

#[derive(Debug, Serialize)]
pub struct ProxyEnableResponse {
    pub user_id: String,
    pub name: String,
    pub username: String,
    /// One-shot password. The server stores only the encrypted blob and
    /// never returns the plaintext again — rotate to obtain a fresh one.
    pub password: String,
    pub socks5_url: String,
    pub http_url: String,
    /// `true` when the proxy driver was not running at write time — the
    /// reconciler converges on next start.
    pub pending: bool,
}

#[derive(Debug, Serialize)]
pub struct ProxyDetailResponse {
    pub user_id: String,
    pub name: String,
    pub username: String,
    pub socks5_url: String,
    pub http_url: String,
    pub created_at: i64,
    pub updated_at: i64,
}

async fn get_proxy_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProxyDetailResponse>, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.proxy_enabled {
        return Err(ApiError::NotFound);
    }
    let d = proxy_driver(&state)?;
    let repo = nsp_db::ProxyRepo::new(&state.db);
    let cred = repo.get_by_user(&id).await?.ok_or(ApiError::NotFound)?;
    let host = d.public_host().await;
    let socks5_port = d.socks5_port().await;
    let http_port = d.http_port().await;
    Ok(Json(ProxyDetailResponse {
        user_id: user.id,
        name: user.name,
        username: cred.username.clone(),
        // The detail endpoint does NOT include the password — the
        // server cannot retrieve plaintext credentials after enable;
        // the URLs are rendered with a placeholder that signals the
        // caller must rotate to obtain fresh material.
        socks5_url: format!("socks5://{}@{host}:{socks5_port}", cred.username),
        http_url: format!("http://{}@{host}:{http_port}", cred.username),
        created_at: cred.created_at,
        updated_at: cred.updated_at,
    }))
}

async fn enable_proxy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<ProxyEnableResponse>), ApiError> {
    let d = proxy_driver(&state)?;
    let material = d.enable_user(&id).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok((
        StatusCode::CREATED,
        Json(ProxyEnableResponse {
            user_id: material.user_id,
            name: material.name,
            username: material.username,
            password: material.password,
            socks5_url: material.socks5_url,
            http_url: material.http_url,
            pending,
        }),
    ))
}

async fn disable_proxy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AckResponse>, ApiError> {
    let d = proxy_driver(&state)?;
    let removed = d.disable_user(&id).await?;
    state.notify_reconciler();
    if !removed {
        return Err(ApiError::NotFound);
    }
    let pending = !d.is_running().await;
    Ok(Json(AckResponse { pending }))
}

async fn rotate_proxy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProxyEnableResponse>, ApiError> {
    let users = UsersRepo::new(&state.db);
    let user = users.get(&id).await?.ok_or(ApiError::NotFound)?;
    if !user.proxy_enabled {
        return Err(ApiError::NotFound);
    }
    let d = proxy_driver(&state)?;
    let material = d.rotate_user(&id).await?;
    state.notify_reconciler();
    let pending = !d.is_running().await;
    Ok(Json(ProxyEnableResponse {
        user_id: material.user_id,
        name: material.name,
        username: material.username,
        password: material.password,
        socks5_url: material.socks5_url,
        http_url: material.http_url,
        pending,
    }))
}
