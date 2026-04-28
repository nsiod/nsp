use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("config error: {0}")]
    Config(String),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("not found")]
    NotFound,

    #[error("internal error: {0}")]
    Internal(String),
}
