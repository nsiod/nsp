//! Pluggable backend for the iptables rule manager.
//!
//! `IptablesBackend` abstracts the syscalls / process invocations that
//! actually manipulate the kernel ruleset. `ProcessBackend` shells out to
//! `iptables` / `iptables-save`; tests and unprivileged environments can swap
//! in an in-memory mock.

use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use crate::error::{NetctlError, Result};

/// A single rule as discovered by `iptables-save`. Only rules emitted with
/// `-A` or `-I` survive — the table/chain creation lines are filtered out by
/// the parser.
#[derive(Debug, Clone)]
pub struct LiveRule {
    pub table: String,
    pub chain: String,
    /// Raw spec after the chain name, excluding the leading `-A <chain>` or
    /// `-I <chain>` directive.
    pub spec: String,
    /// Comment tag (`nsp:<source>:<uuid>`) when present, else `None`.
    pub comment_tag: Option<String>,
}

#[async_trait]
pub trait IptablesBackend: Send + Sync {
    /// `iptables -t <table> -C <chain> <spec>` — returns `Ok(())` when the
    /// rule already exists, `Err(Rejected)` when iptables reports a syntax or
    /// semantic error, and `Err(Backend)` for I/O failures.
    async fn check(&self, table: &str, chain: &str, spec: &[String]) -> Result<bool>;

    /// `iptables -t <table> -A <chain> <spec>`.
    async fn append(&self, table: &str, chain: &str, spec: &[String]) -> Result<()>;

    /// `iptables -t <table> -D <chain> <spec>`.
    async fn delete(&self, table: &str, chain: &str, spec: &[String]) -> Result<()>;

    /// Enumerate the live ruleset (all tables). Used by `reconcile`.
    async fn list_all(&self) -> Result<Vec<LiveRule>>;
}

/// Production backend: shells out to `iptables` / `iptables-save` on
/// `$PATH`. The constructor does not run anything — availability is probed
/// lazily on the first `check`/`append`/`list_all` call.
#[derive(Debug, Clone, Default)]
pub struct ProcessBackend {
    bin: String,
    save_bin: String,
}

impl ProcessBackend {
    pub fn new() -> Self {
        Self {
            bin: "iptables".to_owned(),
            save_bin: "iptables-save".to_owned(),
        }
    }

    /// Use explicit binary paths. Useful for tests and for picking between
    /// `iptables-legacy` and `iptables-nft` at runtime.
    pub fn with_paths(bin: impl Into<String>, save_bin: impl Into<String>) -> Self {
        Self {
            bin: bin.into(),
            save_bin: save_bin.into(),
        }
    }

    async fn run(
        &self,
        table: &str,
        op: &str,
        chain: &str,
        spec: &[String],
    ) -> Result<(bool, String)> {
        let mut args: Vec<String> = vec![
            "-t".to_owned(),
            table.to_owned(),
            op.to_owned(),
            chain.to_owned(),
        ];
        args.extend(spec.iter().cloned());
        let output = Command::new(&self.bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| NetctlError::Backend(format!("spawn {}: {e}", self.bin)))?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Ok((output.status.success(), stderr))
    }
}

#[async_trait]
impl IptablesBackend for ProcessBackend {
    async fn check(&self, table: &str, chain: &str, spec: &[String]) -> Result<bool> {
        let (ok, stderr) = self.run(table, "-C", chain, spec).await?;
        if ok {
            return Ok(true);
        }
        // `-C` is documented to exit non-zero when the rule is absent without
        // producing stderr output. Treat an empty stderr as "rule missing".
        if stderr.trim().is_empty() {
            return Ok(false);
        }
        Err(NetctlError::Rejected(stderr.trim().to_owned()))
    }

    async fn append(&self, table: &str, chain: &str, spec: &[String]) -> Result<()> {
        let (ok, stderr) = self.run(table, "-A", chain, spec).await?;
        if ok {
            Ok(())
        } else {
            Err(NetctlError::Rejected(stderr.trim().to_owned()))
        }
    }

    async fn delete(&self, table: &str, chain: &str, spec: &[String]) -> Result<()> {
        let (ok, stderr) = self.run(table, "-D", chain, spec).await?;
        if ok {
            Ok(())
        } else {
            // Deleting a rule that no longer exists is common during
            // reconcile; bubble up as Rejected so the caller can decide.
            Err(NetctlError::Rejected(stderr.trim().to_owned()))
        }
    }

    async fn list_all(&self) -> Result<Vec<LiveRule>> {
        let output = Command::new(&self.save_bin)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| NetctlError::Backend(format!("spawn {}: {e}", self.save_bin)))?;
        if !output.status.success() {
            return Err(NetctlError::Backend(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(parse_iptables_save(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }
}

/// Parse `iptables-save` output into a flat list of rules. Table headers
/// (`*filter`, `COMMIT`, `:CHAIN ...`) are stripped; only `-A <chain> ...` /
/// `-I <chain> ...` lines are kept.
pub(crate) fn parse_iptables_save(text: &str) -> Vec<LiveRule> {
    let mut out = Vec::new();
    let mut current_table = String::from("filter");
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line == "COMMIT" || line.starts_with(':') {
            continue;
        }
        if let Some(t) = line.strip_prefix('*') {
            current_table = t.trim().to_owned();
            continue;
        }
        let (chain, spec) = match parse_rule_line(line) {
            Some(v) => v,
            None => continue,
        };
        let comment_tag = extract_comment_tag(&spec);
        out.push(LiveRule {
            table: current_table.clone(),
            chain,
            spec,
            comment_tag,
        });
    }
    out
}

fn parse_rule_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let head = parts.next()?;
    if head != "-A" && head != "-I" {
        return None;
    }
    let chain = parts.next()?.to_owned();
    let rest: Vec<&str> = parts.collect();
    Some((chain, rest.join(" ")))
}

fn extract_comment_tag(spec: &str) -> Option<String> {
    // Look for `--comment "nsp:..."` or `--comment nsp:...`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iptables_save_extracts_rules_and_comments() {
        let sample = r#"# Generated
*nat
:PREROUTING ACCEPT [0:0]
:POSTROUTING ACCEPT [0:0]
-A POSTROUTING -s 10.66.0.0/24 -o eth0 -m comment --comment "nsp:wg-driver:01hxyz" -j MASQUERADE
COMMIT
*filter
:INPUT ACCEPT [0:0]
-A FORWARD -i wg0 -j ACCEPT
-A INPUT -p tcp --dport 22 -m comment --comment nsp:user:abc -j ACCEPT
COMMIT
"#;
        let rules = parse_iptables_save(sample);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].table, "nat");
        assert_eq!(rules[0].chain, "POSTROUTING");
        assert_eq!(
            rules[0].comment_tag.as_deref(),
            Some("nsp:wg-driver:01hxyz")
        );
        assert_eq!(rules[1].table, "filter");
        assert_eq!(rules[1].chain, "FORWARD");
        assert!(rules[1].comment_tag.is_none());
        assert_eq!(rules[2].comment_tag.as_deref(), Some("nsp:user:abc"));
    }
}
