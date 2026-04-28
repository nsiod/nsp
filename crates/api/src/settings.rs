//! Settings endpoints (`/api/settings`, `/api/reload`).
//!
//! Exposes the singleton settings row and drives hot reload of the SS /
//! WG drivers whenever a reload-critical field changes. Password rotations
//! bump `settings.token_generation`, forcing re-login on every active session
//! via the auth middleware.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use ipnetwork::Ipv4Network;
use nsp_core::auth::hash_password;
use nsp_db::{SettingsPatch, SettingsRepo, SettingsRow};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/settings", get(get_settings).patch(patch_settings))
        .route("/api/reload", post(reload))
}

pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

#[derive(Debug, Serialize)]
struct SettingsView {
    domain: Option<String>,
    wg_subnet: Option<String>,
    ss_listen_port: i64,
    wg_listen_port: i64,
    token_generation: i64,
    updated_at: i64,
}

impl From<&SettingsRow> for SettingsView {
    fn from(row: &SettingsRow) -> Self {
        Self {
            domain: row.domain.clone(),
            wg_subnet: row.wg_subnet.clone(),
            ss_listen_port: row.ss_listen_port,
            wg_listen_port: row.wg_listen_port,
            token_generation: row.token_generation,
            updated_at: row.updated_at,
        }
    }
}

async fn get_settings(State(state): State<Arc<AppState>>) -> Result<Json<SettingsView>, ApiError> {
    let row = SettingsRepo::new(&state.db).get().await?;
    Ok(Json(SettingsView::from(&row)))
}

/// Tri-state patch body. Fields absent from the request body leave the
/// corresponding column untouched; `null` clears it (where nullable);
/// any concrete value replaces it.
///
/// The body uses `serde::deserialize_with` quirks to implement the
/// Option<Option<_>> tri-state, so structure this like
/// `{ domain: "..." | null, wg_subnet: ... }`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsPatchBody {
    #[serde(default, deserialize_with = "de_opt_opt")]
    domain: Option<Option<String>>,
    #[serde(default, deserialize_with = "de_opt_opt")]
    wg_subnet: Option<Option<String>>,
    #[serde(default)]
    ss_listen_port: Option<u16>,
    #[serde(default)]
    wg_listen_port: Option<u16>,
    #[serde(default)]
    new_password: Option<SecretString>,
}

fn de_opt_opt<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

async fn patch_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SettingsPatchBody>,
) -> Result<Json<SettingsView>, ApiError> {
    let repo = SettingsRepo::new(&state.db);
    let current = repo.get().await?;

    // Validate and detect subnet change before committing. Parse once —
    // the conflict-detection path must use the same validated value as
    // the eventual runtime apply, never a silently-coerced `None`.
    if let Some(opt) = &body.wg_subnet {
        let target: Option<Ipv4Network> = match opt {
            Some(s) => Some(
                s.parse()
                    .map_err(|e| ApiError::BadRequest(format!("wg_subnet: {e}")))?,
            ),
            None => None,
        };
        if let Some(wg) = state.wg.as_ref() {
            let conflicts = wg
                .peers_outside_subnet(target)
                .await
                .map_err(ApiError::from)?;
            if !conflicts.is_empty() {
                return Err(ApiError::SubnetConflict(conflicts));
            }
        }
    }
    if let Some(port) = body.ss_listen_port {
        if port == 0 {
            return Err(ApiError::BadRequest(
                "ss_listen_port must be non-zero".into(),
            ));
        }
    }
    if let Some(port) = body.wg_listen_port {
        if port == 0 {
            return Err(ApiError::BadRequest(
                "wg_listen_port must be non-zero".into(),
            ));
        }
        if i64::from(port) != current.wg_listen_port {
            return Err(ApiError::BadRequest(
                "wg_listen_port changes require a process restart".into(),
            ));
        }
    }

    let mut patch = SettingsPatch {
        domain: body.domain,
        wg_subnet: body.wg_subnet.clone(),
        ss_listen_port: body.ss_listen_port.map(i64::from),
        wg_listen_port: None,
        ..Default::default()
    };
    if let Some(pw) = body.new_password {
        let phc = hash_password(&pw).map_err(ApiError::Core)?;
        patch.admin_password_hash = Some(phc);
    }

    let updated = repo.patch(patch).await?;

    apply_runtime(&state, Some(&current), &updated).await?;

    Ok(Json(SettingsView::from(&updated)))
}

async fn reload(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let row = SettingsRepo::new(&state.db).get().await?;
    apply_runtime(&state, None, &row).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Apply any runtime-visible changes. When `before` is `None` the caller
/// is `/api/reload` and the function re-asserts the full DB state into
/// the drivers.
async fn apply_runtime(
    state: &Arc<AppState>,
    before: Option<&SettingsRow>,
    after: &SettingsRow,
) -> Result<(), ApiError> {
    let force = before.is_none();
    let domain_changed = before.map(|b| b.domain != after.domain).unwrap_or(true);
    if let Some(ss) = state.ss_driver.as_ref() {
        let port_changed = before
            .map(|b| b.ss_listen_port != after.ss_listen_port)
            .unwrap_or(true);
        if port_changed || domain_changed || force {
            let port = u16::try_from(after.ss_listen_port)
                .map_err(|_| ApiError::BadRequest("ss_listen_port overflow".into()))?;
            ss.set_listen(None, Some(port), after.domain.clone())
                .await
                .map_err(ApiError::Ss)?;
        }
    }

    if let Some(wg) = state.wg.as_ref() {
        let subnet_changed = before
            .map(|b| b.wg_subnet != after.wg_subnet)
            .unwrap_or(true);
        if subnet_changed || force {
            let target: Option<Ipv4Network> = after
                .wg_subnet
                .as_deref()
                .and_then(|s| s.parse::<Ipv4Network>().ok());
            wg.set_subnet(target).await.map_err(ApiError::from)?;
        }
        if domain_changed || force {
            wg.set_endpoint_host(after.domain.clone()).await;
        }
    }

    state.notify_reconciler();
    Ok(())
}
