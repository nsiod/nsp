//! Database layer: sqlx SQLite pool, migrations, and repositories.

#![forbid(unsafe_code)]

use std::path::Path;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};
use thiserror::Error;

pub mod iptables;
pub mod repo;
pub mod settings;
pub mod traffic;

pub use iptables::{IptablesRepo, IptablesRuleInsert, IptablesRuleRow};
pub use repo::{
    AuditLogRow, AuditRepo, ServerConfigRepo, SsRepo, SsUserRow, UserRow, UsersRepo, WgPeerInsert,
    WgPeerRow, WgRepo,
};
pub use settings::{SettingsPatch, SettingsRepo, SettingsRow};
pub use traffic::{
    bucket_for, RecordOutcome, WgTrafficRepo, WgTrafficSample, WgTrafficSummary,
    TRAFFIC_BUCKET_SECS,
};

pub type Pool = SqlitePool;

pub type Result<T> = std::result::Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("not found")]
    NotFound,

    #[error("invalid state: {0}")]
    Invalid(String),
}

/// Open (or create) the sqlite database at `path`, enable WAL, and run all
/// embedded migrations.
pub async fn open(path: &Path) -> Result<Pool> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;

    run_migrations(&pool).await?;
    Ok(pool)
}

/// Apply all migrations in `migrations/` (embedded at compile time).
pub async fn run_migrations(pool: &Pool) -> Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_tables() {
        let dir = tempfile_dir();
        let path = dir.join("t.db");
        let pool = open(&path).await.expect("open db");
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'",
        )
        .fetch_one(&pool)
        .await
        .expect("query");
        assert_eq!(row.0, 1);
        drop(pool);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nsp-db-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
