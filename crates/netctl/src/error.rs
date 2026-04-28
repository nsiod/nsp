//! Error types for the netctl crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, NetctlError>;

#[derive(Debug, Error)]
pub enum NetctlError {
    #[error("iptables binary unavailable: {0}")]
    Unavailable(String),

    #[error("invalid rule: {0}")]
    Invalid(String),

    #[error("iptables rejected rule: {0}")]
    Rejected(String),

    #[error("ssh guard: {0}")]
    SshGuard(String),

    #[error("rule not found: {0}")]
    NotFound(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("backend: {0}")]
    Backend(String),

    #[error(transparent)]
    Db(#[from] nsp_db::DbError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
