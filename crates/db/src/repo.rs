//! Repository stubs. Each struct wraps a `&Pool` borrow for per-request use.

use crate::Pool;

pub struct UsersRepo<'a> {
    pub pool: &'a Pool,
}

pub struct SsRepo<'a> {
    pub pool: &'a Pool,
}

pub struct WgRepo<'a> {
    pub pool: &'a Pool,
}

pub struct ServerConfigRepo<'a> {
    pub pool: &'a Pool,
}

pub struct AuditRepo<'a> {
    pub pool: &'a Pool,
}

impl<'a> ServerConfigRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    /// Fetch a config value by key. Returns `None` when absent.
    pub async fn get(&self, key: &str) -> crate::Result<Option<Vec<u8>>> {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT value FROM server_config WHERE key = ?")
                .bind(key)
                .fetch_optional(self.pool)
                .await?;
        Ok(row.map(|(v,)| v))
    }

    /// Upsert a config value.
    pub async fn set(&self, key: &str, value: &[u8]) -> crate::Result<()> {
        sqlx::query(
            "INSERT INTO server_config(key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

/// A user row decoupled from any protocol enablement. Consumers join
/// with `ss_credentials` / `wg_peers` separately via the per-protocol
/// repos when they need the encrypted key material.
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub ss_enabled: bool,
    pub wg_enabled: bool,
    pub note: Option<String>,
}

type UserTuple = (String, String, i64, i64, i64, Option<String>);

fn user_row_from_tuple(t: UserTuple) -> UserRow {
    let (id, name, created_at, ss_enabled, wg_enabled, note) = t;
    UserRow {
        id,
        name,
        created_at,
        ss_enabled: ss_enabled != 0,
        wg_enabled: wg_enabled != 0,
        note,
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => {
            let code = db_err.code();
            matches!(code.as_deref(), Some("2067") | Some("1555") | Some("23000"))
        }
        _ => false,
    }
}

impl<'a> UsersRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    pub async fn count(&self) -> crate::Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(self.pool)
            .await?;
        Ok(n)
    }

    /// Insert a naked user (both protocol flags 0). Returns
    /// `Err(DbError::Invalid)` when `name` conflicts with an existing
    /// row.
    pub async fn create(&self, id: &str, name: &str, note: Option<&str>) -> crate::Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO users(id, name, created_at, ss_enabled, wg_enabled, note)
             VALUES (?, ?, ?, 0, 0, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(now)
        .bind(note)
        .execute(self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                crate::DbError::Invalid(format!("user name already exists: {name}"))
            } else {
                crate::DbError::Sqlx(e)
            }
        })?;
        Ok(())
    }

    /// Fetch a single user row by id.
    pub async fn get(&self, id: &str) -> crate::Result<Option<UserRow>> {
        let row: Option<UserTuple> = sqlx::query_as(
            "SELECT id, name, created_at, ss_enabled, wg_enabled, note
               FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(user_row_from_tuple))
    }

    /// List every user in creation order.
    pub async fn list(&self) -> crate::Result<Vec<UserRow>> {
        let rows: Vec<UserTuple> = sqlx::query_as(
            "SELECT id, name, created_at, ss_enabled, wg_enabled, note
               FROM users
              ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(user_row_from_tuple).collect())
    }

    /// Rename a user. Returns `true` when a row was updated.
    /// `Err(DbError::Invalid)` on name collisions.
    pub async fn rename(&self, id: &str, new_name: &str) -> crate::Result<bool> {
        let result = sqlx::query("UPDATE users SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(id)
            .execute(self.pool)
            .await
            .map_err(|e| {
                if is_unique_violation(&e) {
                    crate::DbError::Invalid(format!("user name already exists: {new_name}"))
                } else {
                    crate::DbError::Sqlx(e)
                }
            })?;
        Ok(result.rows_affected() > 0)
    }

    /// Replace the free-form note on a user row.
    pub async fn update_note(&self, id: &str, note: Option<&str>) -> crate::Result<bool> {
        let result = sqlx::query("UPDATE users SET note = ? WHERE id = ?")
            .bind(note)
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete a user. Cascades to `ss_credentials` and `wg_peers` via
    /// the `ON DELETE CASCADE` foreign keys declared in the schema.
    pub async fn delete(&self, id: &str) -> crate::Result<bool> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

impl<'a> AuditRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, limit: i64) -> crate::Result<Vec<AuditLogRow>> {
        let limit = limit.clamp(1, 500);
        let rows: Vec<AuditLogTuple> = sqlx::query_as(
            "SELECT id, ts, actor, action, target, detail
               FROM audit_log
              ORDER BY ts DESC, id DESC
              LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(audit_row_from_tuple).collect())
    }

    pub async fn append(
        &self,
        actor: &str,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> crate::Result<()> {
        let ts = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO audit_log(ts, actor, action, target, detail) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(ts)
        .bind(actor)
        .bind(action)
        .bind(target)
        .bind(detail)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AuditLogRow {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
}

type AuditLogTuple = (i64, i64, String, String, Option<String>, Option<String>);

fn audit_row_from_tuple(t: AuditLogTuple) -> AuditLogRow {
    AuditLogRow {
        id: t.0,
        ts: t.1,
        actor: t.2,
        action: t.3,
        target: t.4,
        detail: t.5,
    }
}

impl<'a> SsRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    /// Create a user with an SS credential in a single transaction. Returns
    /// `Err(DbError::Invalid)` if `name` already exists.
    pub async fn create_user(
        &self,
        id: &str,
        name: &str,
        psk_enc: &[u8],
        note: Option<&str>,
    ) -> crate::Result<()> {
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO users(id, name, created_at, ss_enabled, wg_enabled, note)
             VALUES (?, ?, ?, 1, 0, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(now)
        .bind(note)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO ss_credentials(user_id, psk_enc, updated_at) VALUES (?, ?, ?)")
            .bind(id)
            .bind(psk_enc)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Delete a user and the associated SS credential.
    pub async fn delete_user(&self, id: &str) -> crate::Result<bool> {
        let result = sqlx::query("DELETE FROM users WHERE id = ? AND ss_enabled = 1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Replace the stored PSK for an existing user.
    pub async fn update_psk(&self, id: &str, psk_enc: &[u8]) -> crate::Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let result =
            sqlx::query("UPDATE ss_credentials SET psk_enc = ?, updated_at = ? WHERE user_id = ?")
                .bind(psk_enc)
                .bind(now)
                .bind(id)
                .execute(self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Fetch a single user row joined with its SS credential.
    pub async fn get_user(&self, id: &str) -> crate::Result<Option<SsUserRow>> {
        let row: Option<SsUserTuple> = sqlx::query_as(
            "SELECT u.id, u.name, c.psk_enc, u.created_at, u.note
             FROM users u
             INNER JOIN ss_credentials c ON c.user_id = u.id
             WHERE u.id = ? AND u.ss_enabled = 1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(ss_user_row_from_tuple))
    }

    /// List every SS-enabled user in creation order.
    pub async fn list_users(&self) -> crate::Result<Vec<SsUserRow>> {
        let rows: Vec<SsUserTuple> = sqlx::query_as(
            "SELECT u.id, u.name, c.psk_enc, u.created_at, u.note
             FROM users u
             INNER JOIN ss_credentials c ON c.user_id = u.id
             WHERE u.ss_enabled = 1
             ORDER BY u.created_at ASC, u.id ASC",
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(ss_user_row_from_tuple).collect())
    }

    /// Enable SS for an existing user: upsert the credential row and
    /// flip `users.ss_enabled = 1` atomically. Requires that `user_id`
    /// already exists in `users`.
    pub async fn enable_user(&self, user_id: &str, psk_enc: &[u8]) -> crate::Result<()> {
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(crate::DbError::NotFound);
        }
        sqlx::query(
            "INSERT INTO ss_credentials(user_id, psk_enc, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(user_id) DO UPDATE SET
                psk_enc = excluded.psk_enc,
                updated_at = excluded.updated_at",
        )
        .bind(user_id)
        .bind(psk_enc)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE users SET ss_enabled = 1 WHERE id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Disable SS for a user: delete the credential row and clear the
    /// `users.ss_enabled` flag. Returns `true` when the user existed
    /// and was updated.
    pub async fn disable_user(&self, user_id: &str) -> crate::Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM ss_credentials WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("UPDATE users SET ss_enabled = 0 WHERE id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }
}

type SsUserTuple = (String, String, Vec<u8>, i64, Option<String>);

fn ss_user_row_from_tuple(t: SsUserTuple) -> SsUserRow {
    let (id, name, psk_enc, created_at, note) = t;
    SsUserRow {
        id,
        name,
        psk_enc,
        created_at,
        note,
    }
}

/// A single SS-enabled user joined with its encrypted PSK blob. Consumers
/// decrypt the blob at the edge so the DB layer never sees plaintext keys.
#[derive(Debug, Clone)]
pub struct SsUserRow {
    pub id: String,
    pub name: String,
    pub psk_enc: Vec<u8>,
    pub created_at: i64,
    pub note: Option<String>,
}

/// Persisted WireGuard peer row. The client-side private key is never
/// persisted — it is returned once on enable/rotate and then discarded.
#[derive(Debug, Clone)]
pub struct WgPeerRow {
    pub id: String,
    pub user_id: Option<String>,
    pub name: Option<String>,
    pub public_key: [u8; 32],
    pub preshared_key_enc: Option<Vec<u8>>,
    pub allowed_ip: String,
    pub endpoint: Option<String>,
    pub keepalive: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input for an insert / rotate. The PSK blob, when present, is the
/// encrypted form produced by `core::crypto::DataKey::seal`.
#[derive(Debug, Clone)]
pub struct WgPeerInsert {
    pub id: String,
    pub user_id: Option<String>,
    pub name: Option<String>,
    pub public_key: [u8; 32],
    pub preshared_key_enc: Option<Vec<u8>>,
    pub allowed_ip: String,
    pub endpoint: Option<String>,
    pub keepalive: Option<i64>,
}

/// Raw tuple shape returned by the `wg_peers` select queries. Kept as a
/// named alias to keep the generic bounds on `sqlx::query_as` tolerable and
/// to satisfy `clippy::type_complexity`.
type WgPeerTuple = (
    String,          // id
    Option<String>,  // user_id
    Option<String>,  // name
    Vec<u8>,         // public_key
    Option<Vec<u8>>, // preshared_key_enc
    String,          // allowed_ip
    Option<String>,  // endpoint
    Option<i64>,     // keepalive
    i64,             // created_at
    i64,             // updated_at
);

impl<'a> WgRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    /// List every persisted peer in creation order.
    pub async fn list(&self) -> crate::Result<Vec<WgPeerRow>> {
        let rows = sqlx::query_as::<_, WgPeerTuple>(
            "SELECT id, user_id, name, public_key,
                    preshared_key_enc, allowed_ip, endpoint, keepalive,
                    created_at, updated_at
               FROM wg_peers
              ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(self.pool)
        .await?;

        rows.into_iter().map(row_into_peer).collect()
    }

    /// Load a peer by its id.
    pub async fn get(&self, id: &str) -> crate::Result<Option<WgPeerRow>> {
        let row = sqlx::query_as::<_, WgPeerTuple>(
            "SELECT id, user_id, name, public_key,
                    preshared_key_enc, allowed_ip, endpoint, keepalive,
                    created_at, updated_at
               FROM wg_peers
              WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;

        row.map(row_into_peer).transpose()
    }

    /// Insert a new peer. Returns the stored row (with timestamps populated).
    pub async fn insert(&self, peer: WgPeerInsert) -> crate::Result<WgPeerRow> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO wg_peers(
                id, user_id, name, public_key,
                preshared_key_enc, allowed_ip, endpoint, keepalive,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&peer.id)
        .bind(peer.user_id.as_deref())
        .bind(peer.name.as_deref())
        .bind(peer.public_key.as_slice())
        .bind(peer.preshared_key_enc.as_deref())
        .bind(&peer.allowed_ip)
        .bind(peer.endpoint.as_deref())
        .bind(peer.keepalive)
        .bind(now)
        .bind(now)
        .execute(self.pool)
        .await?;

        Ok(WgPeerRow {
            id: peer.id,
            user_id: peer.user_id,
            name: peer.name,
            public_key: peer.public_key,
            preshared_key_enc: peer.preshared_key_enc,
            allowed_ip: peer.allowed_ip,
            endpoint: peer.endpoint,
            keepalive: peer.keepalive,
            created_at: now,
            updated_at: now,
        })
    }

    /// Replace the keypair of an existing peer (used for rotation). Returns
    /// the new row or `None` when `id` is unknown. Client private keys are
    /// never persisted, so rotation only updates the stored public half.
    pub async fn rotate_keys(
        &self,
        id: &str,
        new_public_key: [u8; 32],
        new_preshared_key_enc: Option<Vec<u8>>,
    ) -> crate::Result<Option<WgPeerRow>> {
        let now = chrono::Utc::now().timestamp();
        let result = sqlx::query(
            "UPDATE wg_peers
                SET public_key = ?,
                    preshared_key_enc = ?,
                    updated_at = ?
              WHERE id = ?",
        )
        .bind(new_public_key.as_slice())
        .bind(new_preshared_key_enc.as_deref())
        .bind(now)
        .bind(id)
        .execute(self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get(id).await
    }

    /// Delete a peer by id. Returns `true` if a row was removed.
    pub async fn delete(&self, id: &str) -> crate::Result<bool> {
        let result = sqlx::query("DELETE FROM wg_peers WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Fetch the peer row attached to `user_id`, if any. A user can
    /// have at most one WG peer.
    pub async fn get_by_user(&self, user_id: &str) -> crate::Result<Option<WgPeerRow>> {
        let row = sqlx::query_as::<_, WgPeerTuple>(
            "SELECT id, user_id, name, public_key,
                    preshared_key_enc, allowed_ip, endpoint, keepalive,
                    created_at, updated_at
               FROM wg_peers
              WHERE user_id = ?
              LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(self.pool)
        .await?;

        row.map(row_into_peer).transpose()
    }

    /// Enable WG for `user_id` by inserting the given peer row and
    /// flipping `users.wg_enabled = 1` atomically. `insert.user_id`
    /// must equal `user_id`.
    pub async fn enable_user(
        &self,
        user_id: &str,
        insert: WgPeerInsert,
    ) -> crate::Result<WgPeerRow> {
        if insert.user_id.as_deref() != Some(user_id) {
            return Err(crate::DbError::Invalid(
                "WgPeerInsert.user_id must match enable_user user_id".into(),
            ));
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(crate::DbError::NotFound);
        }
        sqlx::query(
            "INSERT INTO wg_peers(
                id, user_id, name, public_key,
                preshared_key_enc, allowed_ip, endpoint, keepalive,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&insert.id)
        .bind(insert.user_id.as_deref())
        .bind(insert.name.as_deref())
        .bind(insert.public_key.as_slice())
        .bind(insert.preshared_key_enc.as_deref())
        .bind(&insert.allowed_ip)
        .bind(insert.endpoint.as_deref())
        .bind(insert.keepalive)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                crate::DbError::Invalid("wg peer conflict (public_key or allowed_ip)".into())
            } else {
                crate::DbError::Sqlx(e)
            }
        })?;
        sqlx::query("UPDATE users SET wg_enabled = 1 WHERE id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(WgPeerRow {
            id: insert.id,
            user_id: insert.user_id,
            name: insert.name,
            public_key: insert.public_key,
            preshared_key_enc: insert.preshared_key_enc,
            allowed_ip: insert.allowed_ip,
            endpoint: insert.endpoint,
            keepalive: insert.keepalive,
            created_at: now,
            updated_at: now,
        })
    }

    /// Disable WG for `user_id`: drop the peer row (if any) and clear
    /// `users.wg_enabled`. Returns `true` when the user existed.
    pub async fn disable_user(&self, user_id: &str) -> crate::Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM wg_peers WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("UPDATE users SET wg_enabled = 0 WHERE id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }
}

fn row_into_peer(row: WgPeerTuple) -> crate::Result<WgPeerRow> {
    let (
        id,
        user_id,
        name,
        public_key,
        preshared_key_enc,
        allowed_ip,
        endpoint,
        keepalive,
        created_at,
        updated_at,
    ) = row;
    let public_key = <[u8; 32]>::try_from(public_key.as_slice())
        .map_err(|_| crate::DbError::Invalid("wg_peers.public_key length".into()))?;
    Ok(WgPeerRow {
        id,
        user_id,
        name,
        public_key,
        preshared_key_enc,
        allowed_ip,
        endpoint,
        keepalive,
        created_at,
        updated_at,
    })
}
