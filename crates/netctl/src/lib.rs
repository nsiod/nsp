//! Unified iptables rule manager for nsp.
//!
//! Every rule on the host is owned by the manager: baseline WireGuard
//! MASQUERADE / FORWARD entries register themselves here, and admin-authored
//! user rules arrive through the HTTP API. The manager persists rules in
//! SQLite (`iptables_rules` table) and applies them via an
//! [`IptablesBackend`] implementation (default: shelling to `iptables`).

#![forbid(unsafe_code)]

pub mod backend;
pub mod error;
pub mod manager;
pub mod model;
pub mod ssh_guard;

pub use backend::{IptablesBackend, LiveRule, ProcessBackend};
pub use error::{NetctlError, Result};
pub use manager::{DefaultManager, IptablesManager, RegisterOptions};
pub use model::{ListFilter, ReconcileReport, RuleSpec, Source, StoredRule};
pub use ssh_guard::SshGuard;

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use crate::backend::LiveRule;

    #[derive(Default)]
    struct MockBackend {
        state: Mutex<MockState>,
    }

    #[derive(Default)]
    struct MockState {
        rules: Vec<LiveRule>,
        /// Controlled test failure mode: when set, the next `append` will fail.
        fail_next_append: Option<String>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self::default()
        }

        #[allow(dead_code)]
        async fn inject_raw(&self, rule: LiveRule) {
            self.state.lock().await.rules.push(rule);
        }

        async fn snapshot(&self) -> Vec<LiveRule> {
            self.state.lock().await.rules.clone()
        }

        async fn arm_append_failure(&self, reason: &str) {
            self.state.lock().await.fail_next_append = Some(reason.to_owned());
        }
    }

    #[async_trait]
    impl IptablesBackend for MockBackend {
        async fn check(&self, table: &str, chain: &str, spec: &[String]) -> Result<bool> {
            let state = self.state.lock().await;
            let joined = spec.join(" ");
            Ok(state
                .rules
                .iter()
                .any(|r| r.table == table && r.chain == chain && r.spec == joined))
        }

        async fn append(&self, table: &str, chain: &str, spec: &[String]) -> Result<()> {
            let mut state = self.state.lock().await;
            if let Some(reason) = state.fail_next_append.take() {
                return Err(NetctlError::Rejected(reason));
            }
            let spec_str = spec.join(" ");
            let tag = extract_tag(&spec_str);
            state.rules.push(LiveRule {
                table: table.to_owned(),
                chain: chain.to_owned(),
                spec: spec_str,
                comment_tag: tag,
            });
            Ok(())
        }

        async fn delete(&self, table: &str, chain: &str, spec: &[String]) -> Result<()> {
            let mut state = self.state.lock().await;
            let joined = spec.join(" ");
            let before = state.rules.len();
            state
                .rules
                .retain(|r| !(r.table == table && r.chain == chain && r.spec == joined));
            if state.rules.len() == before {
                return Err(NetctlError::Rejected("no matching rule".into()));
            }
            Ok(())
        }

        async fn list_all(&self) -> Result<Vec<LiveRule>> {
            Ok(self.state.lock().await.rules.clone())
        }
    }

    fn extract_tag(spec: &str) -> Option<String> {
        let mut iter = spec.split_whitespace().peekable();
        while let Some(tok) = iter.next() {
            if tok == "--comment" {
                if let Some(next) = iter.next() {
                    let trimmed = next.trim_matches('"');
                    if trimmed.starts_with("nsp:") {
                        return Some(trimmed.to_owned());
                    }
                }
            }
        }
        None
    }

    async fn pool() -> nsp_db::Pool {
        let dir = std::env::temp_dir().join(format!(
            "nsp-netctl-test-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        nsp_db::open(&dir.join("t.db")).await.expect("open db")
    }

    fn wg_spec() -> RuleSpec {
        RuleSpec::new(
            "nat",
            "POSTROUTING",
            "-s 10.66.0.0/24 -o eth0 -j MASQUERADE",
        )
    }

    #[tokio::test]
    async fn register_persists_and_installs_rule() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend.clone(), pool);

        let rows = mgr
            .register(Source::WgDriver, vec![wg_spec()], Default::default())
            .await
            .expect("register");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, Source::WgDriver);

        let live = backend.snapshot().await;
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].chain, "POSTROUTING");
        assert_eq!(
            live[0].comment_tag.as_deref(),
            Some(rows[0].comment_tag().as_str())
        );

        let listed = mgr.list(Default::default()).await.expect("list");
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn list_filter_by_source_round_trip() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend, pool);

        mgr.register(Source::WgDriver, vec![wg_spec()], Default::default())
            .await
            .unwrap();
        mgr.register(
            Source::User,
            vec![RuleSpec::new(
                "filter",
                "INPUT",
                "-p tcp --dport 80 -j ACCEPT",
            )],
            Default::default(),
        )
        .await
        .unwrap();

        let only_wg = mgr
            .list(ListFilter {
                source: Some(Source::WgDriver),
            })
            .await
            .unwrap();
        assert_eq!(only_wg.len(), 1);
        assert_eq!(only_wg[0].source, Source::WgDriver);

        let only_user = mgr
            .list(ListFilter {
                source: Some(Source::User),
            })
            .await
            .unwrap();
        assert_eq!(only_user.len(), 1);
        assert_eq!(only_user[0].source, Source::User);
    }

    #[tokio::test]
    async fn remove_by_source_uninstalls_and_deletes() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend.clone(), pool);

        mgr.register(Source::WgDriver, vec![wg_spec()], Default::default())
            .await
            .unwrap();
        assert_eq!(backend.snapshot().await.len(), 1);

        let dropped = mgr.remove_by_source(Source::WgDriver).await.unwrap();
        assert_eq!(dropped, 1);
        assert!(backend.snapshot().await.is_empty());
        let listed = mgr.list(Default::default()).await.unwrap();
        assert!(listed.is_empty());
    }

    #[tokio::test]
    async fn delete_non_user_rule_is_forbidden() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend, pool);

        let rows = mgr
            .register(Source::WgDriver, vec![wg_spec()], Default::default())
            .await
            .unwrap();
        let err = mgr.remove_user_rule(&rows[0].id).await.unwrap_err();
        assert!(matches!(err, NetctlError::Forbidden(_)));
    }

    #[tokio::test]
    async fn ssh_guard_blocks_user_drop_on_port_22() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend, pool);

        let bad = RuleSpec::new("filter", "INPUT", "-p tcp --dport 22 -j DROP");
        let err = mgr
            .register(Source::User, vec![bad.clone()], Default::default())
            .await
            .unwrap_err();
        assert!(matches!(err, NetctlError::SshGuard(_)));

        let forced = RegisterOptions { force: true };
        let rows = mgr.register(Source::User, vec![bad], forced).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn wg_driver_source_bypasses_ssh_guard() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend, pool);

        // Even a DROP-on-22 rule should register when the driver source asks
        // for it: guard policy is user-only to keep internal drivers stable.
        let rule = RuleSpec::new("filter", "INPUT", "-p tcp --dport 22 -j DROP");
        mgr.register(Source::WgDriver, vec![rule], Default::default())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reconcile_reinserts_missing_and_prunes_stale() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend.clone(), pool);

        let rows = mgr
            .register(Source::WgDriver, vec![wg_spec()], Default::default())
            .await
            .unwrap();
        // Simulate a crashed binary: empty live ruleset + a stray nsp tag.
        {
            let mut state = backend.state.lock().await;
            state.rules.clear();
            state.rules.push(LiveRule {
                table: "filter".into(),
                chain: "FORWARD".into(),
                spec: "-i wg0 -j ACCEPT -m comment --comment nsp:wg-driver:stale".into(),
                comment_tag: Some("nsp:wg-driver:stale".into()),
            });
        }

        let report = mgr.reconcile().await.unwrap();
        assert_eq!(report.reinserted, 1);
        assert_eq!(report.pruned, 1);

        let live = backend.snapshot().await;
        assert_eq!(live.len(), 1);
        assert_eq!(
            live[0].comment_tag.as_deref(),
            Some(rows[0].comment_tag().as_str())
        );
    }

    #[tokio::test]
    async fn register_rolls_back_on_backend_failure() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend.clone(), pool);
        backend.arm_append_failure("synthetic failure").await;

        let err = mgr
            .register(Source::User, vec![wg_spec()], Default::default())
            .await
            .unwrap_err();
        assert!(matches!(err, NetctlError::Rejected(_)));

        let listed = mgr.list(Default::default()).await.unwrap();
        assert!(listed.is_empty(), "db rollback not applied: {listed:?}");
    }

    #[tokio::test]
    async fn validate_rejects_shell_metacharacters() {
        let pool = pool().await;
        let backend = Arc::new(MockBackend::new());
        let mgr = DefaultManager::new(backend, pool);

        let bad = RuleSpec::new("filter", "INPUT", "-p tcp --dport 22; rm -rf /");
        let err = mgr
            .register(Source::User, vec![bad], Default::default())
            .await
            .unwrap_err();
        assert!(matches!(err, NetctlError::Invalid(_)));
    }
}
