//! Audit log HTTP endpoints.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    middleware,
    routing::get,
    Json, Router,
};
use nsp_db::{AuditLogRow, AuditRepo};
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/audit", get(list))
}

pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Serialize)]
struct AuditEntryDto {
    id: i64,
    ts: i64,
    actor: String,
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl From<AuditLogRow> for AuditEntryDto {
    fn from(row: AuditLogRow) -> Self {
        Self {
            id: row.id,
            ts: row.ts,
            actor: row.actor,
            action: row.action,
            target: row.target,
            detail: row.detail,
        }
    }
}

async fn list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<AuditEntryDto>>, ApiError> {
    let rows = AuditRepo::new(&state.db).list(params.limit).await?;
    Ok(Json(rows.into_iter().map(AuditEntryDto::from).collect()))
}
