//! HTTP endpoints for the unified iptables rule manager.
//!
//! All routes live under `/api/iptables/...` and require a valid admin JWT.
//! Mutations are restricted to the `User` source — attempts to delete a
//! `WgDriver`-owned rule return 403. The SSH guard intercepts submissions
//! that would drop port 22; callers can retry with `force=true` after the UI
//! confirms.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use nsp_netctl::{
    IptablesManager, ListFilter, ReconcileReport, RegisterOptions, RuleSpec, Source, StoredRule,
};
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, routes::require_auth, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/:id", axum::routing::delete(delete_one))
        .route("/verify", post(verify))
        .route("/reconcile", post(reconcile))
}

pub fn protected_router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    router().route_layer(middleware::from_fn_with_state(state, require_auth))
}

fn manager(state: &Arc<AppState>) -> Result<Arc<dyn IptablesManager>, ApiError> {
    state
        .iptables
        .clone()
        .ok_or_else(|| ApiError::Unavailable("iptables manager unavailable".into()))
}

// ---------- DTOs ----------

#[derive(Debug, Clone, Serialize)]
struct RuleDto {
    id: String,
    source: &'static str,
    priority: i32,
    table: String,
    chain: String,
    spec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
    comment_tag: String,
    created_at: i64,
    updated_at: i64,
}

impl From<StoredRule> for RuleDto {
    fn from(r: StoredRule) -> Self {
        let comment_tag = r.comment_tag();
        Self {
            id: r.id,
            source: r.source.as_tag(),
            priority: r.priority,
            table: r.table,
            chain: r.chain,
            spec: r.spec,
            comment: r.comment,
            comment_tag,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ListParams {
    /// Filter by source tag (`user`, `wg-driver`). Missing => all rows.
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateBody {
    table: String,
    chain: String,
    spec: String,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    priority: i32,
    /// Bypass the SSH guard. UI sets this only after the user confirms.
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct VerifyBody {
    table: String,
    chain: String,
    spec: String,
    #[serde(default)]
    force: bool,
}

// ---------- handlers ----------

async fn list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<RuleDto>>, ApiError> {
    let mgr = manager(&state)?;
    let source = match params.source.as_deref() {
        None => None,
        Some(tag) => Some(
            Source::from_tag(tag)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown source tag: {tag}")))?,
        ),
    };
    let rows = mgr.list(ListFilter { source }).await?;
    Ok(Json(rows.into_iter().map(RuleDto::from).collect()))
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<RuleDto>), ApiError> {
    let mgr = manager(&state)?;
    let spec = RuleSpec {
        table: body.table,
        chain: body.chain,
        spec: body.spec,
        comment: body.comment,
        priority: body.priority,
    };
    let opts = RegisterOptions { force: body.force };
    let mut rows = mgr.register(Source::User, vec![spec], opts).await?;
    let row = rows.pop().ok_or_else(|| ApiError::Internal)?;
    Ok((StatusCode::CREATED, Json(RuleDto::from(row))))
}

async fn delete_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mgr = manager(&state)?;
    mgr.remove_user_rule(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct VerifyOk {
    ok: bool,
}

async fn verify(
    State(state): State<Arc<AppState>>,
    Json(body): Json<VerifyBody>,
) -> Result<Json<VerifyOk>, ApiError> {
    let mgr = manager(&state)?;
    let spec = RuleSpec::new(body.table, body.chain, body.spec);
    let opts = RegisterOptions { force: body.force };
    mgr.verify(&spec, opts).await?;
    Ok(Json(VerifyOk { ok: true }))
}

async fn reconcile(State(state): State<Arc<AppState>>) -> Result<Json<ReconcileReport>, ApiError> {
    let mgr = manager(&state)?;
    let report = mgr.reconcile().await?;
    Ok(Json(report))
}
