//! `settings` singleton row: editable system settings decoupled from
//! driver runtime state.
//!
//! Patch semantics use `Option<Option<T>>` for nullable fields:
//! * `None`       -> don't touch the column
//! * `Some(None)` -> clear the column to NULL
//! * `Some(Some(v))` -> set the column to `v`
//!
//! Non-nullable fields use plain `Option<T>` (`None` = no change).

use crate::Pool;

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub domain: Option<String>,
    pub wg_subnet: Option<String>,
    pub ss_listen_port: i64,
    pub wg_listen_port: i64,
    pub admin_password_hash: Option<String>,
    pub token_generation: i64,
    pub updated_at: i64,
}

type SettingsTuple = (
    Option<String>, // domain
    Option<String>, // wg_subnet
    i64,            // ss_listen_port
    i64,            // wg_listen_port
    Option<String>, // admin_password_hash
    i64,            // token_generation
    i64,            // updated_at
);

fn row_from_tuple(t: SettingsTuple) -> SettingsRow {
    SettingsRow {
        domain: t.0,
        wg_subnet: t.1,
        ss_listen_port: t.2,
        wg_listen_port: t.3,
        admin_password_hash: t.4,
        token_generation: t.5,
        updated_at: t.6,
    }
}

#[derive(Debug, Default, Clone)]
pub struct SettingsPatch {
    pub domain: Option<Option<String>>,
    pub wg_subnet: Option<Option<String>>,
    pub ss_listen_port: Option<i64>,
    pub wg_listen_port: Option<i64>,
    pub admin_password_hash: Option<String>,
}

impl SettingsPatch {
    pub fn is_empty(&self) -> bool {
        self.domain.is_none()
            && self.wg_subnet.is_none()
            && self.ss_listen_port.is_none()
            && self.wg_listen_port.is_none()
            && self.admin_password_hash.is_none()
    }
}

pub struct SettingsRepo<'a> {
    pub pool: &'a Pool,
}

impl<'a> SettingsRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    /// Load the singleton row. The migration seeds a default row so
    /// this never returns `None` in practice; we surface an `Invalid`
    /// error if the row is missing because that indicates a broken
    /// install.
    pub async fn get(&self) -> crate::Result<SettingsRow> {
        let row: Option<SettingsTuple> = sqlx::query_as(
            "SELECT domain, wg_subnet, ss_listen_port, wg_listen_port,
                    admin_password_hash, token_generation, updated_at
               FROM settings WHERE id = 1",
        )
        .fetch_optional(self.pool)
        .await?;
        row.map(row_from_tuple)
            .ok_or_else(|| crate::DbError::Invalid("settings row missing".into()))
    }

    /// Apply `patch` and return the resulting row.
    pub async fn patch(&self, patch: SettingsPatch) -> crate::Result<SettingsRow> {
        if patch.is_empty() {
            return self.get().await;
        }

        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;

        if let Some(v) = patch.domain {
            sqlx::query("UPDATE settings SET domain = ? WHERE id = 1")
                .bind(v)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(v) = patch.wg_subnet {
            sqlx::query("UPDATE settings SET wg_subnet = ? WHERE id = 1")
                .bind(v)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(v) = patch.ss_listen_port {
            sqlx::query("UPDATE settings SET ss_listen_port = ? WHERE id = 1")
                .bind(v)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(v) = patch.wg_listen_port {
            sqlx::query("UPDATE settings SET wg_listen_port = ? WHERE id = 1")
                .bind(v)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(v) = patch.admin_password_hash {
            sqlx::query(
                "UPDATE settings
                    SET admin_password_hash = ?,
                        token_generation = token_generation + 1
                  WHERE id = 1",
            )
            .bind(v)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE settings SET updated_at = ? WHERE id = 1")
            .bind(now)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        self.get().await
    }

    /// Explicitly bump the token generation counter. Handlers call this
    /// for out-of-band session invalidation without changing the password hash.
    pub async fn bump_token_generation(&self) -> crate::Result<i64> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "UPDATE settings
                SET token_generation = token_generation + 1,
                    updated_at = ?
              WHERE id = 1",
        )
        .bind(now)
        .execute(self.pool)
        .await?;
        let (tgen,): (i64,) = sqlx::query_as("SELECT token_generation FROM settings WHERE id = 1")
            .fetch_one(self.pool)
            .await?;
        Ok(tgen)
    }

    /// Read the current token generation without fetching the full row.
    pub async fn token_generation(&self) -> crate::Result<i64> {
        let (tgen,): (i64,) = sqlx::query_as("SELECT token_generation FROM settings WHERE id = 1")
            .fetch_one(self.pool)
            .await?;
        Ok(tgen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open;

    async fn test_pool() -> Pool {
        let dir = std::env::temp_dir().join(format!(
            "nsp-settings-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        open(&dir.join("t.db")).await.expect("open db")
    }

    #[tokio::test]
    async fn defaults_and_round_trip() {
        let pool = test_pool().await;
        let repo = SettingsRepo::new(&pool);
        let row = repo.get().await.unwrap();
        assert_eq!(row.ss_listen_port, 4433);
        assert_eq!(row.wg_listen_port, 51820);
        assert_eq!(row.token_generation, 1);
        assert!(row.domain.is_none());
        assert!(row.wg_subnet.is_none());

        let patched = repo
            .patch(SettingsPatch {
                domain: Some(Some("example.com".into())),
                wg_subnet: Some(Some("10.255.0.0/16".into())),
                ss_listen_port: Some(4500),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(patched.domain.as_deref(), Some("example.com"));
        assert_eq!(patched.wg_subnet.as_deref(), Some("10.255.0.0/16"));
        assert_eq!(patched.ss_listen_port, 4500);
        // unchanged fields
        assert_eq!(patched.wg_listen_port, 51820);
        assert_eq!(patched.token_generation, 1);
    }

    #[tokio::test]
    async fn clear_nullable_fields() {
        let pool = test_pool().await;
        let repo = SettingsRepo::new(&pool);
        repo.patch(SettingsPatch {
            domain: Some(Some("x.test".into())),
            wg_subnet: Some(Some("10.0.0.0/24".into())),
            ..Default::default()
        })
        .await
        .unwrap();
        let row = repo
            .patch(SettingsPatch {
                domain: Some(None),
                wg_subnet: Some(None),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(row.domain.is_none());
        assert!(row.wg_subnet.is_none());
    }

    #[tokio::test]
    async fn password_change_bumps_token_generation() {
        let pool = test_pool().await;
        let repo = SettingsRepo::new(&pool);
        let before = repo.get().await.unwrap().token_generation;
        let after = repo
            .patch(SettingsPatch {
                admin_password_hash: Some("phc".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(after.token_generation, before + 1);
        assert_eq!(after.admin_password_hash.as_deref(), Some("phc"));
    }

    #[tokio::test]
    async fn bump_token_generation_direct() {
        let pool = test_pool().await;
        let repo = SettingsRepo::new(&pool);
        let before = repo.token_generation().await.unwrap();
        let returned = repo.bump_token_generation().await.unwrap();
        assert_eq!(returned, before + 1);
        assert_eq!(repo.token_generation().await.unwrap(), before + 1);
    }
}
