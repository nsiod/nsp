//! Unified iptables rule manager.
//!
//! `IptablesManager` is the trait every call-site depends on. `DefaultManager`
//! is the only production implementation: it persists rules in SQLite via
//! `IptablesRepo`, applies them to the kernel via an `IptablesBackend`, and
//! enforces the SSH-guard policy.

use std::sync::Arc;

use async_trait::async_trait;
use nsp_db::{IptablesRepo, IptablesRuleInsert, IptablesRuleRow, Pool};
use uuid::Uuid;

use crate::backend::{IptablesBackend, LiveRule};
use crate::error::{NetctlError, Result};
use crate::model::{ListFilter, ReconcileReport, RuleSpec, Source, StoredRule};
use crate::ssh_guard::SshGuard;

/// Options for a single `register` call.
#[derive(Debug, Clone, Default)]
pub struct RegisterOptions {
    /// When `true`, bypass the SSH-guard block. Only the API sets this.
    pub force: bool,
}

#[async_trait]
pub trait IptablesManager: Send + Sync {
    /// Persist and install a batch of rules from a single source. Returns
    /// the stored rows in insertion order.
    async fn register(
        &self,
        source: Source,
        rules: Vec<RuleSpec>,
        opts: RegisterOptions,
    ) -> Result<Vec<StoredRule>>;

    /// Remove every rule owned by `source`. Returns the number of rules
    /// successfully dropped (both from DB and from the kernel).
    async fn remove_by_source(&self, source: Source) -> Result<usize>;

    /// Remove one rule by id. 403 when the owning source != `User`; used by
    /// the API's DELETE handler.
    async fn remove_user_rule(&self, id: &str) -> Result<()>;

    /// Remove one rule by id, but only when its owning source is
    /// `Control`. Used by the control-center poller's declarative
    /// reconciler when a previously-installed rule is no longer in
    /// the snapshot. Returns `Forbidden` when the rule is owned by a
    /// different source so a malformed snapshot can never reach into
    /// `User` / `WgDriver` rules.
    async fn remove_control_rule(&self, id: &str) -> Result<()>;

    /// Enumerate persisted rules.
    async fn list(&self, filter: ListFilter) -> Result<Vec<StoredRule>>;

    /// Dry-run validator. Shells `iptables -C` without touching persistent
    /// state. Also runs the SSH guard unless `force` is set.
    async fn verify(&self, spec: &RuleSpec, opts: RegisterOptions) -> Result<()>;

    /// Reconcile persisted rules with the live ruleset. Re-inserts any
    /// persisted rule that's missing, and prunes any `nsp:*` rule that no
    /// longer has a DB row.
    async fn reconcile(&self) -> Result<ReconcileReport>;
}

pub struct DefaultManager {
    backend: Arc<dyn IptablesBackend>,
    pool: Pool,
    guard: SshGuard,
}

impl DefaultManager {
    pub fn new(backend: Arc<dyn IptablesBackend>, pool: Pool) -> Self {
        Self {
            backend,
            pool,
            guard: SshGuard::default(),
        }
    }

    pub fn with_guard(mut self, guard: SshGuard) -> Self {
        self.guard = guard;
        self
    }

    fn repo(&self) -> IptablesRepo<'_> {
        IptablesRepo::new(&self.pool)
    }

    /// Validate + tokenize a rule. Comment metadata (`-m comment --comment
    /// nsp:<src>:<id>`) is appended by the caller — this only checks that
    /// the raw spec is shell-safe.
    fn validate_tokens(spec: &RuleSpec) -> Result<Vec<String>> {
        if spec.table.trim().is_empty() {
            return Err(NetctlError::Invalid("table is empty".into()));
        }
        if spec.chain.trim().is_empty() {
            return Err(NetctlError::Invalid("chain is empty".into()));
        }
        if spec.spec.trim().is_empty() {
            return Err(NetctlError::Invalid("spec is empty".into()));
        }
        for tok in spec.spec.split_whitespace() {
            if tok.contains(';') || tok.contains('|') || tok.contains('`') || tok.contains('$') {
                return Err(NetctlError::Invalid(format!(
                    "shell metacharacter in spec token: {tok}"
                )));
            }
        }
        Ok(spec.spec.split_whitespace().map(str::to_owned).collect())
    }

    fn tokens_with_comment(spec_tokens: &[String], tag: &str) -> Vec<String> {
        let mut out = spec_tokens.to_vec();
        out.push("-m".to_owned());
        out.push("comment".to_owned());
        out.push("--comment".to_owned());
        out.push(tag.to_owned());
        out
    }

    async fn install_rule(&self, row: &StoredRule, spec_tokens: &[String]) -> Result<()> {
        let tag = row.comment_tag();
        let full = Self::tokens_with_comment(spec_tokens, &tag);
        self.backend.append(&row.table, &row.chain, &full).await
    }

    /// Shared implementation of the per-id deletion guarded by
    /// `expected_source`. Returns `Forbidden` when the row exists but
    /// is owned by another source, so handlers can never accidentally
    /// reach across source boundaries.
    async fn remove_one_by_id(&self, id: &str, expected_source: Source) -> Result<()> {
        let row = self
            .repo()
            .get(id)
            .await?
            .ok_or_else(|| NetctlError::NotFound(id.to_owned()))?;
        let stored = row_to_stored(row)?;
        if stored.source != expected_source {
            return Err(NetctlError::Forbidden(format!(
                "rule {} is owned by {} and cannot be deleted as {}",
                stored.id,
                stored.source.as_tag(),
                expected_source.as_tag()
            )));
        }
        self.uninstall_rule(&stored).await?;
        if !self.repo().delete(&stored.id).await? {
            return Err(NetctlError::NotFound(stored.id));
        }
        Ok(())
    }

    async fn uninstall_rule(&self, row: &StoredRule) -> Result<()> {
        let spec_tokens: Vec<String> = row.spec.split_whitespace().map(str::to_owned).collect();
        let tag = row.comment_tag();
        let full = Self::tokens_with_comment(&spec_tokens, &tag);
        match self.backend.delete(&row.table, &row.chain, &full).await {
            Ok(()) => Ok(()),
            // Missing rule is not fatal for removal.
            Err(NetctlError::Rejected(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl IptablesManager for DefaultManager {
    #[tracing::instrument(skip(self, rules), fields(src = source.as_tag(), n = rules.len()))]
    async fn register(
        &self,
        source: Source,
        rules: Vec<RuleSpec>,
        opts: RegisterOptions,
    ) -> Result<Vec<StoredRule>> {
        let mut out = Vec::with_capacity(rules.len());
        for spec in rules {
            let tokens = Self::validate_tokens(&spec)?;
            if source == Source::User && !opts.force {
                if let Some(reason) = self.guard.evaluate(&spec) {
                    return Err(NetctlError::SshGuard(reason));
                }
            }
            let id = Uuid::now_v7().to_string();
            let insert = IptablesRuleInsert {
                id: id.clone(),
                source: source.as_tag().to_owned(),
                priority: spec.priority,
                table: spec.table.clone(),
                chain: spec.chain.clone(),
                spec: spec.spec.clone(),
                comment: spec.comment.clone(),
            };
            let row = self.repo().insert(insert).await?;
            let stored = row_to_stored(row)?;
            // Install after persisting so a partial kernel insert still has a
            // DB record to reconcile against. If the kernel rejects the rule,
            // roll the row back.
            if let Err(e) = self.install_rule(&stored, &tokens).await {
                let _ = self.repo().delete(&stored.id).await;
                return Err(e);
            }
            out.push(stored);
        }
        Ok(out)
    }

    #[tracing::instrument(skip(self), fields(src = source.as_tag()))]
    async fn remove_by_source(&self, source: Source) -> Result<usize> {
        let rows = self
            .repo()
            .list(Some(source.as_tag()))
            .await?
            .into_iter()
            .map(row_to_stored)
            .collect::<Result<Vec<_>>>()?;
        let mut removed = 0;
        for row in rows {
            self.uninstall_rule(&row).await?;
            if self.repo().delete(&row.id).await? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    #[tracing::instrument(skip(self))]
    async fn remove_user_rule(&self, id: &str) -> Result<()> {
        self.remove_one_by_id(id, Source::User).await
    }

    #[tracing::instrument(skip(self))]
    async fn remove_control_rule(&self, id: &str) -> Result<()> {
        self.remove_one_by_id(id, Source::Control).await
    }

    async fn list(&self, filter: ListFilter) -> Result<Vec<StoredRule>> {
        let tag = filter.source.map(|s| s.as_tag());
        let rows = self.repo().list(tag).await?;
        rows.into_iter().map(row_to_stored).collect()
    }

    async fn verify(&self, spec: &RuleSpec, opts: RegisterOptions) -> Result<()> {
        let tokens = Self::validate_tokens(spec)?;
        if !opts.force {
            if let Some(reason) = self.guard.evaluate(spec) {
                return Err(NetctlError::SshGuard(reason));
            }
        }
        // Shell `iptables -C` and interpret the result. "Rule present" and
        // "rule absent but syntactically valid" are both successes here.
        match self.backend.check(&spec.table, &spec.chain, &tokens).await {
            Ok(_present) => Ok(()),
            Err(NetctlError::Rejected(msg)) => {
                if msg.to_lowercase().contains("no such")
                    || msg.to_lowercase().contains("does a matching rule exist")
                {
                    Ok(())
                } else {
                    Err(NetctlError::Rejected(msg))
                }
            }
            Err(e) => Err(e),
        }
    }

    #[tracing::instrument(skip(self))]
    async fn reconcile(&self) -> Result<ReconcileReport> {
        let mut report = ReconcileReport::default();
        let persisted = self
            .repo()
            .list(None)
            .await?
            .into_iter()
            .map(row_to_stored)
            .collect::<Result<Vec<_>>>()?;
        let live = self.backend.list_all().await?;

        // 1. Re-insert missing rules.
        for row in &persisted {
            let tokens: Vec<String> = row.spec.split_whitespace().map(str::to_owned).collect();
            let tag = row.comment_tag();
            if live_contains(&live, row, &tag) {
                report.kept += 1;
                continue;
            }
            self.install_rule(row, &tokens).await?;
            report.reinserted += 1;
        }

        // 2. Prune stale nsp:* rules with no matching DB row.
        let known_tags: std::collections::HashSet<String> =
            persisted.iter().map(StoredRule::comment_tag).collect();
        for live_rule in &live {
            let Some(tag) = live_rule.comment_tag.as_ref() else {
                continue;
            };
            if !tag.starts_with("nsp:") || known_tags.contains(tag) {
                continue;
            }
            let spec_tokens: Vec<String> = live_rule
                .spec
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            if let Err(e) = self
                .backend
                .delete(&live_rule.table, &live_rule.chain, &spec_tokens)
                .await
            {
                tracing::warn!(target: "nsp::netctl", tag = %tag, error = %e, "failed to prune stale nsp rule");
                continue;
            }
            report.pruned += 1;
        }

        Ok(report)
    }
}

fn live_contains(live: &[LiveRule], row: &StoredRule, tag: &str) -> bool {
    live.iter().any(|r| {
        r.table == row.table && r.chain == row.chain && r.comment_tag.as_deref() == Some(tag)
    })
}

fn row_to_stored(row: IptablesRuleRow) -> Result<StoredRule> {
    let source = Source::from_tag(&row.source).ok_or_else(|| {
        NetctlError::Invalid(format!("unknown iptables rule source: {}", row.source))
    })?;
    Ok(StoredRule {
        id: row.id,
        source,
        priority: row.priority,
        table: row.table,
        chain: row.chain,
        spec: row.spec,
        comment: row.comment,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
