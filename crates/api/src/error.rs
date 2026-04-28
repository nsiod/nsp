//! RFC 7807 problem+json error type for the HTTP API.

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use nsp_core::CoreError;
use nsp_db::DbError;
use nsp_netctl::NetctlError;
use nsp_ss_driver::SsError;
use nsp_wg_driver::WgError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFoundDetail(String),

    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    /// SSH-guard tripped on a user-submitted iptables rule. Returned as HTTP
    /// 409 with a JSON body `{ "code": "ssh-guard", "warn": "..." }` so the
    /// UI can render a dedicated confirmation dialog.
    #[error("ssh guard: {0}")]
    SshGuard(String),

    /// WG subnet change is blocked because peers already hold allowed_ips
    /// outside the proposed subnet. Returned as HTTP 409 with a JSON body
    /// `{ "code": "wg-subnet-conflict", "conflicts": ["peer-id", ...] }`.
    #[error("wg subnet conflict: {} peer(s)", .0.len())]
    SubnetConflict(Vec<String>),

    #[error("service unavailable: {0}")]
    Unavailable(String),

    #[error("rate limited")]
    RateLimited,

    #[error("internal error")]
    Internal,

    #[error(transparent)]
    Core(#[from] CoreError),

    #[error(transparent)]
    Db(DbError),

    #[error(transparent)]
    Ss(#[from] SsError),
}

impl From<DbError> for ApiError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::NotFound => Self::NotFound,
            DbError::Invalid(msg) => Self::BadRequest(msg),
            other => Self::Db(other),
        }
    }
}

impl From<WgError> for ApiError {
    fn from(err: WgError) -> Self {
        match err {
            WgError::NotFound(id) => Self::NotFoundDetail(id),
            WgError::Invalid(msg) => Self::BadRequest(msg),
            WgError::NotStarted => Self::Unavailable("wireguard driver not started".into()),
            WgError::Ipam(e) => Self::Conflict(e.to_string()),
            WgError::Db(e) => Self::Db(e),
            WgError::Core(e) => Self::Core(e),
            other => {
                tracing::error!(error = %other, "wg driver error");
                Self::Internal
            }
        }
    }
}

impl From<NetctlError> for ApiError {
    fn from(err: NetctlError) -> Self {
        match err {
            NetctlError::Invalid(msg) => Self::BadRequest(msg),
            NetctlError::Rejected(msg) => Self::BadRequest(msg),
            NetctlError::NotFound(id) => Self::NotFoundDetail(id),
            NetctlError::Forbidden(msg) => Self::Forbidden(msg),
            NetctlError::SshGuard(msg) => Self::SshGuard(msg),
            NetctlError::Unavailable(msg) => Self::Unavailable(msg),
            NetctlError::Backend(msg) => Self::Unavailable(msg),
            NetctlError::Db(e) => Self::Db(e),
            NetctlError::Io(e) => {
                tracing::error!(error = %e, "netctl io error");
                Self::Internal
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct Problem<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    title: &'a str,
    status: u16,
    detail: String,
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound | Self::NotFoundDetail(_) => StatusCode::NOT_FOUND,
            // SSH guard overloads 409 because the UI needs a retry-with-force
            // affordance; plain Conflict renders the default error dialog.
            Self::Conflict(_) | Self::SshGuard(_) | Self::SubnetConflict(_) => StatusCode::CONFLICT,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Ss(err) => ss_status(err),
            Self::Internal | Self::Core(_) | Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn problem_type(&self) -> &'static str {
        match self {
            Self::Unauthorized => "about:blank#unauthorized",
            Self::BadRequest(_) => "about:blank#bad-request",
            Self::NotFound | Self::NotFoundDetail(_) => "about:blank#not-found",
            Self::Conflict(_) => "about:blank#conflict",
            Self::SubnetConflict(_) => "about:blank#wg-subnet-conflict",
            Self::Forbidden(_) => "about:blank#forbidden",
            Self::SshGuard(_) => "about:blank#ssh-guard",
            Self::Unavailable(_) => "about:blank#unavailable",
            Self::RateLimited => "about:blank#rate-limited",
            Self::Ss(err) => ss_problem_type(err),
            _ => "about:blank#internal",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::Unauthorized => "Unauthorized",
            Self::BadRequest(_) => "Bad Request",
            Self::NotFound | Self::NotFoundDetail(_) => "Not Found",
            Self::Conflict(_) => "Conflict",
            Self::SubnetConflict(_) => "WG Subnet Conflict",
            Self::Forbidden(_) => "Forbidden",
            Self::SshGuard(_) => "SSH Guard",
            Self::Unavailable(_) => "Service Unavailable",
            Self::RateLimited => "Too Many Requests",
            Self::Ss(err) => ss_title(err),
            _ => "Internal Server Error",
        }
    }
}

fn ss_status(err: &SsError) -> StatusCode {
    match err {
        SsError::NotFound => StatusCode::NOT_FOUND,
        SsError::Invalid(_) => StatusCode::BAD_REQUEST,
        SsError::NotRunning => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn ss_problem_type(err: &SsError) -> &'static str {
    match err {
        SsError::NotFound => "about:blank#not-found",
        SsError::Invalid(_) => "about:blank#bad-request",
        SsError::NotRunning => "about:blank#unavailable",
        _ => "about:blank#internal",
    }
}

fn ss_title(err: &SsError) -> &'static str {
    match err {
        SsError::NotFound => "Not Found",
        SsError::Invalid(_) => "Bad Request",
        SsError::NotRunning => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

#[derive(Debug, Serialize)]
struct SshGuardBody<'a> {
    code: &'a str,
    warn: &'a str,
}

#[derive(Debug, Serialize)]
struct SubnetConflictBody<'a> {
    code: &'a str,
    conflicts: &'a [String],
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        if let Self::SshGuard(reason) = &self {
            // Dedicated shape so the UI can match on `{code: "ssh-guard"}`
            // and surface a force-retry affordance rather than the generic
            // error toast.
            let body = SshGuardBody {
                code: "ssh-guard",
                warn: reason.as_str(),
            };
            return (
                status,
                [(header::CONTENT_TYPE, "application/json")],
                Json(body),
            )
                .into_response();
        }
        if let Self::SubnetConflict(conflicts) = &self {
            let body = SubnetConflictBody {
                code: "wg-subnet-conflict",
                conflicts: conflicts.as_slice(),
            };
            return (
                status,
                [(header::CONTENT_TYPE, "application/json")],
                Json(body),
            )
                .into_response();
        }

        let problem = Problem {
            kind: self.problem_type(),
            title: self.title(),
            status: status.as_u16(),
            detail: self.to_string(),
        };
        if matches!(self, Self::Internal | Self::Core(_) | Self::Db(_))
            || matches!(
                self,
                Self::Ss(
                    SsError::Config(_)
                        | SsError::Task(_)
                        | SsError::Core(_)
                        | SsError::Db(_)
                        | SsError::Io(_)
                )
            )
        {
            tracing::error!(error = %self, "internal api error");
        }
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(problem),
        )
            .into_response()
    }
}
