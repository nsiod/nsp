//! Typed errors surfaced by the proxy driver.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("server config: {0}")]
    Config(String),

    #[error("task: {0}")]
    Task(String),

    #[error("driver not running")]
    NotRunning,

    #[error("user not found")]
    NotFound,

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("database: {0}")]
    Db(#[from] nsp_db::DbError),

    #[error("core: {0}")]
    Core(#[from] nsp_core::CoreError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
