//! Repository for the unified `iptables_rules` table.
//!
//! The netctl crate owns the trait/business logic; this module only exposes
//! typed SELECT/INSERT/DELETE primitives so the `nsp-db` crate remains the
//! single boundary between SQLite and the rest of the tree.

use crate::{Pool, Result};

/// Raw row as persisted. Stable across netctl refactors because migrations
/// are append-only.
#[derive(Debug, Clone)]
pub struct IptablesRuleRow {
    pub id: String,
    pub source: String,
    pub priority: i32,
    pub table: String,
    pub chain: String,
    pub spec: String,
    pub comment: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input for `IptablesRepo::insert`. Timestamps are filled in by the repo.
#[derive(Debug, Clone)]
pub struct IptablesRuleInsert {
    pub id: String,
    pub source: String,
    pub priority: i32,
    pub table: String,
    pub chain: String,
    pub spec: String,
    pub comment: Option<String>,
}

type IptablesTuple = (
    String,         // id
    String,         // source
    i32,            // priority
    String,         // table
    String,         // chain
    String,         // spec
    Option<String>, // comment
    i64,            // created_at
    i64,            // updated_at
);

fn tuple_to_row(t: IptablesTuple) -> IptablesRuleRow {
    let (id, source, priority, table, chain, spec, comment, created_at, updated_at) = t;
    IptablesRuleRow {
        id,
        source,
        priority,
        table,
        chain,
        spec,
        comment,
        created_at,
        updated_at,
    }
}

pub struct IptablesRepo<'a> {
    pub pool: &'a Pool,
}

impl<'a> IptablesRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    /// Insert a new rule. Returns the stored row with timestamps populated.
    pub async fn insert(&self, insert: IptablesRuleInsert) -> Result<IptablesRuleRow> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO iptables_rules(
                id, source, priority, \"table\", chain, spec, comment, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&insert.id)
        .bind(&insert.source)
        .bind(insert.priority)
        .bind(&insert.table)
        .bind(&insert.chain)
        .bind(&insert.spec)
        .bind(insert.comment.as_deref())
        .bind(now)
        .bind(now)
        .execute(self.pool)
        .await?;

        Ok(IptablesRuleRow {
            id: insert.id,
            source: insert.source,
            priority: insert.priority,
            table: insert.table,
            chain: insert.chain,
            spec: insert.spec,
            comment: insert.comment,
            created_at: now,
            updated_at: now,
        })
    }

    /// Load by primary key.
    pub async fn get(&self, id: &str) -> Result<Option<IptablesRuleRow>> {
        let row: Option<IptablesTuple> = sqlx::query_as(
            "SELECT id, source, priority, \"table\", chain, spec, comment, created_at, updated_at
               FROM iptables_rules
              WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(tuple_to_row))
    }

    /// List persisted rules, optionally filtered by `source` tag (e.g. `user`
    /// or `wg-driver`). Ordered by (priority asc, created_at asc, id asc) so
    /// callers see a stable enumeration regardless of insertion jitter.
    pub async fn list(&self, source: Option<&str>) -> Result<Vec<IptablesRuleRow>> {
        let rows: Vec<IptablesTuple> = if let Some(tag) = source {
            sqlx::query_as(
                "SELECT id, source, priority, \"table\", chain, spec, comment, created_at, updated_at
                   FROM iptables_rules
                  WHERE source = ?
                  ORDER BY priority ASC, created_at ASC, id ASC",
            )
            .bind(tag)
            .fetch_all(self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT id, source, priority, \"table\", chain, spec, comment, created_at, updated_at
                   FROM iptables_rules
                  ORDER BY priority ASC, created_at ASC, id ASC",
            )
            .fetch_all(self.pool)
            .await?
        };
        Ok(rows.into_iter().map(tuple_to_row).collect())
    }

    /// Delete a rule by id. Returns `true` when a row was removed.
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM iptables_rules WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn pool() -> Pool {
        let dir = std::env::temp_dir().join(format!(
            "nsp-db-iptables-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::open(&dir.join("t.db")).await.unwrap()
    }

    fn sample_insert(id: &str, source: &str) -> IptablesRuleInsert {
        IptablesRuleInsert {
            id: id.to_owned(),
            source: source.to_owned(),
            priority: 0,
            table: "filter".into(),
            chain: "INPUT".into(),
            spec: "-p tcp --dport 80 -j ACCEPT".into(),
            comment: Some("sample".into()),
        }
    }

    #[tokio::test]
    async fn insert_get_delete_round_trip() {
        let pool = pool().await;
        let repo = IptablesRepo::new(&pool);
        let id = Uuid::now_v7().to_string();

        let row = repo.insert(sample_insert(&id, "user")).await.unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.source, "user");
        assert!(row.created_at > 0);

        let fetched = repo.get(&id).await.unwrap().expect("row present");
        assert_eq!(fetched.spec, "-p tcp --dport 80 -j ACCEPT");

        assert!(repo.delete(&id).await.unwrap());
        assert!(repo.get(&id).await.unwrap().is_none());
        assert!(!repo.delete(&id).await.unwrap());
    }

    #[tokio::test]
    async fn list_filter_by_source_tag() {
        let pool = pool().await;
        let repo = IptablesRepo::new(&pool);
        let user_id = Uuid::now_v7().to_string();
        let wg_id = Uuid::now_v7().to_string();
        repo.insert(sample_insert(&user_id, "user")).await.unwrap();
        repo.insert(sample_insert(&wg_id, "wg-driver"))
            .await
            .unwrap();

        let only_user = repo.list(Some("user")).await.unwrap();
        assert_eq!(only_user.len(), 1);
        assert_eq!(only_user[0].id, user_id);

        let only_wg = repo.list(Some("wg-driver")).await.unwrap();
        assert_eq!(only_wg.len(), 1);
        assert_eq!(only_wg[0].id, wg_id);

        let all = repo.list(None).await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
