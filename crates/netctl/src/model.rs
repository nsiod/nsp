//! Data model for the iptables rule manager.

use serde::{Deserialize, Serialize};

/// Origin of a stored rule. `User` is writable through the API; every other
/// source is read-only and owned by an internal driver.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    User,
    WgDriver,
}

impl Source {
    pub fn as_tag(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::WgDriver => "wg-driver",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "user" => Some(Self::User),
            "wg-driver" => Some(Self::WgDriver),
            _ => None,
        }
    }
}

/// A rule as submitted to the manager: the `iptables` table/chain, the raw
/// argument string after `-A <chain>`, and a human-friendly comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSpec {
    pub table: String,
    pub chain: String,
    /// Raw spec after the chain name. The manager splits it on whitespace and
    /// refuses shell metacharacters.
    pub spec: String,
    pub comment: Option<String>,
    /// Ascending ordering within a source; lower values are applied first.
    #[serde(default)]
    pub priority: i32,
}

impl RuleSpec {
    pub fn new(
        table: impl Into<String>,
        chain: impl Into<String>,
        spec: impl Into<String>,
    ) -> Self {
        Self {
            table: table.into(),
            chain: chain.into(),
            spec: spec.into(),
            comment: None,
            priority: 0,
        }
    }

    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// A rule as persisted. `id` is a uuidv7, `comment_tag` is the
/// `nsp:<source>:<id>` marker that is always appended to the live rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRule {
    pub id: String,
    pub source: Source,
    pub priority: i32,
    pub table: String,
    pub chain: String,
    pub spec: String,
    pub comment: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl StoredRule {
    /// Canonical comment marker written into the live iptables rule.
    pub fn comment_tag(&self) -> String {
        format!("nsp:{}:{}", self.source.as_tag(), self.id)
    }
}

/// Filter applied to `IptablesManager::list`.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub source: Option<Source>,
}

/// Summary returned by `reconcile`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconcileReport {
    /// Rules that were missing from live iptables and were re-inserted.
    pub reinserted: usize,
    /// Stray `nsp:*` rules that had no DB row and were removed.
    pub pruned: usize,
    /// Rules already present with matching comment tag (no change).
    pub kept: usize,
}
